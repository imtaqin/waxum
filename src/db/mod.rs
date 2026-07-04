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
//! - [`sqlite_raw`] — hand-rolled safe wrapper over `libsqlite3-sys`.

pub mod contacts;
pub mod schema;
pub mod session;
pub mod sqlite_raw;
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

    let path = std::env::var("SQLITE_PATH").unwrap_or_else(|_| "wa-rs.db".to_string());
    (format!("sqlite://{}", path), DbBackend::SQLite)
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
