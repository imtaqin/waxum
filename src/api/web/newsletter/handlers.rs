use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::Deserialize;
use wacore_binary::Jid;

use crate::api::web::newsletter::models::*;
use crate::error::ApiError;
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

fn parse_jid(s: &str) -> Result<Jid, ApiError> {
    s.parse()
        .map_err(|e| ApiError::InvalidJid(format!("{s}: {e}")))
}

fn to_debug_json<T: std::fmt::Debug>(v: &T) -> serde_json::Value {
    serde_json::json!(format!("{:?}", v))
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/newsletters/subscribed",
    tag = "newsletter",
    params(("session_id" = String, Path, description = "Session ID")),
    responses((status = 200, description = "Subscribed newsletters", body = NewsletterListResponse))
)]
pub async fn list_subscribed(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let list = client
        .newsletter()
        .list_subscribed()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(
        serde_json::json!({ "newsletters": to_debug_json(&list) }),
    ))
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/newsletters/{jid}/metadata",
    tag = "newsletter",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("jid" = String, Path, description = "Newsletter JID"),
    ),
    responses((status = 200, description = "Newsletter metadata", body = NewsletterMetadataResponse))
)]
pub async fn get_metadata(
    State(state): State<AppState>,
    Path((session_id, jid)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let jid = parse_jid(&jid)?;
    let meta = client
        .newsletter()
        .get_metadata(&jid)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(
        serde_json::json!({ "metadata": to_debug_json(&meta) }),
    ))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/newsletters",
    tag = "newsletter",
    params(("session_id" = String, Path, description = "Session ID")),
    request_body = CreateNewsletterRequest,
    responses((status = 200, description = "Newsletter created", body = NewsletterMetadataResponse))
)]
pub async fn create_newsletter(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(req): Json<CreateNewsletterRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let desc = req.description.as_deref();
    let meta = client
        .newsletter()
        .create(&req.name, desc)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(
        serde_json::json!({ "metadata": to_debug_json(&meta) }),
    ))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/newsletters/{jid}/join",
    tag = "newsletter",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("jid" = String, Path, description = "Newsletter JID"),
    ),
    responses((status = 200, description = "Joined"))
)]
pub async fn join_newsletter(
    State(state): State<AppState>,
    Path((session_id, jid)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let jid = parse_jid(&jid)?;
    let meta = client
        .newsletter()
        .join(&jid)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(
        serde_json::json!({ "metadata": to_debug_json(&meta) }),
    ))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/newsletters/{jid}/leave",
    tag = "newsletter",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("jid" = String, Path, description = "Newsletter JID"),
    ),
    responses((status = 200, description = "Left"))
)]
pub async fn leave_newsletter(
    State(state): State<AppState>,
    Path((session_id, jid)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let jid = parse_jid(&jid)?;
    client
        .newsletter()
        .leave(&jid)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(serde_json::json!({ "success": true })))
}

#[utoipa::path(
    delete,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/newsletters/{jid}",
    tag = "newsletter",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("jid" = String, Path, description = "Newsletter JID"),
    ),
    responses((status = 200, description = "Deleted"))
)]
pub async fn delete_newsletter(
    State(state): State<AppState>,
    Path((session_id, jid)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let jid = parse_jid(&jid)?;
    client
        .newsletter()
        .delete(&jid)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(serde_json::json!({ "success": true })))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/newsletters/{jid}/change-owner",
    tag = "newsletter",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("jid" = String, Path, description = "Newsletter JID"),
    ),
    request_body = ChangeOwnerRequest,
    responses((status = 200, description = "Owner changed"))
)]
pub async fn change_owner(
    State(state): State<AppState>,
    Path((session_id, jid)): Path<(String, String)>,
    Json(req): Json<ChangeOwnerRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let jid = parse_jid(&jid)?;
    let user = parse_jid(&req.user)?;
    client
        .newsletter()
        .change_owner(&jid, &user)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(serde_json::json!({ "success": true })))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/newsletters/{jid}/demote",
    tag = "newsletter",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("jid" = String, Path, description = "Newsletter JID"),
    ),
    request_body = ChangeOwnerRequest,
    responses((status = 200, description = "Demoted"))
)]
pub async fn demote_admin(
    State(state): State<AppState>,
    Path((session_id, jid)): Path<(String, String)>,
    Json(req): Json<ChangeOwnerRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let jid = parse_jid(&jid)?;
    let user = parse_jid(&req.user)?;
    client
        .newsletter()
        .demote_admin(&jid, &user)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(serde_json::json!({ "success": true })))
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/newsletters/{jid}/admin-info",
    tag = "newsletter",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("jid" = String, Path, description = "Newsletter JID"),
    ),
    responses((status = 200, description = "Admin info", body = NewsletterAdminInfoResponse))
)]
pub async fn get_admin_info(
    State(state): State<AppState>,
    Path((session_id, jid)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let jid = parse_jid(&jid)?;
    let info = client
        .newsletter()
        .get_admin_info(&jid)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(
        serde_json::json!({ "admin_info": to_debug_json(&info) }),
    ))
}

#[derive(Deserialize)]
pub struct FollowersParams {
    pub limit: Option<u32>,
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/newsletters/{jid}/followers",
    tag = "newsletter",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("jid" = String, Path, description = "Newsletter JID"),
    ),
    responses((status = 200, description = "Followers", body = NewsletterFollowersResponse))
)]
pub async fn get_followers(
    State(state): State<AppState>,
    Path((session_id, jid)): Path<(String, String)>,
    Query(params): Query<FollowersParams>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let jid = parse_jid(&jid)?;
    let count = params.limit.unwrap_or(50);
    let followers = client
        .newsletter()
        .get_followers(&jid, count)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(
        serde_json::json!({ "followers": to_debug_json(&followers) }),
    ))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/newsletters/{jid}/mute",
    tag = "newsletter",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("jid" = String, Path, description = "Newsletter JID"),
    ),
    request_body = MuteRequest,
    responses((status = 200, description = "Mute toggled"))
)]
pub async fn set_mute(
    State(state): State<AppState>,
    Path((session_id, jid)): Path<(String, String)>,
    Json(req): Json<MuteRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let jid = parse_jid(&jid)?;
    client
        .newsletter()
        .set_follower_mute(&jid, req.muted)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(
        serde_json::json!({ "success": true, "muted": req.muted }),
    ))
}
