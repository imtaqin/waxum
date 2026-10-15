//! Startup reconnect, the stale-socket watchdog and forced client rebuilds.

use std::time::Duration;

use crate::api::shared::sessions::models::SessionStatus;
use crate::state::AppState;

use super::*;

/// On engine boot, walk every previously-paired session and start a
/// reconnect attempt in the background. Sessions that have no stored
/// credentials (never paired or freshly logged out) are skipped — those
/// stay disconnected until the user re-pairs from the dashboard.
pub async fn reconnect_all_on_startup(state: AppState) {
    let sessions = match state.session_manager().list_sessions().await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("[startup] list_sessions failed: {}", e);
            return;
        }
    };

    let stagger = crate::preflight::session_startup_stagger();
    tracing::info!(
        stagger_ms = stagger.as_millis() as u64,
        "[startup] auto-reconnect: found {} sessions in DB",
        sessions.len()
    );

    match state.session_manager().restore_auto_disabled_webhooks().await {
        Ok(0) => {}
        Ok(n) => tracing::warn!(
            "[startup] re-enabled {n} webhook(s) that an older release auto-disabled after repeated failures"
        ),
        Err(e) => tracing::warn!("[startup] restore_auto_disabled_webhooks failed: {}", e),
    }

    for session in sessions {
        load_session_webhooks(&state, &session.id).await;

        if is_cloud_session(&session) {
            tracing::debug!("[startup] skip session {} (whatsapp_cloud)", session.id);
            continue;
        }
        let should_reconnect = matches!(
            session.status,
            SessionStatus::LoggedIn | SessionStatus::Connected | SessionStatus::Connecting
        ) || session.is_logged_in;
        if !should_reconnect {
            tracing::debug!(
                "[startup] skip session {} (status={:?})",
                session.id,
                session.status
            );
            continue;
        }

        let storage_path = match state.session_manager().get_storage_path(&session.id).await {
            Ok(Some(p)) => p,
            _ => {
                tracing::warn!("[startup] no storage path for session {}", session.id);
                continue;
            }
        };

        let runtime = state.get_or_create_session(&session.id, &storage_path);
        runtime.set_status(SessionStatus::Connecting);

        let state_clone = state.clone();
        let sid = session.id.clone();
        tokio::spawn(async move {
            tracing::info!("[startup] reconnecting session {}", sid);
            if let Err(e) = connect_client(&state_clone, &sid, None).await {
                tracing::warn!("[startup] reconnect failed for {}: {}", sid, e);
                if let Some(runtime) = state_clone.get_session(&sid) {
                    runtime.set_status(SessionStatus::Disconnected);
                }
            }
        });

        tokio::time::sleep(stagger).await;
    }
}

