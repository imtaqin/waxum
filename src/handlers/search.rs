//! Message history search: ingestion helpers + search endpoints.
//!
//! History is captured best-effort in BOTH directions:
//!
//! - [`record_incoming`] is called from the event loop in
//!   [`crate::handlers::sessions`] for every `Event::Messages` batch
//!   (also covers messages the account sends from OTHER devices —
//!   `is_from_me` maps them to direction `out`).
//! - [`record_outgoing`] is called from every `execute_*` send core in
//!   [`crate::handlers::messages`] right after the send resolves, so
//!   HTTP, scheduled, and blast sends are all indexed. Content
//!   (text/caption/type) is re-derived from the outgoing protobuf with
//!   the same extractor the webhook payload uses.
//!
//! Both helpers swallow DB errors (warn and continue): message
//! history is an index, never a reason to fail a send or drop a
//! receive. Set `MESSAGE_HISTORY_ENABLED=false` to disable ingestion
//! entirely (search then only covers rows written before).
//!
//! Endpoints:
//! - `GET /api/v1/sessions/{sid}/messages/search?q=&limit=&offset=`
//! - `GET /api/v1/messages/search?q=&session=&limit=&offset=` (fleet).

use axum::{
    extract::{Path, Query, State},
    Json,
};
use wacore_binary::jid::Jid;
use whatsapp_rust_chat_store::{ArrivalCursor, MessageKind};

use crate::db::messages::{self, MediaPointer, MessageRow, NewMessage};
use crate::error::ApiError;
use crate::models::media::MediaType;
use crate::models::search::{
    ChatMessagesQuery, MessageFleetSearchQuery, MessageHit, MessageMedia, MessageSearchQuery,
    MessageSearchResponse, SessionMessagesQuery, SessionMessagesResponse,
};
use crate::state::AppState;

/// Whether history ingestion is active. Read once from
/// `MESSAGE_HISTORY_ENABLED` (default true) and cached.
fn history_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("MESSAGE_HISTORY_ENABLED")
            .map(|v| {
                !matches!(
                    v.to_ascii_lowercase().as_str(),
                    "0" | "false" | "no" | "off"
                )
            })
            .unwrap_or(true)
    })
}

/// Index one incoming message. Never fails the caller.
pub(crate) async fn record_incoming(
    state: &AppState,
    session_id: &str,
    msg: &waproto::whatsapp::Message,
    info: &wacore::types::message::MessageInfo,
) {
    if !history_enabled() {
        return;
    }
    let (text, caption, msg_type, _) = crate::handlers::sessions::extract_message_content(msg);
    let quoted = crate::handlers::sessions::extract_quoted_context(msg);
    let row = NewMessage {
        message_id: info.id.to_string(),
        session_id: session_id.to_string(),
        chat_jid: info.source.chat.to_string(),
        sender_jid: info.source.sender.to_string(),
        direction: if info.source.is_from_me {
            "out".to_string()
        } else {
            "in".to_string()
        },
        msg_type,
        body: text.or(caption),
        msg_timestamp: info.timestamp,
        media: crate::handlers::sessions::extract_media_pointer(msg),
        quoted_message_id: quoted.as_ref().map(|(id, _)| id.clone()),
        quoted_sender_jid: quoted.and_then(|(_, participant)| participant),
    };
    if let Err(e) = messages::insert(state.session_manager().pool(), &row).await {
        tracing::warn!("message history insert (incoming) failed: {}", e);
    }
}

