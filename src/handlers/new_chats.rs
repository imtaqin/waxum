//! New-outgoing-chat counter and limit (#157). See [`crate::db::new_chats`]
//! for why this exists and what is stored.
//!
//! A chat is "new" when the session sends to a direct chat it has no
//! history with: no earlier send through waxum and no stored message in
//! either direction. Groups, newsletters and broadcasts never count, and
//! neither does answering someone who wrote first.
//!
//! [`admit`] runs on every send. For a known chat it is one indexed
//! lookup. For a new one it enforces the session's limit, if any, and
//! records the chat. It only ever refuses on the limit itself: if the
//! bookkeeping fails the send goes ahead, because a counter must not be
//! the reason a message is lost.
//!
//! waxum sees only what is sent through it. Chats started from the phone
//! or WhatsApp Web on the same account are not counted, so the figure
//! WhatsApp acts on can be higher than the one reported here.

use axum::{
    extract::{Path, State},
    Json,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use wacore_binary::jid::Jid;

use crate::db::new_chats as store;
use crate::error::ApiError;
use crate::state::AppState;

const HOUR: i64 = 3600;
const MAX_WINDOW_HOURS: i64 = 24 * 30;

/// New chats the session started in each rolling window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, ToSchema)]
pub struct NewChatCounts {
    pub last_3h: i64,
    pub last_6h: i64,
    pub last_12h: i64,
    pub last_24h: i64,
}

fn count(started: &[i64], now: i64) -> NewChatCounts {
    let within = |hours: i64| started.iter().filter(|t| **t > now - hours * HOUR).count() as i64;
    NewChatCounts {
        last_3h: within(3),
        last_6h: within(6),
        last_12h: within(12),
        last_24h: within(24),
    }
}

/// Seconds until the oldest chat in the window ages out and a new one is
/// allowed again. `None` when the window still has room.
fn retry_after(started: &[i64], now: i64, max_new_chats: i64, window_hours: i64) -> Option<i64> {
    let window = window_hours * HOUR;
    let in_window: Vec<i64> = started
        .iter()
        .copied()
        .filter(|t| *t > now - window)
        .collect();
    if (in_window.len() as i64) < max_new_chats {
        return None;
    }
    let frees_slot = in_window[in_window.len() - max_new_chats.max(1) as usize];
    Some((frees_slot + window - now).max(1))
}

/// Whether sends to this JID take part at all.
fn is_direct_chat(jid: &str) -> bool {
    jid.ends_with("@s.whatsapp.net") || jid.ends_with("@lid")
}

/// Called before every send. `to` is the resolved recipient, `requested`
/// what the caller passed (a phone number resolves to a LID, and history
/// may be stored under either).
pub async fn admit(
    state: &AppState,
    session_id: &str,
    to: &Jid,
    requested: &str,
) -> Result<(), ApiError> {
    let to = to.to_string();
    if !is_direct_chat(&to) {
        return Ok(());
    }
    let mut jids = vec![to.clone()];
    if let Ok(requested) = crate::handlers::messages::parse_jid(requested) {
        let requested = requested.to_string();
        if requested != to {
            jids.push(requested);
        }
    }

    let pool = state.session_manager().pool();
    match store::chat_known(pool, session_id, &jids).await {
        Ok(true) => return Ok(()),
        Ok(false) => {}
        Err(e) => {
            tracing::warn!(session_id = %session_id, "new-chat lookup failed, not counting this send: {e}");
            return Ok(());
        }
    }

    let now = chrono::Utc::now().timestamp();
    if let Ok(Some((max_new_chats, window_hours))) = store::get_limit(pool, session_id).await {
        let since = now - window_hours * HOUR;
        if let Ok(started) = store::started_since(pool, session_id, since).await {
            if let Some(retry_after_secs) = retry_after(&started, now, max_new_chats, window_hours)
            {
                return Err(ApiError::NewChatLimit {
                    max_new_chats,
                    window_hours,
                    retry_after_secs,
                });
            }
        }
    }

    if let Err(e) = store::record(pool, session_id, &to, now).await {
        tracing::warn!(session_id = %session_id, "could not record a new chat: {e}");
    }
    Ok(())
}

/// The session's rolling new-chat figures.
pub(crate) async fn counts(state: &AppState, session_id: &str) -> NewChatCounts {
    let now = chrono::Utc::now().timestamp();
    let pool = state.session_manager().pool();
    match store::started_since(pool, session_id, now - 24 * HOUR).await {
        Ok(started) => count(&started, now),
        Err(_) => NewChatCounts::default(),
    }
}

/// Stores the rolling figures at the moment WhatsApp logged the session
/// out, and returns them for the event that reports it.
pub(crate) async fn snapshot_on_logout(
    state: &AppState,
    session_id: &str,
    reason: &str,
) -> NewChatCounts {
    let figures = counts(state, session_id).await;
    let incident = store::Incident {
        occurred_at: chrono::Utc::now().timestamp(),
        reason: reason.to_string(),
        last_3h: figures.last_3h,
        last_6h: figures.last_6h,
        last_12h: figures.last_12h,
        last_24h: figures.last_24h,
    };
    if let Err(e) =
        store::record_incident(state.session_manager().pool(), session_id, &incident).await
    {
        tracing::warn!(session_id = %session_id, "could not record the new-chat snapshot: {e}");
    }
    figures
}

/// A ceiling on new outgoing chats.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct NewChatLimit {
    /// New chats allowed inside the window.
    #[schema(example = 40)]
    pub max_new_chats: i64,
    /// Rolling window in hours. Defaults to 24.
    #[serde(default = "default_window_hours")]
    #[schema(example = 24)]
    pub window_hours: i64,
}

