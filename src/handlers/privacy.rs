use axum::{
    extract::{Path, State},
    Json,
};

use crate::error::ApiError;
use crate::models::privacy::*;
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

    runtime.get_client().ok_or(ApiError::NotConnected)
}
