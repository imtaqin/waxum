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

/// A channel's metadata as plain JSON fields.
fn newsletter_json(m: &whatsapp_rust::features::NewsletterMetadata) -> serde_json::Value {
    use whatsapp_rust::features::{NewsletterRole, NewsletterVerification};
    let role = m.role.as_ref().map(|r| match r {
        NewsletterRole::Owner => "owner",
        NewsletterRole::Admin => "admin",
        NewsletterRole::Subscriber => "subscriber",
        NewsletterRole::Guest => "guest",
        _ => "subscriber",
    });
    serde_json::json!({
        "jid": m.jid.to_string(),
        "name": m.name,
        "description": m.description,
        "subscribers_count": m.subscriber_count,
        "verified": matches!(m.verification, NewsletterVerification::Verified),
        "picture_url": m.picture_url,
        "preview_url": m.preview_url,
        "invite_code": m.invite_code,
        "role": role,
        "muted": m.muted.unwrap_or(false),
        "created_at": m.creation_time,
    })
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
        serde_json::json!({ "newsletters": list.iter().map(newsletter_json).collect::<Vec<_>>() }),
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
        serde_json::json!({ "metadata": newsletter_json(&meta) }),
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
        serde_json::json!({ "metadata": newsletter_json(&meta) }),
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
        serde_json::json!({ "metadata": newsletter_json(&meta) }),
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

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/newsletters/{jid}/messages",
    tag = "newsletter",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("jid" = String, Path, description = "Newsletter JID"),
        NewsletterMessagesQuery
    ),
    responses(
        (status = 200, description = "Channel posts, oldest first", body = NewsletterMessagesResponse),
        (status = 503, description = "Not connected")
    )
)]
pub async fn list_messages(
    State(state): State<AppState>,
    Path((session_id, jid)): Path<(String, String)>,
    axum::extract::Query(query): axum::extract::Query<NewsletterMessagesQuery>,
) -> Result<Json<NewsletterMessagesResponse>, ApiError> {
    use base64::Engine as _;
    use wacore::proto_helpers::MessageExt;
    let client = get_client(&state, &session_id)?;
    let jid: Jid = jid.parse().map_err(|_| ApiError::InvalidJid(jid.clone()))?;
    let count = query.count.unwrap_or(50).clamp(1, 100);
    let messages = client
        .newsletter()
        .get_messages(jid, count, query.before)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let messages = messages
        .into_iter()
        .map(|m| {
            let text = m.message.as_ref().and_then(|msg| {
                let base = msg.get_base_message();
                base.conversation
                    .clone()
                    .or_else(|| {
                        base.extended_text_message
                            .as_option()
                            .and_then(|e| e.text.clone())
                    })
                    .or_else(|| {
                        base.image_message
                            .as_option()
                            .and_then(|i| i.caption.clone())
                    })
                    .or_else(|| {
                        base.video_message
                            .as_option()
                            .and_then(|v| v.caption.clone())
                    })
            });
            NewsletterMessageItem {
                message_id: m.message_id.map(|id| id.to_string()).unwrap_or_default(),
                server_id: m.server_id,
                timestamp: m.timestamp,
                text,
                views: m.views_count,
                reactions: m
                    .reactions
                    .into_iter()
                    .map(|r| NewsletterReaction {
                        emoji: r.code,
                        count: r.count,
                    })
                    .collect(),
                message_proto: m.message.as_ref().map(|msg| {
                    base64::engine::general_purpose::STANDARD
                        .encode(waproto::codec::message_to_vec(msg))
                }),
            }
        })
        .collect();
    Ok(Json(NewsletterMessagesResponse { messages }))
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/newsletters/invite/{code}",
    tag = "newsletter",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("code" = String, Path, description = "Invite code from a whatsapp.com/channel/ link")
    ),
    responses((status = 200, description = "Channel the link leads to", body = NewsletterMetadataResponse))
)]
pub async fn get_metadata_by_invite(
    State(state): State<AppState>,
    Path((session_id, code)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let code = code.rsplit('/').next().unwrap_or(&code).to_string();
    let meta = client
        .newsletter()
        .get_metadata_by_invite(&code)
        .await
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    Ok(Json(
        serde_json::json!({ "metadata": newsletter_json(&meta) }),
    ))
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct NewsletterReactRequest {
    /// Emoji to react with; empty removes the reaction.
    pub emoji: String,
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/newsletters/{jid}/messages/{server_id}/react",
    tag = "newsletter",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("jid" = String, Path, description = "Newsletter JID"),
        ("server_id" = u64, Path, description = "Post server id")
    ),
    request_body = NewsletterReactRequest,
    responses((status = 200, description = "Reaction sent", body = crate::api::common::SuccessResponse))
)]
pub async fn react_to_post(
    State(state): State<AppState>,
    Path((session_id, jid, server_id)): Path<(String, String, u64)>,
    Json(request): Json<NewsletterReactRequest>,
) -> Result<Json<crate::api::common::SuccessResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let jid: Jid = jid.parse().map_err(|_| ApiError::InvalidJid(jid.clone()))?;
    client
        .newsletter()
        .send_reaction_raw(&jid, server_id, &request.emoji)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(crate::api::common::SuccessResponse::new()))
}
