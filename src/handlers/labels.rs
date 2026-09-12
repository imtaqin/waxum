use axum::{
    extract::{Path, State},
    Json,
};

use crate::error::ApiError;
use crate::models::labels::*;
use crate::state::AppState;

fn get_client(
    state: &AppState,
    session_id: &str,
) -> Result<std::sync::Arc<whatsapp_rust::Client>, ApiError> {
    let runtime = state
        .get_session(session_id)
        .ok_or(ApiError::NotConnected)?;
    runtime.get_live_client().ok_or(ApiError::NotConnected)
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/labels",
    tag = "labels",
    params(("session_id" = String, Path, description = "Session ID")),
    request_body = CreateLabelRequest,
    responses((status = 200, description = "Label created"))
)]
pub async fn create_label(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(req): Json<CreateLabelRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let color: i32 = req.color_id.unwrap_or(0);
    client
        .labels()
        .create_label(&req.label_id, &req.name, color)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(
        serde_json::json!({ "success": true, "label_id": req.label_id }),
    ))
}

#[utoipa::path(
    delete,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/labels/{label_id}",
    tag = "labels",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("label_id" = String, Path, description = "Label ID"),
    ),
    responses((status = 200, description = "Label deleted"))
)]
pub async fn delete_label(
    State(state): State<AppState>,
    Path((session_id, label_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let client = get_client(&state, &session_id)?;
    client
        .labels()
        .delete_label(&label_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(serde_json::json!({ "success": true })))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/labels/{label_id}/chats/{chat_jid}",
    tag = "labels",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("label_id" = String, Path, description = "Label ID"),
        ("chat_jid" = String, Path, description = "Chat JID"),
    ),
    responses((status = 200, description = "Chat labeled"))
)]
pub async fn add_chat_label(
    State(state): State<AppState>,
    Path((session_id, label_id, chat_jid)): Path<(String, String, String)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let jid: wacore_binary::Jid = chat_jid
        .parse()
        .map_err(|e| ApiError::InvalidJid(format!("{chat_jid}: {e}")))?;
    client
        .labels()
        .add_chat_label(&label_id, &jid)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(serde_json::json!({ "success": true })))
}

#[utoipa::path(
    delete,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/labels/{label_id}/chats/{chat_jid}",
    tag = "labels",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("label_id" = String, Path, description = "Label ID"),
        ("chat_jid" = String, Path, description = "Chat JID"),
    ),
    responses((status = 200, description = "Chat label removed"))
)]
pub async fn remove_chat_label(
    State(state): State<AppState>,
    Path((session_id, label_id, chat_jid)): Path<(String, String, String)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let jid: wacore_binary::Jid = chat_jid
        .parse()
        .map_err(|e| ApiError::InvalidJid(format!("{chat_jid}: {e}")))?;
    client
        .labels()
        .remove_chat_label(&label_id, &jid)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(serde_json::json!({ "success": true })))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/labels/{label_id}/messages",
    tag = "labels",
    params(("session_id" = String, Path, description = "Session ID"), ("label_id" = String, Path, description = "Label ID")),
    request_body = MessageLabelRequest,
    responses((status = 200, description = "Message labeled"))
)]
pub async fn add_message_label(
    State(state): State<AppState>,
    Path((session_id, label_id)): Path<(String, String)>,
    Json(req): Json<MessageLabelRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let chat_jid: wacore_binary::Jid = req
        .chat_jid
        .parse()
        .map_err(|e| ApiError::InvalidJid(format!("{}: {e}", req.chat_jid)))?;
    client
        .labels()
        .add_message_label(&label_id, &chat_jid, &req.message_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(serde_json::json!({ "success": true })))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/labels/{label_id}/messages/remove",
    tag = "labels",
    params(("session_id" = String, Path, description = "Session ID"), ("label_id" = String, Path, description = "Label ID")),
    request_body = MessageLabelRequest,
    responses((status = 200, description = "Message label removed"))
)]
pub async fn remove_message_label(
    State(state): State<AppState>,
    Path((session_id, label_id)): Path<(String, String)>,
    Json(req): Json<MessageLabelRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let chat_jid: wacore_binary::Jid = req
        .chat_jid
        .parse()
        .map_err(|e| ApiError::InvalidJid(format!("{}: {e}", req.chat_jid)))?;
    client
        .labels()
        .remove_message_label(&label_id, &chat_jid, &req.message_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(serde_json::json!({ "success": true })))
}

#[utoipa::path(
    put,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/quick-replies",
    request_body = QuickReplyRequest,
    responses((status = 200, description = "Quick reply upserted"))
)]
pub async fn set_quick_reply(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(req): Json<QuickReplyRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let client = get_client(&state, &session_id)?;
    client
        .quick_replies()
        .set_quick_reply(
            &req.id,
            &req.shortcut,
            &req.message,
            req.keywords,
            req.count,
        )
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(serde_json::json!({ "success": true, "id": req.id })))
}

#[utoipa::path(
    delete,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/quick-replies/{id}",
    tag = "labels",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("id" = String, Path, description = "Quick reply ID"),
    ),
    responses((status = 200, description = "Quick reply deleted"))
)]
pub async fn delete_quick_reply(
    State(state): State<AppState>,
    Path((session_id, id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let client = get_client(&state, &session_id)?;
    client
        .quick_replies()
        .delete_quick_reply(&id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(serde_json::json!({ "success": true })))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/settings/link-previews",
    params(("session_id" = String, Path, description = "Session ID")),
    request_body = LinkPreviewsRequest,
    responses((status = 200, description = "Link previews setting updated"))
)]
pub async fn set_link_previews(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(req): Json<LinkPreviewsRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let client = get_client(&state, &session_id)?;
    client
        .app_state_settings()
        .set_link_previews_disabled(req.disabled)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(
        serde_json::json!({ "success": true, "disabled": req.disabled }),
    ))
}
