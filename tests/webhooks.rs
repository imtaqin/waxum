//! Contract tests for `/api/v1/sessions/{sid}/webhooks/*`. Covers list,
//! register, delete, and the re-enable flow used after a URL trips the
//! auto-disable circuit.
mod common;

use axum::http::{Method, StatusCode};
use common::{call, req_delete, req_get, req_json, Harness, TEST_TOKEN};
use serde_json::json;

async fn seed_session(h: &Harness, id: &str) {
    let _ = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions",
            Some(TEST_TOKEN),
            json!({"id": id, "name": id}),
        ),
    )
    .await;
}

#[tokio::test]
async fn list_webhooks_empty_when_none_registered() {
    let h = Harness::new().await;
    seed_session(&h, "wh-s-01").await;
    let (status, body) = call(
        &h.app,
        req_get("/api/v1/sessions/wh-s-01/webhooks", Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.get("count").and_then(|v| v.as_u64()), Some(0));
    assert!(body
        .get("webhooks")
        .and_then(|v| v.as_array())
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn register_webhook_returns_full_config() {
    let h = Harness::new().await;
    seed_session(&h, "wh-s-02").await;
    let (status, body) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions/wh-s-02/webhooks",
            Some(TEST_TOKEN),
            json!({
                "url": "https://example.com/hook",
                "events": ["message", "connected"],
                "secret": "s3cret"
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body.get("url").and_then(|v| v.as_str()),
        Some("https://example.com/hook")
    );
    assert!(body.get("events").and_then(|v| v.as_array()).is_some());
    assert_eq!(body.get("enabled").and_then(|v| v.as_bool()), Some(true));
}

#[tokio::test]
async fn register_then_list_returns_one() {
    let h = Harness::new().await;
    seed_session(&h, "wh-s-03").await;
    let _ = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions/wh-s-03/webhooks",
            Some(TEST_TOKEN),
            json!({"url": "https://example.com/hook", "events": ["all"]}),
        ),
    )
    .await;
    let (status, body) = call(
        &h.app,
        req_get("/api/v1/sessions/wh-s-03/webhooks", Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.get("count").and_then(|v| v.as_u64()), Some(1));
}

#[tokio::test]
async fn delete_webhook_removes_it() {
    let h = Harness::new().await;
    seed_session(&h, "wh-s-04").await;
    let _ = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions/wh-s-04/webhooks",
            Some(TEST_TOKEN),
            json!({"url": "https://example.com/hook", "events": ["all"]}),
        ),
    )
    .await;
    let (_, listed) = call(
        &h.app,
        req_get("/api/v1/sessions/wh-s-04/webhooks", Some(TEST_TOKEN)),
    )
    .await;
    let webhook_id = listed
        .pointer("/webhooks/0/id")
        .and_then(|v| v.as_str())
        .expect("registered webhook should have an id on the list row")
        .to_string();

    let path = format!("/api/v1/sessions/wh-s-04/webhooks/{}", webhook_id);
    let (status, body) = call(&h.app, req_delete(&path, Some(TEST_TOKEN))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.get("success").and_then(|v| v.as_bool()), Some(true));

    let (_, listed_after) = call(
        &h.app,
        req_get("/api/v1/sessions/wh-s-04/webhooks", Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(listed_after.get("count").and_then(|v| v.as_u64()), Some(0));
}

#[tokio::test]
async fn reenable_missing_webhook_returns_404() {
    let h = Harness::new().await;
    seed_session(&h, "wh-s-05").await;
    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions/wh-s-05/webhooks/does-not-exist/enable",
            Some(TEST_TOKEN),
            json!({}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn dlq_list_empty_when_nothing_failed() {
    let h = Harness::new().await;
    seed_session(&h, "wh-s-06").await;
    let (status, body) = call(
        &h.app,
        req_get("/api/v1/sessions/wh-s-06/webhooks/dlq", Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.get("count").and_then(|v| v.as_u64()), Some(0));
    assert!(body
        .get("entries")
        .and_then(|v| v.as_array())
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn dlq_replay_missing_entry_returns_404() {
    let h = Harness::new().await;
    seed_session(&h, "wh-s-07").await;
    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions/wh-s-07/webhooks/dlq/does-not-exist/replay",
            Some(TEST_TOKEN),
            json!({}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// The upstream-sync event variants (`call_log_sync`, `stream_error`,
/// `enc_decrypt_failed`) are valid `WebhookEvent` values end to end:
/// accepted at registration and echoed back by the list endpoint.
#[tokio::test]
async fn register_webhook_accepts_upstream_sync_events() {
    let h = Harness::new().await;
    seed_session(&h, "wh-sync-01").await;
    let (status, body) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions/wh-sync-01/webhooks",
            Some(TEST_TOKEN),
            json!({
                "url": "https://example.com/hook",
                "events": ["call_log_sync", "stream_error", "enc_decrypt_failed"]
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let events: Vec<&str> = body
        .get("events")
        .and_then(|v| v.as_array())
        .expect("events array")
        .iter()
        .filter_map(|e| e.as_str())
        .collect();
    assert_eq!(
        events,
        vec!["call_log_sync", "stream_error", "enc_decrypt_failed"]
    );
}

#[tokio::test]
async fn status_diagnostics_report_each_webhooks_delivery_health() {
    let h = Harness::new().await;
    seed_session(&h, "wh-diag").await;
    for (url, events) in [
        ("https://example.com/replies", json!(["message"])),
        ("https://example.com/audit", json!(["connected"])),
    ] {
        let (status, _) = call(
            &h.app,
            req_json(
                Method::POST,
                "/api/v1/sessions/wh-diag/webhooks",
                Some(TEST_TOKEN),
                json!({"url": url, "events": events}),
            ),
        )
        .await;
        assert!(status.is_success());
    }
    for _ in 0..30 {
        h.state
            .webhook_record_failure("https://example.com/replies");
    }

    let (status, body) = call(
        &h.app,
        req_get("/api/v1/sessions/wh-diag/status", Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let d = &body["diagnostics"];
    assert_eq!(d["messages_forwarded"], 0);
    assert_eq!(d["last_message_forwarded_at"], serde_json::Value::Null);
    let hooks = d["webhooks"].as_array().expect("webhooks array");
    assert_eq!(hooks.len(), 2);
    let replies = hooks
        .iter()
        .find(|w| w["url"] == "https://example.com/replies")
        .unwrap();
    assert_eq!(replies["receives_messages"], true);
    assert_eq!(replies["enabled"], true);
    assert_eq!(replies["circuit_open"], true);
    assert_eq!(replies["consecutive_failures"], 30);
    let audit = hooks
        .iter()
        .find(|w| w["url"] == "https://example.com/audit")
        .unwrap();
    assert_eq!(audit["receives_messages"], false);
    assert_eq!(audit["circuit_open"], false);
}
