//! Models for message history search.
//!
//! Message history is ingested best-effort from the event stream
//! (incoming) and the send core (outgoing) into the `messages` table
//! (see [`crate::db::messages`]). The types below cover the search
//! endpoint query/response shapes for
//! `GET /api/v1/sessions/{sid}/messages/search` and the fleet-wide
//! `GET /api/v1/messages/search`.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// One message matched by search, newest first.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MessageHit {
    #[schema(example = 42)]
    pub id: i64,

    /// WhatsApp message id.
    #[schema(example = "3EB0C8F1A2B3C4D5E6")]
    pub message_id: String,

    #[schema(example = "main")]
    pub session_id: String,

    /// Chat the message belongs to (DM partner or group JID).
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub chat_jid: String,

    /// Actual sender; differs from `chat_jid` inside groups.
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub sender_jid: String,

    /// `in` (received) or `out` (sent by this gateway).
    #[schema(example = "in")]
    pub direction: String,

    /// Type slug: `text`, `image`, `video`, `audio`, `ptt`,
    /// `document`, `sticker`, `location`, `contact`, ...
    #[schema(example = "text")]
    pub msg_type: String,

    /// Searchable text: message body, or caption for media. Null for
    /// content-free types (stickers, locations).
    #[schema(example = "are we still on for lunch tomorrow?")]
    pub body: Option<String>,

    /// Highlighted match context, present only on backends with cheap
    /// snippet support (SQLite FTS5, Postgres). Contains `<b>` tags
    /// around matched terms.
    #[schema(example = "are we still on for <b>lunch</b> tomorrow?")]
    pub snippet: Option<String>,

    /// Message time as `%Y-%m-%d %H:%M:%S` UTC text.
    #[schema(example = "2026-07-21 10:30:00")]
    pub msg_timestamp: String,
}

/// Search result page.
#[derive(Debug, Serialize, ToSchema)]
pub struct MessageSearchResponse {
    pub messages: Vec<MessageHit>,

    /// Hits in THIS page (not the global match total).
    #[schema(example = 20)]
    pub count: usize,
}

/// Query params for `GET /api/v1/sessions/{session_id}/messages/search`.
#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct MessageSearchQuery {
    /// Free-form search text. Matched against message bodies/captions
    /// via the backend's full-text index (LIKE fallback).
    pub q: String,

    /// Page size (default 20, max 200).
    pub limit: Option<i64>,

    /// Rows to skip (default 0).
    pub offset: Option<i64>,
}

/// Query params for the fleet-wide `GET /api/v1/messages/search`.
#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct MessageFleetSearchQuery {
    /// Free-form search text.
    pub q: String,

    /// Restrict to this session id (all sessions when omitted).
    pub session: Option<String>,

    /// Page size (default 20, max 200).
    pub limit: Option<i64>,

    /// Rows to skip (default 0).
    pub offset: Option<i64>,
}
