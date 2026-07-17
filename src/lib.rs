//! Waxum library surface. Re-exports the top-level modules so integration
//! tests under `tests/` can build the same router and state the binary
//! uses. The binary in `src/main.rs` also depends on these modules.

pub mod db;
pub mod device_props;
pub mod error;
pub mod handlers;
pub mod metrics;
pub mod middleware;
pub mod models;
pub mod nats;
pub mod net;
pub mod preflight;
pub mod routes;
pub mod state;
