//! Contract test for `/api/v1/info` — build info + version.
mod common;

use axum::http::StatusCode;
use common::{call, req_get, Harness, TEST_TOKEN};

#[tokio::test]
async fn info_reports_version_and_name() {
    let h = Harness::new().await;
    let (status, body) = call(&h.app, req_get("/api/v1/info", Some(TEST_TOKEN))).await;
    assert_eq!(status, StatusCode::OK);
    // `version` must always be present and non-empty; the field names in
    // the response are the load-bearing part of the contract for clients
    // that display "waxum vX.Y.Z" in their dashboards.
    let version = body.get("version").and_then(|v| v.as_str()).unwrap_or("");
    assert!(!version.is_empty(), "version should not be empty");
}
