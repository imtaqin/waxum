//! Read access to a session's chat store, row for row, plus a live stream
//! of what changes. Remote clients (waxum desktop and the phone app in
//! server mode) render from these exactly as they would from a local store.
//!
//! - `GET /api/v1/sessions/{sid}/store/chats?archived=&limit=`
//! - `GET /api/v1/sessions/{sid}/store/chats/{jid}`
//! - `GET /api/v1/sessions/{sid}/store/chats/{jid}/messages?before_ms=&before_seq=&limit=`
//! - `GET /api/v1/sessions/{sid}/store/chats/{jid}/messages/{id}`
//! - `GET /api/v1/sessions/{sid}/store/chats/{jid}/messages/{id}/reactions`
//! - `GET /api/v1/sessions/{sid}/store/chats/{jid}/messages/{id}/receipts`
//! - `GET /api/v1/sessions/{sid}/store/chats/{jid}/kinds?kinds=image,video&limit=`
//! - `GET /api/v1/sessions/{sid}/store/chats/{jid}/links?limit=`
//! - `GET /api/v1/sessions/{sid}/store/recent?kinds=sticker&limit=`
//! - `GET /api/v1/sessions/{sid}/store/starred?limit=`
//! - `GET /api/v1/sessions/{sid}/store/search?q=&chat=&limit=`
//! - `GET /api/v1/sessions/{sid}/store/contacts`
//! - `GET /api/v1/sessions/{sid}/store/contacts/{jid}`
//! - `GET /api/v1/sessions/{sid}/store/events` (server-sent events)

use std::convert::Infallible;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::{Path, Query, State},
    response::sse::{Event, KeepAlive, Sse},
    Json,
};
use futures::stream::Stream;
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use wacore::types::events::{ChannelEventHandler, Event as WaEvent, EventInterest, EventKind};
use wacore::types::presence::{ChatPresence, ChatPresenceMedia};
use wacore_binary::jid::Jid;
use whatsapp_rust_chat_store::wire::{
    WireChat, WireContact, WireEvent, WireMessage, WireReaction, WireReceipt,
};
use whatsapp_rust_chat_store::{ChatStore, MessageCursor};

use crate::error::ApiError;
use crate::state::AppState;

fn store(state: &AppState, session_id: &str) -> Result<Arc<ChatStore>, ApiError> {
    let runtime = state
        .get_session(session_id)
        .ok_or_else(|| ApiError::SessionNotFound(session_id.to_string()))?;
    runtime.get_chat_store().ok_or(ApiError::NotConnected)
}

fn parse(jid: &str) -> Result<Jid, ApiError> {
    Jid::from_str(jid).map_err(|_| ApiError::InvalidJid(jid.to_string()))
}

fn internal(e: impl std::fmt::Display) -> ApiError {
    ApiError::Internal(e.to_string())
}

#[derive(Debug, Deserialize)]
pub struct ChatsQuery {
    pub archived: Option<bool>,
    pub limit: Option<i64>,
}

pub async fn chats(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(q): Query<ChatsQuery>,
) -> Result<Json<Vec<WireChat>>, ApiError> {
    let rows = store(&state, &session_id)?
        .chats(q.archived.unwrap_or(true), q.limit.unwrap_or(10_000))
        .await
        .map_err(internal)?;
    Ok(Json(rows.iter().map(WireChat::from).collect()))
}

pub async fn chat(
    State(state): State<AppState>,
    Path((session_id, jid)): Path<(String, String)>,
) -> Result<Json<Option<WireChat>>, ApiError> {
    let row = store(&state, &session_id)?
        .chat(&parse(&jid)?)
        .await
        .map_err(internal)?;
    Ok(Json(row.as_ref().map(WireChat::from)))
}

#[derive(Debug, Deserialize)]
pub struct MessagesQuery {
    pub before_ms: Option<i64>,
    pub before_seq: Option<i64>,
    pub limit: Option<i64>,
}

