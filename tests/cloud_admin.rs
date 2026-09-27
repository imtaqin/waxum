//! Contract tests for the Cloud API administration/BSP routes: the
//! provider guard, input validation that must fail before any Graph API
//! call, path-ID sanitising, the missing-business_id message, bearer
//! auth, and `statuses[]` deliveries being accepted by the webhook.
mod common;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
};
use common::{call, req_get, req_json, Harness, TEST_TOKEN};
use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::Sha256;

const APP_SECRET: &str = "fake-app-secret";

async fn create_session(h: &Harness, session_id: &str, cloud: Option<Value>) {
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
    let Some(extra) = cloud else {
        return;
    };
    let mut body = json!({
        "waba_id": "102290129340398",
        "phone_number_id": "106540352242922",
        "access_token": "fake-cloud-access-token",
        "app_secret": APP_SECRET,
        "webhook_verify_token": "fake-verify-token",
    });
    for (k, v) in extra.as_object().unwrap() {
        body[k] = v.clone();
    }
    let (status, resp) = call(
        &h.app,
        req_json(
            Method::POST,
            &format!("/api/v1/sessions/{session_id}/cloud/connect"),
            Some(TEST_TOKEN),
            body,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "connect_cloud body: {resp}");
}

async fn expect_400(h: &Harness, method: Method, path: &str, body: Value, needle: &str) {
    let req = if method == Method::GET {
        req_get(path, Some(TEST_TOKEN))
    } else {
        req_json(method, path, Some(TEST_TOKEN), body)
    };
    let (status, resp) = call(&h.app, req).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{path}: {resp}");
    assert!(
        resp.to_string().contains(needle),
        "{path}: expected {needle:?} in {resp}"
    );
}

#[tokio::test]
async fn admin_routes_reject_a_whatsapp_web_session_with_400() {
    let h = Harness::new().await;
    create_session(&h, "adm-web", None).await;
    let p = "/api/v1/sessions/adm-web";
    let cases = [
        (Method::GET, format!("{p}/cloud/waba"), json!(null)),
        (Method::GET, format!("{p}/cloud/phone-numbers"), json!(null)),
        (Method::POST, format!("{p}/cloud/deregister"), json!({})),
        (Method::GET, format!("{p}/cloud/templates"), json!(null)),
        (Method::GET, format!("{p}/cloud/qr-codes"), json!(null)),
        (Method::GET, format!("{p}/cloud/blocked-users"), json!(null)),
        (
            Method::GET,
            format!("{p}/cloud/subscribed-apps"),
            json!(null),
        ),
        (
            Method::GET,
            format!("{p}/cloud/business-profile"),
            json!(null),
        ),
        (
            Method::POST,
            format!("{p}/cloud/typing"),
            json!({"message_id": "wamid.X"}),
        ),
        (
            Method::POST,
            format!("{p}/messages/order-status"),
            json!({"to": "1", "body": "b", "reference_id": "r", "status": "shipped"}),
        ),
    ];
    for (method, path, body) in cases {
        expect_400(&h, method, &path, body, "whatsapp_cloud").await;
    }
}

#[tokio::test]
async fn invalid_input_is_rejected_before_calling_meta() {
    let h = Harness::new().await;
    create_session(&h, "adm-1", Some(json!({"business_id": "102290129340399"}))).await;
    let p = "/api/v1/sessions/adm-1";
    let cases = [
        (
            Method::POST,
            format!("{p}/cloud/register"),
            json!({"pin": "12345"}),
            "6 digits",
        ),
        (
            Method::POST,
            format!("{p}/cloud/two-step-pin"),
            json!({"pin": "abcdef"}),
            "6 digits",
        ),
        (
            Method::POST,
            format!("{p}/cloud/request-code"),
            json!({"code_method": "EMAIL"}),
            "code_method",
        ),
        (
            Method::POST,
            format!("{p}/cloud/templates"),
            json!({"name": "t", "language": "en", "category": "PROMO", "components": []}),
            "category",
        ),
        (
            Method::POST,
            format!("{p}/cloud/templates"),
            json!({"name": "t", "language": "en", "category": "UTILITY", "components": {}}),
            "components",
        ),
        (
            Method::POST,
            format!("{p}/cloud/templates/123"),
            json!({}),
            "at least one",
        ),
        (
            Method::GET,
            format!("{p}/cloud/qr-codes?format=GIF"),
            json!(null),
            "format",
        ),
        (
            Method::POST,
            format!("{p}/cloud/qr-codes"),
            json!({"prefilled_message": "hi", "generate_qr_image": "JPG"}),
            "generate_qr_image",
        ),
        (
            Method::POST,
            format!("{p}/cloud/blocked-users"),
            json!({"users": []}),
            "users",
        ),
        (
            Method::POST,
            format!("{p}/cloud/business-profile"),
            json!({}),
            "at least one",
        ),
        (
            Method::POST,
            format!("{p}/cloud/business-profile"),
            json!({"websites": ["a", "b", "c"]}),
            "websites",
        ),
        (
            Method::POST,
            format!("{p}/cloud/subscribed-apps"),
            json!({"override_callback_uri": "https://example.com/hook"}),
            "together",
        ),
        (
            Method::GET,
            format!("{p}/cloud/analytics?start=1&end=2&granularity=WEEK"),
            json!(null),
            "granularity",
        ),
        (
            Method::GET,
            format!("{p}/cloud/analytics?start=1&end=2&granularity=DAY&country_codes=US).x"),
            json!(null),
            "country_codes",
        ),
        (
            Method::GET,
            format!("{p}/cloud/flows/123/metrics?metric=CPU&granularity=DAY"),
            json!(null),
            "metric",
        ),
        (
            Method::GET,
            format!(
                "{p}/cloud/flows/123/metrics?metric=ENDPOINT_REQUEST_COUNT&granularity=DAY&since=28-01-2024"
            ),
            json!(null),
            "since",
        ),
        (
            Method::POST,
            format!("{p}/messages/order-details"),
            json!({"to": "1", "region": "US", "body": "b", "parameters": {}}),
            "region",
        ),
        (
            Method::POST,
            format!("{p}/cloud/assigned-users"),
            json!({"user_id": "123", "tasks": []}),
            "tasks",
        ),
    ];
    for (method, path, body, needle) in cases {
        expect_400(&h, method, &path, body, needle).await;
    }
}

#[tokio::test]
async fn encoded_slashes_in_path_ids_never_reach_the_graph_api() {
    let h = Harness::new().await;
    create_session(&h, "adm-2", Some(json!({}))).await;
    let p = "/api/v1/sessions/adm-2";
    let cases = [
        (
            Method::GET,
            format!("{p}/cloud/templates/me%2Faccounts"),
            "template_id",
        ),
        (
            Method::DELETE,
            format!("{p}/cloud/qr-codes/..%2F..%2Fme"),
            "code",
        ),
        (
            Method::GET,
            format!("{p}/cloud/credit-sharing/1%3Ffields%3Dx"),
            "allocation_config_id",
        ),
        (
            Method::GET,
            format!(
                "{p}/cloud/flows/1%2Fassets/metrics?metric=ENDPOINT_REQUEST_COUNT&granularity=DAY"
            ),
            "flow_id",
        ),
    ];
    for (method, path, needle) in cases {
        expect_400(&h, method, &path, json!(null), needle).await;
    }
    expect_400(
        &h,
        Method::POST,
        &format!("{p}/cloud/credit-sharing"),
        json!({"credit_line_id": "1/../me", "waba_currency": "USD"}),
        "credit_line_id",
    )
    .await;
}

#[tokio::test]
async fn business_scoped_routes_name_the_missing_business_id() {
    let h = Harness::new().await;
    create_session(&h, "adm-3", Some(json!({}))).await;
    let p = "/api/v1/sessions/adm-3";
    for route in [
        "owned-wabas",
        "client-wabas",
        "credit-lines",
        "system-users",
        "business-portfolio",
    ] {
        expect_400(
            &h,
            Method::GET,
            &format!("{p}/cloud/{route}"),
            json!(null),
            "business_id",
        )
        .await;
    }
}

#[tokio::test]
async fn admin_routes_require_a_bearer_token() {
    let h = Harness::new().await;
    create_session(&h, "adm-4", Some(json!({}))).await;
    for path in [
        "/api/v1/sessions/adm-4/cloud/templates",
        "/api/v1/sessions/adm-4/cloud/debug-token",
        "/api/v1/sessions/adm-4/cloud/credit-lines",
    ] {
        let (status, _) = call(&h.app, req_get(path, None)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{path}");
    }
}

#[tokio::test]
async fn signed_status_delivery_is_accepted_by_the_webhook() {
    let h = Harness::new().await;
    create_session(&h, "adm-5", Some(json!({}))).await;
    let raw = json!({"entry": [{"changes": [{"value": {
        "metadata": {"phone_number_id": "106540352242922"},
        "statuses": [
            {"id": "wamid.X", "recipient_id": "6281", "status": "read", "timestamp": "1700000000"},
            {"id": "wamid.P", "from": "6281", "type": "payment", "status": "captured",
             "payment": {"reference_id": "ref-1"}, "timestamp": "1700000001"}
        ]
    }}]}]})
    .to_string();
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(APP_SECRET.as_bytes()).unwrap();
    mac.update(raw.as_bytes());
    let sig = hex::encode(mac.finalize().into_bytes());
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/v1/sessions/adm-5/cloud/webhook")
        .header("content-type", "application/json")
        .header("X-Hub-Signature-256", format!("sha256={sig}"))
        .body(Body::from(raw))
        .unwrap();
    let (status, _) = call(&h.app, req).await;
    assert_eq!(status, StatusCode::OK);
}
