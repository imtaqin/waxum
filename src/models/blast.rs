//! Models for the blast (bulk-send) feature.
//!
//! A blast job fans one message payload out to many recipients with
//! pacing (per-send `delay_ms` plus optional random `jitter_ms`), retry
//! with an attempts cap, a dead-letter state for exhausted recipients,
//! and optional cross-job dedup so repeat campaigns never re-send to a
//! recipient a previous blast already delivered to.
//!
//! The types below cover the create request/response, the stored job
//! and recipient representations returned by the management endpoints,
//! and the query filters for the list endpoints. Persistence lives in
//! [`crate::db::blast`]; the worker and HTTP handlers live in
//! [`crate::handlers::blast`].

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Lifecycle of a blast job.
///
/// `pending` — created (optionally waiting for its `send_at`);
/// `running` — claimed by the worker, sends in flight; `completed` —
/// every recipient terminal and none failed; `completed_with_failures`
/// — finished but at least one recipient landed in the DLQ;
/// `cancelled` — revoked via the cancel endpoint, remaining recipients
/// never send; `failed` — the job itself broke (e.g. its stored body no
/// longer deserializes), individual sends are not retried.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BlastJobStatus {
    Pending,
    Running,
    Completed,
    CompletedWithFailures,
    Cancelled,
    Failed,
}

impl BlastJobStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            BlastJobStatus::Pending => "pending",
            BlastJobStatus::Running => "running",
            BlastJobStatus::Completed => "completed",
            BlastJobStatus::CompletedWithFailures => "completed_with_failures",
            BlastJobStatus::Cancelled => "cancelled",
            BlastJobStatus::Failed => "failed",
        }
    }

    /// Parse a stored status string back into the enum. Unknown values
    /// (older rows, hand-edited DB) degrade to `pending` so the worker
    /// re-examines the job rather than silently dropping it.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "running" => BlastJobStatus::Running,
            "completed" => BlastJobStatus::Completed,
            "completed_with_failures" => BlastJobStatus::CompletedWithFailures,
            "cancelled" => BlastJobStatus::Cancelled,
            "failed" => BlastJobStatus::Failed,
            _ => BlastJobStatus::Pending,
        }
    }

    /// True once the job will never send again on its own.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            BlastJobStatus::Completed
                | BlastJobStatus::CompletedWithFailures
                | BlastJobStatus::Cancelled
                | BlastJobStatus::Failed
        )
    }
}

/// Lifecycle of one recipient inside a blast job.
///
/// `pending` — awaiting a send attempt; `sending` — claimed by the
/// worker mid-dispatch (transient); `sent` — delivered, `message_id`
/// set; `failed` — reserved terminal failure state (the retry endpoint
/// requeues it); `dlq` — exhausted `max_attempts`, `last_error` holds
/// the final dispatch error; `skipped_dup` — filtered at creation time
/// by intra-array or cross-job dedup, never sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BlastRecipientStatus {
    Pending,
    Sending,
    Sent,
    Failed,
    Dlq,
    SkippedDup,
}

impl BlastRecipientStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            BlastRecipientStatus::Pending => "pending",
            BlastRecipientStatus::Sending => "sending",
            BlastRecipientStatus::Sent => "sent",
            BlastRecipientStatus::Failed => "failed",
            BlastRecipientStatus::Dlq => "dlq",
            BlastRecipientStatus::SkippedDup => "skipped_dup",
        }
    }

    /// Parse a stored status string; unknown values degrade to
    /// `pending` so the worker re-examines them.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "sending" => BlastRecipientStatus::Sending,
            "sent" => BlastRecipientStatus::Sent,
            "failed" => BlastRecipientStatus::Failed,
            "dlq" => BlastRecipientStatus::Dlq,
            "skipped_dup" => BlastRecipientStatus::SkippedDup,
            _ => BlastRecipientStatus::Pending,
        }
    }
}

/// Per-job tuning knobs, stored as JSON text on the job row so the
/// worker can re-read them after a restart.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
pub struct BlastOptions {
    /// Base pause between consecutive sends, in milliseconds.
    #[schema(example = 1000)]
    pub delay_ms: u64,

    /// Random extra pause uniformly drawn from `0..=jitter_ms` per send.
    #[schema(example = 250)]
    pub jitter_ms: u64,

    /// Total send attempts per recipient before it lands in the DLQ.
    #[schema(example = 3)]
    pub max_attempts: u32,

    /// Skip recipients any previous blast of the same session already
    /// delivered to (status `sent`).
    pub dedup_across_jobs: bool,
}

impl Default for BlastOptions {
    fn default() -> Self {
        BlastOptions {
            delay_ms: 1000,
            jitter_ms: 0,
            max_attempts: 3,
            dedup_across_jobs: false,
        }
    }
}

/// Request body for `POST /api/v1/sessions/{session_id}/blast`.
///
/// `body` must match the request shape of `endpoint` (e.g. for `text`
/// a `SendTextRequest`). The `to` field inside `body` is required for
/// shape validation but is overridden per recipient at send time, and
/// any `send_at` inside `body` is forced to `null` so a blast never
/// round-trips through the scheduler.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateBlastRequest {
    /// Send endpoint key, same keys the scheduler dispatch uses:
    /// `text`, `image`, `cta-url`, ...
    #[schema(example = "text")]
    pub endpoint: String,

    /// Request payload matching the endpoint's request struct.
    #[schema(example = json!({"to": "placeholder@s.whatsapp.net", "text": "Hello!"}))]
    pub body: serde_json::Value,

    /// Recipients as JIDs or bare phone numbers.
    #[schema(example = json!(["559999999999@s.whatsapp.net", "558888888888"]))]
    pub recipients: Vec<String>,

    /// Base pause between sends in ms (default 1000).
    pub delay_ms: Option<u64>,

    /// Random extra 0..=jitter_ms pause per send (default 0).
    pub jitter_ms: Option<u64>,

    /// Attempts per recipient before DLQ (default 3).
    pub max_attempts: Option<u32>,

    /// Skip recipients already `sent` by a previous blast of this
    /// session (default false).
    pub dedup_across_jobs: Option<bool>,

    /// Optional delayed start: the job stays `pending` until this UTC
    /// time, then the worker picks it up.
    #[schema(example = "2026-01-01T12:00:00Z")]
    pub send_at: Option<DateTime<Utc>>,
}

