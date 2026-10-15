//! Routes for connection.

use super::handlers;
use crate::api::MAX_MEDIA_UPLOAD_BYTES;
use crate::state::AppState;
use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post};
use axum::Router;

pub fn session_routes() -> Router<AppState> {
    Router::new()
        .route("/{session_id}/cloud/connect", post(handlers::connect_cloud))
        .route(
            "/{session_id}/cloud/embedded-signup/exchange",
            post(handlers::embedded_signup_exchange),
        )
        .route(
            "/{session_id}/cloud/webhook",
            get(handlers::cloud_webhook_verify).post(handlers::cloud_webhook_receive),
        )
        .route(
            "/{session_id}/cloud/media",
            post(handlers::cloud_upload_media).layer(DefaultBodyLimit::max(MAX_MEDIA_UPLOAD_BYTES)),
        )
        .route(
            "/{session_id}/cloud/media/{media_id}",
            get(handlers::cloud_get_media).delete(handlers::cloud_delete_media),
        )
        .route(
            "/{session_id}/cloud/media/{media_id}/download",
            get(handlers::cloud_download_media),
        )
        .route(
            "/{session_id}/messages/template",
            post(handlers::send_template),
        )
}
