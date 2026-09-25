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

use crate::models::media::MediaType;

/// Download pointer for a media message, shaped to be passed straight
/// into `POST /sessions/{session_id}/media/download` as-is (same field
/// names and encodings as
/// [`crate::models::media::DownloadMediaRequest`]).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MessageMedia {
    pub direct_path: String,
    pub media_key: String,
    pub file_sha256: String,
    pub file_enc_sha256: String,
    pub file_length: u64,
    pub media_type: MediaType,
    pub mimetype: String,
}

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

    /// Sender's WhatsApp display name, from the `contacts` table.
    /// Only populated by `GET /messages/chat/{chat_jid}` — always
    /// `null` from `/messages/search`.
    #[schema(example = "Jane Doe")]
    pub push_name: Option<String>,

    /// Download pointer, present when `msg_type` is a media type.
    /// Only populated by `GET /messages/chat/{chat_jid}` — always
    /// `null` from `/messages/search`.
    pub media: Option<MessageMedia>,

    /// WhatsApp message id this message is replying to
    /// (`ContextInfo.stanzaId`). `null` when the message is not a
    /// reply. Only populated by `GET /messages/chat/{chat_jid}` —
    /// always `null` from `/messages/search`.
    #[schema(example = "3EB0C8F1A2B3C4D5E6")]
    pub quoted_message_id: Option<String>,

    /// Sender of the quoted message (`ContextInfo.participant`).
    /// `null` when not a reply, or when WhatsApp omitted the field.
    /// Only populated by `GET /messages/chat/{chat_jid}` — always
    /// `null` from `/messages/search`.
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub quoted_sender_jid: Option<String>,
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

/// Query params for `GET /api/v1/sessions/{session_id}/messages/chat/{chat_jid}`.
#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ChatMessagesQuery {
    /// Page size (default 20, max 200).
    pub limit: Option<i64>,

    /// Rows to skip (default 0).
    pub offset: Option<i64>,
}

/// Query params for `GET /api/v1/sessions/{session_id}/messages`.
#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct SessionMessagesQuery {
    /// Keyset cursor: the `seq` of the last message on the previous page,
    /// taken verbatim from `next_cursor`. Omit for the newest page.
    pub after: Option<i64>,

    /// Page size (default 20, max 200).
    pub limit: Option<i64>,
}

/// Session-wide message page in store-arrival order (newest first),
/// backed by the upstream chat store rather than the search index.
#[derive(Debug, Serialize, ToSchema)]
pub struct SessionMessagesResponse {
    pub messages: Vec<MessageHit>,

    /// Messages in THIS page.
    #[schema(example = 20)]
    pub count: usize,

    /// Pass as `after` to fetch the next (older) page. Null on a short
    /// page, which means the end of the stored history.
    #[schema(example = 421)]
    pub next_cursor: Option<i64>,
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
