//! Tests for the scheduled-send feature.
//!
//! Unit-level: `send_at` serde parsing, the immediate-vs-scheduled
//! decision, status/event string mappings, and the send-response JSON
//! shape.
//!
//! Integration-level (through the full HTTP pipeline on a temp SQLite
//! DB, no live WhatsApp client): parking a future send, listing per
//! session and fleet-wide, cancelling, and the past-`send_at` fallthrough
//! to the immediate path. The dispatcher loop is not started by the
//! harness, so parked rows deterministically stay `pending`.
mod common;

use axum::http::{Method, StatusCode};
use chrono::{Duration, Utc};
use common::{call, req_delete, req_get, req_json, Harness, TEST_TOKEN};
use serde_json::json;

use waxum::handlers::schedule::{should_schedule, SCHEDULE_GRACE};
use waxum::models::messages::SendTextRequest;
use waxum::models::schedule::{ScheduledStatus, SendResponse};
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

#[test]
fn send_at_parses_iso8601_and_defaults_to_none() {
    let with: SendTextRequest = serde_json::from_value(json!({
        "to": "559999999999@s.whatsapp.net",
        "text": "hi",
        "send_at": "2026-01-01T12:00:00Z"
    }))
    .expect("deserialize with send_at");
    assert_eq!(
        with.send_at.map(|d| d.to_rfc3339()),
        Some("2026-01-01T12:00:00+00:00".to_string())
    );

    let without: SendTextRequest = serde_json::from_value(json!({
        "to": "559999999999@s.whatsapp.net",
        "text": "hi"
    }))
    .expect("deserialize without send_at");
    assert!(without.send_at.is_none());
}

#[test]
fn should_schedule_only_beyond_grace_window() {
    let now = Utc::now();
    assert!(!should_schedule(now - Duration::seconds(60), now));
    assert!(!should_schedule(now, now));
    assert!(!should_schedule(now + SCHEDULE_GRACE, now));
    assert!(should_schedule(
        now + SCHEDULE_GRACE + Duration::seconds(1),
        now
    ));
    assert!(should_schedule(now + Duration::hours(2), now));
}

#[test]
fn scheduled_status_roundtrips_strings() {
    for (s, expect) in [
        ("pending", ScheduledStatus::Pending),
        ("sending", ScheduledStatus::Sending),
        ("sent", ScheduledStatus::Sent),
        ("failed", ScheduledStatus::Failed),
        ("cancelled", ScheduledStatus::Cancelled),
        ("garbage", ScheduledStatus::Pending),
    ] {
        assert_eq!(ScheduledStatus::from_str(s), expect);
        if s != "garbage" {
            assert_eq!(expect.as_str(), s);
        }
    }
    assert_eq!(
        serde_json::to_string(&ScheduledStatus::Pending).unwrap(),
        "\"pending\""
    );
}

#[test]
fn webhook_event_strings_cover_scheduled_events() {
    assert_eq!(WebhookEvent::ScheduledSent.as_str(), "scheduled_sent");
    assert_eq!(WebhookEvent::ScheduledFailed.as_str(), "scheduled_failed");
    assert_eq!(
        WebhookEvent::from_str("scheduled_sent"),
        Some(WebhookEvent::ScheduledSent)
    );
    assert_eq!(
        WebhookEvent::from_str("scheduled_failed"),
        Some(WebhookEvent::ScheduledFailed)
    );
    assert!(WebhookEvent::ScheduledSent.matches("scheduled_sent"));
    assert!(!WebhookEvent::ScheduledSent.matches("scheduled_failed"));
    assert!(WebhookEvent::All.matches("scheduled_sent"));
}

