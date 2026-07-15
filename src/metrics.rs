//! Prometheus text-exposition on `GET /metrics`.
//!
//! Bypasses the JWT middleware so a scraper can poll without a token; put
//! the endpoint behind a network ACL if that's a concern.
//!
//! Gauges:
//!
//! - `waxum_sessions_total` — number of session runtimes resident in
//!   memory.
//! - `waxum_sessions_live` — sessions whose upstream client agrees it's
//!   connected AND logged in (source of truth for /status).
//! - `waxum_process_threads` — thread count from `/proc/self/status`.
//! - `waxum_process_open_fds` — FD count from `/proc/self/fd`.
//! - `waxum_webhook_circuits_open` — webhook target URLs currently in the
//!   OPEN circuit-breaker state; alert when this is non-zero for long.

use axum::{extract::State, http::StatusCode, response::IntoResponse};
use prometheus::{Encoder, IntGauge, Registry, TextEncoder};
use std::sync::OnceLock;

use crate::state::AppState;

static REGISTRY: OnceLock<Registry> = OnceLock::new();
static SESSIONS_TOTAL: OnceLock<IntGauge> = OnceLock::new();
static SESSIONS_LIVE: OnceLock<IntGauge> = OnceLock::new();
static PROCESS_THREADS: OnceLock<IntGauge> = OnceLock::new();
static PROCESS_OPEN_FDS: OnceLock<IntGauge> = OnceLock::new();
static WEBHOOK_CIRCUITS_OPEN: OnceLock<IntGauge> = OnceLock::new();

fn registry() -> &'static Registry {
    REGISTRY.get_or_init(|| {
        let r = Registry::new();
        let sessions_total = IntGauge::new(
            "waxum_sessions_total",
            "Total session runtimes resident in the gateway",
        )
        .unwrap();
        let sessions_live = IntGauge::new(
            "waxum_sessions_live",
            "Sessions whose underlying client reports connected + logged in",
        )
        .unwrap();
        let process_threads = IntGauge::new(
            "waxum_process_threads",
            "Thread count for the waxum process (from /proc/self/status)",
        )
        .unwrap();
        let process_open_fds = IntGauge::new(
            "waxum_process_open_fds",
            "Open file descriptor count for the waxum process",
        )
        .unwrap();
        let webhook_circuits_open = IntGauge::new(
            "waxum_webhook_circuits_open",
            "Webhook target URLs currently in open-circuit state (skipped)",
        )
        .unwrap();
        r.register(Box::new(sessions_total.clone())).unwrap();
        r.register(Box::new(sessions_live.clone())).unwrap();
        r.register(Box::new(process_threads.clone())).unwrap();
        r.register(Box::new(process_open_fds.clone())).unwrap();
        r.register(Box::new(webhook_circuits_open.clone())).unwrap();
        SESSIONS_TOTAL.set(sessions_total).ok();
        SESSIONS_LIVE.set(sessions_live).ok();
        PROCESS_THREADS.set(process_threads).ok();
        PROCESS_OPEN_FDS.set(process_open_fds).ok();
        WEBHOOK_CIRCUITS_OPEN.set(webhook_circuits_open).ok();
        r
    })
}

fn read_proc_threads() -> Option<i64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix("Threads:") {
            return rest.trim().parse().ok();
        }
    }
    None
}

fn read_proc_open_fds() -> Option<i64> {
    let entries = std::fs::read_dir("/proc/self/fd").ok()?;
    Some(entries.count() as i64)
}

pub async fn metrics_handler(State(state): State<AppState>) -> impl IntoResponse {
    let reg = registry();

    let mut total = 0i64;
    let mut live = 0i64;
    for s in state.session_iter() {
        total += 1;
        if s.is_alive() {
            live += 1;
        }
    }
    SESSIONS_TOTAL.get().unwrap().set(total);
    SESSIONS_LIVE.get().unwrap().set(live);
    if let Some(t) = read_proc_threads() {
        PROCESS_THREADS.get().unwrap().set(t);
    }
    if let Some(f) = read_proc_open_fds() {
        PROCESS_OPEN_FDS.get().unwrap().set(f);
    }
    WEBHOOK_CIRCUITS_OPEN
        .get()
        .unwrap()
        .set(state.webhook_circuits_open_count() as i64);

    let encoder = TextEncoder::new();
    let metric_families = reg.gather();
    let mut buf = Vec::new();
    match encoder.encode(&metric_families, &mut buf) {
        Ok(()) => (
            StatusCode::OK,
            [("Content-Type", encoder.format_type().to_string())],
            buf,
        )
            .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}
