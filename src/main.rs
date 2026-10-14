//! # waxum
//!
//! Multi-session REST + WebSocket gateway around the `whatsapp-rust` client
//! library. One process fronts N WhatsApp Web sessions and exposes them
//! through a single HTTP API, a NATS JetStream event bus, and outbound HMAC
//! webhooks.
//!
//! ## Architecture at a glance
//!
//! - [`state`] — process-wide [`AppState`] with the session registry,
//!   webhook table, NATS handle, and per-session runtime state (client
//!   handle, QR queue, pair code, status, pair telemetry, logout history,
//!   circuit-breaker state).
//! - [`routes`] — axum router. Builds `/health`, `/metrics`, the versioned
//!   `/api/v1/*` tree, and the Swagger UI (`/swagger-ui`, `/api-docs`).
//! - [`handlers`] — one module per REST domain (sessions, messages, groups,
//!   contacts, presence, chatstate, media, webhooks, NATS, operations, …).
//! - [`db`] — three-backend DB layer (Postgres, MySQL, and a hand-rolled
//!   SQLite wrapper over `libsqlite3-sys` 0.37 that shares the copy shipped
//!   by `whatsapp-rust-sqlite-storage`).
//! - [`nats`] — JetStream publisher + outbound `wa.send.>` consumer.
//! - [`metrics`] — Prometheus exposition for `/metrics`.
//! - [`middleware::jwt`] — bearer-token gate with a superadmin bypass.
//!
//! ## Runtime tuning
//!
//! Overridable via environment (or `.env`):
//!
//! - `WA_RS_WORKER_THREADS` — tokio worker threads (default: CPU count).
//! - `WA_RS_BLOCKING_THREADS` — tokio blocking pool (default 2048).
//! - `WA_RS_MYSQL_MAX_POOL` — MySQL connections (default 64).
//! - `WA_RS_PORT` / `WA_RS_HOST` — bind address (default `0.0.0.0:3451`).
//! - `SCHEDULER_POLL_MS` — scheduled-send dispatcher poll interval in
//!   milliseconds (default 1000).
//! - `BLAST_POLL_MS` — blast (bulk-send) worker poll interval in
//!   milliseconds (default 1000).
//! - `MESSAGE_HISTORY_ENABLED` — set to `false` to disable best-effort
//!   message-history ingestion (the search index; default true).
//! - `RECONNECT_WATCHDOG_POLL_MS` — reconnect watchdog tick interval in
//!   milliseconds (default 30000).
//! - `RECONNECT_MAX_ATTEMPTS` — consecutive reconnect failures before the
//!   watchdog forces a full client rebuild (default 10).
//! - `RECONNECT_MAX_STUCK_SECS` — seconds a session may sit in
//!   `Connecting` before the watchdog forces a rebuild, regardless of the
//!   attempt count (default 600).
//! - `ACCOUNT_LOCK_BACKOFF_SECS` — comma-separated cooldown schedule
//!   applied when WhatsApp locks an account (`AccountLocked`); each
//!   repeat lock walks one step further (default `300,900,3600`, i.e.
//!   5 → 15 → 60 min). A manual `POST /sessions/:id/connect` lifts the
//!   cooldown.
//!
//! ## Storage
//!
//! - Session storage dirs live at `WHATSAPP_STORAGE_PATH` (default
//!   `./whatsapp_sessions`), one directory per session id containing the
//!   upstream `whatsapp.db` SQLite store.
//! - Registry (`sessions`, `webhooks`, `contacts`, `webhook_dlq`,
//!   `scheduled_messages`, `blast_jobs`, `blast_recipients`) lives in
//!   whichever DB `DATABASE_URL` points at.
use anyhow::Result;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tower_governor::{
    governor::{GovernorConfig, GovernorConfigBuilder},
    key_extractor::PeerIpKeyExtractor,
    GovernorLayer,
};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

const VERSION: &str = env!("CARGO_PKG_VERSION");

const BANNER: &str = r#"
   ▄▄▌ ▐ ▄▌ ▄▄▄· ▐▄• ▄  ▄• ▄▌• ▌ ▄ ·.
   ██· █▌▐█▐█ ▀█  █▌█▌▪█▪██▌·██ ▐███▪
   ██▪▐█▐▐▌▄█▀▀█  ·██· █▌▐█▌▐█ ▌▐▌▐█·
   ▐█▌██▐█▌▐█ ▪▐▌▪▐█·█▌▐█▄█▌██ ██▌▐█▌
    ▀▀▀▀ ▀▪ ▀  ▀ •▀▀ ▀▀ ▀▀▀ ▀▀  █▪▀▀▀
      ┈┈  premium whatsapp gateway  ┈┈
"#;

use waxum::console;
use waxum::db;
use waxum::handlers;
use waxum::middleware;
use waxum::models;
use waxum::nats;
use waxum::preflight;
use waxum::routes;
use waxum::state;
use waxum::storage;

