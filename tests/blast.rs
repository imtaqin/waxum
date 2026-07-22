//! Tests for the blast (bulk-send) feature.
//!
//! Unit-level: request serde and option defaults, status/event string
//! mappings, and endpoint/body validation.
//!
//! Integration-level (through the full HTTP pipeline on a temp SQLite
//! DB, no live WhatsApp client): job creation with intra-array dedup,
//! validation failures (unknown endpoint, bad body, bad recipients,
//! unknown session), listing per session and fleet-wide, recipients
//! pagination, the cancel flow, and the retry guard rails. The blast
//! worker is not started by the harness, so jobs deterministically
//! stay `pending` and no sends execute.
mod common;

use axum::http::{Method, StatusCode};
use common::{call, req_get, req_json, Harness, TEST_TOKEN};
use serde_json::json;

use waxum::handlers::schedule::validate_body;
use waxum::models::blast::{
    BlastJobStatus, BlastOptions, BlastRecipientStatus, CreateBlastRequest,
};
use waxum::models::webhooks::WebhookEvent;

async fn seed_session(h: &Harness, id: &str) {
    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions",
            Some(TEST_TOKEN),
            json!({"id": id, "name": id}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

fn blast_body(recipients: serde_json::Value) -> serde_json::Value {
    json!({
        "endpoint": "text",
        "body": {"to": "placeholder@s.whatsapp.net", "text": "promo"},
        "recipients": recipients,
        "delay_ms": 5
    })
}

#[test]
fn create_request_serde_and_option_defaults() {
    let minimal: CreateBlastRequest = serde_json::from_value(json!({
        "endpoint": "text",
        "body": {"to": "a@s.whatsapp.net", "text": "hi"},
        "recipients": ["1@s.whatsapp.net"]
    }))
    .expect("deserialize minimal request");
    let opts = minimal.options();
    assert_eq!(opts.delay_ms, 1000);
    assert_eq!(opts.jitter_ms, 0);
    assert_eq!(opts.max_attempts, 3);
    assert!(!opts.dedup_across_jobs);
    assert!(minimal.send_at.is_none());

    let full: CreateBlastRequest = serde_json::from_value(json!({
        "endpoint": "text",
        "body": {"to": "a@s.whatsapp.net", "text": "hi"},
        "recipients": ["1@s.whatsapp.net"],
        "delay_ms": 50,
        "jitter_ms": 10,
        "max_attempts": 5,
        "dedup_across_jobs": true,
        "send_at": "2026-01-01T12:00:00Z"
    }))
    .expect("deserialize full request");
    let opts = full.options();
    assert_eq!(opts.delay_ms, 50);
    assert_eq!(opts.jitter_ms, 10);
    assert_eq!(opts.max_attempts, 5);
    assert!(opts.dedup_across_jobs);
    assert_eq!(
        full.send_at.map(|d| d.to_rfc3339()),
        Some("2026-01-01T12:00:00+00:00".to_string())
    );
}

#[test]
fn blast_status_enums_roundtrip_strings() {
    for (s, expect) in [
        ("pending", BlastJobStatus::Pending),
        ("running", BlastJobStatus::Running),
        ("completed", BlastJobStatus::Completed),
        (
            "completed_with_failures",
            BlastJobStatus::CompletedWithFailures,
        ),
        ("cancelled", BlastJobStatus::Cancelled),
        ("failed", BlastJobStatus::Failed),
        ("garbage", BlastJobStatus::Pending),
    ] {
        assert_eq!(BlastJobStatus::from_str(s), expect);
        if s != "garbage" {
            assert_eq!(expect.as_str(), s);
        }
    }
    assert!(BlastJobStatus::Completed.is_terminal());
    assert!(BlastJobStatus::CompletedWithFailures.is_terminal());
    assert!(BlastJobStatus::Cancelled.is_terminal());
    assert!(BlastJobStatus::Failed.is_terminal());
    assert!(!BlastJobStatus::Pending.is_terminal());
    assert!(!BlastJobStatus::Running.is_terminal());

    for (s, expect) in [
        ("pending", BlastRecipientStatus::Pending),
        ("sending", BlastRecipientStatus::Sending),
        ("sent", BlastRecipientStatus::Sent),
        ("failed", BlastRecipientStatus::Failed),
        ("dlq", BlastRecipientStatus::Dlq),
        ("skipped_dup", BlastRecipientStatus::SkippedDup),
        ("garbage", BlastRecipientStatus::Pending),
    ] {
        assert_eq!(BlastRecipientStatus::from_str(s), expect);
        if s != "garbage" {
            assert_eq!(expect.as_str(), s);
        }
    }
}

#[test]
fn webhook_event_strings_cover_blast_events() {
    assert_eq!(WebhookEvent::BlastProgress.as_str(), "blast_progress");
    assert_eq!(WebhookEvent::BlastCompleted.as_str(), "blast_completed");
    assert_eq!(
        WebhookEvent::from_str("blast_progress"),
        Some(WebhookEvent::BlastProgress)
    );
    assert_eq!(
        WebhookEvent::from_str("blast_completed"),
        Some(WebhookEvent::BlastCompleted)
    );
    assert!(WebhookEvent::BlastProgress.matches("blast_progress"));
    assert!(!WebhookEvent::BlastProgress.matches("blast_completed"));
    assert!(WebhookEvent::All.matches("blast_completed"));
}

#[test]
fn validate_body_accepts_known_endpoint_and_rejects_garbage() {
    assert!(validate_body("text", &json!({"to": "a@s.whatsapp.net", "text": "hi"})).is_ok());
    assert!(validate_body("text", &json!({"to": "a@s.whatsapp.net"})).is_err());
    assert!(validate_body("definitely-not-an-endpoint", &json!({})).is_err());
}

#[test]
fn options_default_matches_documented_values() {
    let d = BlastOptions::default();
    assert_eq!(d.delay_ms, 1000);
    assert_eq!(d.jitter_ms, 0);
    assert_eq!(d.max_attempts, 3);
    assert!(!d.dedup_across_jobs);
}

#[tokio::test]
async fn create_blast_dedups_recipients_and_lists_job() {
    let h = Harness::new().await;
    seed_session(&h, "blast-s-01").await;

    let (status, body) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions/blast-s-01/blast",
            Some(TEST_TOKEN),
            blast_body(json!([
                "559999999999@s.whatsapp.net",
                "558888888888",
                "559999999999@s.whatsapp.net"
            ])),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], 2);
    assert_eq!(body["skipped_dup"], 1);
    assert_eq!(body["status"], "pending");
    let job_id = body["job_id"].as_str().expect("job_id");
    assert!(!job_id.is_empty());

    let (status, body) = call(
        &h.app,
        req_get("/api/v1/sessions/blast-s-01/blasts", Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["count"], 1);
    let job = &body["jobs"][0];
    assert_eq!(job["id"], job_id);
    assert_eq!(job["session_id"], "blast-s-01");
    assert_eq!(job["endpoint"], "text");
    assert_eq!(job["status"], "pending");
    assert_eq!(job["total"], 2);
    assert_eq!(job["skipped_dup_count"], 1);
    assert_eq!(job["options"]["delay_ms"], 5);
    assert_eq!(job["options"]["max_attempts"], 3);

    let (status, body) = call(
        &h.app,
        req_get(
            &format!("/api/v1/sessions/blast-s-01/blasts/{job_id}"),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["id"], job_id);

    let (status, body) = call(
        &h.app,
        req_get(
            &format!("/api/v1/sessions/blast-s-01/blasts/{job_id}/recipients"),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["count"], 2);
    assert_eq!(body["recipients"][0]["status"], "pending");
    let recipients: Vec<&str> = body["recipients"]
        .as_array()
        .expect("recipients array")
        .iter()
        .filter_map(|r| r["recipient"].as_str())
        .collect();
    assert!(recipients.contains(&"559999999999@s.whatsapp.net"));
    assert!(recipients.contains(&"558888888888@s.whatsapp.net"));

    let (status, body) = call(
        &h.app,
        req_get(
            &format!("/api/v1/sessions/blast-s-01/blasts/{job_id}/recipients?status=sent"),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["count"], 0);

    let (_, body) = call(
        &h.app,
        req_get(
            "/api/v1/sessions/blast-s-01/blasts?status=running",
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(body["count"], 0);

    let (_, body) = call(
        &h.app,
        req_get("/api/v1/blasts?session=blast-s-01", Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(body["count"], 1);

    let (_, body) = call(
        &h.app,
        req_get(
            "/api/v1/blasts?session=blast-s-01&status=pending",
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(body["count"], 1);
}

#[tokio::test]
async fn create_blast_validation_failures() {
    let h = Harness::new().await;
    seed_session(&h, "blast-s-02").await;
    let path = "/api/v1/sessions/blast-s-02/blast";

    let (status, body) = call(
        &h.app,
        req_json(
            Method::POST,
            path,
            Some(TEST_TOKEN),
            json!({
                "endpoint": "definitely-not-an-endpoint",
                "body": {"to": "a@s.whatsapp.net", "text": "hi"},
                "recipients": ["559999999999@s.whatsapp.net"]
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .contains("unknown endpoint"));

    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            path,
            Some(TEST_TOKEN),
            json!({
                "endpoint": "text",
                "body": {"unexpected": true},
                "recipients": ["559999999999@s.whatsapp.net"]
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, body) = call(
        &h.app,
        req_json(
            Method::POST,
            path,
            Some(TEST_TOKEN),
            blast_body(json!([
                "559999999999@s.whatsapp.net",
                "!!!",
                "also not a jid"
            ])),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let msg = body["error"]["message"].as_str().unwrap_or_default();
    assert!(msg.contains("!!!"));
    assert!(msg.contains("also not a jid"));

    let (status, _) = call(
        &h.app,
        req_json(Method::POST, path, Some(TEST_TOKEN), blast_body(json!([]))),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn create_blast_unknown_session_returns_404() {
    let h = Harness::new().await;
    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions/blast-missing/blast",
            Some(TEST_TOKEN),
            blast_body(json!(["559999999999@s.whatsapp.net"])),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn dedup_across_jobs_on_fresh_session_skips_nothing() {
    let h = Harness::new().await;
    seed_session(&h, "blast-s-03").await;

    let mut payload = blast_body(json!(["559999999999@s.whatsapp.net", "558888888888"]));
    payload["dedup_across_jobs"] = json!(true);
    let (status, body) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions/blast-s-03/blast",
            Some(TEST_TOKEN),
            payload,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], 2);
    assert_eq!(body["skipped_dup"], 0);
}

#[tokio::test]
async fn cancel_flow_then_conflict_then_not_found() {
    let h = Harness::new().await;
    seed_session(&h, "blast-s-04").await;

    let (_, body) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions/blast-s-04/blast",
            Some(TEST_TOKEN),
            blast_body(json!(["559999999999@s.whatsapp.net"])),
        ),
    )
    .await;
    let job_id = body["job_id"].as_str().expect("job_id");
    let cancel_path = format!("/api/v1/sessions/blast-s-04/blasts/{job_id}/cancel");

    let (status, body) = call(
        &h.app,
        req_json(Method::POST, &cancel_path, Some(TEST_TOKEN), json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "cancelled");
    assert!(body["finished_at"].is_string());

    let (status, _) = call(
        &h.app,
        req_json(Method::POST, &cancel_path, Some(TEST_TOKEN), json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions/blast-s-04/blasts/does-not-exist/cancel",
            Some(TEST_TOKEN),
            json!({}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (_, body) = call(
        &h.app,
        req_get(
            &format!("/api/v1/sessions/blast-s-04/blasts/{job_id}/recipients"),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(body["count"], 1);
    assert_eq!(body["recipients"][0]["status"], "pending");

    let (_, body) = call(
        &h.app,
        req_get(
            "/api/v1/sessions/blast-s-04/blasts?status=cancelled",
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(body["count"], 1);
}

#[tokio::test]
async fn retry_without_dlq_recipients_is_a_400() {
    let h = Harness::new().await;
    seed_session(&h, "blast-s-05").await;

    let (_, body) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions/blast-s-05/blast",
            Some(TEST_TOKEN),
            blast_body(json!(["559999999999@s.whatsapp.net"])),
        ),
    )
    .await;
    let job_id = body["job_id"].as_str().expect("job_id");

    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            &format!("/api/v1/sessions/blast-s-05/blasts/{job_id}/retry"),
            Some(TEST_TOKEN),
            json!({}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions/blast-s-05/blasts/does-not-exist/retry",
            Some(TEST_TOKEN),
            json!({}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn detail_and_recipients_of_unknown_job_are_404() {
    let h = Harness::new().await;
    seed_session(&h, "blast-s-06").await;

    let (status, _) = call(
        &h.app,
        req_get("/api/v1/sessions/blast-s-06/blasts/nope", Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = call(
        &h.app,
        req_get(
            "/api/v1/sessions/blast-s-06/blasts/nope/recipients",
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delayed_start_is_stored_on_the_job() {
    let h = Harness::new().await;
    seed_session(&h, "blast-s-07").await;

    let mut payload = blast_body(json!(["559999999999@s.whatsapp.net"]));
    payload["send_at"] = json!("2026-06-01T12:00:00Z");
    let (status, body) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions/blast-s-07/blast",
            Some(TEST_TOKEN),
            payload,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let job_id = body["job_id"].as_str().expect("job_id");

    let (_, body) = call(
        &h.app,
        req_get(
            &format!("/api/v1/sessions/blast-s-07/blasts/{job_id}"),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(body["status"], "pending");
    assert_eq!(body["send_at"], "2026-06-01T12:00:00Z");
}
