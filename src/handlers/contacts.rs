use axum::{
    extract::{Path, State},
    Json,
};
use wacore_binary::jid::{Jid, JidExt};

use crate::error::ApiError;
use crate::models::contacts::*;
use crate::state::AppState;

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/contacts/check",
    tag = "contacts",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = CheckOnWhatsAppRequest,
    responses(
        (status = 200, description = "Check results", body = CheckOnWhatsAppResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn check_on_whatsapp(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<CheckOnWhatsAppRequest>,
) -> Result<Json<CheckOnWhatsAppResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;

    let phones: Vec<&str> = request.phones.iter().map(|s| s.as_str()).collect();

    let results = client
        .contacts()
        .is_on_whatsapp(&phones)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let results = results
        .into_iter()
        .map(|r| WhatsAppCheckResult {
            phone: r.jid.user().to_string(),
            jid: Some(r.jid.to_string()),
            is_registered: r.is_registered,
        })
        .collect();

    Ok(Json(CheckOnWhatsAppResponse { results }))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/contacts/info",
    tag = "contacts",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = GetContactInfoRequest,
    responses(
        (status = 200, description = "Contact info", body = ContactInfoResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn get_contact_info(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<GetContactInfoRequest>,
) -> Result<Json<ContactInfoResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;

    let phones: Vec<&str> = request.phones.iter().map(|s| s.as_str()).collect();

    let results = client
        .contacts()
        .get_info(&phones)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let contacts = results
        .into_iter()
        .map(|info| ContactInfo {
            jid: info.jid.to_string(),
            lid: info.lid.map(|l| l.to_string()),
            is_registered: info.is_registered,
            is_business: info.is_business,
            status: info.status,
            picture_id: info.picture_id.map(|id| id.to_string()),
        })
        .collect();

    Ok(Json(ContactInfoResponse { contacts }))
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/contacts/{jid}/picture",
    tag = "contacts",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("jid" = String, Path, description = "Contact JID")
    ),
    responses(
        (status = 200, description = "Profile picture", body = ProfilePictureResponse),
        (status = 404, description = "Session or contact not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn get_profile_picture(
    State(state): State<AppState>,
    Path((session_id, jid)): Path<(String, String)>,
) -> Result<Json<ProfilePictureResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let jid: Jid = jid
        .parse()
        .map_err(|_| ApiError::InvalidJid(jid.clone()))?;

    let picture = client
        .contacts()
        .get_profile_picture(&jid, false)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    match picture {
        Some(pic) => Ok(Json(ProfilePictureResponse {
            url: Some(pic.url),
            direct_path: pic.direct_path,
            picture_id: Some(pic.id),
        })),
        None => Ok(Json(ProfilePictureResponse {
            url: None,
            direct_path: None,
            picture_id: None,
        })),
    }
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/contacts/users",
    tag = "contacts",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = GetUserInfoRequest,
    responses(
        (status = 200, description = "User info", body = UserInfoResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn get_user_info(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<GetUserInfoRequest>,
) -> Result<Json<UserInfoResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;

    let jids: Result<Vec<Jid>, _> = request
        .jids
        .iter()
        .map(|s| s.parse().map_err(|_| ApiError::InvalidJid(s.clone())))
        .collect();
    let jids = jids?;

    let results = client
        .contacts()
        .get_user_info(&jids)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let users = results
        .into_iter()
        .map(|(jid, info)| UserInfo {
            jid: jid.to_string(),
            lid: info.lid.map(|l: Jid| l.to_string()),
            status: info.status,
            is_business: info.is_business,
            picture_id: info.picture_id,
        })
        .collect();

    Ok(Json(UserInfoResponse { users }))
}

fn get_client(
    state: &AppState,
    session_id: &str,
) -> Result<std::sync::Arc<whatsapp_rust::Client>, ApiError> {
    let runtime = state
        .get_session(session_id)
        .ok_or(ApiError::NotConnected)?;

    runtime.get_client().ok_or(ApiError::NotConnected)
}
