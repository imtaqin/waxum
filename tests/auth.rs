//! Contract test for the auth gate. Every route under `/api/v1/*` must
//! reject a missing or wrong bearer token; the four probe endpoints
//! (`/livez`, `/readyz`, `/health`, `/metrics`) must not.
mod common;

use axum::http::StatusCode;
use common::{call, req_get, Harness, TEST_TOKEN};

#[tokio::test]
async fn api_v1_rejects_missing_token() {
    let h = Harness::new().await;
    let (status, _) = call(&h.app, req_get("/api/v1/sessions", None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn api_v1_rejects_wrong_token() {
    let h = Harness::new().await;
    let (status, _) = call(&h.app, req_get("/api/v1/sessions", Some("nope"))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn api_v1_accepts_superadmin_token() {
    let h = Harness::new().await;
    let (status, _) = call(&h.app, req_get("/api/v1/sessions", Some(TEST_TOKEN))).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn probes_bypass_auth() {
    let h = Harness::new().await;
    for path in ["/livez", "/readyz", "/health", "/metrics"] {
        let (status, _) = call(&h.app, req_get(path, None)).await;
        assert_eq!(status, StatusCode::OK, "{} should bypass auth", path);
    }
}
