//! Contract tests for the business handlers
//! (`src/handlers/business.rs`, five routes under
//! `/api/v1/sessions/{session_id}/business`).
//!
//! `tests/presence.rs` is the worked example for this kind of module; read
//! its doc comment first. All five gate on `get_client` before touching
//! their JID (query, body, or path, depending on the route), so the first
//! three assertions run as table sweeps.
//!
//! Three of the five routes (`catalog`, `collections`, `order`) take their
//! JID and other required fields as `Query<T>` params with no default, so
//! the routes table below always supplies a complete, valid query string —
//! otherwise extraction would reject the request before the client gate is
//! ever reached, which would make the 503 sweeps assert the wrong thing.
//! That extraction behaviour is what the query-specific tests at the bottom
//! pin down instead.

mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use common::{call, req_delete, req_get, req_json, Harness, TEST_TOKEN};
use serde_json::{json, Value};

const SESSION: &str = "s-business";
const BIZ_JID: &str = "559999999999@s.whatsapp.net";

/// Every business route, as (method, path suffix, request body).
///
/// The suffix is appended to `/api/v1/sessions/{session_id}` and, for the
/// `Query`-based GET routes, already carries a complete query string. A
/// `None` body marks a handler with no `Json` extractor in its signature.
fn routes() -> Vec<(Method, String, Option<Value>)> {
    vec![
        (
            Method::GET,
            format!("/business/catalog?jid={BIZ_JID}"),
            None,
        ),
        (
            Method::GET,
            format!("/business/collections?jid={BIZ_JID}"),
            None,
        ),
        (
            Method::GET,
            format!("/business/order?jid={BIZ_JID}&order_id=order-1&token=tok-1"),
            None,
        ),
        (
            Method::PATCH,
            "/business/profile".to_string(),
            Some(json!({ "description": "test" })),
        ),
        (
            Method::DELETE,
            "/business/cover-photo/photo-1".to_string(),
            None,
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
/// still fails. Every route here gates on the client before touching the
/// JID, so a DB-only session is 503, never 404 or 400.
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

/// `jid` has no default on `CatalogParams`, so a request missing it fails
/// `Query` extraction before the handler body runs — a client error, not the
/// 503 an unknown/DB-only session would produce.
#[tokio::test]
async fn get_catalog_rejects_a_request_missing_jid() {
    let h = Harness::new().await;
    seed_session(&h, SESSION).await;

    let (status, _) = call(
        &h.app,
        req_get(
            &format!("/api/v1/sessions/{SESSION}/business/catalog"),
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
        "a missing `jid` query param is a client error, got {status}"
    );
}

/// Same as the catalog case, but for `order_id`/`token` on `OrderParams` —
/// `jid` alone is not a complete query for this route.
#[tokio::test]
async fn get_order_rejects_a_request_missing_order_id_and_token() {
    let h = Harness::new().await;
    seed_session(&h, SESSION).await;

    let (status, _) = call(
        &h.app,
        req_get(
            &format!("/api/v1/sessions/{SESSION}/business/order?jid={BIZ_JID}"),
            Some(TEST_TOKEN),
        ),
    )
    .await;

    assert!(
        status.is_client_error(),
        "a missing `order_id`/`token` is a client error, got {status}"
    );
}

/// Every field on `BusinessProfileUpdateRequest` is optional, so `{}` is a
/// valid body — the only way to trip `Json` extraction here is a type
/// mismatch, not a missing field.
#[tokio::test]
async fn update_business_profile_rejects_a_type_mismatch() {
    let h = Harness::new().await;
    seed_session(&h, SESSION).await;

    let (status, _) = call(
        &h.app,
        req_json(
            Method::PATCH,
            &format!("/api/v1/sessions/{SESSION}/business/profile"),
            Some(TEST_TOKEN),
            json!({ "websites": "not-an-array" }),
        ),
    )
    .await;

    assert!(
        status.is_client_error(),
        "`websites` must be an array, got {status}"
    );
}
