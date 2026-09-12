//! Contract tests for the mex handlers (`src/handlers/mex.rs`), the
//! passthrough to WhatsApp's internal GraphQL ("mex") endpoint.
//!
//! # The contract
//!
//! `mex_query` and `mex_mutate` back `/api/v1/sessions/{session_id}/mex/query`
//! and `.../mex/mutate`. Both were registered with utoipa in `src/main.rs`
//! long before they were wired into `create_router`, so for several releases
//! they were published in the OpenAPI document while every request to them
//! fell through to the router's 404. The two characterization tests that
//! pinned that gap lived here and were deleted when the routes were
//! registered in `src/routes/mod.rs` (IMT-28); the four tests below are the
//! contract the handlers had implemented all along.
//!
//! Both handlers resolve their client through the same local `get_client` as
//! every other session-scoped module, so once routed they follow the pattern
//! documented in `tests/presence.rs` exactly: 401 without a token, 503 for an
//! unknown session, 503 for a session that exists in the database but has
//! never connected, and a body rejection at extraction time for a body that
//! does not deserialize.
//!
//! Nothing past the client gate is assertable — `mex()` is a thin passthrough
//! whose only logic is `build_mex_doc`'s fallback name and the flattening of
//! upstream GraphQL errors, both of which need a live client to reach.

mod common;

use axum::http::{Method, StatusCode};
use common::{call, req_json, Harness, TEST_TOKEN};
use serde_json::json;

const SESSION: &str = "s-mex";

const QUERY_PATH: &str = "/api/v1/sessions/s-mex/mex/query";
const MUTATE_PATH: &str = "/api/v1/sessions/s-mex/mex/mutate";

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

fn query_body() -> serde_json::Value {
    json!({ "doc_id": "1234567890", "variables": {} })
}

/// The JWT middleware is layered around the whole router, so a missing token
/// is rejected ahead of routing. This says nothing about whether the route is
/// registered — which is why every test below that cares about routing sends
/// a token.
#[tokio::test]
async fn mex_routes_require_a_token() {
    let h = Harness::new().await;

    for path in [QUERY_PATH, MUTATE_PATH] {
        let (status, _) = call(&h.app, req_json(Method::POST, path, None, query_body())).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "unauthenticated {path}");
    }
}

#[tokio::test]
async fn mex_routes_reject_an_unknown_session() {
    let h = Harness::new().await;

    for path in [
        "/api/v1/sessions/does-not-exist/mex/query",
        "/api/v1/sessions/does-not-exist/mex/mutate",
    ] {
        let (status, _) = call(
            &h.app,
            req_json(Method::POST, path, Some(TEST_TOKEN), query_body()),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::SERVICE_UNAVAILABLE,
            "{path} should fail at the client gate"
        );
    }
}

/// `get_client` reads the live registry, not the `sessions` table, so a
/// session row on its own does not change the outcome — 503, never 404.
#[tokio::test]
async fn mex_routes_reject_a_session_that_never_connected() {
    let h = Harness::new().await;
    seed_session(&h, SESSION).await;

    for path in [QUERY_PATH, MUTATE_PATH] {
        let (status, _) = call(
            &h.app,
            req_json(Method::POST, path, Some(TEST_TOKEN), query_body()),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::SERVICE_UNAVAILABLE,
            "{path} has a DB row but no live client"
        );
    }
}

/// `doc_id` and `variables` are both required — only `doc_name` has a serde
/// default. `Json<T>` is the last extractor, so a body missing either one is
/// rejected before the handler body and never reaches the client gate.
#[tokio::test]
async fn mex_routes_reject_a_body_missing_required_fields() {
    let h = Harness::new().await;
    seed_session(&h, SESSION).await;

    let bad_bodies = [
        json!({}),
        json!({ "variables": {} }),
        json!({ "doc_id": "1234567890" }),
        json!({ "doc_id": 1234567890, "variables": {} }),
    ];

    for path in [QUERY_PATH, MUTATE_PATH] {
        for body in &bad_bodies {
            let (status, _) = call(
                &h.app,
                req_json(Method::POST, path, Some(TEST_TOKEN), body.clone()),
            )
            .await;
            assert_ne!(
                status,
                StatusCode::SERVICE_UNAVAILABLE,
                "{path} with {body}: extraction must fail before the client gate"
            );
            assert!(
                status.is_client_error(),
                "{path} with {body}: expected a client error, got {status}"
            );
        }
    }
}
