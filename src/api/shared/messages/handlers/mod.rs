//! Send handlers for every message type, grouped by topic. Each `send_*` is the
//! HTTP entry point; `execute_*` is the shared core also used by scheduled
//! and blast sends. Everything is re-exported here, so callers keep using
//! `messages::handlers::<name>`.

mod calls;
mod common;
mod contact_location;
mod interactive;
mod media;
mod newsletter;
mod payments;
mod text;

pub use calls::*;
pub(crate) use common::*;
pub use contact_location::*;
pub use interactive::*;
pub use media::*;
pub use newsletter::*;
pub use payments::*;
pub use text::*;
