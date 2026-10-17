//! Routes for newsletter.

use super::handlers;
use crate::state::AppState;
use axum::routing::{delete, get, post};
use axum::Router;

pub fn session_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/{session_id}/newsletters/{jid}/messages",
            get(handlers::list_messages),
        )
        .route(
            "/{session_id}/newsletters/subscribed",
            get(handlers::list_subscribed),
        )
        .route(
            "/{session_id}/newsletters",
            post(handlers::create_newsletter),
        )
        .route(
            "/{session_id}/newsletters/{jid}/metadata",
            get(handlers::get_metadata),
        )
        .route(
            "/{session_id}/newsletters/{jid}/join",
            post(handlers::join_newsletter),
        )
        .route(
            "/{session_id}/newsletters/{jid}/leave",
            post(handlers::leave_newsletter),
        )
        .route(
            "/{session_id}/newsletters/{jid}",
            delete(handlers::delete_newsletter),
        )
        .route(
            "/{session_id}/newsletters/{jid}/change-owner",
            post(handlers::change_owner),
        )
        .route(
            "/{session_id}/newsletters/{jid}/demote",
            post(handlers::demote_admin),
        )
        .route(
            "/{session_id}/newsletters/{jid}/admin-info",
            get(handlers::get_admin_info),
        )
        .route(
            "/{session_id}/newsletters/{jid}/followers",
            get(handlers::get_followers),
        )
        .route(
            "/{session_id}/newsletters/{jid}/mute",
            post(handlers::set_mute),
        )
}