use routes::create_router;
use state::AppState;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "WhatsApp REST API",
        version = "0.1.0",
        description = "Multi-session REST API gateway for WhatsApp Web client",
        license(name = "MIT")
    ),
    servers(
        (url = "http://localhost:3451", description = "Local development server")
    ),
    modifiers(&SecurityAddon),
    paths(

        waxum::api::shared::sessions::handlers::create_session,
        waxum::api::shared::sessions::handlers::list_sessions,
        waxum::api::shared::sessions::handlers::get_session,
        waxum::api::shared::sessions::handlers::delete_session,
        waxum::api::shared::sessions::handlers::get_session_status,
        waxum::api::shared::sessions::handlers::get_qr_code,
        waxum::api::shared::sessions::handlers::connect_session,
        waxum::api::shared::sessions::handlers::connect_and_wait,
        waxum::api::shared::sessions::handlers::pair_session,
        waxum::api::shared::sessions::handlers::disconnect_session,
        waxum::api::shared::sessions::handlers::logout_session,
        waxum::api::shared::sessions::handlers::export_session,
        waxum::api::shared::sessions::handlers::import_session,
        waxum::api::shared::sessions::handlers::get_device_info,

        handlers::cloud::connect_cloud,
        handlers::cloud::embedded_signup_exchange,
        handlers::cloud_commerce::get_commerce_settings,
        handlers::cloud_commerce::update_commerce_settings,
        handlers::cloud_commerce::send_product,
        handlers::cloud_commerce::send_product_list,
        handlers::cloud_commerce::send_catalog,
        handlers::cloud_admin::get_waba,
        handlers::cloud_admin::owned_wabas,
        handlers::cloud_admin::client_wabas,
        handlers::cloud_admin::business_portfolio,
        handlers::cloud_admin::list_phone_numbers,
        handlers::cloud_admin::get_phone_number,
        handlers::cloud_admin::register_phone,
        handlers::cloud_admin::deregister_phone,
        handlers::cloud_admin::request_code,
        handlers::cloud_admin::verify_code,
        handlers::cloud_admin::set_two_step_pin,
        handlers::cloud_admin::debug_token,
        handlers::cloud_admin::get_business_profile,
        handlers::cloud_admin::update_business_profile,
        handlers::cloud_admin::upload_business_profile_photo,
        handlers::cloud_admin::send_typing,
        handlers::cloud_admin::list_subscribed_apps,
        handlers::cloud_admin::subscribe_app,
        handlers::cloud_admin::unsubscribe_app,
        handlers::cloud_admin::list_templates,
        handlers::cloud_admin::create_template,
        handlers::cloud_admin::delete_template,
        handlers::cloud_admin::template_namespace,
        handlers::cloud_admin::get_template,
        handlers::cloud_admin::edit_template,
        handlers::cloud_admin::list_qr_codes,
        handlers::cloud_admin::create_qr_code,
        handlers::cloud_admin::get_qr_code,
        handlers::cloud_admin::update_qr_code,
        handlers::cloud_admin::delete_qr_code,
        handlers::cloud_admin::list_blocked_users,
        handlers::cloud_admin::block_users,
        handlers::cloud_admin::unblock_users,
        handlers::cloud_admin::get_business_compliance,
        handlers::cloud_admin::set_business_compliance,
        handlers::cloud_admin::analytics,
        handlers::cloud_admin::conversation_analytics,
        handlers::cloud_admin::credit_lines,
        handlers::cloud_admin::credit_line_allocations,
        handlers::cloud_admin::share_credit_line,
        handlers::cloud_admin::get_credit_sharing,
        handlers::cloud_admin::revoke_credit_sharing,
        handlers::cloud_admin::list_assigned_users,
        handlers::cloud_admin::assign_user,
        handlers::cloud_admin::list_system_users,
        handlers::cloud_admin::flow_metrics,
        handlers::cloud_admin::flow_public_key,
        handlers::cloud_admin::migrate_flows,
        handlers::cloud_admin::send_order_details,
        handlers::cloud_admin::send_order_status,
        handlers::cloud::send_template,
        handlers::cloud_flows::create_flow,
        handlers::cloud_flows::list_flows,
        handlers::cloud_flows::get_flow,
        handlers::cloud_flows::update_flow_metadata,
        handlers::cloud_flows::delete_flow,
        handlers::cloud_flows::update_flow_json,
        handlers::cloud_flows::get_flow_assets,
        handlers::cloud_flows::publish_flow,
        handlers::cloud_flows::deprecate_flow,
        handlers::cloud_flows::send_flow,
        handlers::cloud_flows::configure_flow_endpoint,

        waxum::api::shared::messages::handlers::send_text,
        waxum::api::shared::messages::handlers::send_image,
        waxum::api::shared::messages::handlers::send_video,
        waxum::api::shared::messages::handlers::send_audio,
        waxum::api::shared::messages::handlers::send_document,
        waxum::api::shared::messages::handlers::send_sticker,
        waxum::api::shared::messages::handlers::send_location,
        waxum::api::shared::messages::handlers::send_contact,
        waxum::api::shared::messages::handlers::edit_message,
        waxum::api::shared::messages::handlers::send_reaction,
        waxum::api::shared::messages::handlers::revoke_message,
        waxum::api::shared::messages::handlers::mark_as_read,
        waxum::api::shared::messages::handlers::send_poll,
        waxum::api::shared::messages::handlers::send_buttons,
        waxum::api::shared::messages::handlers::send_list,
        waxum::api::shared::messages::handlers::send_interactive,
        waxum::api::shared::messages::handlers::send_cta_url,
        waxum::api::shared::messages::handlers::send_quick_reply,
        waxum::api::shared::messages::handlers::send_newsletter_admin_invite,
        waxum::api::shared::messages::handlers::send_newsletter_follower_invite,
        waxum::api::shared::messages::handlers::send_order,
        waxum::api::shared::messages::handlers::send_invoice,
        waxum::api::shared::messages::handlers::send_payment_invite,
        waxum::api::shared::messages::handlers::send_pin_message,
        waxum::api::shared::messages::handlers::forward_message,
        waxum::api::shared::messages::handlers::send_poll_update,
        waxum::api::shared::messages::handlers::send_buttons_response,
        waxum::api::shared::messages::handlers::send_list_response,
        waxum::api::shared::messages::handlers::send_interactive_response,
        waxum::api::shared::messages::handlers::send_highly_structured,
        waxum::api::shared::messages::handlers::send_template_button_reply,
        waxum::api::shared::messages::handlers::send_comment,
        waxum::api::shared::messages::handlers::send_scheduled_call,
        waxum::api::shared::messages::handlers::send_scheduled_call_edit,
        waxum::api::shared::messages::handlers::send_payment,
        waxum::api::shared::messages::handlers::request_payment,
        waxum::api::shared::messages::handlers::cancel_payment_request,
        waxum::api::shared::messages::handlers::decline_payment_request,
        waxum::api::shared::messages::handlers::send_newsletter_forward,

        handlers::contacts::check_on_whatsapp,
        handlers::contacts::get_contact_info,
        handlers::contacts::get_profile_picture,
        handlers::profile::update_profile,
        handlers::profile::remove_profile_picture,
        handlers::contacts::get_user_info,
        handlers::contacts::list_contacts,
        handlers::contacts::resolve_lid,
        handlers::contacts::save_contact,

        handlers::groups::list_groups,
        handlers::groups::get_group,
        handlers::groups::get_group_info,
        handlers::groups_management::create_group,
        handlers::groups_management::set_group_subject,
        handlers::groups_management::set_group_description,
        handlers::groups_management::leave_group,
        handlers::groups_management::add_participants,
        handlers::groups_management::remove_participants,
        handlers::groups_management::promote_participants,
        handlers::groups_management::demote_participants,
        handlers::groups_management::get_invite_link,
        handlers::groups_management::set_group_settings,

        handlers::presence::set_presence,
        handlers::presence::subscribe_presence,

        handlers::chatstate::send_chatstate,
        handlers::chatstate::send_typing,

        handlers::blocking::get_blocklist,
        handlers::blocking::block_contact,
        handlers::blocking::unblock_contact,
        handlers::blocking::is_blocked,

        handlers::calls::reject_call,
        handlers::calls::ring_call,
        handlers::calls::tts_call,
        handlers::calls::play_call,
        handlers::calls::accept_call,
        handlers::calls::terminate_call,
        handlers::calls::set_call_video_orientation,
        handlers::calls::transcribe_call,

        handlers::status::send_status_reaction,

        handlers::media::upload_media,
        handlers::media::download_media,

        handlers::privacy::get_privacy_settings,

        handlers::mex::mex_query,
        handlers::mex::mex_mutate,

        handlers::operations::spam_report,
        handlers::operations::tctoken_issue,
        handlers::operations::tctoken_get,
        handlers::operations::tctoken_prune,
        handlers::operations::tctoken_list,
        handlers::operations::set_auto_reconnect,
        handlers::operations::get_auto_reconnect,
        handlers::operations::set_history_sync,
        handlers::operations::get_history_sync,
        handlers::operations::pause_session,
        handlers::operations::resume_session,
        handlers::operations::appstate_resync,

        waxum::api::shared::webhooks::handlers::list_webhooks,
        waxum::api::shared::webhooks::handlers::register_webhook,
        waxum::api::shared::webhooks::handlers::unregister_webhook,
        waxum::api::shared::webhooks::handlers::reenable_webhook,
        waxum::api::shared::webhooks::handlers::list_webhook_dlq,
        waxum::api::shared::webhooks::handlers::replay_webhook_dlq_entry,

        waxum::api::shared::nats::handlers::nats_status,
        waxum::api::shared::nats::handlers::nats_purge_stream,
        waxum::api::shared::nats::handlers::nats_list_consumers,

        waxum::api::shared::schedule::handlers::list_session_scheduled,
        waxum::api::shared::schedule::handlers::cancel_scheduled,
        waxum::api::shared::schedule::handlers::list_all_scheduled,

        waxum::api::shared::blast::handlers::create_blast,
        waxum::api::shared::blast::handlers::list_session_blasts,
        waxum::api::shared::blast::handlers::get_blast,
        waxum::api::shared::blast::handlers::list_blast_recipients,
        waxum::api::shared::blast::handlers::cancel_blast,
        waxum::api::shared::search::handlers::search_session_messages,
        waxum::api::shared::search::handlers::search_all_messages,
        waxum::api::shared::search::handlers::list_chat_messages,
        waxum::api::shared::search::handlers::list_session_messages,

        waxum::api::shared::tokens::handlers::mint_token,
        waxum::api::shared::tokens::handlers::list_tokens,
        waxum::api::shared::tokens::handlers::revoke_token,

        handlers::business::get_catalog,
        handlers::business::get_collections,
        handlers::business::get_order,
        handlers::business::update_business_profile,
        handlers::business::remove_cover_photo,

        handlers::newsletter::list_subscribed,
        handlers::newsletter::get_metadata,
        handlers::newsletter::create_newsletter,
        handlers::newsletter::join_newsletter,
        handlers::newsletter::leave_newsletter,
        handlers::newsletter::delete_newsletter,
        handlers::newsletter::change_owner,
        handlers::newsletter::demote_admin,
        handlers::newsletter::get_admin_info,
        handlers::newsletter::get_followers,
        handlers::newsletter::set_mute,

        handlers::labels::create_label,
        handlers::labels::delete_label,
        handlers::labels::add_chat_label,
        handlers::labels::remove_chat_label,
        handlers::labels::add_message_label,
        handlers::labels::remove_message_label,
        handlers::labels::set_quick_reply,
        handlers::labels::delete_quick_reply,
        handlers::labels::set_link_previews,
        waxum::api::shared::new_chats::handlers::get_new_chat_limit,
        waxum::api::shared::new_chats::handlers::set_new_chat_limit,
        waxum::api::shared::new_chats::handlers::clear_new_chat_limit,

        handlers::bots::list_bots,
        handlers::bots::get_capping,
    ),
    components(
        schemas(
            waxum::api::shared::sessions::models::SessionInfo,
            waxum::api::shared::sessions::models::SessionStatus,
            waxum::api::shared::sessions::models::CreateSessionRequest,
            waxum::api::shared::sessions::models::CreateSessionResponse,
            waxum::api::shared::sessions::models::SessionListResponse,
            waxum::api::shared::sessions::models::PairCodeRequest,
            waxum::api::shared::sessions::models::PairCodeResponse,
            waxum::api::shared::sessions::models::ConnectRequest,
            waxum::api::shared::sessions::models::DevicePropsRequest,
            waxum::api::shared::sessions::models::QrCodeResponse,
            waxum::api::shared::sessions::models::SessionStatusResponse,
            waxum::api::shared::sessions::models::SessionDiagnostics,
            waxum::api::shared::sessions::models::WebhookHealth,
            waxum::api::shared::sessions::models::DeviceInfo,

            models::cloud::ConnectCloudRequest,
            models::cloud::ConnectCloudResponse,
            models::cloud::EmbeddedSignupExchangeRequest,
            models::cloud_commerce::UpdateCommerceSettingsRequest,
            models::cloud_commerce::SendProductRequest,
            models::cloud_commerce::ProductSection,
            models::cloud_commerce::SendProductListRequest,
            models::cloud_commerce::SendCatalogRequest,
            models::cloud_admin::RegisterPhoneRequest,
            models::cloud_admin::OnPremBackup,
            models::cloud_admin::RequestCodeRequest,
            models::cloud_admin::VerifyCodeRequest,
            models::cloud_admin::TwoStepPinRequest,
            models::cloud_admin::UpdateBusinessProfileRequest,
            models::cloud_admin::TypingIndicatorRequest,
            models::cloud_admin::SubscribeAppRequest,
            models::cloud_admin::CreateTemplateRequest,
            models::cloud_admin::EditTemplateRequest,
            models::cloud_admin::CreateQrCodeRequest,
            models::cloud_admin::UpdateQrCodeRequest,
            models::cloud_admin::BlockUsersRequest,
            models::cloud_admin::BusinessComplianceRequest,
            models::cloud_admin::MigrateFlowsRequest,
            models::cloud_admin::SendOrderDetailsRequest,
            models::cloud_admin::SendOrderStatusRequest,
            models::cloud_admin::AssignUserRequest,
            models::cloud_admin::ShareCreditLineRequest,
            models::cloud::EmbeddedSignupExchangeResponse,
            models::cloud::SendTemplateRequest,
            models::cloud::CreateFlowRequest,
            models::cloud::UpdateFlowMetadataRequest,
            models::cloud::ConfigureFlowEndpointRequest,
            models::cloud::ConfigureFlowEndpointResponse,
            models::cloud::SendFlowRequest,

            waxum::api::shared::messages::models::SendTextRequest,
            waxum::api::shared::messages::models::SendImageRequest,
            waxum::api::shared::messages::models::SendVideoRequest,
            waxum::api::shared::messages::models::SendAudioRequest,
            waxum::api::shared::messages::models::SendDocumentRequest,
            waxum::api::shared::messages::models::SendStickerRequest,
            waxum::api::shared::messages::models::SendLocationRequest,
            waxum::api::shared::messages::models::SendContactRequest,
            waxum::api::shared::messages::models::ContactCard,
            waxum::api::shared::messages::models::ContactPhone,
            waxum::api::shared::messages::models::EditMessageRequest,
            waxum::api::shared::messages::models::SendReactionRequest,
            waxum::api::shared::messages::models::MediaData,
            waxum::api::shared::messages::models::MessageResponse,
            waxum::api::shared::messages::models::RevokeMessageRequest,
            waxum::api::shared::messages::models::MarkAsReadRequest,
            waxum::api::shared::messages::models::SendPollRequest,
            waxum::api::shared::messages::models::SendButtonsRequest,
            waxum::api::shared::messages::models::ButtonItem,
            waxum::api::shared::messages::models::SendListRequest,
            waxum::api::shared::messages::models::ListSection,
            waxum::api::shared::messages::models::ListRow,
            waxum::api::shared::messages::models::SendInteractiveRequest,
            waxum::api::shared::messages::models::NativeFlowButtonItem,
            waxum::api::shared::messages::models::SendCtaUrlRequest,
            waxum::api::shared::messages::models::SendQuickReplyRequest,
            waxum::api::shared::messages::models::QuickReplyButtonItem,
            waxum::api::shared::messages::models::SendNewsletterAdminInviteRequest,
            waxum::api::shared::messages::models::SendNewsletterFollowerInviteRequest,
            waxum::api::shared::messages::models::SendOrderRequest,
            waxum::api::shared::messages::models::SendInvoiceRequest,
            waxum::api::shared::messages::models::SendPaymentInviteRequest,
            waxum::api::shared::messages::models::SendPinMessageRequest,
            waxum::api::shared::messages::models::ForwardMessageRequest,
            waxum::api::shared::messages::models::SendPollUpdateRequest,
            waxum::api::shared::messages::models::SendButtonsResponseRequest,
            waxum::api::shared::messages::models::SendListResponseRequest,
            waxum::api::shared::messages::models::SendInteractiveResponseRequest,
            waxum::api::shared::messages::models::SendHighlyStructuredRequest,
            waxum::api::shared::messages::models::SendTemplateButtonReplyRequest,
            waxum::api::shared::messages::models::SendCommentRequest,
            waxum::api::shared::messages::models::SendScheduledCallRequest,
            waxum::api::shared::messages::models::SendScheduledCallEditRequest,
            waxum::api::shared::messages::models::SendPaymentRequest,
            waxum::api::shared::messages::models::RequestPaymentRequest,
            waxum::api::shared::messages::models::CancelPaymentRequestRequest,
            waxum::api::shared::messages::models::DeclinePaymentRequestRequest,
            waxum::api::shared::messages::models::SendNewsletterForwardRequest,
            waxum::api::shared::messages::models::SpamReportRequest,
            waxum::api::shared::messages::models::SpamReportResponse,

            models::contacts::CheckOnWhatsAppRequest,
            models::profile::UpdateProfileRequest,
            models::contacts::CheckOnWhatsAppResponse,
            models::contacts::WhatsAppCheckResult,
            models::contacts::GetContactInfoRequest,
            models::contacts::ContactInfoResponse,
            models::contacts::ContactInfo,
            models::contacts::ProfilePictureResponse,
            models::contacts::GetUserInfoRequest,
            models::contacts::UserInfoResponse,
            models::contacts::UserInfo,
            models::contacts::StoredContact,
            models::contacts::StoredContactListResponse,
            models::contacts::LidPnEntryResponse,
            models::contacts::SaveContactRequest,

            models::groups::GroupListResponse,
            models::groups::GroupInfo,
            models::groups::GroupInfoCached,
            models::groups::GroupParticipant,
            models::groups::ParticipantRole,
            models::groups::CreateGroupRequest,
            models::groups::CreateGroupResponse,
            models::groups::ParticipantsRequest,
            models::groups::ParticipantsResponse,
            models::groups::ParticipantChangeResult,
            models::groups::SetSubjectRequest,
            models::groups::SetDescriptionRequest,
            models::groups::InviteLinkResponse,
            models::groups::SetGroupSettingsRequest,
            models::groups::MembershipApprovalMode,
            models::groups::MemberAddMode,
            models::groups::MemberLinkMode,

            models::presence::SetPresenceRequest,
            models::presence::PresenceStatus,

            models::chatstate::SendChatStateRequest,
            models::chatstate::ChatStateType,
            handlers::chatstate::TypingRequest,

            models::blocking::BlocklistResponse,
            models::blocking::BlockRequest,
            handlers::blocking::BlockStatusResponse,

            models::calls::RejectCallRequest,
            models::calls::RingCallRequest,
            models::calls::RingCallResponse,
            models::calls::AcceptCallRequest,
            models::calls::TerminateCallRequest,
            models::calls::TranscriptResponse,
            models::calls::SetVideoOrientationRequest,

            models::status::StatusReactionRequest,

            models::media::UploadMediaResponse,
            models::media::MediaType,

            models::privacy::PrivacySettingsResponse,
            models::privacy::PrivacySettingItem,

            models::mex::MexQueryRequest,
            models::mex::MexMutateRequest,
            waxum::api::shared::search::models::MessageHit,
            waxum::api::shared::search::models::MessageMedia,
            waxum::api::shared::search::models::MessageSearchResponse,

            models::common::SuccessResponse,

            nats::models::NatsStatusResponse,
            nats::models::NatsStreamInfo,
            nats::models::SendResult,

            models::business::CatalogQuery,
            models::business::CollectionsQuery,
            models::business::OrderQuery,
            models::business::BusinessProfileUpdateRequest,
            models::business::SetCoverPhotoRequest,
            models::business::BusinessCatalogResponse,
            models::business::BusinessCollectionsResponse,
            models::business::BusinessOrderResponse,
            models::business::BusinessProfileResponse,

            models::newsletter::CreateNewsletterRequest,
            models::newsletter::UpdateNewsletterRequest,
            models::newsletter::ChangeOwnerRequest,
            models::newsletter::MuteRequest,
            models::newsletter::InviteMetadataRequest,
            models::newsletter::NewsletterMetadataResponse,
            models::newsletter::NewsletterListResponse,
            models::newsletter::NewsletterAdminInfoResponse,
            models::newsletter::NewsletterFollowersResponse,

            models::labels::CreateLabelRequest,
            models::labels::ChatLabelRequest,
            models::labels::MessageLabelRequest,
            models::labels::QuickReplyRequest,
            models::labels::LinkPreviewsRequest,

            models::bots::BotListResponse,
            models::bots::CappingResponse,
            waxum::api::shared::tokens::models::MintTokenResponse,
            waxum::api::shared::tokens::models::TokenSummary,
            waxum::api::shared::tokens::models::TokenListResponse,

            waxum::api::shared::schedule::models::ScheduledMessage,
            waxum::api::shared::schedule::models::ScheduledStatus,
            waxum::api::shared::schedule::models::ScheduledListResponse,
            waxum::api::shared::schedule::models::SendResponse,

            waxum::api::shared::blast::models::CreateBlastRequest,
            waxum::api::shared::blast::models::CreateBlastResponse,
            waxum::api::shared::blast::models::BlastJob,
            waxum::api::shared::blast::models::BlastJobStatus,
            waxum::api::shared::blast::models::BlastJobListResponse,
            waxum::api::shared::blast::models::BlastOptions,
            waxum::api::shared::blast::models::BlastRecipient,
            waxum::api::shared::blast::models::BlastRecipientStatus,
            waxum::api::shared::blast::models::BlastRecipientListResponse,
        )
    ),
    tags(
        (name = "sessions", description = "Session management and authentication"),
        (name = "messages", description = "Send and manage messages"),
        (name = "contacts", description = "Contact information and lookup"),
        (name = "profile", description = "Edit the linked account display name, About text and picture"),
        (name = "groups", description = "Group management"),
        (name = "presence", description = "Online status"),
        (name = "chatstate", description = "Typing indicators"),
        (name = "blocking", description = "Block and unblock contacts"),
        (name = "calls", description = "Incoming call handling (reject)"),
        (name = "status", description = "Status/story reactions"),
        (name = "media", description = "Media upload"),
        (name = "webhooks", description = "Webhook registration for events"),
        (name = "mex", description = "GraphQL/MEX operations"),
        (name = "business", description = "Business catalog, collections and profile"),
        (name = "newsletter", description = "Newsletter/Channel admin & follower ops"),
        (name = "labels", description = "Labels, quick replies and link-preview settings"),
        (name = "bots", description = "Bot directory and capping"),
        (name = "operations", description = "Spam reporting, TCToken, reconnection, and sync operations"),
        (name = "scheduler", description = "Scheduled (deferred) message sending"),
        (name = "blast", description = "Bulk-send (blast) jobs with pacing, retry and DLQ"),
        (name = "nats", description = "NATS JetStream management and status"),
        (name = "tokens", description = "Mint, list, and revoke session-scoped bearer tokens")
    )
)]
struct ApiDoc;

struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(components) = openapi.components.as_mut() {
            components.add_security_scheme(
                "bearer_auth",
                utoipa::openapi::security::SecurityScheme::Http(
                    utoipa::openapi::security::HttpBuilder::new()
                        .scheme(utoipa::openapi::security::HttpAuthScheme::Bearer)
                        .bearer_format("JWT")
                        .description(Some("Enter your Superadmin Token from server logs"))
                        .build(),
                ),
            );
        }
    }
}

/// Builds the global per-peer-IP rate limiter (`RATE_LIMIT_PER_SECOND` /
/// `RATE_LIMIT_BURST` env vars, default 60/150) and spawns the background
/// task that periodically evicts stale entries from its state map --
/// without that, the map only grows and becomes an unbounded memory leak
/// in front of a public server. Only called when `RATE_LIMIT_ENABLED=true`
/// (default off, see [`async_main`]): keyed on the TCP peer address
/// ([`PeerIpKeyExtractor`]), so behind an unconfigured reverse proxy
/// (Docker Compose, Traefik, Dokploy) every client shares one quota --
/// #83/#84 hit exactly that, so this stays opt-in rather than a default
/// every self-hosted deployment has to fight.
///
/// Keyed on the TCP peer address rather than `X-Forwarded-For`/
/// `X-Real-IP`: those headers are client-controlled unless a trusted
/// proxy strips and re-sets them, and trusting them blindly would let a
/// client bypass the limit entirely by sending a fresh fake IP on every
/// request. Applied as the outermost layer in [`async_main`] so every
/// request is rate-limited before it costs a JWT validation or reaches a
/// handler.
fn build_rate_limiter(
) -> Arc<GovernorConfig<PeerIpKeyExtractor, governor::middleware::NoOpMiddleware>> {
    let per_second: u64 = std::env::var("RATE_LIMIT_PER_SECOND")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60);
    let burst: u32 = std::env::var("RATE_LIMIT_BURST")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(150);
    let conf = Arc::new(
        GovernorConfigBuilder::default()
            .key_extractor(PeerIpKeyExtractor)
            .per_second(per_second)
            .burst_size(burst)
            .finish()
            .expect("valid rate-limit configuration"),
    );

    let limiter = conf.limiter().clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            limiter.retain_recent();
        }
    });

    conf
}

