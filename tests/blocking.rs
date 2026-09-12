//! Contract tests for the blocking handlers
//! (`src/handlers/blocking.rs`, four routes under
//! `/api/v1/sessions/{session_id}/blocking`, plus `is_blocked` which lives
//! under `.../blocking/check/{jid}`).
//!
//! `tests/presence.rs` is the worked example for this kind of module; read
//! its doc comment first. All four routes gate on `get_client` before
//! touching the JID (query/body/path, depending on the route), so the first
//! three assertions run as table sweeps: auth, unknown session, DB-only
//! session all resolve identically across the set. The fourth — body
//! extraction — only applies to the two `POST` routes, so it gets a named
//! test per route below the sweeps.

mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use common::{call, req_get, req_json, Harness, TEST_TOKEN};
use serde_json::{json, Value};

const SESSION: &str = "s-blocking";
const CONTACT_JID: &str = "559999999999@s.whatsapp.net";

/// Every blocking route, as (method, path suffix, request body).
///
/// The suffix is appended to `/api/v1/sessions/{session_id}`. A `None` body
/// marks a handler with no `Json` extractor in its signature.
fn routes() -> Vec<(Method, String, Option<Value>)> {
    vec![
        (Method::GET, "/blocking/list".to_string(), None),
        (
            Method::POST,
            "/blocking/block".to_string(),
            Some(json!({ "jid": CONTACT_JID })),
        ),
        (
            Method::POST,
            "/blocking/unblock".to_string(),
            Some(json!({ "jid": CONTACT_JID })),
        ),
        (Method::GET, format!("/blocking/check/{CONTACT_JID}"), None),
    ]
}

/// Builds a request for one routes-table entry against `session_id`.
fn request(
    method: Method,
    session_id: &str,
    suffix: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> Request<Body> {
    let path = format!("/api/v1/sessions/{session_id}{suffix}");
    match body {
        Some(body) => req_json(method, &path, token, body),
        None => req_get(&path, token),
    }
}

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

    for (method, suffix, body) in routes() {
        let (status, _) = call(
            &h.app,
            request(method.clone(), SESSION, &suffix, None, body),
        )
        .await;

        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "{method} {suffix} must be rejected without a bearer token"
        );
    }
}

#[tokio::test]
async fn every_route_returns_503_for_an_unknown_session() {
    let h = Harness::new().await;

    for (method, suffix, body) in routes() {
        let (status, _) = call(
            &h.app,
            request(
                method.clone(),
                "does-not-exist",
                &suffix,
                Some(TEST_TOKEN),
                body,
            ),
        )
        .await;

        assert_eq!(
            status,
            StatusCode::SERVICE_UNAVAILABLE,
            "{method} {suffix} must report an unknown session as not connected"
        );
    }
}

/// The session exists in the database but has no live client, so `get_client`
/// still fails. Every route here gates on the client before parsing the JID
/// (query/body/path), so a DB-only session is 503, never 404 or 400.
#[tokio::test]
async fn every_route_rejects_a_session_that_never_connected() {
    let h = Harness::new().await;
    seed_session(&h, SESSION).await;

    for (method, suffix, body) in routes() {
        let (status, _) = call(
            &h.app,
            request(method.clone(), SESSION, &suffix, Some(TEST_TOKEN), body),
        )
        .await;

        assert_eq!(
            status,
            StatusCode::SERVICE_UNAVAILABLE,
            "{method} {suffix} must treat a DB-only session as not connected"
        );
    }
}

#[tokio::test]
async fn block_contact_rejects_a_body_without_jid() {
    let h = Harness::new().await;
    seed_session(&h, SESSION).await;

    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            &format!("/api/v1/sessions/{SESSION}/blocking/block"),
            Some(TEST_TOKEN),
            json!({}),
        ),
    )
    .await;

    assert!(
        status.is_client_error(),
        "a missing `jid` is a client error, got {status}"
    );
}

#[tokio::test]
async fn unblock_contact_rejects_a_body_without_jid() {
    let h = Harness::new().await;
    seed_session(&h, SESSION).await;

    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            &format!("/api/v1/sessions/{SESSION}/blocking/unblock"),
            Some(TEST_TOKEN),
            json!({}),
        ),
    )
    .await;

    assert!(
        status.is_client_error(),
        "a missing `jid` is a client error, got {status}"
    );
}
