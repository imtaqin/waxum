use axum::{
    extract::{Path, State},
    Json,
};
use std::sync::atomic::Ordering;

use crate::api::shared::messages::models::{
    SpamReportRequest as ApiSpamReportRequest, SpamReportResponse,
};
use crate::error::ApiError;
use crate::state::AppState;

// --- Spam Report ---

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct TcTokenIssueResponse {
    pub tokens: Vec<TcTokenItem>,
}

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct TcTokenItem {
    pub jid: String,
    pub timestamp: i64,
}

#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct TcTokenIssueRequest {
    /// List of JIDs to issue tokens for
    pub jids: Vec<String>,
}

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct TcTokenGetResponse {
    pub jid: String,
    pub token_timestamp: Option<i64>,
    pub sender_timestamp: Option<i64>,
    pub found: bool,
}

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct TcTokenPruneResponse {
    pub pruned_count: u32,
}

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct TcTokenListResponse {
    pub jids: Vec<String>,
}

#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct AutoReconnectRequest {
    pub enabled: bool,
}

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct AutoReconnectResponse {
    pub enabled: bool,
    pub error_count: u32,
}

#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct HistorySyncRequest {
    /// Set to true to skip history sync
    pub skip: bool,
}

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct HistorySyncResponse {
    pub skip_history_sync: bool,
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/spam/report",
    tag = "operations",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = ApiSpamReportRequest,
    responses(
        (status = 200, description = "Spam report submitted", body = SpamReportResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn spam_report(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<ApiSpamReportRequest>,
) -> Result<Json<SpamReportResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;

    let from_jid = match &request.from_jid {
        Some(jid_str) => Some(
            jid_str
                .parse()
                .map_err(|_| ApiError::InvalidJid(jid_str.clone()))?,
        ),
        None => None,
    };

    let participant_jid = match &request.participant_jid {
        Some(jid_str) => Some(
            jid_str
                .parse()
                .map_err(|_| ApiError::InvalidJid(jid_str.clone()))?,
        ),
        None => None,
    };

    let group_jid = match &request.group_jid {
        Some(jid_str) => Some(
            jid_str
                .parse()
                .map_err(|_| ApiError::InvalidJid(jid_str.clone()))?,
        ),
        None => None,
    };

    let spam_flow = match request.spam_flow.to_lowercase().as_str() {
        "group_spam_banner_report" => whatsapp_rust::SpamFlow::GroupSpamBannerReport,
        "group_info_report" => whatsapp_rust::SpamFlow::GroupInfoReport,
        "contact_info" => whatsapp_rust::SpamFlow::ContactInfo,
        "status_report" => whatsapp_rust::SpamFlow::StatusReport,
        _ => whatsapp_rust::SpamFlow::MessageMenu,
    };

    let report_request = whatsapp_rust::SpamReportRequest {
        message_id: request.message_id,
        message_timestamp: request.message_timestamp,
        from_jid,
        participant_jid,
        group_jid,
        group_subject: request.group_subject,
        spam_flow,
        raw_message: None,
        media_type: request.media_type,
        ..Default::default()
    };

    let result = client
        .send_spam_report(report_request)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(SpamReportResponse {
        success: true,
        report_id: result.report_id,
    }))
}

// --- TCToken ---

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/tctoken/issue",
    tag = "operations",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = TcTokenIssueRequest,
    responses(
        (status = 200, description = "Tokens issued", body = TcTokenIssueResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn tctoken_issue(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<TcTokenIssueRequest>,
) -> Result<Json<TcTokenIssueResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;

    let jids: Vec<wacore_binary::jid::Jid> = request
        .jids
        .iter()
        .map(|s| s.parse().map_err(|_| ApiError::InvalidJid(s.clone())))
        .collect::<Result<_, _>>()?;

    let tokens = client
        .tc_token()
        .issue_tokens(&jids)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let items = tokens
        .into_iter()
        .map(|t| TcTokenItem {
            jid: t.jid.to_string(),
            timestamp: t.timestamp,
        })
        .collect();

    Ok(Json(TcTokenIssueResponse { tokens: items }))
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/tctoken/{jid}",
    tag = "operations",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("jid" = String, Path, description = "Contact JID")
    ),
    responses(
        (status = 200, description = "Token info", body = TcTokenGetResponse),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn tctoken_get(
    State(state): State<AppState>,
    Path((session_id, jid)): Path<(String, String)>,
) -> Result<Json<TcTokenGetResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;

    let entry = client
        .tc_token()
        .get(&jid)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    match entry {
        Some(e) => Ok(Json(TcTokenGetResponse {
            jid: jid.clone(),
            token_timestamp: Some(e.token_timestamp),
            sender_timestamp: e.sender_timestamp,
            found: true,
        })),
        None => Ok(Json(TcTokenGetResponse {
            jid,
            token_timestamp: None,
            sender_timestamp: None,
            found: false,
        })),
    }
}

#[utoipa::path(
    delete,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/tctoken/expired",
    tag = "operations",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    responses(
        (status = 200, description = "Expired tokens pruned", body = TcTokenPruneResponse),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn tctoken_prune(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<TcTokenPruneResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;

    let pruned = client
        .tc_token()
        .prune_expired()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(TcTokenPruneResponse {
        pruned_count: pruned,
    }))
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/tctoken/list",
    tag = "operations",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    responses(
        (status = 200, description = "All JIDs with tokens", body = TcTokenListResponse),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn tctoken_list(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<TcTokenListResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;

    let jids = client
        .tc_token()
        .get_all_jids()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(TcTokenListResponse { jids }))
}

// --- Auto Reconnect ---

#[utoipa::path(
    put,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/reconnect",
    tag = "operations",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = AutoReconnectRequest,
    responses(
        (status = 200, description = "Auto-reconnect updated", body = AutoReconnectResponse),
        (status = 404, description = "Session not found")
    )
)]
pub async fn set_auto_reconnect(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<AutoReconnectRequest>,
) -> Result<Json<AutoReconnectResponse>, ApiError> {
    let runtime = config_runtime(&state, &session_id).await?;

    runtime.set_auto_reconnect_pref(request.enabled);

    let error_count = match runtime.get_client() {
        Some(client) => {
            client
                .enable_auto_reconnect
                .store(request.enabled, Ordering::Relaxed);
            client.stats().reconnect_errors
        }
        None => 0,
    };

    Ok(Json(AutoReconnectResponse {
        enabled: request.enabled,
        error_count,
    }))
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/reconnect",
    tag = "operations",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    responses(
        (status = 200, description = "Auto-reconnect status", body = AutoReconnectResponse),
        (status = 404, description = "Session not found")
    )
)]
pub async fn get_auto_reconnect(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<AutoReconnectResponse>, ApiError> {
    let runtime = config_runtime(&state, &session_id).await?;

    let enabled = runtime.auto_reconnect_enabled();
    let error_count = runtime
        .get_client()
        .map(|c| c.stats().reconnect_errors)
        .unwrap_or(0);

    Ok(Json(AutoReconnectResponse {
        enabled,
        error_count,
    }))
}

// --- History Sync ---

#[utoipa::path(
    put,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/history-sync",
    tag = "operations",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = HistorySyncRequest,
    responses(
        (status = 200, description = "History sync setting updated", body = HistorySyncResponse),
        (status = 404, description = "Session not found")
    )
)]
pub async fn set_history_sync(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<HistorySyncRequest>,
) -> Result<Json<HistorySyncResponse>, ApiError> {
    let runtime = config_runtime(&state, &session_id).await?;

    runtime.set_skip_history_sync_pref(request.skip);
    if let Some(client) = runtime.get_client() {
        client.set_skip_history_sync(request.skip);
    }

    Ok(Json(HistorySyncResponse {
        skip_history_sync: request.skip,
    }))
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/history-sync",
    tag = "operations",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    responses(
        (status = 200, description = "History sync setting", body = HistorySyncResponse),
        (status = 404, description = "Session not found")
    )
)]
pub async fn get_history_sync(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<HistorySyncResponse>, ApiError> {
    let runtime = config_runtime(&state, &session_id).await?;

    Ok(Json(HistorySyncResponse {
        skip_history_sync: runtime.skip_history_sync(),
    }))
}

/// Response of the pause/resume endpoints.
#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct PauseStateResponse {
    /// Whether the session is paused after the operation.
    pub paused: bool,
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/pause",
    tag = "operations",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    responses(
        (status = 200, description = "Session paused", body = PauseStateResponse),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn pause_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<PauseStateResponse>, ApiError> {
    let client = get_installed_client(&state, &session_id)?;

    client.pause().await;

    Ok(Json(PauseStateResponse {
        paused: client.is_paused(),
    }))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/resume",
    tag = "operations",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    responses(
        (status = 200, description = "Session resumed", body = PauseStateResponse),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn resume_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<PauseStateResponse>, ApiError> {
    let client = get_installed_client(&state, &session_id)?;

    client.resume();

    Ok(Json(PauseStateResponse {
        paused: client.is_paused(),
    }))
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

/// Resolve the installed client without requiring a live socket. A paused
/// session has no connection by definition, so pause/resume must not gate
/// on [`SessionState::get_live_client`] the way send-path handlers do;
/// they only need the client instance itself to be installed.
fn get_installed_client(
    state: &AppState,
    session_id: &str,
) -> Result<std::sync::Arc<whatsapp_rust::Client>, ApiError> {
    let runtime = state
        .get_session(session_id)
        .ok_or(ApiError::NotConnected)?;

    runtime.get_client().ok_or(ApiError::NotConnected)
}

/// Request body for `POST /sessions/{id}/appstate/resync`.
#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct AppStateResyncRequest {
    /// Collections to re-fetch: `critical_block`, `critical_unblock_low`,
    /// `regular_low`, `regular_high`, `regular`.
    pub collections: Vec<String>,
    /// `incremental` (default) asks for patches after the stored version;
    /// `snapshot` discards the stored state and rebuilds the collection
    /// from the server's snapshot.
    pub mode: Option<String>,
}

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct AppStateResyncResponse {
    /// Fetched, applied and persisted.
    pub synced: Vec<String>,
    /// Refused by the server outright (400/404); retrying will not clear these.
    pub fatal: Vec<String>,
    /// Not synced, but a later attempt can succeed.
    pub retryable: Vec<String>,
    /// Left to another sync already fetching the collection.
    pub skipped: Vec<String>,
    /// True when every requested collection came back synced.
    pub all_synced: bool,
}

/// Parse one collection name against upstream's [`whatsapp_rust::WAPatchName`],
/// rejecting anything that falls back to `Unknown` — that variant is a parse
/// fallback, not a collection the server has, and upstream refuses a request
/// that names it.
fn parse_patch_name(name: &str) -> Result<whatsapp_rust::WAPatchName, ApiError> {
    use std::str::FromStr;

    let parsed =
        whatsapp_rust::WAPatchName::from_str(name).unwrap_or(whatsapp_rust::WAPatchName::Unknown);
    if parsed == whatsapp_rust::WAPatchName::Unknown {
        return Err(ApiError::BadRequest(format!(
            "unknown app-state collection: {name} (expected one of: critical_block, \
             critical_unblock_low, regular_low, regular_high, regular)"
        )));
    }
    Ok(parsed)
}

fn parse_resync_mode(mode: Option<&str>) -> Result<whatsapp_rust::AppStateResyncMode, ApiError> {
    match mode {
        None | Some("incremental") => Ok(whatsapp_rust::AppStateResyncMode::Incremental),
        Some("snapshot") => Ok(whatsapp_rust::AppStateResyncMode::Snapshot),
        Some(other) => Err(ApiError::BadRequest(format!(
            "unknown resync mode: {other} (expected \"incremental\" or \"snapshot\")"
        ))),
    }
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/appstate/resync",
    tag = "operations",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = AppStateResyncRequest,
    responses(
        (status = 200, description = "App-state resync report", body = AppStateResyncResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn appstate_resync(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<AppStateResyncRequest>,
) -> Result<Json<AppStateResyncResponse>, ApiError> {
    if request.collections.is_empty() {
        return Err(ApiError::BadRequest(
            "collections must not be empty".to_string(),
        ));
    }
    let collections = request
        .collections
        .iter()
        .map(|name| parse_patch_name(name))
        .collect::<Result<Vec<_>, _>>()?;
    let mode = parse_resync_mode(request.mode.as_deref())?;

    let client = get_client(&state, &session_id)?;
    let report = client
        .resync_app_state(collections, mode)
        .await
        .map_err(|e| match e {
            whatsapp_rust::AppStateError::NotConnected => ApiError::NotConnected,
            whatsapp_rust::AppStateError::InvalidRequest(msg) => ApiError::BadRequest(msg),
            whatsapp_rust::AppStateError::Internal(err) => ApiError::Internal(err.to_string()),
            other => ApiError::Internal(other.to_string()),
        })?;

    Ok(Json(AppStateResyncResponse {
        synced: report
            .synced
            .iter()
            .map(|n| n.as_str().to_string())
            .collect(),
        fatal: report
            .fatal
            .iter()
            .map(|n| n.as_str().to_string())
            .collect(),
        retryable: report
            .retryable
            .iter()
            .map(|n| n.as_str().to_string())
            .collect(),
        skipped: report
            .skipped
            .iter()
            .map(|n| n.as_str().to_string())
            .collect(),
        all_synced: report.all_synced(),
    }))
}

/// Resolve the session's runtime for per-session config read/write
/// endpoints, creating the in-memory runtime from the DB row when the
/// session has no live client in this process. Config must be
/// serviceable while the socket is down — disabling auto-reconnect on a
/// dead session is exactly the case that needs it — so unlike
/// [`get_client`] this never returns 503 for a known session.
async fn config_runtime(
    state: &AppState,
    session_id: &str,
) -> Result<std::sync::Arc<crate::state::SessionState>, ApiError> {
    let _ = state
        .session_manager()
        .get_session(session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::SessionNotFound(session_id.to_string()))?;

    if let Some(runtime) = state.get_session(session_id) {
        return Ok(runtime);
    }

    let storage_path = state
        .session_manager()
        .get_storage_path(session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .unwrap_or_else(|| format!("{}/{}", state.base_storage_path(), session_id));

    Ok(state.get_or_create_session(session_id, &storage_path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_patch_name_accepts_every_real_collection() {
        for (name, expected) in [
            ("critical_block", whatsapp_rust::WAPatchName::CriticalBlock),
            (
                "critical_unblock_low",
                whatsapp_rust::WAPatchName::CriticalUnblockLow,
            ),
            ("regular_low", whatsapp_rust::WAPatchName::RegularLow),
            ("regular_high", whatsapp_rust::WAPatchName::RegularHigh),
            ("regular", whatsapp_rust::WAPatchName::Regular),
        ] {
            assert_eq!(parse_patch_name(name).unwrap(), expected);
        }
    }

    #[test]
    fn parse_patch_name_rejects_unknown_and_the_unknown_fallback() {
        assert!(parse_patch_name("bogus").is_err());
        assert!(parse_patch_name("").is_err());
        assert!(parse_patch_name("unknown").is_err());
        assert!(parse_patch_name("Critical_Block").is_err());
    }

    #[test]
    fn parse_resync_mode_defaults_to_incremental() {
        assert_eq!(
            parse_resync_mode(None).unwrap(),
            whatsapp_rust::AppStateResyncMode::Incremental
        );
        assert_eq!(
            parse_resync_mode(Some("incremental")).unwrap(),
            whatsapp_rust::AppStateResyncMode::Incremental
        );
        assert_eq!(
            parse_resync_mode(Some("snapshot")).unwrap(),
            whatsapp_rust::AppStateResyncMode::Snapshot
        );
        assert!(parse_resync_mode(Some("full")).is_err());
    }
}
