//! Cloud API support in the console and in the session lifecycle: a
//! whatsapp_cloud session is reported as logged in, refuses multi-device
//! connect/pair, survives bulk purge, and the console can create one and
//! render its page with the Cloud panel instead of the QR pairing flow.
mod common;

use axum::Router;
use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
};
use common::{call, call_bytes, req_get, req_json, Harness, TEST_TOKEN};
use serde_json::{json, Value};

/// The API router plus the console, wired the way `main.rs` does; the
/// shared `Harness` only mounts the API.
fn full_app(h: &Harness) -> Router {
    waxum::routes::create_router()
        .merge(waxum::console::console_router())
        .layer(axum::middleware::from_fn_with_state(
            h.state.clone(),
            waxum::middleware::jwt::jwt_auth_middleware,
        ))
        .with_state(h.state.clone())
}

fn cloud_credentials() -> Value {
    json!({
        "waba_id": "102290129340398",
        "phone_number_id": "106540352242922",
        "access_token": "fake-cloud-access-token",
        "app_secret": "fake-app-secret",
        "webhook_verify_token": "fake-verify-token",
    })
}

fn console_req(method: Method, path: &str, body: Option<Value>) -> Request<Body> {
    let b = Request::builder()
        .method(method)
        .uri(path)
        .header("cookie", format!("waxum_console={TEST_TOKEN}"));
    match body {
        Some(v) => b
            .header("content-type", "application/json")
            .body(Body::from(v.to_string()))
            .unwrap(),
        None => b.body(Body::empty()).unwrap(),
    }
}

