//! Own-profile endpoints (#159). No WhatsApp connection is available in
//! tests, so these cover what is decided before anything is sent: input
//! validation, and that a valid request on an offline session is refused.
mod common;

use axum::http::{Method, StatusCode};
use common::{call, req_delete, req_json, Harness, TEST_TOKEN};
use serde_json::json;

const PROFILE: &str = "/api/v1/sessions/pf-1/profile";

async fn session(h: &Harness) {
    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions",
            Some(TEST_TOKEN),
            json!({"id": "pf-1"}),
        ),
    )
    .await;
    assert!(status.is_success());
}

async fn put(h: &Harness, body: serde_json::Value) -> StatusCode {
    call(
        &h.app,
        req_json(Method::PUT, PROFILE, Some(TEST_TOKEN), body),
    )
    .await
    .0
}

#[tokio::test]
async fn invalid_profile_changes_are_rejected_before_anything_is_sent() {
    let h = Harness::new().await;
    session(&h).await;

    assert_eq!(put(&h, json!({})).await, StatusCode::BAD_REQUEST);
    assert_eq!(
        put(&h, json!({"name": "   "})).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        put(&h, json!({"name": "x".repeat(26)})).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        put(&h, json!({"about": "x".repeat(140)})).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        put(
            &h,
            json!({"picture": {"data": "bm90IGFuIGltYWdl", "mimetype": "image/png"}})
        )
        .await,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn a_valid_change_on_an_offline_session_is_refused() {
    let h = Harness::new().await;
    session(&h).await;

    assert_eq!(
        put(&h, json!({"name": "Support", "about": ""})).await,
        StatusCode::SERVICE_UNAVAILABLE
    );
    let (status, _) = call(
        &h.app,
        req_delete(&format!("{PROFILE}/picture"), Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}
