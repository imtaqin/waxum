//! Server-sent events tail over the in-memory ring buffer.
//!
//! `broadcast_to_webhooks` pushes a preview of every outbound event
//! into a bounded ring on `AppState`, which the console overview reads
//! for its "Live events" panel. This handler streams the same events
//! out to any HTTP client as `text/event-stream`, so an operator can
//! `curl -N /api/v1/events/tail` (or an integration point like n8n)
//! and watch traffic without registering a webhook first.
//!
//! Implementation notes:
//!
//! - On connect the handler drains a small backlog (up to 50 items)
//!   from the ring, so late subscribers immediately see the tail of
//!   recent activity. After that a background task polls the ring
//!   every 500 ms for new entries and forwards anything with an
//!   `at_epoch_ms` strictly greater than the last emitted.
//! - The ring is bounded at 200 items — if a burst produces more
//!   events than that inside a poll window a subscriber will miss the
//!   middle. Acceptable for an ops tail; anything that needs
//!   guaranteed delivery should still use a webhook.
//! - Query params:
//!   `session=<sid>` — filter to a single session id.
//!   `event=<name>`  — filter to a single event type name.

use std::convert::Infallible;
use std::time::Duration;

use axum::{
    extract::{Query, State},
    response::sse::{Event, KeepAlive, Sse},
};
use futures::stream::Stream;
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::state::{AppState, ConsoleEvent};

#[derive(Debug, Deserialize)]
pub struct EventsTailQuery {
    pub session: Option<String>,
    pub event: Option<String>,
}

fn matches(
    e: &ConsoleEvent,
    session_filter: Option<&str>,
    event_filter: Option<&str>,
) -> bool {
    if let Some(s) = session_filter {
        if e.session_id != s {
            return false;
        }
    }
    if let Some(s) = event_filter {
        if e.event_type != s {
            return false;
        }
    }
    true
}

fn to_sse(ev: ConsoleEvent) -> Event {
    let payload =
        serde_json::to_string(&ev).unwrap_or_else(|_| String::from("{\"error\":\"serialize\"}"));
    Event::default()
        .event(ev.event_type.clone())
        .id(ev.at_epoch_ms.to_string())
        .data(payload)
}

pub async fn events_tail(
    State(state): State<AppState>,
    Query(q): Query<EventsTailQuery>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let session_filter = q.session;
    let event_filter = q.event;

    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(64);

    let backlog: Vec<ConsoleEvent> = state
        .recent_events(50)
        .into_iter()
        .rev()
        .filter(|e| matches(e, session_filter.as_deref(), event_filter.as_deref()))
        .collect();

    let start_at = backlog.last().map(|e| e.at_epoch_ms).unwrap_or(0);
    for ev in backlog {
        let _ = tx.send(Ok(to_sse(ev))).await;
    }

    tokio::spawn(async move {
        let mut last_at = start_at;
        let mut ticker = tokio::time::interval(Duration::from_millis(500));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            if tx.is_closed() {
                return;
            }
            let recent = state.recent_events(200);
            let mut fresh: Vec<ConsoleEvent> = recent
                .into_iter()
                .rev()
                .filter(|e| {
                    e.at_epoch_ms > last_at
                        && matches(e, session_filter.as_deref(), event_filter.as_deref())
                })
                .collect();
            if let Some(latest) = fresh.last() {
                last_at = latest.at_epoch_ms;
            }
            for ev in fresh.drain(..) {
                if tx.send(Ok(to_sse(ev))).await.is_err() {
                    return;
                }
            }
        }
    });

    let stream = ReceiverStream::new(rx);
    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}
