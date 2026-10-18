//! Background tasks every waxum process runs next to its API: startup
//! reconnect, the reconnect watchdog, the scheduled-send dispatcher and
//! the blast worker. Shared by the server binary and [`crate::embedded`].

use crate::api::shared::{blast, schedule, sessions};
use crate::state::AppState;

/// Spawns the background tasks on the current tokio runtime.
pub fn spawn(state: &AppState) {
    tokio::spawn(sessions::handlers::reconnect_all_on_startup(state.clone()));
    tokio::spawn(sessions::handlers::run_reconnect_watchdog(state.clone()));
    tokio::spawn(schedule::handlers::run_scheduler(state.clone()));
    tokio::spawn(blast::handlers::run_blast_worker(state.clone()));
}
