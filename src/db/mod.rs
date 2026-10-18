//! Multi-backend database layer.
//!
//! The gateway supports three backends selected from `DATABASE_URL`:
//!
//! - **Postgres** (`postgres://…`) — via `deadpool_postgres`.
//! - **MySQL** (`mysql://…`) — via `mysql_async` with a tuned pool.
//! - **SQLite** — the zero-config default when no URL is provided.
//!   Uses a thin FFI wrapper in [`sqlite_raw`] over `libsqlite3-sys` 0.37,
//!   because we ride on the same shared library that
//!   `whatsapp-rust-sqlite-storage` already brings in and cannot pull a
//!   second `rusqlite`-owned copy.
//!
//! Submodules:
//!
//! - [`schema`] — idempotent DDL that runs at startup.
//! - [`session`] — session/webhook registry (the [`SessionManager`] type).
//! - [`contacts`] — WhatsApp contact directory store.
//! - [`webhook_dlq`] — dead-letter queue for failed webhook deliveries.
//! - [`scheduled`] — parked scheduled sends awaiting dispatch.
//! - [`blast`] — bulk-send jobs and their recipient rows.
//! - [`messages`] — message history + full-text search.
//! - [`new_chats`] — new-outgoing-chat counter, limit and incidents.
//! - [`sqlite_raw`] — hand-rolled safe wrapper over `libsqlite3-sys`.

pub mod blast;
pub mod contacts;
pub mod messages;
pub mod new_chats;
pub mod scheduled;
pub mod schema;
pub mod session;
pub mod sqlite_raw;
pub mod tokens;
pub mod webhook_dlq;

pub use session::SessionManager;

/// Detect backend from DATABASE_URL or legacy POSTGRES_* vars.
/// Returns the connection URL and backend type.
pub fn resolve_database_url() -> (String, DbBackend) {
    if let Ok(url) = std::env::var("DATABASE_URL") {
        let trimmed = url.trim();
        if !trimmed.is_empty() {
            let backend = if trimmed.starts_with("mysql") {
                DbBackend::MySQL
            } else if trimmed.starts_with("sqlite") || trimmed.starts_with("file:") {
                DbBackend::SQLite
            } else {
                DbBackend::Postgres
            };
            return (trimmed.to_string(), backend);
        }
    }

    if std::env::var("POSTGRES_HOST").is_ok() || std::env::var("POSTGRES_USER").is_ok() {
        let host = std::env::var("POSTGRES_HOST").unwrap_or_else(|_| "localhost".to_string());
        let port = std::env::var("POSTGRES_PORT").unwrap_or_else(|_| "5432".to_string());
        let user = std::env::var("POSTGRES_USER").unwrap_or_else(|_| "postgres".to_string());
        let password =
            std::env::var("POSTGRES_PASSWORD").unwrap_or_else(|_| "postgres".to_string());
        let db = std::env::var("POSTGRES_DB").unwrap_or_else(|_| "wagateway".to_string());
        let url = format!("postgres://{}:{}@{}:{}/{}", user, password, host, port, db);
        return (url, DbBackend::Postgres);
    }

    if std::env::var("MYSQL_HOST").is_ok() || std::env::var("MYSQL_USER").is_ok() {
        let host = std::env::var("MYSQL_HOST").unwrap_or_else(|_| "localhost".to_string());
        let port = std::env::var("MYSQL_PORT").unwrap_or_else(|_| "3306".to_string());
        let user = std::env::var("MYSQL_USER").unwrap_or_else(|_| "root".to_string());
        let password = std::env::var("MYSQL_PASSWORD").unwrap_or_else(|_| "".to_string());
        let db = std::env::var("MYSQL_DB").unwrap_or_else(|_| "wagateway".to_string());
        let url = format!("mysql://{}:{}@{}:{}/{}", user, password, host, port, db);
        return (url, DbBackend::MySQL);
    }

    let path = std::env::var("SQLITE_PATH").unwrap_or_else(|_| {
        let storage = std::env::var("WHATSAPP_STORAGE_PATH")
            .unwrap_or_else(|_| "./whatsapp_sessions".to_string());
        let joined = std::path::Path::new(&storage).join("waxum.db");
        let s = joined.to_string_lossy().replace('\\', "/");
        if !s.starts_with("./") && !s.starts_with('/') && !s.contains(':') {
            format!("./{}", s)
        } else {
            s
        }
    });
    let _ = std::path::Path::new(&path)
        .parent()
        .map(std::fs::create_dir_all);
    (format!("sqlite://{}", path), DbBackend::SQLite)
}

/// Opens a connection pool for `database_url` on the given backend and
/// checks that it answers.
pub async fn connect(database_url: &str, backend: DbBackend) -> anyhow::Result<session::DbPool> {
    let pool = match backend {
        DbBackend::Postgres => {
            use deadpool_postgres::{Manager, ManagerConfig, RecyclingMethod};
            use tokio_postgres::NoTls;

            let pg_config: tokio_postgres::Config = database_url.parse()?;
            let mgr_config = ManagerConfig {
                recycling_method: RecyclingMethod::Fast,
            };
            let mgr = Manager::from_config(pg_config, NoTls, mgr_config);
            let pg_pool = deadpool_postgres::Pool::builder(mgr).max_size(16).build()?;
            let _ = pg_pool.get().await?;
            session::DbPool::Postgres(pg_pool)
        }
        DbBackend::MySQL => {
            let opts = mysql_async::Opts::from_url(database_url)?;
            let max_pool: usize = std::env::var("WA_RS_MYSQL_MAX_POOL")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(64);
            let constraints = mysql_async::PoolConstraints::new(4, max_pool).unwrap_or_else(|| {
                mysql_async::PoolConstraints::new(2, 10).expect("safe defaults")
            });
            let pool_opts = mysql_async::PoolOpts::default()
                .with_constraints(constraints)
                .with_inactive_connection_ttl(std::time::Duration::from_secs(300))
                .with_ttl_check_interval(std::time::Duration::from_secs(60));
            let opts = mysql_async::OptsBuilder::from_opts(opts).pool_opts(pool_opts);
            let my_pool = mysql_async::Pool::new(opts);
            let _conn = my_pool.get_conn().await?;
            session::DbPool::MySQL(my_pool)
        }
        DbBackend::SQLite => {
            let path = database_url
                .strip_prefix("sqlite://")
                .unwrap_or(database_url);
            let handle = sqlite_raw::open(path)?;
            session::DbPool::SQLite(handle)
        }
    };
    Ok(pool)
}

#[derive(Clone, Copy, Debug)]
pub enum DbBackend {
    Postgres,
    MySQL,
    SQLite,
}

pub fn mask_url(url: &str) -> String {
    if let Some(at_pos) = url.find('@') {
        if let Some(colon_pos) = url[..at_pos].rfind(':') {
            if let Some(slash_pos) = url[..colon_pos].rfind('/') {
                let prefix = &url[..slash_pos + 1];
                let user_end = colon_pos;
                let user = &url[slash_pos + 1..user_end];
                let suffix = &url[at_pos..];
                return format!("{}{}:***{}", prefix, user, suffix);
            }
        }
    }
    url.to_string()
}
