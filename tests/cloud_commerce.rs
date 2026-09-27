//! Contract tests for WhatsApp Commerce on `whatsapp_cloud` sessions:
//! the provider guard, bearer auth, and request validation that must
//! reject a bad request before any Graph API call is made.
mod common;

use axum::http::{Method, StatusCode};
use common::{call, req_get, req_json, Harness, TEST_TOKEN};
use serde_json::json;

async fn create_session(h: &Harness, session_id: &str, cloud: bool) {
    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions",
            Some(TEST_TOKEN),
            json!({"id": session_id}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    if !cloud {
        return;
    }
    let (status, body) = call(
        &h.app,
        req_json(
            Method::POST,
            &format!("/api/v1/sessions/{session_id}/cloud/connect"),
            Some(TEST_TOKEN),
            json!({
                "waba_id": "102290129340398",
                "phone_number_id": "106540352242922",
                "access_token": "fake-cloud-access-token",
                "app_secret": "fake-app-secret",
                "webhook_verify_token": "fake-verify-token",
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "connect_cloud body: {body}");
}

fn product_list(sections: serde_json::Value) -> serde_json::Value {
    json!({
        "to": "6281234567890",
        "catalog_id": "CAT",
        "header": "Picks",
        "body": "Tap one",
        "sections": sections,
    })
}

#[tokio::test]
async fn commerce_routes_reject_a_whatsapp_web_session_with_400() {
    let h = Harness::new().await;
    create_session(&h, "com-web", false).await;

    let (status, body) = call(
        &h.app,
        req_get(
            "/api/v1/sessions/com-web/cloud/commerce-settings",
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let posts = [
        (
            "/api/v1/sessions/com-web/cloud/commerce-settings",
            json!({"is_cart_enabled": true}),
        ),
        (
            "/api/v1/sessions/com-web/messages/product",
            json!({"to": "1", "catalog_id": "CAT", "product_retailer_id": "SKU"}),
        ),
        (
            "/api/v1/sessions/com-web/messages/product-list",
            product_list(json!([{"title": "A", "product_retailer_ids": ["SKU"]}])),
        ),
        (
            "/api/v1/sessions/com-web/messages/catalog",
            json!({"to": "1", "body": "Shop"}),
        ),
    ];
    for (path, payload) in posts {
        let (status, body) = call(
            &h.app,
            req_json(Method::POST, path, Some(TEST_TOKEN), payload),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{path}: {body}");
    }
}

#[tokio::test]
async fn commerce_settings_update_needs_at_least_one_flag() {
    let h = Harness::new().await;
    create_session(&h, "com-1", true).await;
    let (status, body) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions/com-1/cloud/commerce-settings",
            Some(TEST_TOKEN),
            json!({}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body.to_string().contains("is_cart_enabled"), "{body}");
}

#[tokio::test]
async fn product_list_over_metas_limits_is_rejected_before_calling_meta() {
    let h = Harness::new().await;
    create_session(&h, "com-2", true).await;

    let too_many: Vec<String> = (0..31).map(|i| format!("SKU-{i}")).collect();
    let cases = [
        product_list(json!([])),
        product_list(json!([{"title": "A", "product_retailer_ids": too_many}])),
        product_list(json!([{"title": "A", "product_retailer_ids": []}])),
    ];
    for payload in cases {
        let (status, body) = call(
            &h.app,
            req_json(
                Method::POST,
                "/api/v1/sessions/com-2/messages/product-list",
                Some(TEST_TOKEN),
                payload,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    }
}

#[tokio::test]
async fn commerce_routes_require_a_bearer_token() {
    let h = Harness::new().await;
    create_session(&h, "com-3", true).await;
    let (status, _) = call(
        &h.app,
        req_get("/api/v1/sessions/com-3/cloud/commerce-settings", None),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions/com-3/messages/catalog",
            None,
            json!({"to": "1", "body": "Shop"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
