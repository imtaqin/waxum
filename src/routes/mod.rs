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
    extract::DefaultBodyLimit,
    routing::{delete, get, patch, post, put},
    Router,
};

use crate::api::MAX_MEDIA_UPLOAD_BYTES;
use crate::handlers;
use crate::state::AppState;

/// Build the fully-wired axum router used by [`crate::main`].
pub fn create_router() -> Router<AppState> {
    Router::new()
        .nest(
            "/api/v1",
            api_routes().layer(axum::middleware::from_fn(
                crate::api::shared::new_chats::handlers::rollback_failed_sends,
            )),
        )
        .route("/health", get(health_check))
        .route("/livez", get(livez))
        .route("/readyz", get(readyz))
        .route("/metrics", get(crate::metrics::metrics_handler))
        .route(
            "/api/v1/info",
            get(crate::api::shared::info::handlers::get_info),
        )
}

fn api_routes() -> Router<AppState> {
    Router::new()
        .route("/voices", get(handlers::calls::list_voices))
        .route("/tts/preview", get(handlers::calls::tts_preview))
        .nest("/sessions", session_routes())
        .nest("/nats", nats_routes())
        .merge(crate::api::shared::events::routes::api_routes())
        .merge(crate::api::shared::tokens::routes::api_routes())
        .merge(crate::api::shared::blast::routes::api_routes())
        .merge(crate::api::shared::schedule::routes::api_routes())
        .merge(crate::api::shared::search::routes::api_routes())
        .merge(crate::api::shared::bulk::routes::api_routes())
        .merge(crate::api::shared::tags::routes::api_routes())
}

fn nats_routes() -> Router<AppState> {
    Router::new().merge(crate::api::shared::nats::routes::nats_routes())
}

