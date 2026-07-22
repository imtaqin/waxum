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

use crate::db::messages::{self, MessageRow, NewMessage};
use crate::error::ApiError;
use crate::models::search::{
    MessageFleetSearchQuery, MessageHit, MessageSearchQuery, MessageSearchResponse,
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
    let row = NewMessage {
        message_id: message_id.to_string(),
        session_id: session_id.to_string(),
        chat_jid: to_jid.to_string(),
        sender_jid: String::new(),
        direction: "out".to_string(),
        msg_type,
        body: text.or(caption),
        msg_timestamp: chrono::Utc::now(),
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
        })
        .collect();
    MessageSearchResponse {
        count: messages.len(),
        messages,
    }
}
