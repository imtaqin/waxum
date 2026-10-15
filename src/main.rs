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
//! - [`api`] — the HTTP surface, one folder per feature under `shared/`
//!   (any session), `web/` (WhatsApp Web) and `cloud/` (Cloud API): handlers,
//!   models and routes together. `api::router` builds `/health`, `/metrics`
//!   and the versioned `/api/v1/*` tree; `api::openapi` is the document behind
//!   the Swagger UI (`/swagger-ui`, `/api-docs`).
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
use waxum::middleware;
use waxum::nats;
use waxum::preflight;
use waxum::state;
use waxum::storage;

use state::AppState;
use waxum::api::openapi::ApiDoc;
use waxum::api::router::create_router;

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
