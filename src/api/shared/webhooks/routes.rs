//! Routes for webhooks.

use super::handlers;
use crate::state::AppState;
use axum::routing::{delete, get, post};
use axum::Router;

pub fn session_routes() -> Router<AppState> {
    Router::new()
        .route("/{session_id}/webhooks", get(handlers::list_webhooks))
        .route("/{session_id}/webhooks", post(handlers::register_webhook))
        .route(
            "/{session_id}/webhooks/dlq",
            get(handlers::list_webhook_dlq),
        )
        .route(
            "/{session_id}/webhooks/dlq/{entry_id}/replay",
            post(handlers::replay_webhook_dlq_entry),
        )
        .route(
            "/{session_id}/webhooks/{webhook_id}",
            delete(handlers::unregister_webhook),
        )
        .route(
            "/{session_id}/webhooks/{webhook_id}/enable",
            post(handlers::reenable_webhook),
        )
}
