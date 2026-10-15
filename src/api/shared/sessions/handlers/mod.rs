//! Session handlers grouped by concern. Everything is re-exported here, so
//! callers keep using `sessions::handlers::<name>`.
//!
//! - [`crud`] — create, list, read and delete sessions, plus status, diagnostics and device info.
//! - [`connection`] — connect, pair, QR, disconnect, unlink and logout, and the client connect/guard helpers.
//! - [`transfer`] — session export and import as a zip of the session directory.
//! - [`lifecycle`] — startup reconnect, the stale-socket watchdog and forced client rebuilds.
//! - [`events`] — whatsapp-rust event handling: persistence, webhook payload shaping and message extraction.

mod connection;
mod crud;
mod events;
mod lifecycle;
mod transfer;

pub use connection::*;
pub use crud::*;
pub(crate) use events::*;
pub use lifecycle::*;
pub use transfer::*;
