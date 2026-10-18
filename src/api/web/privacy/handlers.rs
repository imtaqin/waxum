use axum::{
    extract::{Path, State},
    Json,
};

use crate::api::web::privacy::models::*;
use crate::error::ApiError;
use crate::state::AppState;

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/privacy/settings",
    tag = "privacy",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    responses(
        (status = 200, description = "Privacy settings", body = PrivacySettingsResponse),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn get_privacy_settings(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<PrivacySettingsResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;

    let result = client
        .fetch_privacy_settings()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let settings = result
        .settings
        .into_iter()
        .map(|s| PrivacySettingItem {
            category: format!("{:?}", s.category),
            value: format!("{:?}", s.value),
        })
        .collect();

    Ok(Json(PrivacySettingsResponse { settings }))
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

fn parse_category(raw: &str) -> Option<wacore::iq::privacy::PrivacyCategory> {
    use wacore::iq::privacy::PrivacyCategory as C;
    Some(
        match raw.to_ascii_lowercase().replace(['_', '-'], "").as_str() {
            "last" | "lastseen" => C::Last,
            "online" => C::Online,
            "profile" | "profilephoto" => C::Profile,
            "status" | "about" => C::Status,
            "groupadd" | "groups" => C::GroupAdd,
            "readreceipts" => C::ReadReceipts,
            "calladd" | "calls" => C::CallAdd,
            "messages" => C::Messages,
            _ => return None,
        },
    )
}

fn parse_value(raw: &str) -> Option<wacore::iq::privacy::PrivacyValue> {
    use wacore::iq::privacy::PrivacyValue as V;
    Some(match raw.to_ascii_lowercase().replace('-', "_").as_str() {
        "all" | "everyone" => V::All,
        "contacts" => V::Contacts,
        "none" | "nobody" => V::None,
        "contact_blacklist" | "contacts_except" => V::ContactBlacklist,
        "match_last_seen" => V::MatchLastSeen,
        "known" => V::Known,
        _ => return None,
    })
}

#[utoipa::path(
    put,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/privacy/settings",
    tag = "privacy",
    params(("session_id" = String, Path, description = "Session ID")),
    request_body = SetPrivacyRequest,
    responses(
        (status = 200, description = "Privacy setting changed", body = crate::api::common::SuccessResponse),
        (status = 400, description = "Unknown category or a value the category does not allow"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn set_privacy_setting(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SetPrivacyRequest>,
) -> Result<Json<crate::api::common::SuccessResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let category = parse_category(&request.category).ok_or_else(|| {
        ApiError::BadRequest(format!("unknown privacy category {}", request.category))
    })?;
    let value = parse_value(&request.value)
        .filter(|v| category.is_valid_value(v))
        .ok_or_else(|| {
            ApiError::BadRequest(format!(
                "{} is not a valid value for {}",
                request.value, request.category
            ))
        })?;
    client
        .set_privacy_setting(category, value)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(crate::api::common::SuccessResponse::new()))
}

#[utoipa::path(
    put,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/privacy/disappearing",
    tag = "privacy",
    params(("session_id" = String, Path, description = "Session ID")),
    request_body = DefaultDisappearingRequest,
    responses(
        (status = 200, description = "Default timer for new chats changed", body = crate::api::common::SuccessResponse),
        (status = 503, description = "Not connected")
    )
)]
pub async fn set_default_disappearing(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<DefaultDisappearingRequest>,
) -> Result<Json<crate::api::common::SuccessResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    client
        .set_default_disappearing_mode(request.seconds)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(crate::api::common::SuccessResponse::new()))
}
