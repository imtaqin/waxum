//! Contract tests for the presence handlers
//! (`POST /sessions/{id}/presence/set`, `POST /sessions/{id}/presence/subscribe`).
//!
//! # Worked example for new contributors
//!
//! This file is the reference pattern for the "cover an untested handler
//! module" good-first-issues. Almost every session-scoped handler in `waxum`
//! has the same shape, so the same four assertions apply:
//!
//! 1. **Auth** — the route sits under `/api/v1`, so a request with no bearer
//!    token is rejected by the JWT middleware before the handler ever runs.
//! 2. **Unknown session** — handlers resolve a live client through their local
//!    `get_client`, which returns `ApiError::NotConnected` (503) when the
//!    session is not in the live registry.
//! 3. **DB-only session** — a session that exists as a database row but has
//!    never connected is *also* 503, not 404. This is the distinction most
//!    worth pinning down: `get_client` consults the live registry, not the
//!    `sessions` table, so creating the session first does not change the
//!    outcome.
//! 4. **Body extraction** — `Json<T>` is the last extractor in the handler
//!    signature, so it runs *before* the handler body. A body that does not
//!    deserialize is therefore rejected by axum at extraction time, without
//!    the client gate being consulted at all.
//!
//! Everything past the `get_client` gate — actually setting presence, actually
//! subscribing — needs a connected WhatsApp client and is out of scope for an
//! integration test. Assert the HTTP contract, not the protocol behaviour.
//!
//! # Pick the shape that fits the module
//!
//! Presence has two routes, so this file spells every case out as its own
//! named test. That is the right call here and the wrong call for a bigger
//! module: assertions 1–3 are identical for every session-scoped route, so
//! writing them out once per route stops being readable somewhere past a
//! handful of them.
//!
//! If the module you are covering has more than a few routes, mirror
//! `tests/groups_management.rs`, `tests/newsletter.rs`, or `tests/labels.rs`
//! instead. Those declare the routes in one table, run assertions 1–3 as
//! sweeps over it — each failure naming the method and path it came from —
//! and keep named tests only for assertion 4, which depends on the individual
//! request type. Same four assertions, same boundary; only the layout differs.

mod common;

use axum::http::{Method, StatusCode};
use common::{call, req_json, Harness, TEST_TOKEN};
use serde_json::json;

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
async fn set_presence_requires_a_token() {
    let h = Harness::new().await;

    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions/s-presence/presence/set",
            None,
            json!({ "status": "available" }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn subscribe_presence_requires_a_token() {
    let h = Harness::new().await;

    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions/s-presence/presence/subscribe",
            None,
            json!({ "jid": "559999999999@s.whatsapp.net" }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn set_presence_unknown_session_returns_503() {
    let h = Harness::new().await;

    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions/does-not-exist/presence/set",
            Some(TEST_TOKEN),
            json!({ "status": "available" }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

/// The session exists in the database but has no live client, so `get_client`
/// still fails — 503, never 404. Both presence routes share the gate, so both
/// are pinned here.
#[tokio::test]
async fn presence_routes_reject_a_session_that_never_connected() {
    let h = Harness::new().await;
    seed_session(&h, "s-presence").await;

    let (set_status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions/s-presence/presence/set",
            Some(TEST_TOKEN),
            json!({ "status": "unavailable" }),
        ),
    )
    .await;
    assert_eq!(
        set_status,
        StatusCode::SERVICE_UNAVAILABLE,
        "a DB-only session has no live client"
    );

    let (sub_status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions/s-presence/presence/subscribe",
            Some(TEST_TOKEN),
            json!({ "jid": "559999999999@s.whatsapp.net" }),
        ),
    )
    .await;
    assert_eq!(sub_status, StatusCode::SERVICE_UNAVAILABLE);
}

/// `status` only accepts `available` / `unavailable` (snake_case, per
/// `PresenceStatus`). An unknown variant fails `Json` extraction, which runs
/// before the handler body — so this is a body rejection, not a 503.
#[tokio::test]
async fn set_presence_rejects_an_unknown_status_variant() {
    let h = Harness::new().await;
    seed_session(&h, "s-presence").await;

    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions/s-presence/presence/set",
            Some(TEST_TOKEN),
            json!({ "status": "invisible" }),
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
        "an unknown presence status is a client error, got {status}"
    );
}

#[tokio::test]
async fn subscribe_presence_rejects_a_body_without_jid() {
    let h = Harness::new().await;
    seed_session(&h, "s-presence").await;

    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions/s-presence/presence/subscribe",
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
