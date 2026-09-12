//! Contract test for the privacy handler
//! (`GET /sessions/{id}/privacy/settings`, `src/handlers/privacy.rs`).
//!
//! One route, no body. `tests/presence.rs` is the worked example this
//! mirrors: auth, unknown session, and DB-only session all resolve through
//! the local `get_client`, which reads the live registry rather than the
//! `sessions` table, so a session row alone never turns a 503 into 200.

mod common;

use axum::http::{Method, StatusCode};
use common::{call, req_get, req_json, Harness, TEST_TOKEN};
use serde_json::json;

const PATH: &str = "/api/v1/sessions/s-privacy/privacy/settings";

/// Creates a session row (no live client is ever attached in tests).
async fn seed_session(h: &Harness, id: &str) {
    let (status, body) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions",
            Some(TEST_TOKEN),
            json!({ "id": id }),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "seeding session {id} should succeed: {body:?}"
    );
}

#[tokio::test]
async fn get_privacy_settings_requires_a_token() {
    let h = Harness::new().await;

    let (status, _) = call(&h.app, req_get(PATH, None)).await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn get_privacy_settings_unknown_session_returns_503() {
    let h = Harness::new().await;

    let (status, _) = call(
        &h.app,
        req_get(
            "/api/v1/sessions/does-not-exist/privacy/settings",
            Some(TEST_TOKEN),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

/// The session exists in the database but has no live client, so
/// `get_client` still fails — 503, never 404.
#[tokio::test]
async fn get_privacy_settings_rejects_a_session_that_never_connected() {
    let h = Harness::new().await;
    seed_session(&h, "s-privacy").await;

    let (status, _) = call(&h.app, req_get(PATH, Some(TEST_TOKEN))).await;

    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "a DB-only session has no live client"
    );
}
