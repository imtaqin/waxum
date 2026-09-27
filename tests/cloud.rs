//! Contract tests for the WhatsApp Cloud API provider surface:
//! `/cloud/connect`, the Meta webhook GET/POST receiver, the
//! secret-leak guard on `GET /sessions/{id}`, and the 400-not-503 guard
//! on endpoints a `whatsapp_cloud` session doesn't support yet.
mod common;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
};
use common::{call, req_json, Harness, TEST_TOKEN};
use hmac::{Hmac, Mac};
use serde_json::json;
use sha2::Sha256;

const WABA_ID: &str = "102290129340398";
const PHONE_NUMBER_ID: &str = "106540352242922";
const ACCESS_TOKEN: &str = "fake-cloud-access-token";
const APP_SECRET: &str = "fake-app-secret";
const VERIFY_TOKEN: &str = "fake-verify-token";

async fn create_cloud_session(h: &Harness, session_id: &str) {
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

    let (status, body) = call(
        &h.app,
        req_json(
            Method::POST,
            &format!("/api/v1/sessions/{session_id}/cloud/connect"),
            Some(TEST_TOKEN),
            json!({
                "waba_id": WABA_ID,
                "phone_number_id": PHONE_NUMBER_ID,
                "access_token": ACCESS_TOKEN,
                "app_secret": APP_SECRET,
                "webhook_verify_token": VERIFY_TOKEN,
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "connect_cloud body: {body}");
}

#[tokio::test]
async fn connect_cloud_sets_provider_and_non_secret_fields() {
    let h = Harness::new().await;
    create_cloud_session(&h, "cloud-1").await;

    let (status, body) = call(
        &h.app,
        common::req_get("/api/v1/sessions/cloud-1", Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body.get("provider").and_then(|v| v.as_str()),
        Some("whatsapp_cloud")
    );
    assert_eq!(
        body.get("cloud_phone_number_id").and_then(|v| v.as_str()),
        Some(PHONE_NUMBER_ID)
    );
}

#[tokio::test]
async fn get_session_never_leaks_the_cloud_access_token_or_app_secret() {
    let h = Harness::new().await;
    create_cloud_session(&h, "cloud-2").await;

    let (status, body) = call(
        &h.app,
        common::req_get("/api/v1/sessions/cloud-2", Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let raw = body.to_string();
    assert!(!raw.contains(ACCESS_TOKEN), "access token leaked: {raw}");
    assert!(!raw.contains(APP_SECRET), "app secret leaked: {raw}");
    assert!(!raw.contains(VERIFY_TOKEN), "verify token leaked: {raw}");

    let (status, body) = call(
        &h.app,
        common::req_get("/api/v1/sessions", Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let raw = body.to_string();
    assert!(
        !raw.contains(ACCESS_TOKEN),
        "access token leaked via list: {raw}"
    );
    assert!(
        !raw.contains(APP_SECRET),
        "app secret leaked via list: {raw}"
    );
}

#[tokio::test]
async fn unsupported_endpoint_on_a_cloud_session_returns_400_not_503() {
    let h = Harness::new().await;
    create_cloud_session(&h, "cloud-3").await;

    let (status, body) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions/cloud-3/messages/location",
            Some(TEST_TOKEN),
            json!({"to": "15551234567", "latitude": 1.0, "longitude": 2.0}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
    let message = body
        .pointer("/error/message")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    assert!(
        message.contains("whatsapp_cloud"),
        "expected a provider-aware 400, got: {message}"
    );
}

#[tokio::test]
async fn webhook_verify_handshake_echoes_challenge_only_on_matching_token() {
    let h = Harness::new().await;
    create_cloud_session(&h, "cloud-4").await;

    let (status, body) = call(
        &h.app,
        common::req_get(
            &format!(
                "/api/v1/sessions/cloud-4/cloud/webhook?hub.mode=subscribe&hub.verify_token={VERIFY_TOKEN}&hub.challenge=1234567890"
            ),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!(1234567890));

    let (status, _) = call(
        &h.app,
        common::req_get(
            "/api/v1/sessions/cloud-4/cloud/webhook?hub.mode=subscribe&hub.verify_token=wrong&hub.challenge=1234567890",
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

fn signed_post(path: &str, body: &serde_json::Value, secret: &str) -> Request<Body> {
    type HmacSha256 = Hmac<Sha256>;
    let raw = body.to_string();
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(raw.as_bytes());
    let sig = hex::encode(mac.finalize().into_bytes());
    Request::builder()
        .method(Method::POST)
        .uri(path)
        .header("content-type", "application/json")
        .header("X-Hub-Signature-256", format!("sha256={sig}"))
        .body(Body::from(raw))
        .expect("build request")
}

fn inbound_text_payload() -> serde_json::Value {
    json!({
        "entry": [{
            "changes": [{
                "value": {
                    "metadata": {"phone_number_id": PHONE_NUMBER_ID, "display_phone_number": "15551234567"},
                    "contacts": [{"wa_id": "15559876543", "profile": {"name": "Ada"}}],
                    "messages": [{
                        "from": "15559876543",
                        "id": "wamid.ABC123",
                        "timestamp": "1700000000",
                        "type": "text",
                        "text": {"body": "hello from meta"}
                    }]
                }
            }]
        }]
    })
}

#[tokio::test]
async fn webhook_post_accepts_a_correctly_signed_delivery() {
    let h = Harness::new().await;
    create_cloud_session(&h, "cloud-5").await;

    let req = signed_post(
        "/api/v1/sessions/cloud-5/cloud/webhook",
        &inbound_text_payload(),
        APP_SECRET,
    );
    let (status, _) = call(&h.app, req).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn webhook_post_rejects_a_badly_signed_delivery() {
    let h = Harness::new().await;
    create_cloud_session(&h, "cloud-6").await;

    let req = signed_post(
        "/api/v1/sessions/cloud-6/cloud/webhook",
        &inbound_text_payload(),
        "wrong-secret",
    );
    let (status, _) = call(&h.app, req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn webhook_routes_bypass_the_bearer_auth_middleware() {
    let h = Harness::new().await;
    create_cloud_session(&h, "cloud-7").await;

    let (status, _) = call(
        &h.app,
        common::req_get(
            "/api/v1/sessions/cloud-7/cloud/webhook?hub.mode=subscribe&hub.verify_token=wrong&hub.challenge=x",
            None,
        ),
    )
    .await;
    assert_ne!(
        status,
        StatusCode::UNAUTHORIZED,
        "cloud webhook route must not require a bearer token"
    );
}