pub async fn messages(
    State(state): State<AppState>,
    Path((session_id, jid)): Path<(String, String)>,
    Query(q): Query<MessagesQuery>,
) -> Result<Json<Vec<WireMessage>>, ApiError> {
    let before = match (q.before_ms, q.before_seq) {
        (Some(timestamp_ms), Some(seq)) => Some(MessageCursor { timestamp_ms, seq }),
        _ => None,
    };
    let rows = store(&state, &session_id)?
        .messages(&parse(&jid)?, before, q.limit.unwrap_or(50).clamp(1, 5000))
        .await
        .map_err(internal)?;
    Ok(Json(rows.iter().map(WireMessage::from).collect()))
}

pub async fn message(
    State(state): State<AppState>,
    Path((session_id, jid, id)): Path<(String, String, String)>,
) -> Result<Json<Option<WireMessage>>, ApiError> {
    let row = store(&state, &session_id)?
        .message(&parse(&jid)?, &id)
        .await
        .map_err(internal)?;
    Ok(Json(row.as_ref().map(WireMessage::from)))
}

pub async fn reactions(
    State(state): State<AppState>,
    Path((session_id, jid, id)): Path<(String, String, String)>,
) -> Result<Json<Vec<WireReaction>>, ApiError> {
    let rows = store(&state, &session_id)?
        .reactions(&parse(&jid)?, &id)
        .await
        .map_err(internal)?;
    Ok(Json(rows.iter().map(WireReaction::from).collect()))
}

pub async fn receipts(
    State(state): State<AppState>,
    Path((session_id, jid, id)): Path<(String, String, String)>,
) -> Result<Json<Vec<WireReceipt>>, ApiError> {
    let rows = store(&state, &session_id)?
        .receipts(&parse(&jid)?, &id)
        .await
        .map_err(internal)?;
    Ok(Json(rows.iter().map(WireReceipt::from).collect()))
}

#[derive(Debug, Deserialize)]
pub struct KindsQuery {
    pub kinds: String,
    pub limit: Option<i64>,
}

pub async fn kinds(
    State(state): State<AppState>,
    Path((session_id, jid)): Path<(String, String)>,
    Query(q): Query<KindsQuery>,
) -> Result<Json<Vec<WireMessage>>, ApiError> {
    let kinds: Vec<&str> = q.kinds.split(',').filter(|k| !k.is_empty()).collect();
    let rows = store(&state, &session_id)?
        .messages_of_kinds(&parse(&jid)?, &kinds, q.limit.unwrap_or(200))
        .await
        .map_err(internal)?;
    Ok(Json(rows.iter().map(WireMessage::from).collect()))
}

/// The newest messages of some kinds across all chats.
pub async fn recent(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(q): Query<KindsQuery>,
) -> Result<Json<Vec<WireMessage>>, ApiError> {
    let kinds: Vec<&str> = q.kinds.split(',').filter(|k| !k.is_empty()).collect();
    let rows = store(&state, &session_id)?
        .recent_of_kinds(&kinds, q.limit.unwrap_or(200))
        .await
        .map_err(internal)?;
    Ok(Json(rows.iter().map(WireMessage::from).collect()))
}

#[derive(Debug, Deserialize)]
pub struct LimitQuery {
    pub limit: Option<i64>,
}

pub async fn links(
    State(state): State<AppState>,
    Path((session_id, jid)): Path<(String, String)>,
    Query(q): Query<LimitQuery>,
) -> Result<Json<Vec<WireMessage>>, ApiError> {
    let rows = store(&state, &session_id)?
        .messages_with_links(&parse(&jid)?, q.limit.unwrap_or(200))
        .await
        .map_err(internal)?;
    Ok(Json(rows.iter().map(WireMessage::from).collect()))
}

pub async fn starred(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(q): Query<LimitQuery>,
) -> Result<Json<Vec<WireMessage>>, ApiError> {
    let rows = store(&state, &session_id)?
        .starred(q.limit.unwrap_or(500))
        .await
        .map_err(internal)?;
    Ok(Json(rows.iter().map(WireMessage::from).collect()))
}

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: String,
    pub chat: Option<String>,
    pub limit: Option<i64>,
}