fn session_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/{session_id}/cloud/connect",
            post(handlers::cloud::connect_cloud),
        )
        .route(
            "/{session_id}/cloud/embedded-signup/exchange",
            post(handlers::cloud::embedded_signup_exchange),
        )
        .route(
            "/{session_id}/cloud/webhook",
            get(handlers::cloud::cloud_webhook_verify).post(handlers::cloud::cloud_webhook_receive),
        )
        .route(
            "/{session_id}/cloud/commerce-settings",
            get(handlers::cloud_commerce::get_commerce_settings)
                .post(handlers::cloud_commerce::update_commerce_settings),
        )
        .route(
            "/{session_id}/messages/product",
            post(handlers::cloud_commerce::send_product),
        )
        .route(
            "/{session_id}/messages/product-list",
            post(handlers::cloud_commerce::send_product_list),
        )
        .route(
            "/{session_id}/messages/catalog",
            post(handlers::cloud_commerce::send_catalog),
        )
        .route(
            "/{session_id}/cloud/waba",
            get(handlers::cloud_admin::get_waba),
        )
        .route(
            "/{session_id}/cloud/owned-wabas",
            get(handlers::cloud_admin::owned_wabas),
        )
        .route(
            "/{session_id}/cloud/client-wabas",
            get(handlers::cloud_admin::client_wabas),
        )
        .route(
            "/{session_id}/cloud/business-portfolio",
            get(handlers::cloud_admin::business_portfolio),
        )
        .route(
            "/{session_id}/cloud/phone-numbers",
            get(handlers::cloud_admin::list_phone_numbers),
        )
        .route(
            "/{session_id}/cloud/phone-number",
            get(handlers::cloud_admin::get_phone_number),
        )
        .route(
            "/{session_id}/cloud/register",
            post(handlers::cloud_admin::register_phone),
        )
        .route(
            "/{session_id}/cloud/deregister",
            post(handlers::cloud_admin::deregister_phone),
        )
        .route(
            "/{session_id}/cloud/request-code",
            post(handlers::cloud_admin::request_code),
        )
        .route(
            "/{session_id}/cloud/verify-code",
            post(handlers::cloud_admin::verify_code),
        )
        .route(
            "/{session_id}/cloud/two-step-pin",
            post(handlers::cloud_admin::set_two_step_pin),
        )
        .route(
            "/{session_id}/cloud/debug-token",
            get(handlers::cloud_admin::debug_token),
        )
        .route(
            "/{session_id}/cloud/business-profile",
            get(handlers::cloud_admin::get_business_profile)
                .post(handlers::cloud_admin::update_business_profile),
        )
        .route(
            "/{session_id}/cloud/business-profile/photo",
            post(handlers::cloud_admin::upload_business_profile_photo)
                .layer(DefaultBodyLimit::max(MAX_MEDIA_UPLOAD_BYTES)),
        )
        .route(
            "/{session_id}/cloud/typing",
            post(handlers::cloud_admin::send_typing),
        )
        .route(
            "/{session_id}/cloud/subscribed-apps",
            get(handlers::cloud_admin::list_subscribed_apps)
                .post(handlers::cloud_admin::subscribe_app)
                .delete(handlers::cloud_admin::unsubscribe_app),
        )
        .route(
            "/{session_id}/cloud/templates",
            get(handlers::cloud_admin::list_templates)
                .post(handlers::cloud_admin::create_template)
                .delete(handlers::cloud_admin::delete_template),
        )
        .route(
            "/{session_id}/cloud/templates/namespace",
            get(handlers::cloud_admin::template_namespace),
        )
        .route(
            "/{session_id}/cloud/templates/{template_id}",
            get(handlers::cloud_admin::get_template).post(handlers::cloud_admin::edit_template),
        )
        .route(
            "/{session_id}/cloud/qr-codes",
            get(handlers::cloud_admin::list_qr_codes).post(handlers::cloud_admin::create_qr_code),
        )
        .route(
            "/{session_id}/cloud/qr-codes/{code}",
            get(handlers::cloud_admin::get_qr_code)
                .post(handlers::cloud_admin::update_qr_code)
                .delete(handlers::cloud_admin::delete_qr_code),
        )
        .route(
            "/{session_id}/cloud/blocked-users",
            get(handlers::cloud_admin::list_blocked_users)
                .post(handlers::cloud_admin::block_users)
                .delete(handlers::cloud_admin::unblock_users),
        )
        .route(
            "/{session_id}/cloud/business-compliance",
            get(handlers::cloud_admin::get_business_compliance)
                .post(handlers::cloud_admin::set_business_compliance),
        )
        .route(
            "/{session_id}/cloud/analytics",
            get(handlers::cloud_admin::analytics),
        )
        .route(
            "/{session_id}/cloud/conversation-analytics",
            get(handlers::cloud_admin::conversation_analytics),
        )
        .route(
            "/{session_id}/cloud/credit-lines",
            get(handlers::cloud_admin::credit_lines),
        )
        .route(
            "/{session_id}/cloud/credit-lines/{credit_line_id}/allocations",
            get(handlers::cloud_admin::credit_line_allocations),
        )
        .route(
            "/{session_id}/cloud/credit-sharing",
            post(handlers::cloud_admin::share_credit_line),
        )
        .route(
            "/{session_id}/cloud/credit-sharing/{allocation_config_id}",
            get(handlers::cloud_admin::get_credit_sharing)
                .delete(handlers::cloud_admin::revoke_credit_sharing),
        )
        .route(
            "/{session_id}/cloud/assigned-users",
            get(handlers::cloud_admin::list_assigned_users)
                .post(handlers::cloud_admin::assign_user),
        )
        .route(
            "/{session_id}/cloud/system-users",
            get(handlers::cloud_admin::list_system_users),
        )
        .route(
            "/{session_id}/cloud/flows/{flow_id}/metrics",
            get(handlers::cloud_admin::flow_metrics),
        )
        .route(
            "/{session_id}/cloud/flow-endpoint/public-key",
            get(handlers::cloud_admin::flow_public_key),
        )
        .route(
            "/{session_id}/cloud/flows-migrate",
            post(handlers::cloud_admin::migrate_flows),
        )
        .route(
            "/{session_id}/messages/order-details",
            post(handlers::cloud_admin::send_order_details),
        )
        .route(
            "/{session_id}/messages/order-status",
            post(handlers::cloud_admin::send_order_status),
        )
        .route(
            "/{session_id}/cloud/media",
            post(handlers::cloud::cloud_upload_media)
                .layer(DefaultBodyLimit::max(MAX_MEDIA_UPLOAD_BYTES)),
        )
        .route(
            "/{session_id}/cloud/media/{media_id}",
            get(handlers::cloud::cloud_get_media).delete(handlers::cloud::cloud_delete_media),
        )
        .route(
            "/{session_id}/cloud/media/{media_id}/download",
            get(handlers::cloud::cloud_download_media),
        )
        .route(
            "/{session_id}/messages/template",
            post(handlers::cloud::send_template),
        )
        .route(
            "/{session_id}/cloud/flows",
            get(handlers::cloud_flows::list_flows).post(handlers::cloud_flows::create_flow),
        )
        .route(
            "/{session_id}/cloud/flows/{flow_id}",
            get(handlers::cloud_flows::get_flow)
                .post(handlers::cloud_flows::update_flow_metadata)
                .delete(handlers::cloud_flows::delete_flow),
        )
        .route(
            "/{session_id}/cloud/flows/{flow_id}/json",
            put(handlers::cloud_flows::update_flow_json),
        )
        .route(
            "/{session_id}/cloud/flows/{flow_id}/assets",
            get(handlers::cloud_flows::get_flow_assets),
        )
        .route(
            "/{session_id}/cloud/flows/{flow_id}/publish",
            post(handlers::cloud_flows::publish_flow),
        )
        .route(
            "/{session_id}/cloud/flows/{flow_id}/deprecate",
            post(handlers::cloud_flows::deprecate_flow),
        )
        .route(
            "/{session_id}/cloud/flows/{flow_id}/send",
            post(handlers::cloud_flows::send_flow),
        )
        .route(
            "/{session_id}/cloud/flow-endpoint",
            post(handlers::cloud_flows::configure_flow_endpoint),
        )
        .route(
            "/{session_id}/cloud/flow-endpoint/exchange",
            post(handlers::cloud_flows::flow_data_exchange),
        )
        .route(
            "/{session_id}/profile",
            put(handlers::profile::update_profile),
        )
        .route(
            "/{session_id}/profile/picture",
            delete(handlers::profile::remove_profile_picture),
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
            "/{session_id}/contacts/{jid}",
            put(handlers::contacts::save_contact),
        )
        .route(
            "/{session_id}/contacts/{jid}/lid",
            get(handlers::contacts::resolve_lid),
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
        .route("/{session_id}/mex/query", post(handlers::mex::mex_query))
        .route("/{session_id}/mex/mutate", post(handlers::mex::mex_mutate))
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
            "/{session_id}/calls/{call_id}/transcript",
            post(handlers::calls::transcribe_call),
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
            "/{session_id}/calls/video-orientation",
            post(handlers::calls::set_call_video_orientation),
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
        .route(
            "/{session_id}/business/catalog",
            get(handlers::business::get_catalog),
        )
        .route(
            "/{session_id}/business/collections",
            get(handlers::business::get_collections),
        )
        .route(
            "/{session_id}/business/order",
            get(handlers::business::get_order),
        )
        .route(
            "/{session_id}/business/profile",
            patch(handlers::business::update_business_profile),
        )
        .route(
            "/{session_id}/business/cover-photo/{photo_id}",
            delete(handlers::business::remove_cover_photo),
        )
        .route(
            "/{session_id}/newsletters/subscribed",
            get(handlers::newsletter::list_subscribed),
        )
        .route(
            "/{session_id}/newsletters",
            post(handlers::newsletter::create_newsletter),
        )
        .route(
            "/{session_id}/newsletters/{jid}/metadata",
            get(handlers::newsletter::get_metadata),
        )
        .route(
            "/{session_id}/newsletters/{jid}/join",
            post(handlers::newsletter::join_newsletter),
        )
        .route(
            "/{session_id}/newsletters/{jid}/leave",
            post(handlers::newsletter::leave_newsletter),
        )
        .route(
            "/{session_id}/newsletters/{jid}",
            delete(handlers::newsletter::delete_newsletter),
        )
        .route(
            "/{session_id}/newsletters/{jid}/change-owner",
            post(handlers::newsletter::change_owner),
        )
        .route(
            "/{session_id}/newsletters/{jid}/demote",
            post(handlers::newsletter::demote_admin),
        )
        .route(
            "/{session_id}/newsletters/{jid}/admin-info",
            get(handlers::newsletter::get_admin_info),
        )
        .route(
            "/{session_id}/newsletters/{jid}/followers",
            get(handlers::newsletter::get_followers),
        )
        .route(
            "/{session_id}/newsletters/{jid}/mute",
            post(handlers::newsletter::set_mute),
        )
        .route("/{session_id}/labels", post(handlers::labels::create_label))
        .route(
            "/{session_id}/labels/{label_id}",
            delete(handlers::labels::delete_label),
        )
        .route(
            "/{session_id}/labels/{label_id}/chats/{chat_jid}",
            post(handlers::labels::add_chat_label).delete(handlers::labels::remove_chat_label),
        )
        .route(
            "/{session_id}/labels/{label_id}/messages",
            post(handlers::labels::add_message_label),
        )
        .route(
            "/{session_id}/labels/{label_id}/messages/remove",
            post(handlers::labels::remove_message_label),
        )
        .route(
            "/{session_id}/quick-replies",
            put(handlers::labels::set_quick_reply),
        )
        .route(
            "/{session_id}/quick-replies/{id}",
            delete(handlers::labels::delete_quick_reply),
        )
        .route(
            "/{session_id}/settings/link-previews",
            post(handlers::labels::set_link_previews),
        )
        .route("/{session_id}/bots", get(handlers::bots::list_bots))
        .route("/{session_id}/capping", get(handlers::bots::get_capping))
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
            "/{session_id}/pause",
            post(handlers::operations::pause_session),
        )
        .route(
            "/{session_id}/resume",
            post(handlers::operations::resume_session),
        )
        .route(
            "/{session_id}/appstate/resync",
            post(handlers::operations::appstate_resync),
        )
        .route(
            "/{session_id}/media/upload",
            post(handlers::media::upload_media)
                .layer(DefaultBodyLimit::max(MAX_MEDIA_UPLOAD_BYTES)),
        )
        .route(
            "/{session_id}/media/download",
            post(handlers::media::download_media),
        )
        .merge(crate::api::shared::sessions::routes::session_routes())
        .merge(crate::api::shared::messages::routes::session_routes())
        .merge(crate::api::shared::webhooks::routes::session_routes())
        .merge(crate::api::shared::new_chats::routes::session_routes())
        .merge(crate::api::shared::blast::routes::session_routes())
        .merge(crate::api::shared::schedule::routes::session_routes())
        .merge(crate::api::shared::search::routes::session_routes())
        .merge(crate::api::shared::bulk::routes::session_routes())
        .merge(crate::api::shared::tags::routes::session_routes())
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
