use axum::{
    extract::{Path, Query, State},
    Json,
};
use wacore_binary::jid::{Jid, Server};
use whatsapp_rust::features::{message_key, message_range, SyncActionMessageRange};

use crate::api::common::SuccessResponse;
use crate::api::shared::messages::handlers::{chat_store, get_client, parse_jid, stored_message};
use crate::api::web::chats::models::{
    ArchiveChatRequest, ChatListQuery, ChatListResponse, ChatSummary, ClearChatRequest,
    DeleteForMeRequest, DisappearingRequest, MarkChatReadRequest, MuteChatRequest, PinChatRequest,
    StarMessageRequest, StarredMessage, StarredResponse,
};
use crate::error::ApiError;
use crate::state::AppState;

fn internal(e: impl std::fmt::Display) -> ApiError {
    ApiError::Internal(e.to_string())
}

/// The chat's newest message as an app-state message range, which lets
/// other devices resolve conflicting archive/read/delete actions. `None`
/// when the chat store has no message for the chat.
async fn last_message_range(
    state: &AppState,
    session_id: &str,
    chat: &Jid,
) -> Option<SyncActionMessageRange> {
    let store = chat_store(state, session_id)?;
    let last = store
        .messages(chat, None, 1)
        .await
        .ok()?
        .into_iter()
        .next()?;
    let ts = last.timestamp.timestamp();
    let participant =
        (!last.from_me && chat.server == Server::Group).then(|| last.sender_jid.to_non_ad());
    let key = message_key(last.id, chat, last.from_me, participant.as_ref());
    Some(message_range(ts, None, vec![(key, ts)]))
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/chats",
    tag = "chats",
    params(("session_id" = String, Path, description = "Session ID"), ChatListQuery),
    responses(
        (status = 200, description = "Chat list, pinned first, then by latest activity", body = ChatListResponse),
        (status = 503, description = "Not connected")
    )
)]
pub async fn list_chats(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(query): Query<ChatListQuery>,
) -> Result<Json<ChatListResponse>, ApiError> {
    let store = chat_store(&state, &session_id).ok_or(ApiError::NotConnected)?;
    let limit = query.limit.unwrap_or(200).clamp(1, 2000);
    let entries = store
        .chats(query.archived.unwrap_or(false), limit)
        .await
        .map_err(internal)?;
    let chats: Vec<ChatSummary> = entries
        .into_iter()
        .map(|c| ChatSummary {
            jid: c.jid.to_string(),
            name: c.name,
            last_message_at: c.last_message_at.map(|t| t.timestamp()),
            last_message_preview: c.last_message_preview,
            last_message_kind: c.last_message_kind.map(|k| k.as_str().to_string()),
            unread_count: c.unread_count,
            pinned: c.pinned_at.is_some(),
            muted_until: c.muted_until.map(|t| t.timestamp()),
            archived: c.archived,
            ephemeral_expiration: c.ephemeral_expiration,
        })
        .collect();
    Ok(Json(ChatListResponse {
        count: chats.len(),
        chats,
    }))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/chats/{jid}/archive",
    tag = "chats",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("jid" = String, Path, description = "Chat JID")
    ),
    request_body = ArchiveChatRequest,
    responses(
        (status = 200, description = "Chat archived or unarchived", body = SuccessResponse),
        (status = 503, description = "Not connected")
    )
)]
pub async fn archive_chat(
    State(state): State<AppState>,
    Path((session_id, jid)): Path<(String, String)>,
    Json(request): Json<ArchiveChatRequest>,
) -> Result<Json<SuccessResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let chat = parse_jid(&jid)?;
    let range = last_message_range(&state, &session_id, &chat).await;
    let actions = client.chat_actions();
    if request.archived {
        actions.archive_chat(&chat, range).await.map_err(internal)?;
    } else {
        actions
            .unarchive_chat(&chat, range)
            .await
            .map_err(internal)?;
    }
    Ok(Json(SuccessResponse::new()))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/chats/{jid}/pin",
    tag = "chats",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("jid" = String, Path, description = "Chat JID")
    ),
    request_body = PinChatRequest,
    responses(
        (status = 200, description = "Chat pinned or unpinned", body = SuccessResponse),
        (status = 503, description = "Not connected")
    )
)]
pub async fn pin_chat(
    State(state): State<AppState>,
    Path((session_id, jid)): Path<(String, String)>,
    Json(request): Json<PinChatRequest>,
) -> Result<Json<SuccessResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let chat = parse_jid(&jid)?;
    let actions = client.chat_actions();
    if request.pinned {
        actions.pin_chat(&chat).await.map_err(internal)?;
    } else {
        actions.unpin_chat(&chat).await.map_err(internal)?;
    }
    Ok(Json(SuccessResponse::new()))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/chats/{jid}/mute",
    tag = "chats",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("jid" = String, Path, description = "Chat JID")
    ),
    request_body = MuteChatRequest,
    responses(
        (status = 200, description = "Chat muted or unmuted", body = SuccessResponse),
        (status = 503, description = "Not connected")
    )
)]
pub async fn mute_chat(
    State(state): State<AppState>,
    Path((session_id, jid)): Path<(String, String)>,
    Json(request): Json<MuteChatRequest>,
) -> Result<Json<SuccessResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let chat = parse_jid(&jid)?;
    let actions = client.chat_actions();
    match (request.muted, request.until) {
        (false, _) => actions.unmute_chat(&chat).await.map_err(internal)?,
        (true, None) => actions.mute_chat(&chat).await.map_err(internal)?,
        (true, Some(until)) => actions
            .mute_chat_until(&chat, until * 1000)
            .await
            .map_err(internal)?,
    }
    Ok(Json(SuccessResponse::new()))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/chats/{jid}/read",
    tag = "chats",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("jid" = String, Path, description = "Chat JID")
    ),
    request_body = MarkChatReadRequest,
    responses(
        (status = 200, description = "Chat marked read or unread on every device", body = SuccessResponse),
        (status = 503, description = "Not connected")
    )
)]
pub async fn mark_chat_read(
    State(state): State<AppState>,
    Path((session_id, jid)): Path<(String, String)>,
    Json(request): Json<MarkChatReadRequest>,
) -> Result<Json<SuccessResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let chat = parse_jid(&jid)?;
    let range = last_message_range(&state, &session_id, &chat).await;
    client
        .chat_actions()
        .mark_chat_as_read(&chat, request.read, range)
        .await
        .map_err(internal)?;
    Ok(Json(SuccessResponse::new()))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/chats/{jid}/clear",
    tag = "chats",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("jid" = String, Path, description = "Chat JID")
    ),
    request_body = ClearChatRequest,
    responses(
        (status = 200, description = "Chat cleared on every device", body = SuccessResponse),
        (status = 503, description = "Not connected")
    )
)]
pub async fn clear_chat(
    State(state): State<AppState>,
    Path((session_id, jid)): Path<(String, String)>,
    Json(request): Json<ClearChatRequest>,
) -> Result<Json<SuccessResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let chat = parse_jid(&jid)?;
    let range = last_message_range(&state, &session_id, &chat).await;
    client
        .chat_actions()
        .clear_chat(&chat, !request.keep_starred, true, range)
        .await
        .map_err(internal)?;
    Ok(Json(SuccessResponse::new()))
}