async fn create_cloud_session_via_console(h: &Harness, id: &str) {
    let mut body = cloud_credentials();
    body["id"] = json!(id);
    let (status, resp) = call(
        &full_app(h),
        console_req(Method::POST, "/sessions/cloud", Some(body)),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{resp}");
    assert_eq!(resp["session"]["provider"], "whatsapp_cloud");
}

#[tokio::test]
async fn console_creates_a_cloud_session_without_leaking_secrets() {
    let h = &Harness::new().await;
    create_cloud_session_via_console(h, "con-cloud-1").await;

    let (status, body) = call(
        &h.app,
        req_get("/api/v1/sessions/con-cloud-1", Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let text = body.to_string();
    assert!(text.contains("106540352242922"));
    assert!(!text.contains("fake-cloud-access-token"));
    assert!(!text.contains("fake-app-secret"));
}

#[tokio::test]
async fn console_cloud_create_requires_the_console_cookie() {
    let h = &Harness::new().await;
    let mut body = cloud_credentials();
    body["id"] = json!("con-cloud-anon");
    let req = Request::builder()
        .method(Method::POST)
        .uri("/sessions/cloud")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let (status, _, _) = call_bytes(&full_app(h), req).await;
    assert_ne!(status, StatusCode::CREATED);
    let (status, _) = call(
        &full_app(h),
        req_get("/api/v1/sessions/con-cloud-anon", Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "no session may be created");
}

#[tokio::test]
async fn cloud_session_page_shows_the_cloud_panel_not_qr_pairing() {
    let h = &Harness::new().await;
    create_cloud_session_via_console(h, "con-cloud-2").await;

    let (status, _, bytes) = call_bytes(
        &full_app(h),
        console_req(Method::GET, "/s/con-cloud-2", None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let html = String::from_utf8(bytes).unwrap();
    assert!(html.contains("panel cloud-panel"), "Cloud panel missing");
    assert!(html.contains("106540352242922"));
    assert!(html.contains(r#"window.__PROVIDER__ = "cloud""#));
    assert!(!html.contains("fake-cloud-access-token"));
    assert!(!html.contains("fake-app-secret"));

    let (_, _, overview) = call_bytes(&full_app(h), console_req(Method::GET, "/", None)).await;
    let overview = String::from_utf8(overview).unwrap();
    assert!(overview.contains("provider-badge cloud"));
    assert!(overview.contains("CLOUD API"));
}

#[tokio::test]
async fn web_session_page_keeps_qr_pairing_and_web_provider() {
    let h = &Harness::new().await;
    let (status, _) = call(
        &full_app(h),
        req_json(
            Method::POST,
            "/api/v1/sessions",
            Some(TEST_TOKEN),
            json!({"id": "con-web-1"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, bytes) =
        call_bytes(&full_app(h), console_req(Method::GET, "/s/con-web-1", None)).await;
    assert_eq!(status, StatusCode::OK);
    let html = String::from_utf8(bytes).unwrap();
    assert!(html.contains(r#"window.__PROVIDER__ = "web""#));
    assert!(html.contains("pair-panel"));
    assert!(!html.contains("panel cloud-panel"));
}

#[tokio::test]
async fn cloud_session_status_is_logged_in_and_multi_device_ops_are_refused() {
    let h = &Harness::new().await;
    create_cloud_session_via_console(h, "con-cloud-3").await;

    let (status, body) = call(
        &full_app(h),
        req_get("/api/v1/sessions/con-cloud-3/status", Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "logged_in");
    assert_eq!(body["is_logged_in"], true);

    for (path, payload) in [
        ("/api/v1/sessions/con-cloud-3/connect", json!({})),
        (
            "/api/v1/sessions/con-cloud-3/pair",
            json!({"phone_number": "628123456789"}),
        ),
    ] {
        let (status, body) = call(
            &full_app(h),
            req_json(Method::POST, path, Some(TEST_TOKEN), payload),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{path}: {body}");
        assert!(
            body.to_string().contains("whatsapp_cloud"),
            "{path}: {body}"
        );
    }

    let (status, _, _) = call_bytes(
        &full_app(h),
        req_get(
            "/api/v1/sessions/con-cloud-3/connect/wait",
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn bulk_purge_never_removes_a_cloud_session_unless_asked_for_all() {
    let h = &Harness::new().await;
    create_cloud_session_via_console(h, "con-cloud-4").await;

    for filter in ["logged_out", "inactive"] {
        let (status, body) = call(
            &full_app(h),
            req_json(
                Method::POST,
                &format!("/api/v1/sessions/purge?filter={filter}&days=0"),
                Some(TEST_TOKEN),
                json!({}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(
            !body.to_string().contains("con-cloud-4"),
            "{filter} purge targeted the cloud session: {body}"
        );
    }
    let (status, _) = call(
        &full_app(h),
        req_get("/api/v1/sessions/con-cloud-4", Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "cloud session must survive purge");

    let (_, stats) = call(&full_app(h), req_get("/api/v1/stats", Some(TEST_TOKEN))).await;
    assert_eq!(stats["session_connected"], 1, "{stats}");
    assert_eq!(stats["session_logged_out"], 0, "{stats}");
}

#[tokio::test]
async fn attaching_cloud_credentials_tears_down_the_multi_device_runtime() {
    let h = &Harness::new().await;
    let (status, _) = call(
        &full_app(h),
        req_json(
            Method::POST,
            "/api/v1/sessions",
            Some(TEST_TOKEN),
            json!({"id": "con-cloud-5"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        h.state.get_session("con-cloud-5").is_some(),
        "POST /sessions starts a multi-device runtime"
    );

    let (status, body) = call(
        &full_app(h),
        req_json(
            Method::POST,
            "/api/v1/sessions/con-cloud-5/cloud/connect",
            Some(TEST_TOKEN),
            cloud_credentials(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        h.state.get_session("con-cloud-5").is_none(),
        "the multi-device runtime must be dropped once the session is cloud"
    );

    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let runtime = h.state.get_session("con-cloud-5");
    assert!(
        runtime.as_ref().and_then(|r| r.get_client()).is_none(),
        "no multi-device client may be installed afterwards"
    );

    let (_, body) = call(
        &full_app(h),
        req_get("/api/v1/sessions/con-cloud-5/status", Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(body["status"], "logged_in");
}