/// Builds the CORS layer from `CORS_ALLOWED_ORIGINS` (comma-separated exact
/// origins, e.g. `https://app.example.com,https://admin.example.com`).
/// Unset keeps the historical permissive default (`Any`) so `cargo run`
/// stays zero-config -- but a bearer token is readable by any origin's JS
/// via `fetch()` under that default, so production deployments should set
/// this.
fn build_cors_layer() -> CorsLayer {
    match std::env::var("CORS_ALLOWED_ORIGINS") {
        Ok(raw) if !raw.trim().is_empty() => {
            let origins: Vec<axum::http::HeaderValue> = raw
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .filter_map(|s| match s.parse() {
                    Ok(v) => Some(v),
                    Err(_) => {
                        tracing::warn!("CORS_ALLOWED_ORIGINS: ignoring invalid origin '{}'", s);
                        None
                    }
                })
                .collect();

            if origins.is_empty() {
                tracing::warn!(
                    "CORS_ALLOWED_ORIGINS was set but contained no valid origins; \
                     falling back to permissive CORS (any origin allowed)"
                );
                return CorsLayer::new()
                    .allow_origin(Any)
                    .allow_methods(Any)
                    .allow_headers(Any);
            }

            tracing::info!("CORS restricted to {} configured origin(s)", origins.len());
            CorsLayer::new()
                .allow_origin(origins)
                .allow_methods(Any)
                .allow_headers(Any)
        }
        _ => {
            tracing::warn!(
                "CORS_ALLOWED_ORIGINS not set -- allowing requests from any origin. \
                 Set a comma-separated allowlist before deploying to production."
            );
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any)
        }
    }
}