#[utoipa::path(
    delete,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/chats/{jid}",
    tag = "chats",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("jid" = String, Path, description = "Chat JID")
    ),
    responses(
        (status = 200, description = "Chat deleted on every device", body = SuccessResponse),
        (status = 503, description = "Not connected")
    )
)]
pub async fn delete_chat(
    State(state): State<AppState>,
    Path((session_id, jid)): Path<(String, String)>,
) -> Result<Json<SuccessResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let chat = parse_jid(&jid)?;
    let range = last_message_range(&state, &session_id, &chat).await;
    client
        .chat_actions()
        .delete_chat(&chat, true, range)
        .await
        .map_err(internal)?;
    Ok(Json(SuccessResponse::new()))
}

#[utoipa::path(
    put,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/chats/{jid}/disappearing",
    tag = "chats",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("jid" = String, Path, description = "Chat or group JID")
    ),
    request_body = DisappearingRequest,
    responses(
        (status = 200, description = "Disappearing-message timer set", body = SuccessResponse),
        (status = 503, description = "Not connected")
    )
)]
pub async fn set_disappearing(
    State(state): State<AppState>,
    Path((session_id, jid)): Path<(String, String)>,
    Json(request): Json<DisappearingRequest>,
) -> Result<Json<SuccessResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let chat = parse_jid(&jid)?;
    if chat.server == Server::Group {
        client
            .groups()
            .set_ephemeral(chat, request.seconds)
            .await
            .map_err(internal)?;
    } else {
        client
            .set_chat_disappearing_timer(chat, request.seconds)
            .await
            .map_err(internal)?;
    }
    Ok(Json(SuccessResponse::new()))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/messages/star",
    tag = "chats",
    params(("session_id" = String, Path, description = "Session ID")),
    request_body = StarMessageRequest,
    responses(
        (status = 200, description = "Message starred or unstarred", body = SuccessResponse),
        (status = 503, description = "Not connected")
    )
)]
pub async fn star_message(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<StarMessageRequest>,
) -> Result<Json<SuccessResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let chat = parse_jid(&request.chat)?;
    let stored = stored_message(&state, &session_id, &chat, &request.message_id).await;
    let from_me = stored.as_ref().is_some_and(|m| m.from_me);
    let participant = stored
        .as_ref()
        .filter(|m| !m.from_me && chat.server == Server::Group)
        .map(|m| m.sender_jid.to_non_ad());
    let actions = client.chat_actions();
    if request.starred {
        actions
            .star_message(&chat, participant.as_ref(), &request.message_id, from_me)
            .await
            .map_err(internal)?;
    } else {
        actions
            .unstar_message(&chat, participant.as_ref(), &request.message_id, from_me)
            .await
            .map_err(internal)?;
    }
    Ok(Json(SuccessResponse::new()))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/messages/delete-for-me",
    tag = "chats",
    params(("session_id" = String, Path, description = "Session ID")),
    request_body = DeleteForMeRequest,
    responses(
        (status = 200, description = "Message deleted on this account's devices only", body = SuccessResponse),
        (status = 503, description = "Not connected")
    )
)]
pub async fn delete_for_me(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<DeleteForMeRequest>,
) -> Result<Json<SuccessResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let chat = parse_jid(&request.chat)?;
    let stored = stored_message(&state, &session_id, &chat, &request.message_id).await;
    let from_me = stored.as_ref().is_some_and(|m| m.from_me);
    let participant = stored
        .as_ref()
        .filter(|m| !m.from_me && chat.server == Server::Group)
        .map(|m| m.sender_jid.to_non_ad());
    let timestamp = stored.as_ref().map(|m| m.timestamp.timestamp());
    client
        .chat_actions()
        .delete_message_for_me(
            &chat,
            participant.as_ref(),
            &request.message_id,
            from_me,
            true,
            timestamp,
        )
        .await
        .map_err(internal)?;
    Ok(Json(SuccessResponse::new()))
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/messages/starred",
    tag = "chats",
    params(("session_id" = String, Path, description = "Session ID")),
    responses(
        (status = 200, description = "Starred messages across all chats, newest first", body = StarredResponse),
        (status = 503, description = "Not connected")
    )
)]
pub async fn list_starred(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<StarredResponse>, ApiError> {
    let store = chat_store(&state, &session_id).ok_or(ApiError::NotConnected)?;
    let messages: Vec<StarredMessage> = store
        .starred(500)
        .await
        .map_err(internal)?
        .into_iter()
        .map(|m| StarredMessage {
            chat_jid: m.chat_jid.to_string(),
            message_id: m.id,
            sender_jid: m.sender_jid.to_string(),
            from_me: m.from_me,
            timestamp: m.timestamp.timestamp(),
            kind: m.kind.as_str().to_string(),
            text: m.text,
        })
        .collect();
    Ok(Json(StarredResponse {
        count: messages.len(),
        messages,
    }))
}