/// Index one outgoing message right after the send resolves. Never
/// fails the caller. `sender_jid` is stored empty — the sender is the
/// session's own account and its JID is not tracked here.
pub(crate) async fn record_outgoing(
    state: &AppState,
    session_id: &str,
    to_jid: &Jid,
    message: &waproto::whatsapp::Message,
    message_id: &str,
) {
    if !history_enabled() {
        return;
    }
    let (text, caption, msg_type, _) = crate::handlers::sessions::extract_message_content(message);
    let quoted = crate::handlers::sessions::extract_quoted_context(message);
    let row = NewMessage {
        message_id: message_id.to_string(),
        session_id: session_id.to_string(),
        chat_jid: to_jid.to_string(),
        sender_jid: String::new(),
        direction: "out".to_string(),
        msg_type,
        body: text.or(caption),
        msg_timestamp: chrono::Utc::now(),
        media: crate::handlers::sessions::extract_media_pointer(message),
        quoted_message_id: quoted.as_ref().map(|(id, _)| id.clone()),
        quoted_sender_jid: quoted.and_then(|(_, participant)| participant),
    };
    if let Err(e) = messages::insert(state.session_manager().pool(), &row).await {
        tracing::warn!("message history insert (outgoing) failed: {}", e);
    }
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/messages/search",
    tag = "messages",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        MessageSearchQuery,
    ),
    responses(
        (status = 200, description = "Matching messages, newest first", body = MessageSearchResponse),
        (status = 400, description = "Empty query"),
        (status = 404, description = "Session not found")
    )
)]
pub async fn search_session_messages(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(q): Query<MessageSearchQuery>,
) -> Result<Json<MessageSearchResponse>, ApiError> {
    if state.get_session(&session_id).is_none() {
        return Err(ApiError::SessionNotFound(session_id));
    }
    let query = q.q.trim();
    if query.is_empty() {
        return Err(ApiError::BadRequest("q must not be empty".into()));
    }
    let limit = q.limit.unwrap_or(20).clamp(1, 200);
    let offset = q.offset.unwrap_or(0).max(0);
    let rows = messages::search(
        state.session_manager().pool(),
        Some(&session_id),
        query,
        limit,
        offset,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(rows_to_response(rows)))
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/messages/search",
    tag = "messages",
    params(
        MessageFleetSearchQuery,
    ),
    responses(
        (status = 200, description = "Matching messages across sessions, newest first", body = MessageSearchResponse),
        (status = 400, description = "Empty query")
    )
)]
pub async fn search_all_messages(
    State(state): State<AppState>,
    Query(q): Query<MessageFleetSearchQuery>,
) -> Result<Json<MessageSearchResponse>, ApiError> {
    let query = q.q.trim();
    if query.is_empty() {
        return Err(ApiError::BadRequest("q must not be empty".into()));
    }
    let limit = q.limit.unwrap_or(20).clamp(1, 200);
    let offset = q.offset.unwrap_or(0).max(0);
    let rows = messages::search(
        state.session_manager().pool(),
        q.session.as_deref(),
        query,
        limit,
        offset,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(rows_to_response(rows)))
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/messages/chat/{chat_jid}",
    tag = "messages",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("chat_jid" = String, Path, description = "Chat JID (DM partner or group JID)"),
        ChatMessagesQuery,
    ),
    responses(
        (status = 200, description = "This chat's history, newest first, with sender push_name and any media download pointer", body = MessageSearchResponse),
        (status = 404, description = "Session not found")
    )
)]
pub async fn list_chat_messages(
    State(state): State<AppState>,
    Path((session_id, chat_jid)): Path<(String, String)>,
    Query(q): Query<ChatMessagesQuery>,
) -> Result<Json<MessageSearchResponse>, ApiError> {
    if state.get_session(&session_id).is_none() {
        return Err(ApiError::SessionNotFound(session_id));
    }
    let limit = q.limit.unwrap_or(20).clamp(1, 200);
    let offset = q.offset.unwrap_or(0).max(0);
    let rows = messages::list_by_chat(
        state.session_manager().pool(),
        &session_id,
        &chat_jid,
        limit,
        offset,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(rows_to_response(rows)))
}

/// Map the upstream chat store's [`MessageKind`] to the `msg_type` slugs
/// the rest of the gateway uses (`text`, `image`, `ptt`, ...).
fn message_kind_slug(kind: &MessageKind) -> String {
    match kind {
        MessageKind::Text => "text",
        MessageKind::Image => "image",
        MessageKind::Video => "video",
        MessageKind::VideoNote => "video_note",
        MessageKind::Audio => "audio",
        MessageKind::VoiceNote => "ptt",
        MessageKind::Sticker => "sticker",
        MessageKind::Document => "document",
        MessageKind::Contact => "contact",
        MessageKind::Location => "location",
        MessageKind::Poll => "poll",
        MessageKind::Event => "event",
        MessageKind::GroupInvite => "group_invite",
        MessageKind::Template => "template",
        MessageKind::TemplateReply => "template_reply",
        MessageKind::Buttons => "buttons",
        MessageKind::ButtonsResponse => "buttons_response",
        MessageKind::List => "list",
        MessageKind::ListResponse => "list_response",
        MessageKind::Interactive => "interactive",
        MessageKind::InteractiveResponse => "interactive_response",
        MessageKind::Undecryptable => "undecryptable",
        MessageKind::ViewOnce => "view_once",
        MessageKind::Hosted => "hosted",
        MessageKind::Bot => "bot",
        MessageKind::Unknown => "unknown",
        MessageKind::Other(slug) => slug.as_str(),
        _ => "unknown",
    }
    .to_string()
}

