//! Models for the scheduled-send feature.
//!
//! Every send endpoint accepts an optional `send_at` ISO-8601 UTC
//! timestamp. When it lies in the future the handler parks the request
//! body in the `scheduled_messages` table instead of sending, and the
//! background scheduler (see [`crate::handlers::schedule`]) dispatches
//! it once due. The types below cover the parked-row representation,
//! the management-endpoint query/response shapes, and the unified send
//! response that distinguishes an immediate send from a scheduled one.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Lifecycle of a parked scheduled message.
///
/// `pending` — waiting for its `send_at`; `sending` — claimed by the
/// scheduler and mid-dispatch (transient); `sent` — delivered,
/// `message_id` set; `failed` — dispatch errored, `error` set;
/// `cancelled` — revoked via the DELETE endpoint before it fired.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScheduledStatus {
    Pending,
    Sending,
    Sent,
    Failed,
    Cancelled,
}

impl ScheduledStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ScheduledStatus::Pending => "pending",
            ScheduledStatus::Sending => "sending",
            ScheduledStatus::Sent => "sent",
            ScheduledStatus::Failed => "failed",
            ScheduledStatus::Cancelled => "cancelled",
        }
    }

    /// Parse a stored status string back into the enum. Unknown values
    /// (older rows, hand-edited DB) degrade to `pending` so they get
    /// re-examined by the scheduler rather than silently dropped.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "sending" => ScheduledStatus::Sending,
            "sent" => ScheduledStatus::Sent,
            "failed" => ScheduledStatus::Failed,
            "cancelled" => ScheduledStatus::Cancelled,
            _ => ScheduledStatus::Pending,
        }
    }
}

/// A parked scheduled message as returned by the management endpoints.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ScheduledMessage {
    #[schema(example = "b3f1c2a4-1234-4cde-9f00-abcdef123456")]
    pub id: String,

    #[schema(example = "main")]
    pub session_id: String,

    /// Send endpoint the body will be dispatched to, e.g. `text`,
    /// `image`, `cta-url`.
    #[schema(example = "text")]
    pub endpoint: String,

    /// UTC time the message becomes due.
    #[schema(example = "2026-01-01T12:00:00Z")]
    pub send_at: DateTime<Utc>,

    pub status: ScheduledStatus,

    /// Dispatch error, present only when `status` is `failed`.
    pub error: Option<String>,

    /// WhatsApp message id, present only when `status` is `sent`.
    #[schema(example = "3EB0C8F1A2B3C4D5E6")]
    pub message_id: Option<String>,

    #[schema(example = "2025-12-31T10:00:00Z")]
    pub created_at: DateTime<Utc>,

    #[schema(example = "2025-12-31T10:00:00Z")]
    pub updated_at: DateTime<Utc>,
}

/// List response for the scheduled-message management endpoints.
#[derive(Debug, Serialize, ToSchema)]
pub struct ScheduledListResponse {
    pub messages: Vec<ScheduledMessage>,

    #[schema(example = 3)]
    pub count: usize,
}

/// Query filter for `GET /api/v1/sessions/{session_id}/scheduled`.
#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ScheduledSessionQuery {
    /// Only return rows with this status (pending, sending, sent,
    /// failed, cancelled).
    pub status: Option<String>,
}

/// Query filter for the fleet-wide `GET /api/v1/scheduled`.
#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ScheduledFleetQuery {
    /// Only return rows belonging to this session id.
    pub session: Option<String>,

    /// Only return rows with this status (pending, sending, sent,
    /// failed, cancelled).
    pub status: Option<String>,
}

/// Unified response returned by every send endpoint.
///
/// When the request carried no future `send_at` the message goes out
/// immediately: `status` is `sent` and `message_id`, `timestamp` and
/// `to` are populated — the same fields the endpoint returned before
/// scheduling existed. When a future `send_at` was supplied the message
/// is parked instead: `status` is `pending` and `schedule_id` /
/// `send_at` identify the scheduler row.
#[derive(Debug, Serialize, ToSchema)]
pub struct SendResponse {
    /// `sent` when delivered immediately, `pending` when scheduled.
    #[schema(example = "sent")]
    pub status: String,

    #[schema(example = "3EB0C8F1A2B3C4D5E6")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<i64>,

    #[schema(example = "559999999999@s.whatsapp.net")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,

    #[schema(example = "b3f1c2a4-1234-4cde-9f00-abcdef123456")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule_id: Option<String>,

    #[schema(example = "2026-01-01T12:00:00Z")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub send_at: Option<DateTime<Utc>>,
}

impl SendResponse {
    /// Wrap the result of an immediate send.
    pub fn sent(resp: crate::models::messages::MessageResponse) -> Self {
        SendResponse {
            status: "sent".to_string(),
            message_id: Some(resp.message_id),
            timestamp: Some(resp.timestamp),
            to: Some(resp.to),
            schedule_id: None,
            send_at: None,
        }
    }

    /// Build the reply for a freshly parked scheduled message.
    pub fn scheduled(schedule_id: String, send_at: DateTime<Utc>) -> Self {
        SendResponse {
            status: "pending".to_string(),
            message_id: None,
            timestamp: None,
            to: None,
            schedule_id: Some(schedule_id),
            send_at: Some(send_at),
        }
    }
}