pub async fn search(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(q): Query<SearchQuery>,
) -> Result<Json<Vec<WireMessage>>, ApiError> {
    let store = store(&state, &session_id)?;
    let limit = q.limit.unwrap_or(50);
    let rows = match q.chat.as_deref().filter(|c| !c.is_empty()) {
        Some(chat) => {
            store
                .search_messages_in_chat(&parse(chat)?, &q.q, limit)
                .await
        }
        None => store.search_messages(&q.q, limit).await,
    }
    .map_err(internal)?;
    Ok(Json(rows.iter().map(WireMessage::from).collect()))
}

pub async fn contacts(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<Vec<WireContact>>, ApiError> {
    let rows = store(&state, &session_id)?
        .contacts()
        .await
        .map_err(internal)?;
    Ok(Json(rows.iter().map(WireContact::from).collect()))
}

pub async fn contact(
    State(state): State<AppState>,
    Path((session_id, jid)): Path<(String, String)>,
) -> Result<Json<Option<WireContact>>, ApiError> {
    let row = store(&state, &session_id)?
        .contact(&parse(&jid)?)
        .await
        .map_err(internal)?;
    Ok(Json(row.as_ref().map(WireContact::from)))
}

fn realtime(event: &WaEvent) -> Option<WireEvent> {
    match event {
        WaEvent::Presence(update) => Some(WireEvent::Presence {
            jid: update.from.to_non_ad_string(),
            available: !update.unavailable,
            last_seen_ms: update.last_seen.map(|t| t.timestamp_millis()),
        }),
        WaEvent::ChatPresence(update) => Some(WireEvent::Typing {
            chat: update.source.chat.to_non_ad_string(),
            sender: update.source.sender.to_non_ad_string(),
            group: update.source.is_group,
            state: match update.state {
                ChatPresence::Composing => "composing",
                _ => "paused",
            }
            .to_string(),
            audio: update.media == ChatPresenceMedia::Audio,
        }),
        WaEvent::PictureUpdate(update) => Some(WireEvent::Picture {
            jid: update.jid.to_non_ad_string(),
        }),
        _ => None,
    }
}

fn to_sse(event: &WireEvent) -> Event {
    Event::default().data(serde_json::to_string(event).unwrap_or_default())
}

/// Store changes and realtime signals for one session, as they happen.
/// Survives reconnects: when the session's store or client goes away the
/// stream waits for the new ones.
pub async fn events(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(256);
    tokio::spawn(async move {
        loop {
            if tx.is_closed() {
                return;
            }
            let runtime = state.get_session(&session_id);
            let store = runtime.as_ref().and_then(|r| r.get_chat_store());
            let client = runtime.as_ref().and_then(|r| r.get_client());
            let Some(store) = store else {
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            };
            let mut changes = store.subscribe();
            let (handler, client_rx) = ChannelEventHandler::new();
            let _subscription = client.as_ref().map(|c| {
                c.subscribe(
                    EventInterest::of(&[
                        EventKind::Presence,
                        EventKind::ChatPresence,
                        EventKind::PictureUpdate,
                    ]),
                    handler,
                )
            });
            // Tell the client to refresh: it may have missed changes while
            // the store was being replaced.
            if tx.send(Ok(to_sse(&WireEvent::Chats))).await.is_err() {
                return;
            }
            loop {
                let event = tokio::select! {
                    change = changes.recv() => match change {
                        Ok(change) => WireEvent::from(&change),
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => WireEvent::Chats,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    },
                    event = client_rx.recv(), if client.is_some() => match event {
                        Ok(event) => match realtime(&event) {
                            Some(e) => e,
                            None => continue,
                        },
                        Err(_) => break,
                    },
                    _ = tokio::time::sleep(Duration::from_secs(5)) => {
                        // The session may have reconnected with a new store.
                        let current = state
                            .get_session(&session_id)
                            .and_then(|r| r.get_chat_store());
                        if !current.is_some_and(|c| Arc::ptr_eq(&c, &store)) {
                            break;
                        }
                        continue;
                    }
                };
                if tx.send(Ok(to_sse(&event))).await.is_err() {
                    return;
                }
            }
        }
    });
    Sse::new(ReceiverStream::new(rx)).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}
