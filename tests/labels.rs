//! Contract tests for the label, quick-reply and link-preview handlers
//! (`src/handlers/labels.rs`, nine routes under
//! `/api/v1/sessions/{session_id}`).
//!
//! # How this file is organised
//!
//! `tests/presence.rs` is the worked example for this kind of module; read
//! its doc comment first. It covers two routes, so it spells every case out
//! as its own test. This module has nine, and the first three assertions are
//! identical for all of them:
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
//! 4. **Body extraction** — `Json<T>` is the last extractor, so it runs
//!    *before* the handler body and rejects a bad request without the client
//!    gate being consulted at all
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

const SESSION: &str = "s-labels";
const LABEL_ID: &str = "label-1";
const QUICK_REPLY_ID: &str = "qr-1";
const CHAT_JID: &str = "559999999999@s.whatsapp.net";

/// Every route served by `handlers::labels`, as (method, path suffix,
/// request body).
///
/// The suffix is appended to `/api/v1/sessions/{session_id}`. A `None` body
/// marks a handler with no `Json` extractor in its signature.
fn routes() -> Vec<(Method, String, Option<Value>)> {
    let message_label = json!({ "chat_jid": CHAT_JID, "message_id": "ABCD1234" });
    vec![
        (
            Method::POST,
            "/labels".to_string(),
            Some(json!({ "label_id": LABEL_ID, "name": "Priority" })),
        ),
        (Method::DELETE, format!("/labels/{LABEL_ID}"), None),
        (
            Method::POST,
            format!("/labels/{LABEL_ID}/chats/{CHAT_JID}"),
            None,
        ),
        (
            Method::DELETE,
            format!("/labels/{LABEL_ID}/chats/{CHAT_JID}"),
            None,
        ),
        (
            Method::POST,
            format!("/labels/{LABEL_ID}/messages"),
            Some(message_label.clone()),
        ),
        (
            Method::POST,
            format!("/labels/{LABEL_ID}/messages/remove"),
            Some(message_label),
        ),
        (
            Method::PUT,
            "/quick-replies".to_string(),
            Some(json!({ "id": QUICK_REPLY_ID, "shortcut": "/hello", "message": "Hello" })),
        ),
        (
            Method::DELETE,
            format!("/quick-replies/{QUICK_REPLY_ID}"),
            None,
        ),
        (
            Method::POST,
            "/settings/link-previews".to_string(),
            Some(json!({ "disabled": true })),
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

/// `get_client` runs before the chat JID is parsed, so an unparseable JID in
/// the path on a disconnected session is still reported as 503 and not as a
/// bad request.
#[tokio::test]
async fn the_client_gate_runs_before_chat_jid_parsing() {
    let h = Harness::new().await;
    seed_session(&h, SESSION).await;

    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            &format!("/api/v1/sessions/{SESSION}/labels/{LABEL_ID}/chats/not-a-jid"),
            Some(TEST_TOKEN),
            json!({}),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn create_label_rejects_a_body_without_name() {
    let h = Harness::new().await;
    seed_session(&h, SESSION).await;

    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            &format!("/api/v1/sessions/{SESSION}/labels"),
            Some(TEST_TOKEN),
            json!({ "label_id": LABEL_ID }),
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
async fn add_message_label_rejects_a_body_without_message_id() {
    let h = Harness::new().await;
    seed_session(&h, SESSION).await;

    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            &format!("/api/v1/sessions/{SESSION}/labels/{LABEL_ID}/messages"),
            Some(TEST_TOKEN),
            json!({ "chat_jid": CHAT_JID }),
        ),
    )
    .await;

    assert!(
        status.is_client_error(),
        "a missing `message_id` is a client error, got {status}"
    );
}

#[tokio::test]
async fn set_quick_reply_rejects_a_body_without_shortcut() {
    let h = Harness::new().await;
    seed_session(&h, SESSION).await;

    let (status, _) = call(
        &h.app,
        req_json(
            Method::PUT,
            &format!("/api/v1/sessions/{SESSION}/quick-replies"),
            Some(TEST_TOKEN),
            json!({ "id": QUICK_REPLY_ID, "message": "Hello" }),
        ),
    )
    .await;

    assert!(
        status.is_client_error(),
        "a missing `shortcut` is a client error, got {status}"
    );
}

#[tokio::test]
async fn set_link_previews_rejects_a_non_boolean_disabled() {
    let h = Harness::new().await;
    seed_session(&h, SESSION).await;

    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            &format!("/api/v1/sessions/{SESSION}/settings/link-previews"),
            Some(TEST_TOKEN),
            json!({ "disabled": "yes" }),
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
        "a non-boolean `disabled` is a client error, got {status}"
    );
}
