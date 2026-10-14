//! Waxum library surface. Re-exports the top-level modules so integration
//! tests under `tests/` can build the same router and state the binary
//! uses. The binary in `src/main.rs` also depends on these modules.

pub mod api;
pub mod bootstrap;
pub mod cloud;
pub mod console;
pub mod db;
pub mod device_props;
pub mod error;
pub mod link_preview;
pub mod metrics;
pub mod middleware;
pub mod nats;
pub mod net;
pub mod net_guard;
pub mod preflight;
pub mod state;
pub mod storage;
