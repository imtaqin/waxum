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
    assert!(body
        .get("sessions")
        .and_then(|v| v.as_array())
        .unwrap()
        .is_empty());
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
    assert_eq!(id.len(), 36);
    assert_eq!(id.chars().filter(|c| *c == '-').count(), 4);
}

#[tokio::test]
async fn create_session_conflicts_on_duplicate_id() {
    let h = Harness::new().await;
    let payload = json!({"id": "s-dup", "name": "First"});
    let (status1, _) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions",
            Some(TEST_TOKEN),
            payload.clone(),
        ),
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
    let (status, _) = call(&h.app, req_get("/api/v1/sessions/nope", Some(TEST_TOKEN))).await;
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
    let (status, body) = call(&h.app, req_get("/api/v1/sessions/s-get", Some(TEST_TOKEN))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.get("id").and_then(|v| v.as_str()), Some("s-get"));
    assert_eq!(body.get("name").and_then(|v| v.as_str()), Some("Gettable"));
}

/// `GET /sessions/{id}` (singular) used to trust the raw cached status with
/// no reconciliation, unlike `GET /sessions` (list) and `GET
/// /sessions/{id}/status`, which both degrade a cached `logged_in` back to
/// `connecting` when the socket isn't actually alive. That gap meant the
/// same session could report `logged_in` from one endpoint and `connecting`
/// from another at the same instant -- reproduced here by setting the
/// runtime's cached status to `LoggedIn` with no live client attached
/// (exactly what's left behind when a socket dies without the crate ever
/// dispatching a disconnect event) and asserting the singular endpoint
/// degrades it the same way the others already did.
#[tokio::test]
async fn get_session_reconciles_stale_logged_in_status_with_no_live_client() {
    let h = Harness::new().await;
    let _ = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions",
            Some(TEST_TOKEN),
            json!({"id": "s-stale"}),
        ),
    )
    .await;

    let runtime = h.state.get_or_create_session("s-stale", "/tmp/s-stale");
    runtime.set_status(waxum::api::shared::sessions::models::SessionStatus::LoggedIn);

    let (status, body) = call(
        &h.app,
        req_get("/api/v1/sessions/s-stale", Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body.get("status").and_then(|v| v.as_str()),
        Some("connecting"),
        "a cached logged_in status with no live client must degrade, not report stale: {body:?}"
    );
    assert_eq!(
        body.get("is_logged_in").and_then(|v| v.as_bool()),
        Some(false)
    );
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

    let (status, _) = call(&h.app, req_get("/api/v1/sessions/s-del", Some(TEST_TOKEN))).await;
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
    let (list_status, list_body) =
        call(&h.app, req_get("/api/v1/sessions", Some(TEST_TOKEN))).await;
    assert_eq!(list_status, StatusCode::OK);
    let entry = list_body
        .get("sessions")
        .and_then(|v| v.as_array())
        .and_then(|a| {
            a.iter()
                .find(|s| s.get("id").and_then(|v| v.as_str()) == Some("s-stat"))
        })
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

/// Issue #70: `GET/PUT /sessions/{id}/reconnect` used to 503 whenever the
/// socket was down — but toggling auto-reconnect is exactly what's needed
/// while a session is down. The endpoints now serve the stored preference
/// from the session runtime, so they must answer 200 with no live client.
#[tokio::test]
async fn reconnect_config_is_serviceable_without_live_socket() {
    let h = Harness::new().await;
    let _ = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions",
            Some(TEST_TOKEN),
            json!({"id": "s-rc"}),
        ),
    )
    .await;

    let (status, body) = call(
        &h.app,
        req_get("/api/v1/sessions/s-rc/reconnect", Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.get("enabled").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(body.get("error_count").and_then(|v| v.as_u64()), Some(0));

    let (status, body) = call(
        &h.app,
        req_json(
            Method::PUT,
            "/api/v1/sessions/s-rc/reconnect",
            Some(TEST_TOKEN),
            json!({"enabled": false}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.get("enabled").and_then(|v| v.as_bool()), Some(false));

    let (status, body) = call(
        &h.app,
        req_get("/api/v1/sessions/s-rc/reconnect", Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body.get("enabled").and_then(|v| v.as_bool()),
        Some(false),
        "the preference must persist on the runtime, not the live client"
    );

    let (status, _) = call(
        &h.app,
        req_get("/api/v1/sessions/ghost/reconnect", Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// Issue #70, same flaw on the history-sync config pair.
#[tokio::test]
async fn history_sync_config_is_serviceable_without_live_socket() {
    let h = Harness::new().await;
    let _ = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions",
            Some(TEST_TOKEN),
            json!({"id": "s-hs"}),
        ),
    )
    .await;

    let (status, body) = call(
        &h.app,
        req_get("/api/v1/sessions/s-hs/history-sync", Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body.get("skip_history_sync").and_then(|v| v.as_bool()),
        Some(false)
    );

    let (status, body) = call(
        &h.app,
        req_json(
            Method::PUT,
            "/api/v1/sessions/s-hs/history-sync",
            Some(TEST_TOKEN),
            json!({"skip": true}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body.get("skip_history_sync").and_then(|v| v.as_bool()),
        Some(true)
    );

    let (status, _) = call(
        &h.app,
        req_get("/api/v1/sessions/ghost/history-sync", Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// Issue #71: /status must expose raw socket liveness separately from
/// `is_logged_in`, so a cached `logged_in` that outlives its socket
/// ("limbo") is detectable. With no client attached the field is false.
#[tokio::test]
async fn status_reports_socket_alive_separately_from_login_state() {
    let h = Harness::new().await;
    let _ = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions",
            Some(TEST_TOKEN),
            json!({"id": "s-limbo"}),
        ),
    )
    .await;

    let runtime = h.state.get_or_create_session("s-limbo", "/tmp/s-limbo");
    runtime.set_status(waxum::api::shared::sessions::models::SessionStatus::LoggedIn);

    let (status, body) = call(
        &h.app,
        req_get("/api/v1/sessions/s-limbo/status", Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body.get("is_logged_in").and_then(|v| v.as_bool()),
        Some(true),
        "cached login state is still reported (status degrades to connecting)"
    );
    assert_eq!(
        body.get("socket_alive").and_then(|v| v.as_bool()),
        Some(false),
        "no live client means the socket is down regardless of cached status"
    );
}

/// Issue #69: re-scanning must be possible on the SAME session slot so a
/// consumer's `session_id`-keyed state survives a re-pair. Without
/// `reuse`, an existing ID still conflicts; with `reuse: true` the same
/// row is returned and a fresh connect is started on it.
#[tokio::test]
async fn create_with_reuse_keeps_session_id_on_repair() {
    let h = Harness::new().await;
    let (status, body) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions",
            Some(TEST_TOKEN),
            json!({"id": "s-repair", "name": "Original"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body.pointer("/session/id").and_then(|v| v.as_str()),
        Some("s-repair")
    );

    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions",
            Some(TEST_TOKEN),
            json!({"id": "s-repair"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    let (status, body) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions",
            Some(TEST_TOKEN),
            json!({"id": "s-repair", "reuse": true}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body.pointer("/session/id").and_then(|v| v.as_str()),
        Some("s-repair")
    );
    assert_eq!(
        body.pointer("/session/name").and_then(|v| v.as_str()),
        Some("Original"),
        "reuse must keep the existing DB row, not overwrite it"
    );

    let (_, list) = call(&h.app, req_get("/api/v1/sessions", Some(TEST_TOKEN))).await;
    assert_eq!(list.get("total").and_then(|v| v.as_u64()), Some(1));
}

/// Issue #69: `reuse: true` on an ID that does not exist yet just
/// behaves like a normal create.
#[tokio::test]
async fn create_with_reuse_on_unknown_id_creates_normally() {
    let h = Harness::new().await;
    let (status, body) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions",
            Some(TEST_TOKEN),
            json!({"id": "s-fresh", "reuse": true}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body.pointer("/session/id").and_then(|v| v.as_str()),
        Some("s-fresh")
    );
}

/// `POST /sessions/{id}/pause` and `/resume` drive the upstream
/// `Client::pause`/`resume`, so they need an installed client instance —
/// a session that exists only as a DB row gets 503, not 404/200.
#[tokio::test]
async fn pause_resume_require_an_installed_client() {
    let h = Harness::new().await;
    let _ = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions",
            Some(TEST_TOKEN),
            json!({"id": "s-pause"}),
        ),
    )
    .await;

    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions/s-pause/pause",
            Some(TEST_TOKEN),
            json!({}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);

    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions/s-pause/resume",
            Some(TEST_TOKEN),
            json!({}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);

    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions/ghost/pause",
            Some(TEST_TOKEN),
            json!({}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

/// `/status` exposes the upstream `Client::is_paused` state; with no
/// client installed it must report `false` rather than omit the field.
#[tokio::test]
async fn status_reports_paused_false_without_client() {
    let h = Harness::new().await;
    let _ = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions",
            Some(TEST_TOKEN),
            json!({"id": "s-paused-f"}),
        ),
    )
    .await;
    let (status, body) = call(
        &h.app,
        req_get("/api/v1/sessions/s-paused-f/status", Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.get("paused").and_then(|v| v.as_bool()), Some(false));
}

/// `POST /sessions/{id}/appstate/resync` validates collection names and
/// the mode before ever touching the client, so bad input is a 400 even
/// with no session connected.
#[tokio::test]
async fn appstate_resync_validates_request_body() {
    let h = Harness::new().await;
    let _ = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions",
            Some(TEST_TOKEN),
            json!({"id": "s-resync"}),
        ),
    )
    .await;

    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions/s-resync/appstate/resync",
            Some(TEST_TOKEN),
            json!({"collections": ["bogus_collection"]}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions/s-resync/appstate/resync",
            Some(TEST_TOKEN),
            json!({"collections": ["regular"], "mode": "full"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions/s-resync/appstate/resync",
            Some(TEST_TOKEN),
            json!({"collections": []}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions/s-resync/appstate/resync",
            Some(TEST_TOKEN),
            json!({"collections": ["critical_block", "regular"], "mode": "snapshot"}),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "a valid request with no live client reaches the 503 gate"
    );
}

/// `GET /sessions/{id}/messages` pages the upstream chat store, which only
/// exists while a client is installed — a session with a runtime but no
/// connected client answers 503, an unknown session 404.
#[tokio::test]
async fn list_session_messages_requires_runtime() {
    let h = Harness::new().await;
    let _ = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions",
            Some(TEST_TOKEN),
            json!({"id": "s-arrival"}),
        ),
    )
    .await;

    let (status, _) = call(
        &h.app,
        req_get("/api/v1/sessions/s-arrival/messages", Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);

    let (status, _) = call(
        &h.app,
        req_get("/api/v1/sessions/ghost/messages", Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
