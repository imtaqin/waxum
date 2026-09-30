//! Contract test for the auth gate. Every route under `/api/v1/*` must
//! reject a missing or wrong bearer token; the four probe endpoints
//! (`/livez`, `/readyz`, `/health`, `/metrics`) must not.
mod common;

use axum::http::{Method, StatusCode};
use common::{call, req_get, req_json, Harness, TEST_TOKEN};
use serde_json::json;
use waxum::middleware::jwt::JwtAuth;

/// Matches the `JWT_SECRET` the harness sets so a test can mint its own
/// tokens with `JwtAuth::with_secret` and have them validate against the
/// app under test.
const TEST_JWT_SECRET: &str = "test-jwt-secret-value";

async fn create_session(h: &Harness, id: &str) {
    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions",
            Some(TEST_TOKEN),
            json!({"id": id}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "session {id} setup failed");
}

#[tokio::test]
async fn api_v1_rejects_missing_token() {
    let h = Harness::new().await;
    let (status, _) = call(&h.app, req_get("/api/v1/sessions", None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn api_v1_rejects_wrong_token() {
    let h = Harness::new().await;
    let (status, _) = call(&h.app, req_get("/api/v1/sessions", Some("nope"))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn api_v1_accepts_superadmin_token() {
    let h = Harness::new().await;
    let (status, _) = call(&h.app, req_get("/api/v1/sessions", Some(TEST_TOKEN))).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn probes_bypass_auth() {
    let h = Harness::new().await;
    for path in ["/livez", "/readyz", "/health", "/metrics"] {
        let (status, _) = call(&h.app, req_get(path, None)).await;
        assert_eq!(status, StatusCode::OK, "{} should bypass auth", path);
    }
}

/// Mints a token via `POST /api/v1/tokens` and returns `(id, token)`.
async fn mint_token(h: &Harness, session_ids: &[&str], name: Option<&str>) -> (String, String) {
    let (status, body) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/tokens",
            Some(TEST_TOKEN),
            json!({"session_ids": session_ids, "name": name}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "mint failed: {body:?}");
    let id = body
        .get("id")
        .and_then(|v| v.as_str())
        .expect("id field")
        .to_string();
    let token = body
        .get("token")
        .and_then(|v| v.as_str())
        .expect("token field")
        .to_string();
    (id, token)
}

#[tokio::test]
async fn mint_token_endpoint_returns_a_working_scoped_token() {
    let h = Harness::new().await;
    create_session(&h, "s-scope-a").await;

    let (_, token) = mint_token(&h, &["s-scope-a"], Some("mobile-app")).await;

    let (status, _) = call(&h.app, req_get("/api/v1/sessions/s-scope-a", Some(&token))).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn mint_token_rejects_empty_session_ids() {
    let h = Harness::new().await;
    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/tokens",
            Some(TEST_TOKEN),
            json!({"session_ids": []}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn mint_token_rejects_unknown_session() {
    let h = Harness::new().await;
    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/tokens",
            Some(TEST_TOKEN),
            json!({"session_ids": ["does-not-exist"]}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn scoped_token_cannot_reach_a_different_session() {
    let h = Harness::new().await;
    create_session(&h, "s-scope-mine").await;
    create_session(&h, "s-scope-other").await;

    let (_, token) = mint_token(&h, &["s-scope-mine"], None).await;

    let (status, _) = call(
        &h.app,
        req_get("/api/v1/sessions/s-scope-mine", Some(&token)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "own session must stay reachable");

    let (status, _) = call(
        &h.app,
        req_get("/api/v1/sessions/s-scope-other", Some(&token)),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a token scoped to one session must not reach another"
    );
}

#[tokio::test]
async fn token_can_be_scoped_to_multiple_sessions() {
    let h = Harness::new().await;
    create_session(&h, "s-multi-a").await;
    create_session(&h, "s-multi-b").await;
    create_session(&h, "s-multi-c").await;

    let (_, token) = mint_token(&h, &["s-multi-a", "s-multi-b"], None).await;

    for id in ["s-multi-a", "s-multi-b"] {
        let (status, _) = call(
            &h.app,
            req_get(&format!("/api/v1/sessions/{id}"), Some(&token)),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{id} should be reachable");
    }
    let (status, _) = call(&h.app, req_get("/api/v1/sessions/s-multi-c", Some(&token))).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "unbound session must reject");
}

#[tokio::test]
async fn scoped_token_cannot_reach_fleet_wide_endpoints() {
    let h = Harness::new().await;
    create_session(&h, "s-scope-fleet").await;

    let (_, token) = mint_token(&h, &["s-scope-fleet"], None).await;

    for path in ["/api/v1/sessions/search", "/api/v1/tokens"] {
        let (status, _) = call(&h.app, req_get(path, Some(&token))).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "{path} is fleet-wide and must reject a session-scoped token"
        );
    }
    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions/purge",
            Some(&token),
            json!({}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn revoked_token_is_rejected() {
    let h = Harness::new().await;
    create_session(&h, "s-revoke").await;

    let (id, token) = mint_token(&h, &["s-revoke"], None).await;

    let (status, _) = call(&h.app, req_get("/api/v1/sessions/s-revoke", Some(&token))).await;
    assert_eq!(status, StatusCode::OK, "must work before revocation");

    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            &format!("/api/v1/tokens/{id}/revoke"),
            Some(TEST_TOKEN),
            json!({}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = call(&h.app, req_get("/api/v1/sessions/s-revoke", Some(&token))).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a revoked token must stop working immediately, before its natural expiry"
    );

    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            &format!("/api/v1/tokens/{id}/revoke"),
            Some(TEST_TOKEN),
            json!({}),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "revoking an already-revoked token is a 404, not a silent no-op"
    );
}

#[tokio::test]
async fn list_tokens_shows_metadata_but_never_the_bearer_value() {
    let h = Harness::new().await;
    create_session(&h, "s-list").await;

    let (id, token) = mint_token(&h, &["s-list"], Some("audit-label")).await;

    let (status, body) = call(&h.app, req_get("/api/v1/tokens", Some(TEST_TOKEN))).await;
    assert_eq!(status, StatusCode::OK);
    let raw = body.to_string();
    assert!(
        !raw.contains(&token),
        "token list response must never include the raw bearer value"
    );

    let entries = body.get("tokens").and_then(|v| v.as_array()).unwrap();
    let entry = entries
        .iter()
        .find(|e| e.get("id").and_then(|v| v.as_str()) == Some(id.as_str()))
        .expect("minted token should be listed");
    assert_eq!(
        entry.get("name").and_then(|v| v.as_str()),
        Some("audit-label")
    );
    assert_eq!(entry.get("revoked").and_then(|v| v.as_bool()), Some(false));
    assert_eq!(
        entry
            .get("session_ids")
            .and_then(|v| v.as_array())
            .map(|a| a.len()),
        Some(1)
    );
}

#[tokio::test]
async fn unscoped_jwt_still_reaches_every_session() {
    let h = Harness::new().await;
    create_session(&h, "s-scope-unrestricted").await;

    let jwt = JwtAuth::with_secret(TEST_JWT_SECRET.to_string());
    let token = jwt
        .generate_token("boot", "superadmin", 1, None)
        .expect("generate unscoped token");

    let (status, _) = call(
        &h.app,
        req_get("/api/v1/sessions/s-scope-unrestricted", Some(&token)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = call(&h.app, req_get("/api/v1/sessions", Some(&token))).await;
    assert_eq!(status, StatusCode::OK);
}

async fn mint_prefix_token(h: &Harness, session_ids: &[&str], prefixes: &[&str]) -> String {
    let (status, body) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/tokens",
            Some(TEST_TOKEN),
            json!({"session_ids": session_ids, "session_prefixes": prefixes, "name": "panel"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "mint failed: {body:?}");
    assert_eq!(body["session_prefixes"], json!(prefixes));
    body["token"].as_str().expect("token").to_string()
}

#[tokio::test]
async fn prefix_token_creates_uses_and_deletes_sessions_under_its_prefix() {
    let h = Harness::new().await;
    let token = mint_prefix_token(&h, &[], &["rq-mkt-"]).await;

    let (status, body) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions",
            Some(&token),
            json!({"id": "rq-mkt-8f3k2a"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, _) = call(
        &h.app,
        req_get("/api/v1/sessions/rq-mkt-8f3k2a/status", Some(&token)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = call(
        &h.app,
        common::req_delete("/api/v1/sessions/rq-mkt-8f3k2a", Some(&token)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn prefix_token_cannot_create_or_reach_sessions_outside_its_prefix() {
    let h = Harness::new().await;
    create_session(&h, "other-tenant").await;
    let token = mint_prefix_token(&h, &[], &["rq-mkt-"]).await;

    for body in [
        json!({"id": "other-new"}),
        json!({"id": "rq-mk"}),
        json!({"name": "no id given"}),
    ] {
        let (status, resp) = call(
            &h.app,
            req_json(Method::POST, "/api/v1/sessions", Some(&token), body.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}: {resp}");
    }

    for path in [
        "/api/v1/sessions/other-tenant",
        "/api/v1/sessions/other-tenant/status",
    ] {
        let (status, _) = call(&h.app, req_get(path, Some(&token))).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}");
    }
    let (status, _) = call(
        &h.app,
        common::req_delete("/api/v1/sessions/other-tenant", Some(&token)),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call(
        &h.app,
        req_get("/api/v1/sessions/other-tenant", Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the other tenant's session must survive"
    );
}

#[tokio::test]
async fn scoped_token_lists_only_its_own_sessions() {
    let h = Harness::new().await;
    create_session(&h, "other-tenant").await;
    create_session(&h, "rq-exact").await;
    create_session(&h, "rq-mkt-a").await;
    let token = mint_prefix_token(&h, &["rq-exact"], &["rq-mkt-"]).await;

    let (status, body) = call(&h.app, req_get("/api/v1/sessions", Some(&token))).await;
    assert_eq!(status, StatusCode::OK);
    let mut ids: Vec<String> = body["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap().to_string())
        .collect();
    ids.sort();
    assert_eq!(ids, vec!["rq-exact", "rq-mkt-a"]);
}

#[tokio::test]
async fn a_token_without_prefixes_still_cannot_create_sessions() {
    let h = Harness::new().await;
    create_session(&h, "s-exact-only").await;
    let (_, token) = mint_token(&h, &["s-exact-only"], None).await;
    let (status, _) = call(
        &h.app,
        req_json(
            Method::POST,
            "/api/v1/sessions",
            Some(&token),
            json!({"id": "s-exact-only-2"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn mint_rejects_prefixes_that_are_too_broad_or_malformed() {
    let h = Harness::new().await;
    for prefixes in [json!([""]), json!(["r"]), json!(["rq/"]), json!(["rq mkt"])] {
        let (status, body) = call(
            &h.app,
            req_json(
                Method::POST,
                "/api/v1/tokens",
                Some(TEST_TOKEN),
                json!({"session_prefixes": prefixes}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{prefixes}: {body}");
    }
    let (status, _) = call(
        &h.app,
        req_json(Method::POST, "/api/v1/tokens", Some(TEST_TOKEN), json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
