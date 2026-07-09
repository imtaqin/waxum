use axum::{
    extract::{Path, State},
    Json,
};
use std::sync::atomic::Ordering;

use crate::error::ApiError;
use crate::models::messages::{SpamReportRequest as ApiSpamReportRequest, SpamReportResponse};
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
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn set_auto_reconnect(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<AutoReconnectRequest>,
) -> Result<Json<AutoReconnectResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;

    client
        .enable_auto_reconnect
        .store(request.enabled, Ordering::Relaxed);

    let error_count = client.stats().reconnect_errors;

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
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn get_auto_reconnect(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<AutoReconnectResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;

    let enabled = client.enable_auto_reconnect.load(Ordering::Relaxed);
    let error_count = client.stats().reconnect_errors;

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
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn set_history_sync(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<HistorySyncRequest>,
) -> Result<Json<HistorySyncResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;

    client.set_skip_history_sync(request.skip);

    Ok(Json(HistorySyncResponse {
        skip_history_sync: client.skip_history_sync_enabled(),
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
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn get_history_sync(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<HistorySyncResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;

    Ok(Json(HistorySyncResponse {
        skip_history_sync: client.skip_history_sync_enabled(),
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
