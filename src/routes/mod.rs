use axum::{
    routing::{delete, get, post, put},
    Router,
};

use crate::handlers;
use crate::middleware::jwt::dashboard_auth_middleware;
use crate::state::AppState;

pub fn create_router() -> Router<AppState> {
    Router::new()
        .nest("/api/v1", api_routes())
        .route("/health", get(health_check))
}

pub fn create_dashboard_router() -> Router<AppState> {
    // Protected routes (require auth)
    let protected = Router::new()
        .route("/", get(handlers::dashboard::dashboard_home))
        .route("/sessions", get(handlers::dashboard::sessions_list))
        .route("/sessions/new", get(handlers::dashboard::session_new_form))
        .route("/sessions/new", post(handlers::dashboard::session_create))
        .route(
            "/sessions/{session_id}",
            get(handlers::dashboard::session_detail),
        )
        .route(
            "/sessions/{session_id}/connect",
            post(handlers::dashboard::session_connect),
        )
        .route(
            "/sessions/{session_id}/disconnect",
            post(handlers::dashboard::session_disconnect),
        )
        .route(
            "/sessions/{session_id}/delete",
            post(handlers::dashboard::session_delete),
        )
        .route(
            "/sessions/{session_id}/pair",
            post(handlers::dashboard::session_pair),
        )
        .route("/settings", get(handlers::dashboard::settings_page))
        .route("/logout", post(handlers::dashboard::logout))
        .layer(axum::middleware::from_fn(dashboard_auth_middleware));

    // Public routes (login page)
    Router::new()
        .route("/login", get(handlers::dashboard::login_page))
        .route("/login", post(handlers::dashboard::login_submit))
        .merge(protected)
}

fn api_routes() -> Router<AppState> {
    Router::new().nest("/sessions", session_routes())
}

fn session_routes() -> Router<AppState> {
    Router::new()
        .route("/", post(handlers::sessions::create_session))
        .route("/", get(handlers::sessions::list_sessions))
        .route("/{session_id}", get(handlers::sessions::get_session))
        .route("/{session_id}", delete(handlers::sessions::delete_session))
        .route(
            "/{session_id}/status",
            get(handlers::sessions::get_session_status),
        )
        .route("/{session_id}/qr", get(handlers::sessions::get_qr_code))
        .route(
            "/{session_id}/connect",
            post(handlers::sessions::connect_session),
        )
        .route("/{session_id}/pair", post(handlers::sessions::pair_session))
        .route(
            "/{session_id}/disconnect",
            post(handlers::sessions::disconnect_session),
        )
        .route(
            "/{session_id}/device",
            get(handlers::sessions::get_device_info),
        )
        .route(
            "/{session_id}/messages/text",
            post(handlers::messages::send_text),
        )
        .route(
            "/{session_id}/messages/image",
            post(handlers::messages::send_image),
        )
        .route(
            "/{session_id}/messages/video",
            post(handlers::messages::send_video),
        )
        .route(
            "/{session_id}/messages/audio",
            post(handlers::messages::send_audio),
        )
        .route(
            "/{session_id}/messages/document",
            post(handlers::messages::send_document),
        )
        .route(
            "/{session_id}/messages/sticker",
            post(handlers::messages::send_sticker),
        )
        .route(
            "/{session_id}/messages/location",
            post(handlers::messages::send_location),
        )
        .route(
            "/{session_id}/messages/contact",
            post(handlers::messages::send_contact),
        )
        .route(
            "/{session_id}/messages/edit",
            post(handlers::messages::edit_message),
        )
        .route(
            "/{session_id}/messages/react",
            post(handlers::messages::send_reaction),
        )
        .route(
            "/{session_id}/messages/revoke",
            post(handlers::messages::revoke_message),
        )
        .route(
            "/{session_id}/messages/read",
            post(handlers::messages::mark_as_read),
        )
        .route(
            "/{session_id}/contacts/check",
            post(handlers::contacts::check_on_whatsapp),
        )
        .route(
            "/{session_id}/contacts/info",
            post(handlers::contacts::get_contact_info),
        )
        .route(
            "/{session_id}/contacts/{jid}/picture",
            get(handlers::contacts::get_profile_picture),
        )
        .route(
            "/{session_id}/contacts/users",
            post(handlers::contacts::get_user_info),
        )
        .route(
            "/{session_id}/groups",
            get(handlers::groups::list_groups).post(handlers::groups_management::create_group),
        )
        .route(
            "/{session_id}/groups/{group_jid}",
            get(handlers::groups::get_group),
        )
        .route(
            "/{session_id}/groups/{group_jid}/info",
            get(handlers::groups::get_group_info),
        )
        .route(
            "/{session_id}/groups/{group_jid}/subject",
            put(handlers::groups_management::set_group_subject),
        )
        .route(
            "/{session_id}/groups/{group_jid}/description",
            put(handlers::groups_management::set_group_description),
        )
        .route(
            "/{session_id}/groups/{group_jid}/leave",
            post(handlers::groups_management::leave_group),
        )
        .route(
            "/{session_id}/groups/{group_jid}/participants",
            post(handlers::groups_management::add_participants)
                .delete(handlers::groups_management::remove_participants),
        )
        .route(
            "/{session_id}/groups/{group_jid}/admins",
            post(handlers::groups_management::promote_participants)
                .delete(handlers::groups_management::demote_participants),
        )
        .route(
            "/{session_id}/groups/{group_jid}/invite-link",
            get(handlers::groups_management::get_invite_link),
        )
        .route(
            "/{session_id}/presence/set",
            post(handlers::presence::set_presence),
        )
        .route(
            "/{session_id}/presence/subscribe",
            post(handlers::presence::subscribe_presence),
        )
        .route(
            "/{session_id}/chatstate/send",
            post(handlers::chatstate::send_chatstate),
        )
        .route(
            "/{session_id}/chatstate/typing",
            post(handlers::chatstate::send_typing),
        )
        .route(
            "/{session_id}/blocking/list",
            get(handlers::blocking::get_blocklist),
        )
        .route(
            "/{session_id}/blocking/block",
            post(handlers::blocking::block_contact),
        )
        .route(
            "/{session_id}/blocking/unblock",
            post(handlers::blocking::unblock_contact),
        )
        .route(
            "/{session_id}/blocking/check/{jid}",
            get(handlers::blocking::is_blocked),
        )
        .route(
            "/{session_id}/media/upload",
            post(handlers::media::upload_media),
        )
        .route(
            "/{session_id}/media/download",
            post(handlers::media::download_media),
        )
        .route(
            "/{session_id}/webhooks",
            get(handlers::webhooks::list_webhooks),
        )
        .route(
            "/{session_id}/webhooks",
            post(handlers::webhooks::register_webhook),
        )
        .route(
            "/{session_id}/webhooks/{webhook_id}",
            delete(handlers::webhooks::unregister_webhook),
        )
}

async fn health_check() -> &'static str {
    "OK"
}
