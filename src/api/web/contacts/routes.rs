//! Routes for contacts.

use super::handlers;
use crate::state::AppState;
use axum::routing::{get, post, put};
use axum::Router;

pub fn session_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/{session_id}/contacts/check",
            post(handlers::check_on_whatsapp),
        )
        .route(
            "/{session_id}/contacts/info",
            post(handlers::get_contact_info),
        )
        .route(
            "/{session_id}/contacts/{jid}/picture",
            get(handlers::get_profile_picture),
        )
        .route("/{session_id}/contacts/{jid}", put(handlers::save_contact))
        .route(
            "/{session_id}/contacts/{jid}/lid",
            get(handlers::resolve_lid),
        )
        .route(
            "/{session_id}/contacts/users",
            post(handlers::get_user_info),
        )
        .route("/{session_id}/contacts", get(handlers::list_contacts))
}
