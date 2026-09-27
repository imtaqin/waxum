//! Contract tests for WhatsApp Flows on `whatsapp_cloud` sessions: the
//! management routes' provider guard, the Data Exchange endpoint's
//! signature/decryption status codes and encrypted ping round-trip over
//! the full HTTP pipeline, and the private-key leak guard.
mod common;

use aes_gcm::aead::consts::U16;
use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::aes::Aes128;
use aes_gcm::{AesGcm, Key, Nonce};
use aws_lc_rs::rsa::{OaepPublicEncryptingKey, PrivateDecryptingKey, OAEP_SHA256_MGF1SHA256};
use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use common::{call, req_get, req_json, Harness, TEST_TOKEN};
use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::Sha256;
use tower::ServiceExt;
use waxum::cloud::flows_crypto::{generate_private_key, private_key_pem};

type FlowCipher = AesGcm<Aes128, U16>;

const APP_SECRET: &str = "fake-app-secret";
const AES_KEY: [u8; 16] = [5u8; 16];
const IV: [u8; 16] = [9u8; 16];

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
                "app_secret": APP_SECRET,
                "webhook_verify_token": "fake-verify-token",
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "connect_cloud body: {body}");
}

/// Creates a cloud session with a Flow endpoint key stored directly on
/// the session -- the HTTP configure route would also call Meta to
/// register the public key, which these offline tests can't reach.
async fn flow_session(h: &Harness, session_id: &str) -> (PrivateDecryptingKey, String) {
    create_session(h, session_id, true).await;
    let key = generate_private_key().unwrap();
    let pem = private_key_pem(&key).unwrap();
    h.state
        .session_manager()
        .set_flow_endpoint(session_id, &pem, None)
        .await
        .expect("store flow key");
    (key, pem)
}

fn encrypt_like_meta(key: &PrivateDecryptingKey, payload: &Value) -> Value {
    let public = OaepPublicEncryptingKey::new(key.public_key()).unwrap();
    let mut wrapped = vec![0u8; public.ciphertext_size()];
    let wrapped = public
        .encrypt(&OAEP_SHA256_MGF1SHA256, &AES_KEY, &mut wrapped, None)
        .unwrap()
        .to_vec();
    let cipher = FlowCipher::new(Key::<FlowCipher>::from_slice(&AES_KEY));
    let ct = cipher
        .encrypt(
            Nonce::<U16>::from_slice(&IV),
            Payload {
                msg: &serde_json::to_vec(payload).unwrap(),
                aad: &[],
            },
        )
        .unwrap();
    json!({
        "encrypted_flow_data": B64.encode(ct),
        "encrypted_aes_key": B64.encode(wrapped),
        "initial_vector": B64.encode(IV),
    })
}

fn decrypt_like_meta(body: &str) -> Value {
    let flipped = IV.map(|b| !b);
    let cipher = FlowCipher::new(Key::<FlowCipher>::from_slice(&AES_KEY));
    let pt = cipher
        .decrypt(
            Nonce::<U16>::from_slice(&flipped),
            Payload {
                msg: &B64.decode(body).unwrap(),
                aad: &[],
            },
        )
        .expect("response decrypts under the flipped IV");
    serde_json::from_slice(&pt).unwrap()
}

fn signed(path: &str, raw: &str, secret: &str) -> Request<Body> {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(raw.as_bytes());
    let sig = hex::encode(mac.finalize().into_bytes());
    Request::builder()
        .method(Method::POST)
        .uri(path)
        .header("content-type", "application/json")
        .header("X-Hub-Signature-256", format!("sha256={sig}"))
        .body(Body::from(raw.to_string()))
        .unwrap()
}

async fn raw_call(h: &Harness, req: Request<Body>) -> (StatusCode, String, Option<String>) {
    let resp = h.app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let ctype = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&bytes).to_string(), ctype)
}

const EXCHANGE: &str = "/api/v1/sessions/{id}/cloud/flow-endpoint/exchange";

fn exchange_path(id: &str) -> String {
    EXCHANGE.replace("{id}", id)
}

