mod common;

use axum::http::{Method, StatusCode};
use common::{call, req_json, Harness, TEST_TOKEN};
use serde_json::json;

/// A body with neither name is rejected before any client lookup, so it is a
/// 400 even for a session that has no installed client.
#[tokio::test]
async fn save_contact_requires_a_name() {
    let h = Harness::new().await;

    let (status, _) = call(
        &h.app,
        req_json(
            Method::PUT,
            "/api/v1/sessions/s-save/contacts/15551234567",
            Some(TEST_TOKEN),
            json!({"save_on_primary_addressbook": true}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// Like every endpoint gated on `get_client`, a session with no live client
/// answers 503 once the body validates.
#[tokio::test]
async fn save_contact_requires_an_installed_client() {
    let h = Harness::new().await;

    let (status, _) = call(
        &h.app,
        req_json(
            Method::PUT,
            "/api/v1/sessions/does-not-exist/contacts/15551234567",
            Some(TEST_TOKEN),
            json!({"full_name": "Jane Doe", "first_name": "Jane"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn save_contact_requires_auth() {
    let h = Harness::new().await;

    let (status, _) = call(
        &h.app,
        req_json(
            Method::PUT,
            "/api/v1/sessions/s-save/contacts/15551234567",
            None,
            json!({"full_name": "Jane Doe"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
