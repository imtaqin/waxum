//! Contract tests for the `/api/v1/sessions/*` metadata surface: create,
//! list, get, delete, and the `/status` sub-route. These only touch the
//! DB layer; no WA client is spun up, so `connect_session` and the
//! message-send endpoints stay out of scope (they need a paired session).
mod common;

use axum::http::{Method, StatusCode};
use common::{call, req_delete, req_get, req_json, Harness, TEST_TOKEN};
use serde_json::json;

#[tokio::test]
async fn list_sessions_is_empty_on_fresh_db() {
    let h = Harness::new().await;
    let (status, body) = call(&h.app, req_get("/api/v1/sessions", Some(TEST_TOKEN))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.get("total").and_then(|v| v.as_u64()), Some(0));
    assert!(body.get("sessions").and_then(|v| v.as_array()).unwrap().is_empty());
}

#[tokio::test]
async fn create_session_returns_session_shape() {
    let h = Harness::new().await;
    let (status, body) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions",
            Some(TEST_TOKEN),
            json!({"id": "s-01", "name": "First"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let session = body.get("session").expect("session field");
    assert_eq!(session.get("id").and_then(|v| v.as_str()), Some("s-01"));
    assert_eq!(session.get("name").and_then(|v| v.as_str()), Some("First"));
    assert!(session.get("status").is_some());
    assert!(session.get("is_logged_in").is_some());
}

#[tokio::test]
async fn create_session_generates_uuid_when_no_id_given() {
    let h = Harness::new().await;
    let (status, body) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions",
            Some(TEST_TOKEN),
            json!({"name": "Auto ID"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let id = body
        .pointer("/session/id")
        .and_then(|v| v.as_str())
        .expect("generated id");
    // UUIDv4 is 36 chars, hyphenated
    assert_eq!(id.len(), 36);
    assert_eq!(id.chars().filter(|c| *c == '-').count(), 4);
}

#[tokio::test]
async fn create_session_conflicts_on_duplicate_id() {
    let h = Harness::new().await;
    let payload = json!({"id": "s-dup", "name": "First"});
    let (status1, _) = call(
        &h.app,
        req_json(Method::POST, "/api/v1/sessions", Some(TEST_TOKEN), payload.clone()),
    )
    .await;
    assert_eq!(status1, StatusCode::OK);
    let (status2, _) = call(
        &h.app,
        req_json(Method::POST, "/api/v1/sessions", Some(TEST_TOKEN), payload),
    )
    .await;
    assert_eq!(status2, StatusCode::CONFLICT);
}

#[tokio::test]
async fn get_session_404_when_missing() {
    let h = Harness::new().await;
    let (status, _) = call(
        &h.app,
        req_get("/api/v1/sessions/nope", Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn get_session_returns_stored_row() {
    let h = Harness::new().await;
    let _ = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions",
            Some(TEST_TOKEN),
            json!({"id": "s-get", "name": "Gettable"}),
        ),
    )
    .await;
    let (status, body) = call(
        &h.app,
        req_get("/api/v1/sessions/s-get", Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.get("id").and_then(|v| v.as_str()), Some("s-get"));
    assert_eq!(body.get("name").and_then(|v| v.as_str()), Some("Gettable"));
}

#[tokio::test]
async fn list_after_creates_returns_all_rows() {
    let h = Harness::new().await;
    for i in 0..3 {
        let _ = call(
            &h.app,
            req_json(
                Method::POST,
                "/api/v1/sessions",
                Some(TEST_TOKEN),
                json!({"id": format!("s-{}", i), "name": format!("N{}", i)}),
            ),
        )
        .await;
    }
    let (status, body) = call(&h.app, req_get("/api/v1/sessions", Some(TEST_TOKEN))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.get("total").and_then(|v| v.as_u64()), Some(3));
    let sessions = body.get("sessions").and_then(|v| v.as_array()).unwrap();
    assert_eq!(sessions.len(), 3);
}

#[tokio::test]
async fn delete_session_removes_row() {
    let h = Harness::new().await;
    let _ = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions",
            Some(TEST_TOKEN),
            json!({"id": "s-del", "name": "Deletable"}),
        ),
    )
    .await;
    let (status, body) = call(
        &h.app,
        req_delete("/api/v1/sessions/s-del", Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.get("success").and_then(|v| v.as_bool()), Some(true));

    let (status, _) = call(
        &h.app,
        req_get("/api/v1/sessions/s-del", Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delete_missing_session_returns_404() {
    let h = Harness::new().await;
    let (status, _) = call(
        &h.app,
        req_delete("/api/v1/sessions/ghost", Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn get_session_status_shape_matches_list_entry() {
    // The two endpoints answering "is this session logged in?" must
    // return the same shape so a client can rely on either. Reported in
    // https://github.com/imtaqin/waxum/issues/33.
    let h = Harness::new().await;
    let _ = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions",
            Some(TEST_TOKEN),
            json!({"id": "s-stat", "name": "S"}),
        ),
    )
    .await;
    let (list_status, list_body) = call(&h.app, req_get("/api/v1/sessions", Some(TEST_TOKEN))).await;
    assert_eq!(list_status, StatusCode::OK);
    let entry = list_body
        .get("sessions")
        .and_then(|v| v.as_array())
        .and_then(|a| a.iter().find(|s| s.get("id").and_then(|v| v.as_str()) == Some("s-stat")))
        .expect("row for s-stat");
    let entry_status = entry.get("status").and_then(|v| v.as_str());
    let entry_logged = entry.get("is_logged_in").and_then(|v| v.as_bool());

    let (get_status, get_body) = call(
        &h.app,
        req_get("/api/v1/sessions/s-stat/status", Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(get_status, StatusCode::OK);
    let get_status_str = get_body.get("status").and_then(|v| v.as_str());
    let get_logged = get_body.get("is_logged_in").and_then(|v| v.as_bool());

    assert_eq!(get_status_str, entry_status);
    assert_eq!(get_logged, entry_logged);
}
