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
//! - [`blocking`], [`privacy`], [`calls`], [`status`], [`mex`], [`info`],
//!   [`fake_reply`] — smaller domains.

pub mod blocking;
pub mod bulk;
pub mod calls;
pub mod chatstate;
pub mod contacts;
pub mod fake_reply;
pub mod groups;
pub mod groups_management;
pub mod info;
pub mod media;
pub mod messages;
pub mod mex;
pub mod nats_handler;
pub mod operations;
pub mod presence;
pub mod privacy;
pub mod sessions;
pub mod status;
pub mod webhooks;