/// Self-heal for sessions whose whatsapp-rust client has stopped making
/// progress, in two shapes. For either one, this forces the same full
/// rebuild a manual `POST .../connect` performs: disconnect the client
/// (bounded by [`REBUILD_DISCONNECT_TIMEOUT`]) and spawn [`connect_client`]
/// fresh.
///
/// Ticks every `RECONNECT_WATCHDOG_POLL_MS` (default 30 s).
///
/// **Stuck reconnecting.** whatsapp-rust's retry loop (inside `bot.run()`)
/// has no attempt cap. It backs off up to 15 minutes between tries and
/// keeps going forever, which reads as "auto-reconnect enabled but stuck"
/// during a long outage or a wedged handshake. A session that has been
/// `Connecting` for longer than `RECONNECT_MAX_STUCK_SECS` (default 600),
/// or whose `reconnect_errors` has reached `RECONNECT_MAX_ATTEMPTS`
/// (default 10), is rebuilt.
///
/// **Stale socket.** A logged-in session whose socket has received nothing
/// for `STALE_SOCKET_SECS` (default 150, `0` disables) is rebuilt. On a
/// healthy connection the client's own keepalive guarantees traffic well
/// inside that window: it pings after 15-30 s of silence and waits 20 s for
/// the reply. Issue #143 showed a socket silent for 51 minutes after the
/// host's IP changed, still reported connected and `reachable`, with no
/// keepalive ping sent and no reconnect attempted. That one-sided,
/// half-open state is what this catches.
///
/// Both paths broadcast a synthetic `disconnected` event, with `forced_by`
/// naming the check, because the crate doesn't always dispatch one for a
/// socket that dies silently. Sessions in an account-lock cooldown
/// (`ACCOUNT_LOCK_BACKOFF_SECS`) and paused sessions are skipped.
pub async fn run_reconnect_watchdog(state: AppState) {
    let poll_ms: u64 = std::env::var("RECONNECT_WATCHDOG_POLL_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30_000);
    let max_attempts: u32 = std::env::var("RECONNECT_MAX_ATTEMPTS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);
    let max_stuck_secs: i64 = std::env::var("RECONNECT_MAX_STUCK_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(600);
    let stale_socket_secs: u64 = std::env::var("STALE_SOCKET_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(150);

    let mut ticker = tokio::time::interval(Duration::from_millis(poll_ms));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    tracing::info!(
        poll_ms,
        max_attempts,
        max_stuck_secs,
        stale_socket_secs,
        "reconnect watchdog started"
    );

    loop {
        ticker.tick().await;
        let now_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
        for (session_id, runtime) in state.session_iter_with_ids() {
            if let Some(remaining) = runtime.lock_cooldown_remaining() {
                tracing::debug!(
                    session_id = %session_id,
                    cooldown_remaining_secs = remaining,
                    "reconnect watchdog: skipping session in account-lock cooldown"
                );
                continue;
            }
            let Some(client) = runtime.get_client() else {
                continue;
            };

            let forced_by = match runtime.get_status() {
                SessionStatus::Connecting => {
                    let stuck_secs = runtime.reconnecting_for_secs().unwrap_or(0);
                    let reconnect_errors = client.stats().reconnect_errors;
                    if reconnect_errors < max_attempts && stuck_secs < max_stuck_secs {
                        continue;
                    }
                    tracing::warn!(
                        session_id = %session_id,
                        reconnect_errors,
                        stuck_secs,
                        "reconnect watchdog: forcing full rebuild after a stuck retry window"
                    );
                    "reconnect_watchdog"
                }
                SessionStatus::LoggedIn | SessionStatus::Connected => {
                    if client.is_paused() {
                        continue;
                    }
                    let last_ms = client.stats().last_data_received_ms;
                    let Some(silent_secs) =
                        stale_socket_silence(now_ms, last_ms, stale_socket_secs)
                    else {
                        continue;
                    };
                    tracing::warn!(
                        session_id = %session_id,
                        silent_secs,
                        "reconnect watchdog: socket received nothing for too long while connected, forcing full rebuild"
                    );
                    "stale_socket_watchdog"
                }
                _ => continue,
            };

            force_rebuild(&state, &session_id, &runtime, &client, forced_by).await;
        }
    }
}

/// How long [`force_rebuild`] waits for the old client to disconnect. A
/// half-open socket can hang a graceful close indefinitely, so this is
/// bounded and the client is dropped either way.
pub(super) const REBUILD_DISCONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Seconds of silence when a connected socket counts as stale, or `None`
/// when it doesn't: the check is disabled (`threshold_secs == 0`),
/// nothing has been received yet (`last_data_received_ms == 0`), or the
/// silence is within the threshold.
pub(super) fn stale_socket_silence(
    now_ms: u64,
    last_data_received_ms: u64,
    threshold_secs: u64,
) -> Option<u64> {
    if threshold_secs == 0 || last_data_received_ms == 0 {
        return None;
    }
    let silent_secs = now_ms.saturating_sub(last_data_received_ms) / 1000;
    (silent_secs > threshold_secs).then_some(silent_secs)
}

pub(super) async fn force_rebuild(
    state: &AppState,
    session_id: &str,
    runtime: &std::sync::Arc<crate::state::SessionState>,
    client: &std::sync::Arc<whatsapp_rust::Client>,
    forced_by: &str,
) {
    if tokio::time::timeout(REBUILD_DISCONNECT_TIMEOUT, client.disconnect())
        .await
        .is_err()
    {
        tracing::warn!(
            session_id = %session_id,
            "reconnect watchdog: old client did not disconnect within {}s; dropping it",
            REBUILD_DISCONNECT_TIMEOUT.as_secs()
        );
    }
    runtime.set_client(None);
    runtime.set_status(SessionStatus::Disconnected);
    runtime.clear_reconnecting();
    let _ = state
        .session_manager()
        .update_session_status(session_id, SessionStatus::Disconnected, false)
        .await;

    let payload = serde_json::json!({
        "session_id": session_id,
        "event": "disconnected",
        "timestamp": chrono::Utc::now().timestamp(),
        "data": { "forced_by": forced_by },
    });
    if let Ok(payload_str) = serde_json::to_string(&payload) {
        state
            .broadcast_to_webhooks(session_id, "disconnected", &payload_str)
            .await;
        state
            .publish_to_nats(session_id, "disconnected", &payload_str)
            .await;
        runtime.broadcast_event(payload_str);
    }

    runtime.set_status(SessionStatus::Connecting);
    let state_clone = state.clone();
    let session_id_clone = session_id.to_string();
    let forced_by = forced_by.to_string();
    tokio::spawn(async move {
        if let Err(e) = connect_client(&state_clone, &session_id_clone, None).await {
            tracing::error!(
                "{}: rebuild failed for {}: {}",
                forced_by,
                session_id_clone,
                e
            );
            if let Some(rt) = state_clone.get_session(&session_id_clone) {
                rt.set_status(SessionStatus::Disconnected);
                rt.set_client(None);
            }
        }
    });
}

#[cfg(test)]
mod stale_socket_tests {
    use super::stale_socket_silence;

    #[test]
    fn a_socket_silent_past_the_threshold_is_stale() {
        let now = 1_790_774_900_000;
        let last = 1_790_771_858_000;
        assert_eq!(stale_socket_silence(now, last, 150), Some(3042));
    }

    #[test]
    fn recent_traffic_disabled_check_or_no_traffic_yet_is_not_stale() {
        let now = 1_000_000_000;
        assert_eq!(stale_socket_silence(now, now - 30_000, 150), None);
        assert_eq!(stale_socket_silence(now, now - 150_000, 150), None);
        assert_eq!(stale_socket_silence(now, now - 3_600_000, 0), None);
        assert_eq!(stale_socket_silence(now, 0, 150), None);
        assert_eq!(stale_socket_silence(now, now + 5_000, 150), None);
    }
}