#[test]
fn send_response_shapes_for_sent_and_pending() {
    let sent = SendResponse::sent(waxum::models::messages::MessageResponse {
        message_id: "MID".to_string(),
        timestamp: 123,
        to: "559999999999@s.whatsapp.net".to_string(),
    });
    let v = serde_json::to_value(&sent).unwrap();
    assert_eq!(v["status"], "sent");
    assert_eq!(v["message_id"], "MID");
    assert!(v.get("schedule_id").is_none());

    let at = Utc::now();
    let pending = SendResponse::scheduled("sched-1".to_string(), at);
    let v = serde_json::to_value(&pending).unwrap();
    assert_eq!(v["status"], "pending");
    assert_eq!(v["schedule_id"], "sched-1");
    assert_eq!(v["send_at"], serde_json::to_value(at).unwrap());
    assert!(v.get("message_id").is_none());
}

#[tokio::test]
async fn future_send_at_parks_message_and_lists_it() {
    let h = Harness::new().await;
    seed_session(&h, "sched-s-01").await;

    let send_at = (Utc::now() + Duration::hours(1)).to_rfc3339();
    let (status, body) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions/sched-s-01/messages/text",
            Some(TEST_TOKEN),
            json!({
                "to": "559999999999@s.whatsapp.net",
                "text": "later",
                "send_at": send_at
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "pending");
    let schedule_id = body["schedule_id"].as_str().expect("schedule_id");
    assert!(!schedule_id.is_empty());
    assert!(body.get("message_id").is_none());

    let (status, body) = call(
        &h.app,
        req_get("/api/v1/sessions/sched-s-01/scheduled", Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["count"], 1);
    let row = &body["messages"][0];
    assert_eq!(row["id"], schedule_id);
    assert_eq!(row["session_id"], "sched-s-01");
    assert_eq!(row["endpoint"], "text");
    assert_eq!(row["status"], "pending");

    let (status, body) = call(
        &h.app,
        req_get(
            "/api/v1/sessions/sched-s-01/scheduled?status=sent",
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["count"], 0);

    let (status, body) = call(
        &h.app,
        req_get("/api/v1/scheduled?session=sched-s-01", Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["count"], 1);

    let (status, body) = call(
        &h.app,
        req_get(
            "/api/v1/scheduled?session=sched-s-01&status=pending",
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["count"], 1);
}

#[tokio::test]
async fn past_send_at_falls_through_to_immediate_send() {
    let h = Harness::new().await;
    seed_session(&h, "sched-s-02").await;

    let send_at = (Utc::now() - Duration::seconds(5)).to_rfc3339();
    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions/sched-s-02/messages/text",
            Some(TEST_TOKEN),
            json!({
                "to": "559999999999@s.whatsapp.net",
                "text": "now",
                "send_at": send_at
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);

    let (_, body) = call(
        &h.app,
        req_get("/api/v1/sessions/sched-s-02/scheduled", Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(body["count"], 0);
}

#[tokio::test]
async fn cancel_pending_then_conflict_then_not_found() {
    let h = Harness::new().await;
    seed_session(&h, "sched-s-03").await;

    let send_at = (Utc::now() + Duration::hours(1)).to_rfc3339();
    let (_, body) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions/sched-s-03/messages/text",
            Some(TEST_TOKEN),
            json!({
                "to": "559999999999@s.whatsapp.net",
                "text": "later",
                "send_at": send_at
            }),
        ),
    )
    .await;
    let schedule_id = body["schedule_id"].as_str().expect("schedule_id");
    let path = format!("/api/v1/sessions/sched-s-03/scheduled/{schedule_id}");

    let (status, body) = call(&h.app, req_delete(&path, Some(TEST_TOKEN))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "cancelled");

    let (status, _) = call(&h.app, req_delete(&path, Some(TEST_TOKEN))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _) = call(
        &h.app,
        req_delete(
            "/api/v1/sessions/sched-s-03/scheduled/does-not-exist",
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (_, body) = call(
        &h.app,
        req_get(
            "/api/v1/sessions/sched-s-03/scheduled?status=cancelled",
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(body["count"], 1);
}

#[tokio::test]
async fn scheduling_unknown_session_returns_404() {
    let h = Harness::new().await;
    let send_at = (Utc::now() + Duration::hours(1)).to_rfc3339();
    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions/sched-missing/messages/text",
            Some(TEST_TOKEN),
            json!({
                "to": "559999999999@s.whatsapp.net",
                "text": "later",
                "send_at": send_at
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
