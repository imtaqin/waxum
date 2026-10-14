//! Endpoints that do not belong to one channel: they work on a session
//! whether it is a linked WhatsApp Web device or a Cloud API connection, or
//! they manage the gateway itself. One folder per feature, each with its
//! `handlers.rs`, `models.rs` (when it has request/response types) and
//! `routes.rs`.
//!
//! - [`sessions`] — CRUD, pair/connect/logout, QR polling, status, diagnostics.
//! - [`messages`] — every send variant; dispatches on the session's channel.
//! - [`webhooks`] — per-session webhook targets and their health.
//! - [`blast`], [`schedule`] — bulk-send jobs and scheduled sends.
//! - [`search`] — message history ingestion and full-text search.
//! - [`new_chats`] — new-chat counter and limit.
//! - [`bulk`], [`tags`], [`events`], [`tokens`], [`nats`], [`info`] — fleet
//!   operations, session tags, the event tail, API tokens, NATS inspection,
//!   build info.

pub mod blast;
pub mod bulk;
pub mod events;
pub mod info;
pub mod messages;
pub mod nats;
pub mod new_chats;
pub mod schedule;
pub mod search;
pub mod sessions;
pub mod tags;
pub mod tokens;
pub mod webhooks;
