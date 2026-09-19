use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::Deserialize;
use wacore_binary::jid::Jid;

use crate::db::contacts::ContactStore;
use crate::error::ApiError;
use crate::models::contacts::*;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct ListContactsQuery {
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: u32,
    #[serde(default)]
    pub offset: u32,
}

fn default_limit() -> u32 {
    100
}

/// Paginated dump of locally-cached contacts for a session. Contacts are
/// upserted automatically from appstate sync mutations, push-name updates,
/// contact-notification stanzas, and inbound messages — no separate sync
/// call required.
#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/contacts",
    tag = "contacts",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("q" = Option<String>, Query, description = "Search filter (name/phone)"),
        ("limit" = Option<u32>, Query, description = "Page size (1-1000, default 100)"),
        ("offset" = Option<u32>, Query, description = "Page offset (default 0)")
    ),
    responses(
        (status = 200, description = "Contact list", body = StoredContactListResponse),
        (status = 404, description = "Session not found")
    )
)]
pub async fn list_contacts(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(query): Query<ListContactsQuery>,
) -> Result<Json<StoredContactListResponse>, ApiError> {
    let store = ContactStore::new(state.session_manager().pool());
    let limit = query.limit.clamp(1, 1000);
    let rows = store
        .list(&session_id, query.q.as_deref(), limit, query.offset)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let total = store
        .count(&session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(StoredContactListResponse {
        contacts: rows
            .into_iter()
            .map(|r| StoredContact {
                jid: r.jid,
                phone: r.phone,
                lid_jid: r.lid_jid,
                full_name: r.full_name,
                first_name: r.first_name,
                push_name: r.push_name,
                business_name: r.business_name,
                source: r.source,
                updated_at: r.updated_at,
            })
            .collect(),
        total,
        limit,
        offset: query.offset,
    }))
}

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
            phone: r.jid.user.to_string(),
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

    runtime.get_live_client().ok_or(ApiError::NotConnected)
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

/// Resolve a LID (`@lid`) JID to its phone number, or a phone-number JID to
/// its LID, via `Client::get_lid_pn_entry`. Answers from the in-memory
/// cache/DB mapping learned from usync, peer messages, pairing, etc. — no
/// live network round-trip, so this can return `None` for a pair the
/// session hasn't observed yet even while connected.
#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/contacts/{jid}/lid",
    tag = "contacts",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("jid" = String, Path, description = "A `@lid` JID or a phone-number JID (with or without `@s.whatsapp.net`)")
    ),
    responses(
        (status = 200, description = "Mapping found", body = LidPnEntryResponse),
        (status = 404, description = "No mapping known for this JID"),
        (status = 503, description = "Session not connected")
    )
)]
pub async fn resolve_lid(
    State(state): State<AppState>,
    Path((session_id, jid)): Path<(String, String)>,
) -> Result<Json<LidPnEntryResponse>, ApiError> {
    let client = crate::handlers::messages::get_client(&state, &session_id)?;
    let target = crate::handlers::messages::parse_jid(&jid)?;

    let entry = AssertSend(async move {
        client
            .get_lid_pn_entry(&target)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))
    })
    .await?
    .ok_or_else(|| ApiError::ContactNotFound(jid.clone()))?;

    Ok(Json(LidPnEntryResponse {
        lid: entry.lid.to_string(),
        phone_number: entry.phone_number.to_string(),
        created_at: entry.created_at,
        learning_source: entry.learning_source.to_string(),
    }))
}

/// Save or rename a contact in the session address book via
/// `chat_actions().save_contact`, which syncs the name to the account's other
/// linked devices. The path JID must be a bare phone-number JID; LIDs, groups
/// and device-specific JIDs are rejected upstream and surface as 400.
#[utoipa::path(
    put,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/contacts/{jid}",
    tag = "contacts",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("jid" = String, Path, description = "Phone-number JID (with or without `@s.whatsapp.net`)")
    ),
    request_body = SaveContactRequest,
    responses(
        (status = 200, description = "Contact saved", body = crate::models::common::SuccessResponse),
        (status = 400, description = "Invalid JID, missing name, or non phone-number JID"),
        (status = 503, description = "Session not connected")
    )
)]
pub async fn save_contact(
    State(state): State<AppState>,
    Path((session_id, jid)): Path<(String, String)>,
    Json(request): Json<SaveContactRequest>,
) -> Result<Json<crate::models::common::SuccessResponse>, ApiError> {
    if request.full_name.is_none() && request.first_name.is_none() {
        return Err(ApiError::BadRequest(
            "at least one of full_name or first_name is required".into(),
        ));
    }
    let client = crate::handlers::messages::get_client(&state, &session_id)?;
    let target = crate::handlers::messages::parse_jid(&jid)?;

    AssertSend(async move {
        client
            .chat_actions()
            .save_contact(
                &target,
                request.full_name,
                request.first_name,
                request.save_on_primary_addressbook,
            )
            .await
            .map_err(|e| match e {
                whatsapp_rust::features::AppStateError::InvalidRequest(m) => {
                    ApiError::BadRequest(m)
                }
                other => ApiError::Internal(other.to_string()),
            })
    })
    .await?;

    Ok(Json(crate::models::common::SuccessResponse::new()))
}