fn default_window_hours() -> i64 {
    24
}

/// The figures recorded when WhatsApp logged the session out.
#[derive(Debug, Serialize, ToSchema)]
pub struct NewChatIncident {
    /// Unix seconds.
    pub occurred_at: i64,
    /// Why WhatsApp ended the session, e.g. `LoggedOut` or `AccountLocked`.
    pub reason: String,
    pub new_chats: NewChatCounts,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct NewChatLimitResponse {
    /// `null` when no limit is set (the default): nothing is refused.
    pub limit: Option<NewChatLimit>,
    /// New chats started through waxum, by rolling window.
    pub new_chats: NewChatCounts,
    /// Seconds until the next new chat is allowed, when the limit is
    /// reached right now. `null` otherwise.
    pub retry_after_seconds: Option<i64>,
    /// Figures at each logout by WhatsApp, newest first. Read the
    /// threshold for this account off these.
    pub incidents: Vec<NewChatIncident>,
}

async fn require_session(state: &AppState, session_id: &str) -> Result<(), ApiError> {
    state
        .session_manager()
        .get_session(session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .map(|_| ())
        .ok_or_else(|| ApiError::SessionNotFound(session_id.to_string()))
}

async fn describe(state: &AppState, session_id: &str) -> Result<NewChatLimitResponse, ApiError> {
    let pool = state.session_manager().pool();
    let now = chrono::Utc::now().timestamp();
    let limit = store::get_limit(pool, session_id).await?;
    let window_hours = limit.map(|(_, w)| w).unwrap_or(24).max(24);
    let started = store::started_since(pool, session_id, now - window_hours * HOUR).await?;
    let incidents = store::incidents(pool, session_id).await?;
    Ok(NewChatLimitResponse {
        retry_after_seconds: limit
            .and_then(|(max, window)| retry_after(&started, now, max, window)),
        limit: limit.map(|(max_new_chats, window_hours)| NewChatLimit {
            max_new_chats,
            window_hours,
        }),
        new_chats: count(&started, now),
        incidents: incidents
            .into_iter()
            .map(|i| NewChatIncident {
                occurred_at: i.occurred_at,
                reason: i.reason,
                new_chats: NewChatCounts {
                    last_3h: i.last_3h,
                    last_6h: i.last_6h,
                    last_12h: i.last_12h,
                    last_24h: i.last_24h,
                },
            })
            .collect(),
    })
}

/// Read the new-chat limit, the current figures and past incidents.
#[utoipa::path(
    get,
    path = "/api/v1/sessions/{session_id}/settings/new-chat-limit",
    tag = "sessions",
    security(("bearer_auth" = [])),
    params(("session_id" = String, Path, description = "Session ID")),
    responses(
        (status = 200, description = "Limit, rolling figures and incidents", body = NewChatLimitResponse),
        (status = 404, description = "Session not found")
    )
)]
pub async fn get_new_chat_limit(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<NewChatLimitResponse>, ApiError> {
    require_session(&state, &session_id).await?;
    Ok(Json(describe(&state, &session_id).await?))
}

/// Limit how many new chats the session may start.
///
/// Once `max_new_chats` chats have been started inside `window_hours`, a
/// send to a number the session has no chat with is refused with `429`
/// and a `Retry-After` header. Sends in existing chats, replies to
/// people who wrote first, and group sends are never refused. Off until
/// set; waxum does not know WhatsApp's threshold and ships no default.
#[utoipa::path(
    put,
    path = "/api/v1/sessions/{session_id}/settings/new-chat-limit",
    tag = "sessions",
    security(("bearer_auth" = [])),
    params(("session_id" = String, Path, description = "Session ID")),
    request_body = NewChatLimit,
    responses(
        (status = 200, description = "Limit stored", body = NewChatLimitResponse),
        (status = 400, description = "max_new_chats or window_hours out of range"),
        (status = 404, description = "Session not found")
    )
)]
pub async fn set_new_chat_limit(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<NewChatLimit>,
) -> Result<Json<NewChatLimitResponse>, ApiError> {
    require_session(&state, &session_id).await?;
    if request.max_new_chats < 1 {
        return Err(ApiError::BadRequest(
            "max_new_chats must be at least 1; DELETE this resource to remove the limit".into(),
        ));
    }
    if !(1..=MAX_WINDOW_HOURS).contains(&request.window_hours) {
        return Err(ApiError::BadRequest(format!(
            "window_hours must be between 1 and {MAX_WINDOW_HOURS}"
        )));
    }
    store::set_limit(
        state.session_manager().pool(),
        &session_id,
        Some((request.max_new_chats, request.window_hours)),
    )
    .await?;
    Ok(Json(describe(&state, &session_id).await?))
}

