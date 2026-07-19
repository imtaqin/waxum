//! Axum router assembly.
//!
//! [`create_router`] wires:
//!
//! - `/health` — DB-free static probe used by the Docker HEALTHCHECK.
//! - `/metrics` — Prometheus text exposition (JWT bypass).
//! - `/api/v1/info` — build info + version.
//! - `/api/v1/sessions/*` — session lifecycle, pair, connect, disconnect,
//!   QR, status, delete, contacts, groups, messages, presence, chatstate,
//!   media, mex, operations, webhooks, and everything else.
//! - `/api/v1/nats/*` — NATS stream ops.

use axum::{
    routing::{delete, get, post, put},
    Router,
};

use crate::handlers;
use crate::state::AppState;

/// Build the fully-wired axum router used by [`crate::main`].
pub fn create_router() -> Router<AppState> {
    Router::new()
        .nest("/api/v1", api_routes())
        .route("/health", get(health_check))
        .route("/livez", get(livez))
        .route("/readyz", get(readyz))
        .route("/metrics", get(crate::metrics::metrics_handler))
        .route("/api/v1/info", get(handlers::info::get_info))
}

fn api_routes() -> Router<AppState> {
    Router::new()
        .route("/stats", get(handlers::bulk::fleet_stats))
        .route(
            "/webhooks/reenable-all",
            post(handlers::bulk::reenable_circuits),
        )
        .route("/events/tail", get(handlers::events::events_tail))
        .route("/voices", get(handlers::calls::list_voices))
        .route("/tts/preview", get(handlers::calls::tts_preview))
        .nest("/sessions", session_routes())
        .nest("/nats", nats_routes())
}

fn nats_routes() -> Router<AppState> {
    Router::new()
        .route("/status", get(handlers::nats_handler::nats_status))
        .route(
            "/streams/{stream_name}/purge",
            post(handlers::nats_handler::nats_purge_stream),
        )
        .route(
            "/streams/{stream_name}/consumers",
            get(handlers::nats_handler::nats_list_consumers),
        )
}

