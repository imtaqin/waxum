pub mod schema;
pub mod session;

pub use session::SessionManager;

use sqlx::AnyPool;

/// Create a database pool from DATABASE_URL env var.
/// Supports: postgres://, mysql://, sqlite://
///
/// Falls back to legacy POSTGRES_* env vars if DATABASE_URL is not set.
pub async fn create_pool() -> anyhow::Result<AnyPool> {
    // Install all drivers
    sqlx::any::install_default_drivers();

    let database_url = if let Ok(url) = std::env::var("DATABASE_URL") {
        url
    } else {
        // Fallback: build URL from legacy POSTGRES_* vars
        let host = std::env::var("POSTGRES_HOST").unwrap_or_else(|_| "localhost".to_string());
        let port = std::env::var("POSTGRES_PORT").unwrap_or_else(|_| "5432".to_string());
        let user = std::env::var("POSTGRES_USER").unwrap_or_else(|_| "postgres".to_string());
        let password =
            std::env::var("POSTGRES_PASSWORD").unwrap_or_else(|_| "postgres".to_string());
        let db = std::env::var("POSTGRES_DB").unwrap_or_else(|_| "wagateway".to_string());

        format!("postgres://{}:{}@{}:{}/{}", user, password, host, port, db)
    };

    let backend_name = if database_url.starts_with("postgres") {
        "PostgreSQL"
    } else if database_url.starts_with("mysql") {
        "MySQL"
    } else if database_url.starts_with("sqlite") {
        "SQLite"
    } else {
        "Unknown"
    };

    // Mask password in log
    let masked_url = mask_url(&database_url);
    tracing::info!("Connecting to {} ({})", backend_name, masked_url);

    // SQLite: ensure create mode is enabled
    let connect_url = if database_url.starts_with("sqlite") && !database_url.contains("mode=") {
        if database_url.contains('?') {
            format!("{}&mode=rwc", database_url)
        } else {
            format!("{}?mode=rwc", database_url)
        }
    } else {
        database_url.clone()
    };

    let pool = AnyPool::connect(&connect_url).await?;

    tracing::info!("Connected to {}", backend_name);

    Ok(pool)
}

fn mask_url(url: &str) -> String {
    // Mask password in connection URL for safe logging
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