/// Remove the new-chat limit. The counting continues.
#[utoipa::path(
    delete,
    path = "/api/v1/sessions/{session_id}/settings/new-chat-limit",
    tag = "sessions",
    security(("bearer_auth" = [])),
    params(("session_id" = String, Path, description = "Session ID")),
    responses(
        (status = 200, description = "Limit removed", body = NewChatLimitResponse),
        (status = 404, description = "Session not found")
    )
)]
pub async fn clear_new_chat_limit(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<NewChatLimitResponse>, ApiError> {
    require_session(&state, &session_id).await?;
    store::set_limit(state.session_manager().pool(), &session_id, None).await?;
    Ok(Json(describe(&state, &session_id).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;

    #[test]
    fn counts_use_rolling_windows() {
        let started = [
            NOW - 30 * HOUR,
            NOW - 20 * HOUR,
            NOW - 7 * HOUR,
            NOW - 4 * HOUR,
            NOW - HOUR,
            NOW - 60,
        ];
        assert_eq!(
            count(&started, NOW),
            NewChatCounts {
                last_3h: 2,
                last_6h: 3,
                last_12h: 4,
                last_24h: 5,
            }
        );
    }

    #[test]
    fn the_limit_frees_up_when_the_oldest_chat_leaves_the_window() {
        let started = [NOW - 23 * HOUR, NOW - 2 * HOUR, NOW - HOUR];
        assert_eq!(retry_after(&started, NOW, 4, 24), None, "room left");
        assert_eq!(
            retry_after(&started, NOW, 3, 24),
            Some(HOUR),
            "full until the 23 h old chat ages out"
        );
        assert_eq!(
            retry_after(&started, NOW, 2, 24),
            Some(22 * HOUR),
            "over the limit: wait for the second-newest to age out"
        );
        assert_eq!(
            retry_after(&started, NOW, 2, 1),
            None,
            "nothing inside a 1 h window"
        );
    }

    #[test]
    fn only_direct_chats_count() {
        assert!(is_direct_chat("628123456789@s.whatsapp.net"));
        assert!(is_direct_chat("123456789012345@lid"));
        assert!(!is_direct_chat("120363000000000000@g.us"));
        assert!(!is_direct_chat("120363000000000000@newsletter"));
        assert!(!is_direct_chat("status@broadcast"));
    }
}
