//! Contract tests for the three health/probe endpoints and `/metrics`.
//! These endpoints bypass the JWT gate and never touch WA state, so they
//! are safe to run in every test binary without extra setup.
mod common;

use axum::http::StatusCode;
use common::{call, req_get, Harness};

#[tokio::test]
async fn livez_is_ok_without_auth() {
    let h = Harness::new().await;
    let (status, body) = call(&h.app, req_get("/livez", None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_str(), Some("OK"));
}

#[tokio::test]
async fn health_alias_is_ok_without_auth() {
    let h = Harness::new().await;
    let (status, body) = call(&h.app, req_get("/health", None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_str(), Some("OK"));
}

#[tokio::test]
async fn readyz_reports_db_ok_and_zero_sessions() {
    let h = Harness::new().await;
    let (status, body) = call(&h.app, req_get("/readyz", None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.get("db").and_then(|v| v.as_str()), Some("ok"));
    assert_eq!(body.get("sessions_known").and_then(|v| v.as_i64()), Some(0));
}

#[tokio::test]
async fn metrics_exposes_prometheus_text() {
    let h = Harness::new().await;
    let (status, body) = call(&h.app, req_get("/metrics", None)).await;
    assert_eq!(status, StatusCode::OK);
    let text = body.as_str().unwrap_or("");
    assert!(
        text.contains("waxum_sessions_total") || text.contains("# HELP"),
        "metrics body should contain the Prometheus text exposition: {}",
        text
    );
}
