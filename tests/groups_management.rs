//! Contract tests for the group-management handlers
//! (`src/handlers/groups_management.rs`, ten routes under
//! `/api/v1/sessions/{session_id}/groups`).
//!
//! # How this file is organised
//!
//! `tests/presence.rs` is the worked example for this kind of module; read
//! its doc comment first. It covers two routes, so it spells every case out
//! as its own test. Group management has ten, and the first three assertions
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

const SESSION: &str = "s-groups";
const GROUP_JID: &str = "120363000000000000@g.us";
const PARTICIPANT: &str = "559999999999@s.whatsapp.net";

/// Every group-management route, as (method, path suffix, request body).
///
/// The suffix is appended to `/api/v1/sessions/{session_id}`. A `None` body
/// marks a handler with no `Json` extractor in its signature.
fn routes() -> Vec<(Method, String, Option<Value>)> {
    let participants = json!({ "participants": [PARTICIPANT] });
    vec![
        (
            Method::POST,
            "/groups".to_string(),
            Some(json!({ "name": "Test Group", "participants": [PARTICIPANT] })),
        ),
        (
            Method::PUT,
            format!("/groups/{GROUP_JID}/subject"),
            Some(json!({ "subject": "New subject" })),
        ),
        (
            Method::PUT,
            format!("/groups/{GROUP_JID}/description"),
            Some(json!({ "description": "New description" })),
        ),
        (Method::POST, format!("/groups/{GROUP_JID}/leave"), None),
        (
            Method::POST,
            format!("/groups/{GROUP_JID}/participants"),
            Some(participants.clone()),
        ),
        (
            Method::DELETE,
            format!("/groups/{GROUP_JID}/participants"),
            Some(participants.clone()),
        ),
        (
            Method::POST,
            format!("/groups/{GROUP_JID}/admins"),
            Some(participants.clone()),
        ),
        (
            Method::DELETE,
            format!("/groups/{GROUP_JID}/admins"),
            Some(participants),
        ),
        (
            Method::GET,
            format!("/groups/{GROUP_JID}/invite-link"),
            None,
        ),
        (
            Method::PUT,
            format!("/groups/{GROUP_JID}/settings"),
            Some(json!({ "member_add_mode": "all_member_add" })),
        ),
    ]
}

/// Builds a request for one [`routes`] entry against `session_id`.
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

/// `get_client` runs before the group JID is parsed, so an unparseable JID on
/// a disconnected session is still reported as 503 and not as a bad request.
#[tokio::test]
async fn the_client_gate_runs_before_group_jid_parsing() {
    let h = Harness::new().await;
    seed_session(&h, SESSION).await;

    let (status, _) = call(
        &h.app,
        req_json(
            Method::PUT,
            &format!("/api/v1/sessions/{SESSION}/groups/not-a-jid/subject"),
            Some(TEST_TOKEN),
            json!({ "subject": "New subject" }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn create_group_rejects_a_body_without_participants() {
    let h = Harness::new().await;
    seed_session(&h, SESSION).await;

    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            &format!("/api/v1/sessions/{SESSION}/groups"),
            Some(TEST_TOKEN),
            json!({ "name": "Test Group" }),
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
        "a missing `participants` is a client error, got {status}"
    );
}

#[tokio::test]
async fn set_group_subject_rejects_a_body_without_subject() {
    let h = Harness::new().await;
    seed_session(&h, SESSION).await;

    let (status, _) = call(
        &h.app,
        req_json(
            Method::PUT,
            &format!("/api/v1/sessions/{SESSION}/groups/{GROUP_JID}/subject"),
            Some(TEST_TOKEN),
            json!({}),
        ),
    )
    .await;

    assert!(
        status.is_client_error(),
        "a missing `subject` is a client error, got {status}"
    );
}

#[tokio::test]
async fn add_participants_rejects_a_participants_field_that_is_not_a_list() {
    let h = Harness::new().await;
    seed_session(&h, SESSION).await;

    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            &format!("/api/v1/sessions/{SESSION}/groups/{GROUP_JID}/participants"),
            Some(TEST_TOKEN),
            json!({ "participants": PARTICIPANT }),
        ),
    )
    .await;

    assert!(
        status.is_client_error(),
        "a scalar `participants` is a client error, got {status}"
    );
}

/// `member_add_mode` only accepts `admin_add` / `all_member_add` (snake_case,
/// per `MemberAddMode`). An unknown variant fails `Json` extraction.
#[tokio::test]
async fn set_group_settings_rejects_an_unknown_member_add_mode() {
    let h = Harness::new().await;
    seed_session(&h, SESSION).await;

    let (status, _) = call(
        &h.app,
        req_json(
            Method::PUT,
            &format!("/api/v1/sessions/{SESSION}/groups/{GROUP_JID}/settings"),
            Some(TEST_TOKEN),
            json!({ "member_add_mode": "anyone" }),
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
        "an unknown member add mode is a client error, got {status}"
    );
}

/// The invite-link route is the only one taking a `Query` rather than a
/// `Json` body, and `reset` is typed `bool`, so a non-boolean value is
/// rejected at extraction time exactly like a malformed body would be.
#[tokio::test]
async fn get_invite_link_rejects_a_non_boolean_reset() {
    let h = Harness::new().await;
    seed_session(&h, SESSION).await;

    let (status, _) = call(
        &h.app,
        req_get(
            &format!("/api/v1/sessions/{SESSION}/groups/{GROUP_JID}/invite-link?reset=maybe"),
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
        "a non-boolean `reset` is a client error, got {status}"
    );
}
