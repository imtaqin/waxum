//! Routes for groups.

use super::{handlers, management};
use crate::state::AppState;
use axum::routing::{get, post, put};
use axum::Router;

pub fn session_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/{session_id}/groups",
            get(handlers::list_groups).post(management::create_group),
        )
        .route("/{session_id}/groups/{group_jid}", get(handlers::get_group))
        .route(
            "/{session_id}/groups/{group_jid}/info",
            get(handlers::get_group_info),
        )
        .route(
            "/{session_id}/groups/{group_jid}/subject",
            put(management::set_group_subject),
        )
        .route(
            "/{session_id}/groups/{group_jid}/description",
            put(management::set_group_description),
        )
        .route(
            "/{session_id}/groups/{group_jid}/leave",
            post(management::leave_group),
        )
        .route(
            "/{session_id}/groups/{group_jid}/participants",
            post(management::add_participants).delete(management::remove_participants),
        )
        .route(
            "/{session_id}/groups/{group_jid}/admins",
            post(management::promote_participants).delete(management::demote_participants),
        )
        .route(
            "/{session_id}/groups/{group_jid}/invite-link",
            get(management::get_invite_link),
        )
        .route(
            "/{session_id}/groups/{group_jid}/settings",
            put(management::set_group_settings),
        )
}
