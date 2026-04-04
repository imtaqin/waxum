use axum::{
    extract::{Path, State},
    Json,
};
use wacore_binary::jid::Jid;

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

    let jids: Vec<Jid> = request.phones.iter().map(Jid::pn).collect();

    let results = do_is_on_whatsapp(client, jids).await?;

    let results = results
        .into_iter()
        .map(|r| WhatsAppCheckResult {
            phone: r.jid.user.clone(),
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

    let jids: Result<Vec<Jid>, _> = request
        .phones
        .iter()
        .map(|s| {
            if s.contains('@') {
                s.parse().map_err(|_| ApiError::InvalidJid(s.clone()))
            } else {
                Ok(Jid::pn(s))
            }
        })
        .collect();
    let jids = jids?;

    let results = do_get_user_info(client, jids).await?;

    let contacts = results
        .into_values()
        .map(|info| ContactInfo {
            jid: info.jid.to_string(),
            lid: info.lid.map(|l| l.to_string()),
            is_registered: true,
            is_business: info.is_business,
            status: info.status,
            picture_id: info.picture_id,
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
    let jid: Jid = jid.parse().map_err(|_| ApiError::InvalidJid(jid.clone()))?;

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

    let results = do_get_user_info(client, jids).await?;

    let users = results
        .into_values()
        .map(|info| UserInfo {
            jid: info.jid.to_string(),
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

/// Helper wrapper to work around a higher-ranked lifetime issue in the
/// whatsapp-rust library's `persist_lid_mappings` closure on nightly-2026-01-30.
/// The future produced by `is_on_whatsapp` / `get_user_info` IS Send in practice
/// (all captured data is Send), but the compiler cannot prove it due to a
/// for-any-lifetime `FnOnce` bound mismatch inside the library.
struct AssertSend<F>(F);
unsafe impl<F: std::future::Future> Send for AssertSend<F> {}
impl<F: std::future::Future> std::future::Future for AssertSend<F> {
    type Output = F::Output;
    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        // SAFETY: we only project through to the inner future
        unsafe { self.map_unchecked_mut(|s| &mut s.0) }.poll(cx)
    }
}

async fn do_is_on_whatsapp(
    client: std::sync::Arc<whatsapp_rust::Client>,
    jids: Vec<Jid>,
) -> Result<Vec<whatsapp_rust::IsOnWhatsAppResult>, ApiError> {
    AssertSend(async move {
        client
            .contacts()
            .is_on_whatsapp(&jids)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))
    })
    .await
}

async fn do_get_user_info(
    client: std::sync::Arc<whatsapp_rust::Client>,
    jids: Vec<Jid>,
) -> Result<std::collections::HashMap<Jid, whatsapp_rust::UserInfo>, ApiError> {
    AssertSend(async move {
        client
            .contacts()
            .get_user_info(&jids)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))
    })
    .await
}
