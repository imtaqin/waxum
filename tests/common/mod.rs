//! Shared test harness. Spins up an `AppState` backed by a temporary SQLite
//! file + the full `create_router()` layered with the same JWT middleware
//! main uses, so every test hits the exact same request pipeline as prod.
//!
//! The tests never touch the real WhatsApp client; they only assert the
//! HTTP contract (status codes and response shapes) around the metadata
//! surface: sessions, webhooks, health probes, and the info + metrics
//! endpoints.

use axum::{
    body::{to_bytes, Body},
    http::{HeaderMap, Method, Request, StatusCode},
    Router,
};
use serde_json::Value;
use std::path::PathBuf;
use tempfile::TempDir;
use tower::ServiceExt;

use waxum::db::{schema, session::DbPool, sqlite_raw};
use waxum::middleware;
use waxum::routes::create_router;
use waxum::state::AppState;

pub const TEST_TOKEN: &str = "test-superadmin";

pub struct Harness {
    pub app: Router,
    #[allow(dead_code)]
    pub pool: DbPool,
    #[allow(dead_code)]
    pub state: AppState,
    pub _tmp: TempDir,
}

impl Harness {
    /// Fresh SQLite DB in a per-test temp dir. `SUPERADMIN_TOKEN` is set to
    /// `TEST_TOKEN` for the duration of the test.
    pub async fn new() -> Self {
        unsafe {
            std::env::set_var("SUPERADMIN_TOKEN", TEST_TOKEN);
            std::env::set_var("JWT_SECRET", "test-jwt-secret-value");
        }
        let tmp = TempDir::new().expect("tempdir");
        let db_path: PathBuf = tmp.path().join("waxum.db");
        let sqlite = sqlite_raw::open(db_path.to_str().unwrap()).expect("open sqlite");
        let pool = DbPool::SQLite(sqlite);
        schema::init_schema(&pool).await.expect("init schema");

        let recordings = waxum::storage::RecordingStore::local(tmp.path().to_str().unwrap());
        let state = AppState::new(pool.clone(), None, recordings).await;
        let app: Router = create_router()
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                middleware::jwt::jwt_auth_middleware,
            ))
            .with_state(state.clone());

        Self {
            app,
            pool,
            state,
            _tmp: tmp,
        }
    }
}

pub fn bearer(token: &str) -> String {
    format!("Bearer {}", token)
}

pub async fn call(app: &Router, req: Request<Body>) -> (StatusCode, Value) {
    let res = app.clone().oneshot(req).await.expect("router response");
    let status = res.status();
    let body_bytes = to_bytes(res.into_body(), 1024 * 1024)
        .await
        .expect("body bytes");
    let json: Value = if body_bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body_bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&body_bytes).to_string()))
    };
    (status, json)
}

/// Variant of [`call`] for routes whose body is not JSON.
///
/// [`call`] parses the body as JSON and falls back to wrapping the raw bytes
/// in a `Value::String`, which is lossy for binary payloads and hides the
/// response headers entirely. `GET /calls/{call_id}/recording.wav` serves
/// `audio/wav` and sets `content-disposition`, so it needs both the untouched
/// bytes and the headers to be assertable.
#[allow(dead_code)]
pub async fn call_bytes(app: &Router, req: Request<Body>) -> (StatusCode, HeaderMap, Vec<u8>) {
    let res = app.clone().oneshot(req).await.expect("router response");
    let status = res.status();
    let headers = res.headers().clone();
    let body_bytes = to_bytes(res.into_body(), 8 * 1024 * 1024)
        .await
        .expect("body bytes");
    (status, headers, body_bytes.to_vec())
}

#[allow(dead_code)]
pub fn req_json(method: Method, path: &str, token: Option<&str>, body: Value) -> Request<Body> {
    let mut b = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if let Some(t) = token {
        b = b.header("authorization", bearer(t));
    }
    b.body(Body::from(body.to_string())).expect("build request")
}

/// Carries `allow(dead_code)` for the same reason as `req_json` and
/// `req_delete`: `common` is compiled once per integration-test binary, so a
/// module that only exercises one HTTP method leaves the other constructors
/// unused and would otherwise fail `clippy -D warnings`.
#[allow(dead_code)]
pub fn req_get(path: &str, token: Option<&str>) -> Request<Body> {
    let mut b = Request::builder().method(Method::GET).uri(path);
    if let Some(t) = token {
        b = b.header("authorization", bearer(t));
    }
    b.body(Body::empty()).expect("build request")
}

#[allow(dead_code)]
pub fn req_delete(path: &str, token: Option<&str>) -> Request<Body> {
    let mut b = Request::builder().method(Method::DELETE).uri(path);
    if let Some(t) = token {
        b = b.header("authorization", bearer(t));
    }
    b.body(Body::empty()).expect("build request")
}
