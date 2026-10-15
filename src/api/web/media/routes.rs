//! Routes for media.

use super::handlers;
use crate::api::MAX_MEDIA_UPLOAD_BYTES;
use crate::state::AppState;
use axum::extract::DefaultBodyLimit;
use axum::routing::post;
use axum::Router;

pub fn session_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/{session_id}/media/upload",
            post(handlers::upload_media).layer(DefaultBodyLimit::max(MAX_MEDIA_UPLOAD_BYTES)),
        )
        .route(
            "/{session_id}/media/download",
            post(handlers::download_media),
        )
}