/// Reads from the vendored diesel chat-store (`vendor/whatsapp-rust-chat-store`),
/// a separate storage system from the FTS-backed `messages` table
/// [`crate::db::messages`] uses -- so `quoted_message_id`/`quoted_sender_jid`
/// on every row here are always `null`, unlike `GET /messages/chat/{chat_jid}`.
#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/messages",
    tag = "messages",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        SessionMessagesQuery,
    ),
    responses(
        (status = 200, description = "Session-wide history in store-arrival order, newest first, with cursor pagination", body = SessionMessagesResponse),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn list_session_messages(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(q): Query<SessionMessagesQuery>,
) -> Result<Json<SessionMessagesResponse>, ApiError> {
    let runtime = state
        .get_session(&session_id)
        .ok_or_else(|| ApiError::SessionNotFound(session_id.clone()))?;
    let store = runtime.get_chat_store().ok_or(ApiError::NotConnected)?;

    let limit = q.limit.unwrap_or(20).clamp(1, 200);
    let after = q.after.map(|seq| ArrivalCursor { seq });
    let page = store
        .messages_by_arrival(after, limit)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let next_cursor = if page.len() as i64 == limit {
        page.last().map(|m| m.seq)
    } else {
        None
    };

    let mut push_names: std::collections::HashMap<String, Option<String>> =
        std::collections::HashMap::new();
    let mut messages = Vec::with_capacity(page.len());
    for m in &page {
        let sender = m.sender_jid.to_string();
        let push_name = if m.from_me {
            None
        } else {
            match push_names.get(&sender) {
                Some(cached) => cached.clone(),
                None => {
                    let name = store
                        .contact(&m.sender_jid)
                        .await
                        .ok()
                        .flatten()
                        .and_then(|c| c.display_name().map(|s| s.to_string()));
                    push_names.insert(sender.clone(), name.clone());
                    name
                }
            }
        };
        messages.push(MessageHit {
            id: m.seq,
            message_id: m.id.clone(),
            session_id: session_id.clone(),
            chat_jid: m.chat_jid.to_string(),
            sender_jid: sender,
            direction: if m.from_me { "out" } else { "in" }.to_string(),
            msg_type: message_kind_slug(&m.kind),
            body: m.text.clone(),
            snippet: None,
            msg_timestamp: m.timestamp.format("%Y-%m-%d %H:%M:%S").to_string(),
            push_name,
            media: m
                .message
                .as_deref()
                .and_then(crate::handlers::sessions::extract_media_pointer)
                .and_then(|p| media_pointer_to_model(&p)),
            quoted_message_id: None,
            quoted_sender_jid: None,
        });
    }

    Ok(Json(SessionMessagesResponse {
        count: messages.len(),
        messages,
        next_cursor,
    }))
}

fn media_pointer_to_model(m: &MediaPointer) -> Option<MessageMedia> {
    let media_type = match m.media_type.as_str() {
        "image" => MediaType::Image,
        "video" => MediaType::Video,
        "audio" => MediaType::Audio,
        "document" => MediaType::Document,
        "sticker" => MediaType::Sticker,
        _ => return None,
    };
    Some(MessageMedia {
        direct_path: m.direct_path.clone(),
        media_key: m.media_key.clone(),
        file_sha256: m.file_sha256.clone(),
        file_enc_sha256: m.file_enc_sha256.clone(),
        file_length: m.file_length.max(0) as u64,
        media_type,
        mimetype: m.mimetype.clone(),
    })
}

fn rows_to_response(rows: Vec<MessageRow>) -> MessageSearchResponse {
    let messages: Vec<MessageHit> = rows
        .iter()
        .map(|r| MessageHit {
            id: r.id,
            message_id: r.message_id.clone(),
            session_id: r.session_id.clone(),
            chat_jid: r.chat_jid.clone(),
            sender_jid: r.sender_jid.clone(),
            direction: r.direction.clone(),
            msg_type: r.msg_type.clone(),
            body: r.body.clone(),
            snippet: r.snippet.clone(),
            msg_timestamp: r.msg_timestamp.clone(),
            push_name: r.push_name.clone(),
            media: r.media.as_ref().and_then(media_pointer_to_model),
            quoted_message_id: r.quoted_message_id.clone(),
            quoted_sender_jid: r.quoted_sender_jid.clone(),
        })
        .collect();
    MessageSearchResponse {
        count: messages.len(),
        messages,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_kind_slug_matches_gateway_slugs() {
        assert_eq!(message_kind_slug(&MessageKind::Text), "text");
        assert_eq!(message_kind_slug(&MessageKind::Image), "image");
        assert_eq!(message_kind_slug(&MessageKind::VoiceNote), "ptt");
        assert_eq!(message_kind_slug(&MessageKind::Document), "document");
        assert_eq!(message_kind_slug(&MessageKind::Sticker), "sticker");
        assert_eq!(
            message_kind_slug(&MessageKind::Other("protocol".to_string())),
            "protocol"
        );
        assert_eq!(message_kind_slug(&MessageKind::Unknown), "unknown");
    }
}
