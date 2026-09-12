//! Contract tests for the newsletter handlers
//! (`src/handlers/newsletter.rs`, eleven routes under
//! `/api/v1/sessions/{session_id}/newsletters`).
//!
//! # How this file is organised
//!
//! `tests/presence.rs` is the worked example for this kind of module; read
//! its doc comment first. It covers two routes, so it spells every case out
//! as its own test. Newsletter has eleven, and the first three assertions
//! are identical for all of them:
//!
//! 1. **Auth** — no bearer token is rejected by the JWT middleware.
//! 2. **Unknown session** — `503 SERVICE_UNAVAILABLE`.
//! 3. **DB-only session** — a session row that never connected is *also*
//!    503, never 404, because `get_client` reads the live registry and not
//!    the `sessions` table.
//!
//! So those three run as table sweeps over the routes table, and each
//! failure names the method and path it came from. The fourth assertion —
//!
//! 4. **Body extraction** — `Json<T>` / `Query<T>` are the last extractors,
//!    so they run *before* the handler body and reject a bad request without
//!    the client gate being consulted at all
//!
//! — depends on the individual request type, so it gets one named test per
//! interesting case below the sweeps.
//!
//! Everything past the `get_client` gate needs a connected WhatsApp client
//! and is out of scope. Assert the HTTP contract, not the protocol
//! behaviour.

mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use common::{call, req_delete, req_get, req_json, Harness, TEST_TOKEN};
use serde_json::{json, Value};

const SESSION: &str = "s-newsletter";
const NEWSLETTER_JID: &str = "120363000000000000@newsletter";
const USER_JID: &str = "559999999999@s.whatsapp.net";

/// Every newsletter route, as (method, path suffix, request body).
///
/// The suffix is appended to `/api/v1/sessions/{session_id}`. A `None` body
/// marks a handler with no `Json` extractor in its signature.
fn routes() -> Vec<(Method, String, Option<Value>)> {
    let user = json!({ "user": USER_JID });
    vec![
        (Method::GET, "/newsletters/subscribed".to_string(), None),
        (
            Method::POST,
            "/newsletters".to_string(),
            Some(json!({ "name": "Test Newsletter" })),
        ),
        (
            Method::GET,
            format!("/newsletters/{NEWSLETTER_JID}/metadata"),
            None,
        ),
        (
            Method::POST,
            format!("/newsletters/{NEWSLETTER_JID}/join"),
            None,
        ),
        (
            Method::POST,
            format!("/newsletters/{NEWSLETTER_JID}/leave"),
            None,
        ),
        (
            Method::DELETE,
            format!("/newsletters/{NEWSLETTER_JID}"),
            None,
        ),
        (
            Method::POST,
            format!("/newsletters/{NEWSLETTER_JID}/change-owner"),
            Some(user.clone()),
        ),
        (
            Method::POST,
            format!("/newsletters/{NEWSLETTER_JID}/demote"),
            Some(user),
        ),
        (
            Method::GET,
            format!("/newsletters/{NEWSLETTER_JID}/admin-info"),
            None,
        ),
        (
            Method::GET,
            format!("/newsletters/{NEWSLETTER_JID}/followers"),
            None,
        ),
        (
            Method::POST,
            format!("/newsletters/{NEWSLETTER_JID}/mute"),
            Some(json!({ "muted": true })),
        ),
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
        None if method == Method::GET => req_get(&path, token),
        None if method == Method::DELETE => req_delete(&path, token),
        None => req_json(method, &path, token, json!({})),
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
/// still fails. This is the distinction most worth pinning down: creating the
/// session first does not turn the 503 into a 404 or a success.
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

/// `get_client` runs before `parse_jid`, so an unparseable newsletter JID on
/// a disconnected session is still reported as 503 and not as a bad request.
#[tokio::test]
async fn the_client_gate_runs_before_newsletter_jid_parsing() {
    let h = Harness::new().await;
    seed_session(&h, SESSION).await;

    let (status, _) = call(
        &h.app,
        req_get(
            &format!("/api/v1/sessions/{SESSION}/newsletters/not-a-jid/metadata"),
            Some(TEST_TOKEN),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

/// `/newsletters/subscribed` is a static segment declared alongside the
/// `/newsletters/{jid}/...` family, so it must keep routing to
/// `list_subscribed` rather than being swallowed as a JID.
#[tokio::test]
async fn the_subscribed_route_is_not_shadowed_by_the_jid_routes() {
    let h = Harness::new().await;

    let (status, _) = call(
        &h.app,
        req_get(
            &format!("/api/v1/sessions/{SESSION}/newsletters/subscribed"),
            Some(TEST_TOKEN),
        ),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "the route must resolve and stop at the client gate, not 404 or 405"
    );
}

#[tokio::test]
async fn create_newsletter_rejects_a_body_without_name() {
    let h = Harness::new().await;
    seed_session(&h, SESSION).await;

    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            &format!("/api/v1/sessions/{SESSION}/newsletters"),
            Some(TEST_TOKEN),
            json!({ "description": "no name given" }),
        ),
    )
    .await;

    assert_ne!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "extraction must fail before the client gate is reached"
    );
    assert!(
        status.is_client_error(),
        "a missing `name` is a client error, got {status}"
    );
}

#[tokio::test]
async fn change_owner_rejects_a_body_without_user() {
    let h = Harness::new().await;
    seed_session(&h, SESSION).await;

    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            &format!("/api/v1/sessions/{SESSION}/newsletters/{NEWSLETTER_JID}/change-owner"),
            Some(TEST_TOKEN),
            json!({}),
        ),
    )
    .await;

    assert!(
        status.is_client_error(),
        "a missing `user` is a client error, got {status}"
    );
}

#[tokio::test]
async fn set_mute_rejects_a_non_boolean_muted() {
    let h = Harness::new().await;
    seed_session(&h, SESSION).await;

    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            &format!("/api/v1/sessions/{SESSION}/newsletters/{NEWSLETTER_JID}/mute"),
            Some(TEST_TOKEN),
            json!({ "muted": "yes" }),
        ),
    )
    .await;

    assert_ne!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "extraction must fail before the client gate is reached"
    );
    assert!(
        status.is_client_error(),
        "a non-boolean `muted` is a client error, got {status}"
    );
}

/// The followers route is the only one taking a `Query`, and `limit` is typed
/// `Option<u32>`, so a non-numeric value is rejected at extraction time
/// exactly like a malformed body would be.
#[tokio::test]
async fn get_followers_rejects_a_non_numeric_limit() {
    let h = Harness::new().await;
    seed_session(&h, SESSION).await;

    let (status, _) = call(
        &h.app,
        req_get(
            &format!("/api/v1/sessions/{SESSION}/newsletters/{NEWSLETTER_JID}/followers?limit=all"),
            Some(TEST_TOKEN),
        ),
    )
    .await;

    assert_ne!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "extraction must fail before the client gate is reached"
    );
    assert!(
        status.is_client_error(),
        "a non-numeric `limit` is a client error, got {status}"
    );
}
