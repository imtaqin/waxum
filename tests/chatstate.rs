//! Contract tests for the chatstate handlers
//! (`POST /sessions/{id}/chatstate/send`, `POST /sessions/{id}/chatstate/typing`,
//! `src/handlers/chatstate.rs`).
//!
//! Both routes call `get_client` before touching their body's `to` field, so
//! (unlike some `calls.rs` routes) the client gate always runs first: an
//! unknown/DB-only session is 503 regardless of body content.
//! `tests/presence.rs` is the worked example this mirrors.

mod common;

use axum::http::{Method, StatusCode};
use common::{call, req_json, Harness, TEST_TOKEN};
use serde_json::json;

const SESSION: &str = "s-chatstate";
const SEND_PATH: &str = "/api/v1/sessions/s-chatstate/chatstate/send";
const TYPING_PATH: &str = "/api/v1/sessions/s-chatstate/chatstate/typing";
const TO: &str = "559999999999@s.whatsapp.net";

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
async fn send_chatstate_requires_a_token() {
    let h = Harness::new().await;

    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            SEND_PATH,
            None,
            json!({ "to": TO, "state": "composing" }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn send_typing_requires_a_token() {
    let h = Harness::new().await;

    let (status, _) = call(
        &h.app,
        req_json(Method::POST, TYPING_PATH, None, json!({ "to": TO })),
    )
    .await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn chatstate_routes_return_503_for_an_unknown_session() {
    let h = Harness::new().await;

    let (send_status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions/does-not-exist/chatstate/send",
            Some(TEST_TOKEN),
            json!({ "to": TO, "state": "composing" }),
        ),
    )
    .await;
    assert_eq!(send_status, StatusCode::SERVICE_UNAVAILABLE);

    let (typing_status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions/does-not-exist/chatstate/typing",
            Some(TEST_TOKEN),
            json!({ "to": TO }),
        ),
    )
    .await;
    assert_eq!(typing_status, StatusCode::SERVICE_UNAVAILABLE);
}

/// The session exists in the database but has no live client, so
/// `get_client` still fails — 503, never 404. Both routes gate on the
/// client before parsing `to`, so this holds regardless of body content.
#[tokio::test]
async fn chatstate_routes_reject_a_session_that_never_connected() {
    let h = Harness::new().await;
    seed_session(&h, SESSION).await;

    let (send_status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            SEND_PATH,
            Some(TEST_TOKEN),
            json!({ "to": TO, "state": "composing" }),
        ),
    )
    .await;
    assert_eq!(
        send_status,
        StatusCode::SERVICE_UNAVAILABLE,
        "a DB-only session has no live client"
    );

    let (typing_status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            TYPING_PATH,
            Some(TEST_TOKEN),
            json!({ "to": TO }),
        ),
    )
    .await;
    assert_eq!(typing_status, StatusCode::SERVICE_UNAVAILABLE);
}

/// `state` only accepts `composing` / `recording` / `paused` (snake_case,
/// per `ChatStateType`). An unknown variant fails `Json` extraction, which
/// runs before the handler body — so this is a body rejection, not a 503.
#[tokio::test]
async fn send_chatstate_rejects_an_unknown_state_variant() {
    let h = Harness::new().await;
    seed_session(&h, SESSION).await;

    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            SEND_PATH,
            Some(TEST_TOKEN),
            json!({ "to": TO, "state": "idle" }),
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
        "an unknown chatstate is a client error, got {status}"
    );
}

#[tokio::test]
async fn send_chatstate_rejects_a_body_missing_state() {
    let h = Harness::new().await;
    seed_session(&h, SESSION).await;

    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            SEND_PATH,
            Some(TEST_TOKEN),
            json!({ "to": TO }),
        ),
    )
    .await;

    assert!(
        status.is_client_error(),
        "a missing `state` is a client error, got {status}"
    );
}

#[tokio::test]
async fn send_typing_rejects_a_body_without_to() {
    let h = Harness::new().await;
    seed_session(&h, SESSION).await;

    let (status, _) = call(
        &h.app,
        req_json(Method::POST, TYPING_PATH, Some(TEST_TOKEN), json!({})),
    )
    .await;

    assert!(
        status.is_client_error(),
        "a missing `to` is a client error, got {status}"
    );
}