#[tokio::test]
async fn encrypted_ping_round_trips_over_http_without_a_bearer_token() {
    let h = Harness::new().await;
    let (key, _) = flow_session(&h, "flow-1").await;

    let envelope = encrypt_like_meta(&key, &json!({"version": "3.0", "action": "ping"}));
    let (status, body, ctype) = raw_call(
        &h,
        signed(&exchange_path("flow-1"), &envelope.to_string(), APP_SECRET),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(ctype.as_deref(), Some("text/plain"));
    assert_eq!(
        decrypt_like_meta(&body),
        json!({"data": {"status": "active"}})
    );
}

#[tokio::test]
async fn bad_signature_gets_metas_432() {
    let h = Harness::new().await;
    let (key, _) = flow_session(&h, "flow-2").await;
    let envelope = encrypt_like_meta(&key, &json!({"action": "ping"}));

    let (status, _, _) = raw_call(
        &h,
        signed(
            &exchange_path("flow-2"),
            &envelope.to_string(),
            "wrong-secret",
        ),
    )
    .await;
    assert_eq!(status.as_u16(), 432);
}

#[tokio::test]
async fn wrong_key_and_tampered_payload_get_the_same_421() {
    let h = Harness::new().await;
    flow_session(&h, "flow-3").await;

    let other = generate_private_key().unwrap();
    let wrong_key = encrypt_like_meta(&other, &json!({"action": "ping"}));
    let (s1, b1, _) = raw_call(
        &h,
        signed(&exchange_path("flow-3"), &wrong_key.to_string(), APP_SECRET),
    )
    .await;

    let (key, _) = flow_session(&h, "flow-3b").await;
    let mut tampered = encrypt_like_meta(&key, &json!({"action": "ping"}));
    let mut ct = B64
        .decode(tampered["encrypted_flow_data"].as_str().unwrap())
        .unwrap();
    ct[0] ^= 0x01;
    tampered["encrypted_flow_data"] = json!(B64.encode(ct));
    let (s2, b2, _) = raw_call(
        &h,
        signed(&exchange_path("flow-3b"), &tampered.to_string(), APP_SECRET),
    )
    .await;

    assert_eq!(s1.as_u16(), 421);
    assert_eq!(s2.as_u16(), 421);
    assert_eq!(b1, b2, "failure body must not reveal which step failed");
}

#[tokio::test]
async fn non_ping_without_a_forward_url_is_refused() {
    let h = Harness::new().await;
    let (key, _) = flow_session(&h, "flow-4").await;
    let envelope = encrypt_like_meta(
        &key,
        &json!({"version": "3.0", "action": "INIT", "flow_token": "t"}),
    );
    let (status, _, _) = raw_call(
        &h,
        signed(&exchange_path("flow-4"), &envelope.to_string(), APP_SECRET),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn exchange_on_a_session_without_a_flow_key_is_404() {
    let h = Harness::new().await;
    create_session(&h, "flow-5", true).await;
    let (status, _, _) = raw_call(&h, signed(&exchange_path("flow-5"), "{}", APP_SECRET)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn flow_private_key_never_leaks_through_session_reads() {
    let h = Harness::new().await;
    let (_, pem) = flow_session(&h, "flow-6").await;
    let marker = pem.lines().nth(1).unwrap().to_string();

    for path in ["/api/v1/sessions/flow-6", "/api/v1/sessions"] {
        let (status, body) = call(&h.app, req_get(path, Some(TEST_TOKEN))).await;
        assert_eq!(status, StatusCode::OK);
        let text = body.to_string();
        assert!(!text.contains(&marker), "{path} leaked the private key");
        assert!(!text.contains("PRIVATE KEY"), "{path} leaked a PEM");
    }
}

#[tokio::test]
async fn configure_rejects_a_private_forward_url_before_calling_meta() {
    let h = Harness::new().await;
    create_session(&h, "flow-7", true).await;
    let (status, body) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions/flow-7/cloud/flow-endpoint",
            Some(TEST_TOKEN),
            json!({"forward_url": "http://127.0.0.1:9/handler"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
}

#[tokio::test]
async fn flow_routes_reject_a_whatsapp_web_session_with_400() {
    let h = Harness::new().await;
    create_session(&h, "flow-web", false).await;

    let cases = [
        (
            Method::GET,
            "/api/v1/sessions/flow-web/cloud/flows",
            json!(null),
        ),
        (
            Method::POST,
            "/api/v1/sessions/flow-web/cloud/flows",
            json!({"name": "x", "categories": ["OTHER"]}),
        ),
        (
            Method::POST,
            "/api/v1/sessions/flow-web/cloud/flows/1/publish",
            json!({}),
        ),
        (
            Method::POST,
            "/api/v1/sessions/flow-web/cloud/flows/1/send",
            json!({"to": "1", "flow_cta": "Go", "body": "b", "screen": "S"}),
        ),
        (
            Method::POST,
            "/api/v1/sessions/flow-web/cloud/flow-endpoint",
            json!({}),
        ),
    ];
    for (method, path, body) in cases {
        let req = if method == Method::GET {
            req_get(path, Some(TEST_TOKEN))
        } else {
            req_json(method, path, Some(TEST_TOKEN), body)
        };
        let (status, resp) = call(&h.app, req).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{path}: {resp}");
    }
}

#[tokio::test]
async fn flow_management_routes_still_require_a_bearer_token() {
    let h = Harness::new().await;
    create_session(&h, "flow-8", true).await;
    let (status, _) = call(&h.app, req_get("/api/v1/sessions/flow-8/cloud/flows", None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
