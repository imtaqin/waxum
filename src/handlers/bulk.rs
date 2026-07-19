use axum::{
    extract::{Query, State},
    Json,
};
use once_cell::sync::Lazy;
use std::time::Instant;

use crate::error::ApiError;
use crate::models::bulk::{
    DisconnectAllResponse, FleetStats, PurgeQuery, PurgeResponse, ReconnectAllResponse,
    ReenableCircuitsResponse, SearchHit, SearchQuery, SearchResponse,
};
use crate::models::sessions::SessionStatus;
use crate::state::AppState;

static UPTIME_ANCHOR: Lazy<Instant> = Lazy::new(Instant::now);

pub fn touch_uptime_anchor() {
    Lazy::force(&UPTIME_ANCHOR);
}

#[utoipa::path(
    post,
    path = "/api/v1/sessions/purge",
    tag = "bulk",
    security(("bearer_auth" = [])),
    params(
        ("filter" = Option<String>, Query, description = "`inactive` (default), `logged_out`, `disconnected`, `all`"),
        ("days" = Option<i64>, Query, description = "Minimum idle days for `inactive`. Default 30"),
        ("dry_run" = Option<bool>, Query, description = "Preview without deleting")
    ),
    responses((status = 200, body = PurgeResponse))
)]
pub async fn purge_sessions(
    State(state): State<AppState>,
    Query(q): Query<PurgeQuery>,
) -> Result<Json<PurgeResponse>, ApiError> {
    let sessions = state
        .session_manager()
        .list_sessions()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let total_before = sessions.len();
    let now = chrono::Utc::now().timestamp();
    let cutoff = now - q.days.max(0) * 86_400;

    let mut targets: Vec<String> = Vec::new();
    for s in &sessions {
        let hit = match q.filter.as_str() {
            "logged_out" => !s.is_logged_in,
            "disconnected" => s.status == SessionStatus::Disconnected,
            "all" => true,
            _ => {
                let idle = s
                    .last_connected_at
                    .unwrap_or(s.updated_at)
                    .min(s.updated_at);
                !s.is_logged_in && idle < cutoff
            }
        };
        if hit {
            targets.push(s.id.clone());
        }
    }

    let kept = total_before.saturating_sub(targets.len());
    if q.dry_run {
        return Ok(Json(PurgeResponse {
            filter: q.filter,
            days: q.days,
            dry_run: true,
            purged: targets,
            kept,
            total_before,
        }));
    }

    let mut purged: Vec<String> = Vec::with_capacity(targets.len());
    for id in targets {
        let storage_path = state
            .session_manager()
            .get_storage_path(&id)
            .await
            .ok()
            .flatten();
        if let Some(runtime) = state.get_session(&id) {
            if let Some(client) = runtime.get_client() {
                client.disconnect().await;
            }
        }
        state.remove_session(&id);
        state.purge_webhooks_for_session(&id);
        if let Some(path) = storage_path {
            let _ = tokio::fs::remove_dir_all(&path).await;
        }
        if state.session_manager().delete_session(&id).await.is_ok() {
            purged.push(id);
        }
    }

    Ok(Json(PurgeResponse {
        filter: q.filter,
        days: q.days,
        dry_run: false,
        purged,
        kept,
        total_before,
    }))
}

#[utoipa::path(
    post,
    path = "/api/v1/sessions/disconnect-all",
    tag = "bulk",
    security(("bearer_auth" = [])),
    responses((status = 200, body = DisconnectAllResponse))
)]
pub async fn disconnect_all(
    State(state): State<AppState>,
) -> Result<Json<DisconnectAllResponse>, ApiError> {
    let sessions = state
        .session_manager()
        .list_sessions()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let mut disconnected = Vec::new();
    let mut skipped = Vec::new();
    let total = sessions.len();

    for s in sessions {
        let runtime = state.get_session(&s.id);
        match runtime.and_then(|r| r.get_client()) {
            Some(client) => {
                client.disconnect().await;
                if let Some(runtime) = state.get_session(&s.id) {
                    runtime.set_status(SessionStatus::Disconnected);
                    runtime.set_client(None);
                }
                let _ = state
                    .session_manager()
                    .update_session_status(&s.id, SessionStatus::Disconnected, s.is_logged_in)
                    .await;
                disconnected.push(s.id);
            }
            None => skipped.push(s.id),
        }
    }

    Ok(Json(DisconnectAllResponse {
        disconnected,
        skipped,
        total,
    }))
}