impl CreateBlastRequest {
    /// Resolve the effective options, applying defaults for unset knobs.
    pub fn options(&self) -> BlastOptions {
        let d = BlastOptions::default();
        BlastOptions {
            delay_ms: self.delay_ms.unwrap_or(d.delay_ms),
            jitter_ms: self.jitter_ms.unwrap_or(d.jitter_ms),
            max_attempts: self.max_attempts.unwrap_or(d.max_attempts),
            dedup_across_jobs: self.dedup_across_jobs.unwrap_or(d.dedup_across_jobs),
        }
    }
}

/// Response for a freshly created blast job.
#[derive(Debug, Serialize, ToSchema)]
pub struct CreateBlastResponse {
    #[schema(example = "b3f1c2a4-1234-4cde-9f00-abcdef123456")]
    pub job_id: String,

    /// Recipients accepted into the job (after dedup).
    #[schema(example = 1500)]
    pub total: usize,

    /// Recipients skipped as duplicates (intra-array repeats plus,
    /// when enabled, cross-job already-sent).
    #[schema(example = 12)]
    pub skipped_dup: usize,

    /// Always `pending` at creation; the worker claims the job when
    /// due.
    #[schema(example = "pending")]
    pub status: String,
}

/// A blast job as returned by the management endpoints.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BlastJob {
    #[schema(example = "b3f1c2a4-1234-4cde-9f00-abcdef123456")]
    pub id: String,

    #[schema(example = "main")]
    pub session_id: String,

    /// Send endpoint the stored body dispatches to.
    #[schema(example = "text")]
    pub endpoint: String,

    pub status: BlastJobStatus,

    /// Pacing / retry / dedup knobs captured at creation time.
    pub options: BlastOptions,

    #[schema(example = 1500)]
    pub total: i64,

    #[schema(example = 1490)]
    pub sent_count: i64,

    /// Failed send attempts so far (transient retries included).
    #[schema(example = 4)]
    pub failed_count: i64,

    /// Recipients that exhausted all attempts and sit in the DLQ.
    #[schema(example = 2)]
    pub dlq_count: i64,

    /// Recipients skipped at creation as duplicates.
    #[schema(example = 12)]
    pub skipped_dup_count: i64,

    /// Delayed start requested at creation, if any.
    #[schema(example = "2026-01-01T12:00:00Z")]
    pub send_at: Option<DateTime<Utc>>,

    #[schema(example = "2025-12-31T10:00:00Z")]
    pub created_at: DateTime<Utc>,

    #[schema(example = "2025-12-31T10:00:05Z")]
    pub started_at: Option<DateTime<Utc>>,

    #[schema(example = "2025-12-31T10:30:00Z")]
    pub finished_at: Option<DateTime<Utc>>,
}

/// List response for the blast-job management endpoints.
#[derive(Debug, Serialize, ToSchema)]
pub struct BlastJobListResponse {
    pub jobs: Vec<BlastJob>,

    #[schema(example = 3)]
    pub count: usize,
}

/// One recipient row as returned by the recipients listing endpoint.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BlastRecipient {
    #[schema(example = 42)]
    pub id: i64,

    #[schema(example = "559999999999@s.whatsapp.net")]
    pub recipient: String,

    pub status: BlastRecipientStatus,

    /// Send attempts made so far.
    #[schema(example = 1)]
    pub attempts: i64,

    /// Last dispatch error, present on `pending` (retry scheduled) and
    /// `dlq` rows.
    pub last_error: Option<String>,

    /// WhatsApp message id, present only when `status` is `sent`.
    #[schema(example = "3EB0C8F1A2B3C4D5E6")]
    pub message_id: Option<String>,

    #[schema(example = "2025-12-31T10:00:00Z")]
    pub updated_at: DateTime<Utc>,
}

/// Paginated recipient listing.
#[derive(Debug, Serialize, ToSchema)]
pub struct BlastRecipientListResponse {
    pub recipients: Vec<BlastRecipient>,

    #[schema(example = 50)]
    pub count: usize,
}

/// Query filter for `GET /api/v1/sessions/{session_id}/blasts`.
#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct BlastSessionQuery {
    /// Only return jobs with this status (pending, running, completed,
    /// completed_with_failures, cancelled, failed).
    pub status: Option<String>,
}

/// Query filter for the fleet-wide `GET /api/v1/blasts`.
#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct BlastFleetQuery {
    /// Only return jobs belonging to this session id.
    pub session: Option<String>,

    /// Only return jobs with this status.
    pub status: Option<String>,
}

/// Query filter for the paginated recipients listing.
#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct BlastRecipientsQuery {
    /// Only return recipients with this status (pending, sending, sent,
    /// failed, dlq, skipped_dup).
    pub status: Option<String>,

    /// Page size (default 100, max 1000).
    pub limit: Option<i64>,

    /// Rows to skip (default 0).
    pub offset: Option<i64>,
}
