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

mod db;
mod error;
mod handlers;
mod middleware;
mod models;
mod routes;
mod state;

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
        (url = "http://localhost:3000", description = "Local development server")
    ),
    paths(
        // Sessions
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
        // Messages
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
        // Contacts
        handlers::contacts::check_on_whatsapp,
        handlers::contacts::get_contact_info,
        handlers::contacts::get_profile_picture,
        handlers::contacts::get_user_info,
        // Groups
        handlers::groups::list_groups,
        handlers::groups::get_group,
        handlers::groups::get_group_info,
        // Presence
        handlers::presence::set_presence,
        // Chat state
        handlers::chatstate::send_chatstate,
        handlers::chatstate::send_typing,
        // Blocking
        handlers::blocking::get_blocklist,
        handlers::blocking::block_contact,
        handlers::blocking::unblock_contact,
        handlers::blocking::is_blocked,
        // Media
        handlers::media::upload_media,
        // Webhooks
        handlers::webhooks::list_webhooks,
        handlers::webhooks::register_webhook,
        handlers::webhooks::unregister_webhook,
    ),
    components(
        schemas(
            // Sessions
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
            // Messages
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
            // Contacts
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
            // Groups
            models::groups::GroupListResponse,
            models::groups::GroupInfo,
            models::groups::GroupInfoCached,
            models::groups::GroupParticipant,
            models::groups::ParticipantRole,
            // Presence
            models::presence::SetPresenceRequest,
            models::presence::PresenceStatus,
            // Chat state
            models::chatstate::SendChatStateRequest,
            models::chatstate::ChatStateType,
            handlers::chatstate::TypingRequest,
            // Blocking
            models::blocking::BlocklistResponse,
            models::blocking::BlockRequest,
            handlers::blocking::BlockStatusResponse,
            // Media
            models::media::UploadMediaResponse,
            models::media::MediaType,
            // Webhooks
            models::webhooks::WebhookConfig,
            models::webhooks::WebhookEvent,
            models::webhooks::RegisterWebhookRequest,
            models::webhooks::WebhookListResponse,
            models::webhooks::WebhookRequest,
            // Common
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
        (name = "webhooks", description = "Webhook registration for events")
    )
)]
struct ApiDoc;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "whatsapp_rest_api=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    tracing::info!("Starting WhatsApp REST API server...");

    // Get database configuration from environment
    let db_host = std::env::var("POSTGRES_HOST").unwrap_or_else(|_| "localhost".to_string());
    let db_port: u16 = std::env::var("POSTGRES_PORT")
        .unwrap_or_else(|_| "5432".to_string())
        .parse()
        .unwrap_or(5432);
    let db_user = std::env::var("POSTGRES_USER").unwrap_or_else(|_| "postgres".to_string());
    let db_password = std::env::var("POSTGRES_PASSWORD").unwrap_or_else(|_| "postgres".to_string());
    let db_name = std::env::var("POSTGRES_DB").unwrap_or_else(|_| "wagateway".to_string());

    tracing::info!("Connecting to PostgreSQL at {}:{}/{}", db_host, db_port, db_name);

    // Build PostgreSQL config
    let mut pg_config = tokio_postgres::Config::new();
    pg_config.host(&db_host);
    pg_config.port(db_port);
    pg_config.user(&db_user);
    pg_config.password(&db_password);
    pg_config.dbname(&db_name);

    // Create connection pool
    let mgr_config = ManagerConfig {
        recycling_method: RecyclingMethod::Fast,
    };
    let mgr = Manager::from_config(pg_config, NoTls, mgr_config);
    let pool = Pool::builder(mgr).max_size(16).build()?;

    // Test connection
    let _ = pool.get().await?;
    tracing::info!("Connected to PostgreSQL");

    // Initialize database schema
    db::schema::init_schema(&pool).await?;
    tracing::info!("Database schema initialized");

    // Initialize application state
    let state = AppState::new(pool).await;

    // Build CORS layer
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // Generate and log superadmin token for initial access
    let superadmin_token = middleware::jwt::generate_superadmin_token();
    tracing::info!("===========================================");
    tracing::info!("SUPERADMIN JWT TOKEN (save this!):");
    tracing::info!("{}", superadmin_token);
    tracing::info!("===========================================");

    // Build router with all routes
    let swagger_router: axum::Router<AppState> = SwaggerUi::new("/swagger-ui")
        .url("/api-docs/openapi.json", ApiDoc::openapi())
        .into();
    let app = create_router()
        .merge(swagger_router)
        .layer(axum::middleware::from_fn(middleware::jwt::jwt_auth_middleware))
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state);

    // Start server
    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    tracing::info!("Server listening on http://{}", addr);
    tracing::info!("Swagger UI available at http://{}/swagger-ui", addr);

    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