#[utoipa::path(
    post,
    path = "/api/v1/sessions/reconnect-all",
    tag = "bulk",
    security(("bearer_auth" = [])),
    responses((status = 200, body = ReconnectAllResponse))
)]
pub async fn reconnect_all(
    State(state): State<AppState>,
) -> Result<Json<ReconnectAllResponse>, ApiError> {
    let sessions = state
        .session_manager()
        .list_sessions()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let mut scheduled = Vec::new();
    let mut skipped = Vec::new();
    let total = sessions.len();

    for s in sessions {
        if !s.is_logged_in {
            skipped.push(s.id);
            continue;
        }
        scheduled.push(s.id.clone());
    }

    let state_clone = state.clone();
    tokio::spawn(async move {
        crate::handlers::sessions::reconnect_all_on_startup(state_clone).await;
    });

    Ok(Json(ReconnectAllResponse {
        scheduled,
        skipped,
        total,
    }))
}

#[utoipa::path(
    get,
    path = "/api/v1/sessions/search",
    tag = "bulk",
    security(("bearer_auth" = [])),
    params(("q" = String, Query, description = "Match on session id, name, phone_number, or push_name (substring, case-insensitive)")),
    responses((status = 200, body = SearchResponse))
)]
pub async fn search_sessions(
    State(state): State<AppState>,
    Query(q): Query<SearchQuery>,
) -> Result<Json<SearchResponse>, ApiError> {
    let needle = q.q.trim().to_lowercase();
    if needle.is_empty() {
        return Err(ApiError::BadRequest("empty query".into()));
    }

    let sessions = state
        .session_manager()
        .list_sessions()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let mut hits = Vec::new();
    for s in sessions {
        let mut matched = Vec::new();
        if s.id.to_lowercase().contains(&needle) {
            matched.push("id".to_string());
        }
        if let Some(ref name) = s.name {
            if name.to_lowercase().contains(&needle) {
                matched.push("name".to_string());
            }
        }
        if let Some(ref phone) = s.phone_number {
            if phone.to_lowercase().contains(&needle) {
                matched.push("phone_number".to_string());
            }
        }
        if let Some(ref push_name) = s.push_name {
            if push_name.to_lowercase().contains(&needle) {
                matched.push("push_name".to_string());
            }
        }

        if !matched.is_empty() {
            hits.push(SearchHit {
                id: s.id,
                name: s.name,
                phone_number: s.phone_number,
                push_name: s.push_name,
                status: s.status,
                is_logged_in: s.is_logged_in,
                match_on: matched,
            });
        }
    }

    let total = hits.len();
    Ok(Json(SearchResponse {
        q: q.q,
        total,
        hits,
    }))
}

#[utoipa::path(
    get,
    path = "/api/v1/stats",
    tag = "bulk",
    security(("bearer_auth" = [])),
    responses((status = 200, body = FleetStats))
)]
pub async fn fleet_stats(State(state): State<AppState>) -> Result<Json<FleetStats>, ApiError> {
    let sessions = state
        .session_manager()
        .list_sessions()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let (mut connected, mut connecting, mut disconnected, mut logged_out) = (0, 0, 0, 0);
    for s in &sessions {
        let effective = state
            .get_session(&s.id)
            .map(|r| r.effective_status())
            .unwrap_or(s.status);
        match effective {
            SessionStatus::Connected | SessionStatus::LoggedIn => connected += 1,
            SessionStatus::Connecting
            | SessionStatus::WaitingForQr
            | SessionStatus::WaitingForPairCode => connecting += 1,
            SessionStatus::Disconnected if !s.is_logged_in => logged_out += 1,
            _ => disconnected += 1,
        }
    }

    let webhook_total: usize = sessions
        .iter()
        .map(|s| state.get_webhooks(&s.id).len())
        .sum();
    let now_ms = chrono::Utc::now().timestamp_millis();
    let event_rate_per_min = state
        .recent_events(200)
        .iter()
        .filter(|e| now_ms - e.at_epoch_ms < 60_000)
        .count() as u32;

    Ok(Json(FleetStats {
        session_total: sessions.len(),
        session_connected: connected,
        session_connecting: connecting,
        session_disconnected: disconnected,
        session_logged_out: logged_out,
        webhook_total,
        webhook_circuits_open: state.webhook_circuits_open_count(),
        event_rate_per_min,
        uptime_seconds: UPTIME_ANCHOR.elapsed().as_secs(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        storage_path: state.base_storage_path().to_string(),
    }))
}

#[utoipa::path(
    post,
    path = "/api/v1/webhooks/reenable-all",
    tag = "bulk",
    security(("bearer_auth" = [])),
    responses((status = 200, body = ReenableCircuitsResponse))
)]
pub async fn reenable_circuits(
    State(state): State<AppState>,
) -> Result<Json<ReenableCircuitsResponse>, ApiError> {
    let urls = state.reenable_all_open_circuits();
    let total = urls.len();
    Ok(Json(ReenableCircuitsResponse {
        reenabled: urls,
        total,
    }))
}
