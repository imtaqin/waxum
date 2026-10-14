//! Axum router assembly.
//!
//! [`create_router`] wires:
//!
//! - `/health`, `/livez`, `/readyz` — DB-free probes (Docker HEALTHCHECK).
//! - `/metrics` — Prometheus text exposition (JWT bypass).
//! - `/api/v1/info` — build info and version.
//! - `/api/v1/*` — everything else, assembled from three channels: [`shared`]
//!   (works on any session), [`web`] (linked WhatsApp Web device) and
//!   [`cloud`] (official Cloud API). Each feature folder owns its own routes;
//!   this file only mounts them.

use axum::{routing::get, Router};

use super::{cloud, shared, web};
use crate::state::AppState;

/// Build the fully-wired axum router used by [`crate::main`].
pub fn create_router() -> Router<AppState> {
    Router::new()
        .nest(
            "/api/v1",
            api_routes().layer(axum::middleware::from_fn(
                shared::new_chats::handlers::rollback_failed_sends,
            )),
        )
        .route("/health", get(health_check))
        .route("/livez", get(livez))
        .route("/readyz", get(readyz))
        .route("/metrics", get(crate::metrics::metrics_handler))
        .route("/api/v1/info", get(shared::info::handlers::get_info))
}

fn api_routes() -> Router<AppState> {
    Router::new()
        .nest("/sessions", session_routes())
        .nest("/nats", shared::nats_routes())
        .merge(shared::api_routes())
        .merge(web::api_routes())
}

fn session_routes() -> Router<AppState> {
    Router::new()
        .merge(shared::session_routes())
        .merge(web::session_routes())
        .merge(cloud::session_routes())
}

async fn health_check() -> &'static str {
    "OK"
}

async fn livez() -> &'static str {
    "OK"
}

/// Deeper readiness probe: verifies the DB pool answers a trivial query
/// and that the process actually has session runtimes registered. Used
/// by Kubernetes-style readiness gates that want to gate traffic until
/// the gateway is fully warm.
async fn readyz(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> axum::response::Response {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use serde_json::json;

    let db_ok = match state.session_manager().pool() {
        crate::db::session::DbPool::Postgres(pool) => match pool.get().await {
            Ok(client) => client.simple_query("SELECT 1").await.is_ok(),
            Err(_) => false,
        },
        crate::db::session::DbPool::MySQL(pool) => {
            use mysql_async::prelude::*;
            match pool.get_conn().await {
                Ok(mut conn) => conn.query_drop("SELECT 1").await.is_ok(),
                Err(_) => false,
            }
        }
        crate::db::session::DbPool::SQLite(handle) => {
            let h = handle.clone();
            tokio::task::spawn_blocking(move || {
                let guard = h.lock();
                crate::db::sqlite_raw::exec_batch(&guard, "SELECT 1").is_ok()
            })
            .await
            .unwrap_or(false)
        }
    };

    let sessions = state
        .session_manager()
        .list_sessions()
        .await
        .map(|s| s.len())
        .unwrap_or(0);
    let session_states: Vec<serde_json::Value> = state
        .session_iter_with_ids()
        .iter()
        .map(|(id, runtime)| {
            json!({
                "id": id,
                "status": runtime.get_status().as_str(),
                "socket_alive": runtime.socket_alive(),
            })
        })
        .collect();
    let sessions_live = session_states
        .iter()
        .filter(|s| {
            s.get("socket_alive")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        })
        .count();
    let body = json!({
        "db": if db_ok { "ok" } else { "fail" },
        "sessions_known": sessions,
        "sessions_live": sessions_live,
        "sessions": session_states,
    });
    let status = if db_ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (status, axum::Json(body)).into_response()
}
