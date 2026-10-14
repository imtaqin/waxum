//! REST handler modules, one per API surface.
//!
//! Every handler resolves its live [`whatsapp_rust::Client`] through
//! [`AppState::get_session`](crate::state::AppState::get_session) followed
//! by [`SessionState::get_live_client`](crate::state::SessionState::get_live_client)
//! — that's the single source of truth for "the session can accept a
//! write right now". Handlers never trust the cached status flag on its
//! own, so `/status` and `/messages/text` can never disagree.
//!
//! Modules:
//!
//! - [`sessions`] — CRUD, pair/connect/disconnect, QR polling, status.
//! - [`messages`] — every send variant (text, media, interactive, …).
//! - [`groups`], [`groups_management`] — group listing + admin ops.
//! - [`contacts`] — check on WhatsApp, list stored contacts.
//! - [`presence`], [`chatstate`] — typing / online indicators.
//! - [`media`] — upload / download binary payloads.
//! - [`webhooks`] — register / list / delete per-session webhook targets.
//! - [`nats_handler`] — inspect NATS streams + purge.
//! - [`operations`] — bulk ops, tctoken issue, auto-reconnect toggle.
//! - [`schedule`] — scheduled-send management endpoints + dispatcher loop.
//! - [`blast`] — bulk-send jobs (create/list/cancel/retry) + worker loop.
//! - [`search`] — message history ingestion + full-text search endpoints.
//! - [`blocking`], [`privacy`], [`calls`], [`status`], [`mex`], [`info`],
//!   [`fake_reply`] — smaller domains.

pub mod blocking;
pub mod bots;
pub mod business;
pub mod calls;
pub mod chatstate;
pub mod cloud;
pub mod cloud_admin;
pub mod cloud_commerce;
pub mod cloud_flows;
pub mod contacts;
pub mod fake_reply;
pub mod groups;
pub mod groups_management;
pub mod labels;
pub mod media;
pub mod mex;
pub mod newsletter;
pub mod operations;
pub mod presence;
pub mod privacy;
pub mod profile;
pub mod status;
