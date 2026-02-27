use anyhow::Result;
use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod};
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tokio_postgres::NoTls;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

const VERSION: &str = env!("CARGO_PKG_VERSION");

const BANNER: &str = r#"
██╗    ██╗ █████╗       ██████╗ ███████╗
██║    ██║██╔══██╗      ██╔══██╗██╔════╝
██║ █╗ ██║███████║█████╗██████╔╝███████╗
██║███╗██║██╔══██║╚════╝██╔══██╗╚════██║
╚███╔███╔╝██║  ██║      ██║  ██║███████║
 ╚══╝╚══╝ ╚═╝  ╚═╝      ╚═╝  ╚═╝╚══════╝
"#;

mod db;
mod error;
mod handlers;
mod middleware;
mod models;
mod routes;
mod state;

use routes::{create_dashboard_router, create_router};
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

        handlers::sessions::create_session,
        handlers::sessions::list_sessions,
        handlers::sessions::get_session,
        handlers::sessions::delete_session,
        handlers::sessions::get_session_status,
        handlers::sessions::get_qr_code,
        handlers::sessions::connect_session,
        handlers::sessions::pair_session,
        handlers::sessions::disconnect_session,
        handlers::sessions::get_device_info,

        handlers::messages::send_text,
        handlers::messages::send_image,
        handlers::messages::send_video,
        handlers::messages::send_audio,
        handlers::messages::send_document,
        handlers::messages::send_sticker,
        handlers::messages::send_location,
        handlers::messages::send_contact,
        handlers::messages::edit_message,
        handlers::messages::send_reaction,
        handlers::messages::revoke_message,
        handlers::messages::mark_as_read,
        handlers::messages::send_poll,
        handlers::messages::send_buttons,
        handlers::messages::send_list,
        handlers::messages::send_interactive,
        handlers::messages::send_newsletter_admin_invite,
        handlers::messages::send_newsletter_follower_invite,
        handlers::messages::send_order,
        handlers::messages::send_invoice,
        handlers::messages::send_payment_invite,
        handlers::messages::send_pin_message,
        handlers::messages::forward_message,
        handlers::messages::send_poll_update,
        handlers::messages::send_buttons_response,
        handlers::messages::send_list_response,
        handlers::messages::send_interactive_response,
        handlers::messages::send_highly_structured,
        handlers::messages::send_template_button_reply,
        handlers::messages::send_comment,
        handlers::messages::send_scheduled_call,
        handlers::messages::send_scheduled_call_edit,
        handlers::messages::send_payment,
        handlers::messages::request_payment,
        handlers::messages::cancel_payment_request,
        handlers::messages::decline_payment_request,
        handlers::messages::send_newsletter_forward,

        handlers::contacts::check_on_whatsapp,
        handlers::contacts::get_contact_info,
        handlers::contacts::get_profile_picture,
        handlers::contacts::get_user_info,

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

        handlers::webhooks::list_webhooks,
        handlers::webhooks::register_webhook,
        handlers::webhooks::unregister_webhook,
    ),
    components(
        schemas(

            models::sessions::SessionInfo,
            models::sessions::SessionStatus,
            models::sessions::CreateSessionRequest,
            models::sessions::CreateSessionResponse,
            models::sessions::SessionListResponse,
            models::sessions::PairCodeRequest,
            models::sessions::PairCodeResponse,
            models::sessions::QrCodeResponse,
            models::sessions::SessionStatusResponse,
            models::sessions::DeviceInfo,

            models::messages::SendTextRequest,
            models::messages::SendImageRequest,
            models::messages::SendVideoRequest,
            models::messages::SendAudioRequest,
            models::messages::SendDocumentRequest,
            models::messages::SendStickerRequest,
            models::messages::SendLocationRequest,
            models::messages::SendContactRequest,
            models::messages::ContactCard,
            models::messages::ContactPhone,
            models::messages::EditMessageRequest,
            models::messages::SendReactionRequest,
            models::messages::MediaData,
            models::messages::MessageResponse,
            models::messages::RevokeMessageRequest,
            models::messages::MarkAsReadRequest,
            models::messages::SendPollRequest,
            models::messages::SendButtonsRequest,
            models::messages::ButtonItem,
            models::messages::SendListRequest,
            models::messages::ListSection,
            models::messages::ListRow,
            models::messages::SendInteractiveRequest,
            models::messages::NativeFlowButtonItem,
            models::messages::SendNewsletterAdminInviteRequest,
            models::messages::SendNewsletterFollowerInviteRequest,
            models::messages::SendOrderRequest,
            models::messages::SendInvoiceRequest,
            models::messages::SendPaymentInviteRequest,
            models::messages::SendPinMessageRequest,
            models::messages::ForwardMessageRequest,
            models::messages::SendPollUpdateRequest,
            models::messages::SendButtonsResponseRequest,
            models::messages::SendListResponseRequest,
            models::messages::SendInteractiveResponseRequest,
            models::messages::SendHighlyStructuredRequest,
            models::messages::SendTemplateButtonReplyRequest,
            models::messages::SendCommentRequest,
            models::messages::SendScheduledCallRequest,
            models::messages::SendScheduledCallEditRequest,
            models::messages::SendPaymentRequest,
            models::messages::RequestPaymentRequest,
            models::messages::CancelPaymentRequestRequest,
            models::messages::DeclinePaymentRequestRequest,
            models::messages::SendNewsletterForwardRequest,
            models::messages::SpamReportRequest,
            models::messages::SpamReportResponse,

            models::contacts::CheckOnWhatsAppRequest,
            models::contacts::CheckOnWhatsAppResponse,
            models::contacts::WhatsAppCheckResult,
            models::contacts::GetContactInfoRequest,
            models::contacts::ContactInfoResponse,
            models::contacts::ContactInfo,
            models::contacts::ProfilePictureResponse,
            models::contacts::GetUserInfoRequest,
            models::contacts::UserInfoResponse,
            models::contacts::UserInfo,

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

            models::media::UploadMediaResponse,
            models::media::MediaType,

            models::privacy::PrivacySettingsResponse,
            models::privacy::PrivacySettingItem,

            models::mex::MexQueryRequest,
            models::mex::MexMutateRequest,
            models::mex::MexApiResponse,
            models::mex::MexGraphQLErrorItem,

            handlers::operations::TcTokenIssueRequest,
            handlers::operations::TcTokenIssueResponse,
            handlers::operations::TcTokenItem,
            handlers::operations::TcTokenGetResponse,
            handlers::operations::TcTokenPruneResponse,
            handlers::operations::TcTokenListResponse,
            handlers::operations::AutoReconnectRequest,
            handlers::operations::AutoReconnectResponse,
            handlers::operations::HistorySyncRequest,
            handlers::operations::HistorySyncResponse,

            models::webhooks::WebhookConfig,
            models::webhooks::WebhookEvent,
            models::webhooks::RegisterWebhookRequest,
            models::webhooks::WebhookListResponse,
            models::webhooks::WebhookRequest,

            models::common::SuccessResponse,
        )
    ),
    tags(
        (name = "sessions", description = "Session management and authentication"),
        (name = "messages", description = "Send and manage messages"),
        (name = "contacts", description = "Contact information and lookup"),
        (name = "groups", description = "Group management"),
        (name = "presence", description = "Online status"),
        (name = "chatstate", description = "Typing indicators"),
        (name = "blocking", description = "Block and unblock contacts"),
        (name = "media", description = "Media upload"),
        (name = "webhooks", description = "Webhook registration for events"),
        (name = "privacy", description = "Privacy settings management"),
        (name = "mex", description = "GraphQL/MEX operations"),
        (name = "newsletter", description = "Newsletter/Channel messages"),
        (name = "operations", description = "Spam reporting, TCToken, reconnection, and sync operations")
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
                        .description(Some(
                            "Enter your Superadmin Token from server logs or /dashboard/settings",
                        ))
                        .build(),
                ),
            );
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Load .env file
    dotenvy::dotenv().ok();

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "wa_rs=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Print banner
    println!("\x1b[96m{}\x1b[0m", BANNER);
    println!("\x1b[97m  WhatsApp Gateway REST API\x1b[0m");
    println!("\x1b[37m  Version: \x1b[96m{}\x1b[0m", VERSION);
    println!();
    println!("\x1b[37m  Author:  \x1b[93m@taqin\x1b[0m");
    println!("\x1b[37m  GitHub:  \x1b[96mhttps://github.com/fdciabdul/wa-rs\x1b[0m");
    println!("\x1b[37m  Docs:    \x1b[96mhttps://wa-rs.imtaqin.id/\x1b[0m");
    println!();

    tracing::info!("Starting WhatsApp REST API server...");

    let db_host = std::env::var("POSTGRES_HOST").unwrap_or_else(|_| "localhost".to_string());
    let db_port: u16 = std::env::var("POSTGRES_PORT")
        .unwrap_or_else(|_| "5432".to_string())
        .parse()
        .unwrap_or(5432);
    let db_user = std::env::var("POSTGRES_USER").unwrap_or_else(|_| "postgres".to_string());
    let db_password = std::env::var("POSTGRES_PASSWORD").unwrap_or_else(|_| "postgres".to_string());
    let db_name = std::env::var("POSTGRES_DB").unwrap_or_else(|_| "wagateway".to_string());

    tracing::info!(
        "Connecting to PostgreSQL at {}:{}/{}",
        db_host,
        db_port,
        db_name
    );

    let mut pg_config = tokio_postgres::Config::new();
    pg_config.host(&db_host);
    pg_config.port(db_port);
    pg_config.user(&db_user);
    pg_config.password(&db_password);
    pg_config.dbname(&db_name);

    let mgr_config = ManagerConfig {
        recycling_method: RecyclingMethod::Fast,
    };
    let mgr = Manager::from_config(pg_config, NoTls, mgr_config);
    let pool = Pool::builder(mgr).max_size(16).build()?;

    let _ = pool.get().await?;
    tracing::info!("Connected to PostgreSQL");

    db::schema::init_schema(&pool).await?;
    tracing::info!("Database schema initialized");

    let state = AppState::new(pool).await;

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

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

    let swagger_router: axum::Router<AppState> = SwaggerUi::new("/swagger-ui")
        .url("/api-docs/openapi.json", ApiDoc::openapi())
        .into();

    let api_app = create_router()
        .merge(swagger_router)
        .layer(axum::middleware::from_fn(
            middleware::jwt::jwt_auth_middleware,
        ));

    let dashboard_app = create_dashboard_router();

    let app = axum::Router::new()
        .nest("/dashboard", dashboard_app)
        .merge(api_app)
        .route(
            "/",
            axum::routing::get(|| async { axum::response::Redirect::to("/dashboard") }),
        )
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], 3451));
    println!("\x1b[32m  Server listening on:\x1b[0m");
    println!(
        "    \x1b[90m→\x1b[0m API:       \x1b[94mhttp://{}/api/v1\x1b[0m",
        addr
    );
    println!(
        "    \x1b[90m→\x1b[0m Dashboard: \x1b[94mhttp://{}/dashboard\x1b[0m",
        addr
    );
    println!(
        "    \x1b[90m→\x1b[0m Swagger:   \x1b[94mhttp://{}/swagger-ui\x1b[0m",
        addr
    );
    println!();

    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