fn session_routes() -> Router<AppState> {
    Router::new()
        .route("/", post(handlers::sessions::create_session))
        .route("/", get(handlers::sessions::list_sessions))
        .route("/purge", post(handlers::bulk::purge_sessions))
        .route("/disconnect-all", post(handlers::bulk::disconnect_all))
        .route("/reconnect-all", post(handlers::bulk::reconnect_all))
        .route("/search", get(handlers::bulk::search_sessions))
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
            "/{session_id}/messages/poll",
            post(handlers::messages::send_poll),
        )
        .route(
            "/{session_id}/messages/buttons",
            post(handlers::messages::send_buttons),
        )
        .route(
            "/{session_id}/messages/list",
            post(handlers::messages::send_list),
        )
        .route(
            "/{session_id}/messages/interactive",
            post(handlers::messages::send_interactive),
        )
        .route(
            "/{session_id}/messages/cta-url",
            post(handlers::messages::send_cta_url),
        )
        .route(
            "/{session_id}/messages/quick-reply",
            post(handlers::messages::send_quick_reply),
        )
        .route(
            "/{session_id}/messages/newsletter-admin-invite",
            post(handlers::messages::send_newsletter_admin_invite),
        )
        .route(
            "/{session_id}/messages/newsletter-follower-invite",
            post(handlers::messages::send_newsletter_follower_invite),
        )
        .route(
            "/{session_id}/messages/order",
            post(handlers::messages::send_order),
        )
        .route(
            "/{session_id}/messages/invoice",
            post(handlers::messages::send_invoice),
        )
        .route(
            "/{session_id}/messages/payment-invite",
            post(handlers::messages::send_payment_invite),
        )
        .route(
            "/{session_id}/messages/pin",
            post(handlers::messages::send_pin_message),
        )
        .route(
            "/{session_id}/messages/forward",
            post(handlers::messages::forward_message),
        )
        .route(
            "/{session_id}/messages/poll-update",
            post(handlers::messages::send_poll_update),
        )
        .route(
            "/{session_id}/messages/buttons-response",
            post(handlers::messages::send_buttons_response),
        )
        .route(
            "/{session_id}/messages/list-response",
            post(handlers::messages::send_list_response),
        )
        .route(
            "/{session_id}/messages/interactive-response",
            post(handlers::messages::send_interactive_response),
        )
        .route(
            "/{session_id}/messages/highly-structured",
            post(handlers::messages::send_highly_structured),
        )
        .route(
            "/{session_id}/messages/template-button-reply",
            post(handlers::messages::send_template_button_reply),
        )
        .route(
            "/{session_id}/messages/comment",
            post(handlers::messages::send_comment),
        )
        .route(
            "/{session_id}/messages/scheduled-call",
            post(handlers::messages::send_scheduled_call),
        )
        .route(
            "/{session_id}/messages/scheduled-call-edit",
            post(handlers::messages::send_scheduled_call_edit),
        )
        .route(
            "/{session_id}/messages/send-payment",
            post(handlers::messages::send_payment),
        )
        .route(
            "/{session_id}/messages/request-payment",
            post(handlers::messages::request_payment),
        )
        .route(
            "/{session_id}/messages/cancel-payment",
            post(handlers::messages::cancel_payment_request),
        )
        .route(
            "/{session_id}/messages/decline-payment",
            post(handlers::messages::decline_payment_request),
        )
        .route(
            "/{session_id}/messages/newsletter-forward",
            post(handlers::messages::send_newsletter_forward),
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
            "/{session_id}/contacts",
            get(handlers::contacts::list_contacts),
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
            "/{session_id}/groups/{group_jid}/settings",
            put(handlers::groups_management::set_group_settings),
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
            "/{session_id}/calls/reject",
            post(handlers::calls::reject_call),
        )
        .route("/{session_id}/calls/ring", post(handlers::calls::ring_call))
        .route("/{session_id}/calls/tts", post(handlers::calls::tts_call))
        .route("/{session_id}/calls/play", post(handlers::calls::play_call))
        .route(
            "/{session_id}/calls/media/ws",
            get(handlers::calls::media_stream_ws),
        )
        .route(
            "/{session_id}/calls/{call_id}/recording.wav",
            get(handlers::calls::get_recording),
        )
        .route(
            "/{session_id}/calls/accept",
            post(handlers::calls::accept_call),
        )
        .route(
            "/{session_id}/calls/terminate",
            post(handlers::calls::terminate_call),
        )
        .route(
            "/{session_id}/status/react",
            post(handlers::status::send_status_reaction),
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
            "/{session_id}/privacy/settings",
            get(handlers::privacy::get_privacy_settings),
        )
        .route("/{session_id}/mex/query", post(handlers::mex::mex_query))
        .route("/{session_id}/mex/mutate", post(handlers::mex::mex_mutate))
        .route(
            "/{session_id}/spam/report",
            post(handlers::operations::spam_report),
        )
        .route(
            "/{session_id}/tctoken/issue",
            post(handlers::operations::tctoken_issue),
        )
        .route(
            "/{session_id}/tctoken/list",
            get(handlers::operations::tctoken_list),
        )
        .route(
            "/{session_id}/tctoken/expired",
            delete(handlers::operations::tctoken_prune),
        )
        .route(
            "/{session_id}/tctoken/{jid}",
            get(handlers::operations::tctoken_get),
        )
        .route(
            "/{session_id}/reconnect",
            get(handlers::operations::get_auto_reconnect)
                .put(handlers::operations::set_auto_reconnect),
        )
        .route(
            "/{session_id}/history-sync",
            get(handlers::operations::get_history_sync).put(handlers::operations::set_history_sync),
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
        .route(
            "/{session_id}/webhooks/{webhook_id}/enable",
            post(handlers::webhooks::reenable_webhook),
        )
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
    let body = json!({
        "db": if db_ok { "ok" } else { "fail" },
        "sessions_known": sessions,
    });
    let status = if db_ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (status, axum::Json(body)).into_response()
}