fn parse_cli_args(args: &[String]) {
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--token" | "-t" => {
                if i + 1 < args.len() {
                    std::env::set_var("SUPERADMIN_TOKEN", &args[i + 1]);
                    i += 2;
                } else {
                    eprintln!("Error: --token requires a value");
                    std::process::exit(1);
                }
            }
            "--db" | "-d" => {
                if i + 1 < args.len() {
                    std::env::set_var("DATABASE_URL", &args[i + 1]);
                    i += 2;
                } else {
                    eprintln!("Error: --db requires a value");
                    std::process::exit(1);
                }
            }
            "--port" | "-p" => {
                if i + 1 < args.len() {
                    std::env::set_var("PORT", &args[i + 1]);
                    i += 2;
                } else {
                    eprintln!("Error: --port requires a value");
                    std::process::exit(1);
                }
            }
            "--proxy" => {
                if i + 1 < args.len() {
                    std::env::set_var("WA_PROXY", &args[i + 1]);
                    i += 2;
                } else {
                    eprintln!("Error: --proxy requires a value");
                    std::process::exit(1);
                }
            }
            "--help" | "-h" => {
                println!("waxum - WhatsApp REST API Gateway");
                println!();
                println!("Usage: waxum [OPTIONS]");
                println!();
                println!("Options:");
                println!("  -t, --token <TOKEN>    Set superadmin token");
                println!("  -d, --db <URL>         Set database URL (postgres/mysql/sqlite)");
                println!("  -p, --port <PORT>      Set server port (default: 3451)");
                println!(
                    "      --proxy <URL>      HTTP/HTTPS proxy for outbound WA media/http calls"
                );
                println!(
                    "      --healthcheck      Exit 0 if the local /health answers 200, else 1"
                );
                println!("  -h, --help             Show this help");
                println!();
                println!("Examples:");
                println!("  waxum --token mysecrettoken");
                println!("  waxum --db sqlite://waxum.db --token mytoken");
                println!("  waxum --db mysql://user:pass@localhost/wars --port 8080");
                std::process::exit(0);
            }
            _ => {
                eprintln!("Unknown argument: {}", args[i]);
                eprintln!("Use --help for usage information");
                std::process::exit(1);
            }
        }
    }
}

fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    if std::env::args().skip(1).any(|a| a == "--healthcheck") {
        std::process::exit(if waxum::bootstrap::healthcheck() {
            0
        } else {
            1
        });
    }
    waxum::bootstrap::drop_root_privileges();

    let worker_threads = std::env::var("WA_RS_WORKER_THREADS")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or_else(|| num_cpus_or(4));
    let max_blocking = std::env::var("WA_RS_BLOCKING_THREADS")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(2048);

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(worker_threads)
        .max_blocking_threads(max_blocking)
        .thread_name("waxum")
        .enable_all()
        .build()?;
    runtime.block_on(async_main(worker_threads, max_blocking))
}

fn num_cpus_or(fallback: usize) -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(fallback)
}

async fn async_main(worker_threads: usize, blocking_threads: usize) -> Result<()> {
    dotenvy::dotenv().ok();

    let args: Vec<String> = std::env::args().collect();
    parse_cli_args(&args);

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "waxum=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    println!("\x1b[96m{}\x1b[0m", BANNER);
    println!("\x1b[97m  WhatsApp Gateway REST API\x1b[0m");
    println!("\x1b[37m  Version: \x1b[96m{}\x1b[0m", VERSION);
    println!();
    println!("\x1b[37m  Author:  \x1b[93m@taqin\x1b[0m");
    println!("\x1b[37m  GitHub:  \x1b[96mhttps://github.com/imtaqin/waxum\x1b[0m");
    println!("\x1b[37m  Docs:    \x1b[96mhttps://waxum.imtaqin.id/\x1b[0m");
    println!();

    tracing::info!(
        worker_threads,
        blocking_threads,
        "Starting WhatsApp REST API server..."
    );

    let (database_url, backend) = db::resolve_database_url();
    let masked = db::mask_url(&database_url);
    let backend_name = match backend {
        db::DbBackend::Postgres => "PostgreSQL",
        db::DbBackend::MySQL => "MySQL",
        db::DbBackend::SQLite => "SQLite",
    };
    tracing::info!("Connecting to {} ({})", backend_name, masked);

    let pool = match backend {
        db::DbBackend::Postgres => {
            use deadpool_postgres::{Manager, ManagerConfig, RecyclingMethod};
            use tokio_postgres::NoTls;

            let pg_config: tokio_postgres::Config = database_url.parse()?;
            let mgr_config = ManagerConfig {
                recycling_method: RecyclingMethod::Fast,
            };
            let mgr = Manager::from_config(pg_config, NoTls, mgr_config);
            let pg_pool = deadpool_postgres::Pool::builder(mgr).max_size(16).build()?;
            let _ = pg_pool.get().await?;
            db::session::DbPool::Postgres(pg_pool)
        }
        db::DbBackend::MySQL => {
            let opts = mysql_async::Opts::from_url(&database_url)?;
            let max_pool: usize = std::env::var("WA_RS_MYSQL_MAX_POOL")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(64);
            let constraints = mysql_async::PoolConstraints::new(4, max_pool).unwrap_or_else(|| {
                mysql_async::PoolConstraints::new(2, 10).expect("safe defaults")
            });
            let pool_opts = mysql_async::PoolOpts::default()
                .with_constraints(constraints)
                .with_inactive_connection_ttl(Duration::from_secs(300))
                .with_ttl_check_interval(Duration::from_secs(60));
            let opts = mysql_async::OptsBuilder::from_opts(opts).pool_opts(pool_opts);
            let my_pool = mysql_async::Pool::new(opts);
            let _conn = my_pool.get_conn().await?;
            db::session::DbPool::MySQL(my_pool)
        }
        db::DbBackend::SQLite => {
            let path = database_url
                .strip_prefix("sqlite://")
                .unwrap_or(&database_url);
            let handle = db::sqlite_raw::open(path)?;
            db::session::DbPool::SQLite(handle)
        }
    };
    tracing::info!("Connected to {}", backend_name);

    db::schema::init_schema(&pool).await?;
    tracing::info!("Database schema initialized");

    let nats_manager = match nats::config::NatsConfig::from_env() {
        Some(config) => {
            tracing::info!("Connecting to NATS at {}", config.url);
            match nats::NatsManager::connect(config).await {
                Ok(manager) => {
                    if let Err(e) = manager.init_streams().await {
                        tracing::error!("Failed to initialize NATS streams: {}", e);
                        None
                    } else {
                        tracing::info!("NATS JetStream initialized");
                        Some(manager)
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to connect to NATS: {}. Continuing without NATS.", e);
                    None
                }
            }
        }
        None => {
            tracing::info!("NATS not configured (NATS_URL not set). Running without NATS.");
            None
        }
    };

    let storage_path = std::env::var("WHATSAPP_STORAGE_PATH")
        .unwrap_or_else(|_| "./whatsapp_sessions".to_string());
    let recordings = match storage::S3Config::from_env() {
        Some(config) => {
            tracing::info!(
                "Connecting call recordings to S3 bucket {} at {}",
                config.bucket,
                config.endpoint
            );
            match storage::RecordingStore::connect_s3(config) {
                Ok(store) => {
                    tracing::info!("Call recordings backed by S3");
                    store
                }
                Err(e) => {
                    tracing::error!(
                        "Failed to set up S3 recording storage: {}. Falling back to local disk.",
                        e
                    );
                    storage::RecordingStore::local(&storage_path)
                }
            }
        }
        None => {
            tracing::info!("S3_BUCKET not set — call recordings stored on local disk");
            storage::RecordingStore::local(&storage_path)
        }
    };

    let nats_enabled = nats_manager.is_some();
    let state = AppState::new(pool, nats_manager, recordings).await;

    waxum::api::shared::bulk::handlers::touch_uptime_anchor();
    preflight::check_fd_limit();
    let _instance_lock = match preflight::acquire_instance_lock(state.base_storage_path()) {
        Ok(guard) => guard,
        Err(msg) => {
            tracing::error!("startup aborted: {msg}");
            anyhow::bail!(msg);
        }
    };

    let reconnect_state = state.clone();
    tokio::spawn(async move {
        waxum::api::shared::sessions::handlers::reconnect_all_on_startup(reconnect_state).await;
    });

    let watchdog_state = state.clone();
    tokio::spawn(async move {
        waxum::api::shared::sessions::handlers::run_reconnect_watchdog(watchdog_state).await;
    });

    let scheduler_state = state.clone();
    tokio::spawn(async move {
        waxum::api::shared::schedule::handlers::run_scheduler(scheduler_state).await;
    });

    let blast_state = state.clone();
    tokio::spawn(async move {
        waxum::api::shared::blast::handlers::run_blast_worker(blast_state).await;
    });

    if nats_enabled {
        if let Some(nats) = state.nats() {
            match nats::consumer::start_consumer(nats.jetstream().clone(), state.clone()).await {
                Ok(_handle) => {
                    tracing::info!("NATS outbound message consumer started");
                }
                Err(e) => {
                    tracing::error!("Failed to start NATS consumer: {}", e);
                }
            }
        }
    }

    let cors = build_cors_layer();
    let rate_limit_enabled = std::env::var("RATE_LIMIT_ENABLED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(false);

    let (superadmin_token, from_env) = middleware::jwt::get_superadmin_token();
    println!();
    println!("\x1b[33m  ┌─────────────────────────────────────────────────────────────┐\x1b[0m");
    println!("\x1b[33m  │\x1b[0m  \x1b[1;97mSUPERADMIN TOKEN\x1b[0m                                            \x1b[33m│\x1b[0m");
    println!("\x1b[33m  ├─────────────────────────────────────────────────────────────┤\x1b[0m");
    if from_env {
        println!("\x1b[33m  │\x1b[0m  \x1b[32mLoaded from SUPERADMIN_TOKEN environment variable\x1b[0m          \x1b[33m│\x1b[0m");
    } else {
        println!("\x1b[33m  │\x1b[0m  \x1b[97m{}\x1b[0m", superadmin_token);
        println!(
            "\x1b[33m  ├─────────────────────────────────────────────────────────────┤\x1b[0m"
        );
        println!("\x1b[33m  │\x1b[0m  \x1b[90mTip: Set SUPERADMIN_TOKEN in .env to use a fixed token\x1b[0m     \x1b[33m│\x1b[0m");
    }
    println!("\x1b[33m  └─────────────────────────────────────────────────────────────┘\x1b[0m");
    println!();

    let mut openapi = ApiDoc::openapi();
    if let Ok(public_base_url) = std::env::var("PUBLIC_BASE_URL") {
        openapi.servers = Some(vec![utoipa::openapi::Server::new(
            public_base_url.trim_end_matches('/'),
        )]);
    }
    let swagger_router: axum::Router<AppState> = SwaggerUi::new("/swagger-ui")
        .url("/api-docs/openapi.json", openapi)
        .into();

    let mut app = create_router()
        .merge(swagger_router)
        .merge(console::console_router())
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            middleware::jwt::jwt_auth_middleware,
        ))
        .layer(TraceLayer::new_for_http())
        .layer(cors);

    if rate_limit_enabled {
        app = app.layer(GovernorLayer::new(build_rate_limiter()));
    }

    let app = app.with_state(state);

    let port: u16 = std::env::var("PORT")
        .unwrap_or_else(|_| "3451".to_string())
        .parse()
        .unwrap_or(3451);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    println!("\x1b[32m  Server listening on:\x1b[0m");
    println!(
        "    \x1b[90m→\x1b[0m API:       \x1b[94mhttp://{}/api/v1\x1b[0m",
        addr
    );
    println!(
        "    \x1b[90m→\x1b[0m Swagger:   \x1b[94mhttp://{}/swagger-ui\x1b[0m",
        addr
    );
    if nats_enabled {
        println!("    \x1b[90m→\x1b[0m NATS:      \x1b[92mConnected\x1b[0m");
    } else {
        println!("    \x1b[90m→\x1b[0m NATS:      \x1b[90mDisabled\x1b[0m");
    }
    println!();

    let listener = TcpListener::bind(addr).await?;
    let server = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    );
    tokio::select! {
        served = server => served?,
        signal = waxum::bootstrap::shutdown_signal() => {
            tracing::info!("received {signal}, shutting down");
        }
    }

    Ok(())
}
