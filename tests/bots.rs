//! Contract tests for the bots handlers
//! (`GET /sessions/{id}/bots`, `GET /sessions/{id}/capping`,
//! `src/handlers/bots.rs`).
//!
//! Two GET routes, no body. `tests/presence.rs` is the worked example this
//! mirrors: auth, unknown session, and DB-only session all resolve through
//! the local `get_client`, which reads the live registry rather than the
//! `sessions` table, so a session row alone never turns a 503 into 200.
//!
//! `get_capping` is worth a note: it calls `get_client` but never touches
//! the client after — its 200 body is a static placeholder pointing callers
//! at `/mex/query` instead. The client gate still runs first, so the 503
//! contract is identical to `list_bots`.

mod common;

use axum::http::{Method, StatusCode};
use common::{call, req_get, req_json, Harness, TEST_TOKEN};
use serde_json::json;

const SESSION: &str = "s-bots";
const ROUTES: [&str; 2] = ["/bots", "/capping"];

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
async fn every_route_requires_a_token() {
    let h = Harness::new().await;

    for suffix in ROUTES {
        let path = format!("/api/v1/sessions/{SESSION}{suffix}");
        let (status, _) = call(&h.app, req_get(&path, None)).await;

        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "{suffix} must be rejected without a bearer token"
        );
    }
}

#[tokio::test]
async fn every_route_returns_503_for_an_unknown_session() {
    let h = Harness::new().await;

    for suffix in ROUTES {
        let path = format!("/api/v1/sessions/does-not-exist{suffix}");
        let (status, _) = call(&h.app, req_get(&path, Some(TEST_TOKEN))).await;

        assert_eq!(
            status,
            StatusCode::SERVICE_UNAVAILABLE,
            "{suffix} must report an unknown session as not connected"
        );
    }
}

/// The session exists in the database but has no live client, so
/// `get_client` still fails — 503, never 404.
#[tokio::test]
async fn every_route_rejects_a_session_that_never_connected() {
    let h = Harness::new().await;
    seed_session(&h, SESSION).await;

    for suffix in ROUTES {
        let path = format!("/api/v1/sessions/{SESSION}{suffix}");
        let (status, _) = call(&h.app, req_get(&path, Some(TEST_TOKEN))).await;

        assert_eq!(
            status,
            StatusCode::SERVICE_UNAVAILABLE,
            "{suffix}: a DB-only session has no live client"
        );
    }
}
