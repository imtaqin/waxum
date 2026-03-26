use sqlx::AnyPool;

pub async fn init_schema(pool: &AnyPool) -> anyhow::Result<()> {
    let backend = detect_backend(pool);

    match backend {
        DbBackend::Postgres => init_postgres(pool).await,
        DbBackend::MySQL => init_mysql(pool).await,
        DbBackend::SQLite => init_sqlite(pool).await,
    }
}

#[derive(Debug, Clone, Copy)]
enum DbBackend {
    Postgres,
    MySQL,
    SQLite,
}

fn detect_backend(pool: &AnyPool) -> DbBackend {
    let url = std::env::var("DATABASE_URL").unwrap_or_default();
    if url.starts_with("postgres") {
        DbBackend::Postgres
    } else if url.starts_with("mysql") {
        DbBackend::MySQL
    } else if url.starts_with("sqlite") {
        DbBackend::SQLite
    } else {
        // Fallback: check pool kind name
        let name = format!("{:?}", pool);
        if name.contains("Postgres") {
            DbBackend::Postgres
        } else if name.contains("MySql") {
            DbBackend::MySQL
        } else {
            DbBackend::SQLite
        }
    }
}

async fn init_postgres(pool: &AnyPool) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS sessions (
            id VARCHAR(255) PRIMARY KEY,
            name VARCHAR(255),
            storage_path TEXT NOT NULL,
            phone_number VARCHAR(50),
            push_name VARCHAR(255),
            status VARCHAR(50) NOT NULL DEFAULT 'disconnected',
            is_logged_in BOOLEAN NOT NULL DEFAULT FALSE,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            last_connected_at TIMESTAMPTZ
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS webhooks (
            id VARCHAR(255) PRIMARY KEY,
            session_id VARCHAR(255) NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            url TEXT NOT NULL,
            events TEXT NOT NULL DEFAULT '',
            secret VARCHAR(255),
            enabled BOOLEAN NOT NULL DEFAULT TRUE,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_webhooks_session_id ON webhooks(session_id)")
        .execute(pool)
        .await?;

    Ok(())
}

async fn init_mysql(pool: &AnyPool) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS sessions (
            id VARCHAR(255) PRIMARY KEY,
            name VARCHAR(255),
            storage_path TEXT NOT NULL,
            phone_number VARCHAR(50),
            push_name VARCHAR(255),
            status VARCHAR(50) NOT NULL DEFAULT 'disconnected',
            is_logged_in TINYINT(1) NOT NULL DEFAULT 0,
            created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
            updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,
            last_connected_at TIMESTAMP NULL
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS webhooks (
            id VARCHAR(255) PRIMARY KEY,
            session_id VARCHAR(255) NOT NULL,
            url TEXT NOT NULL,
            events VARCHAR(2000) NOT NULL DEFAULT '',
            secret VARCHAR(255),
            enabled TINYINT(1) NOT NULL DEFAULT 1,
            created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE,
            INDEX idx_webhooks_session_id (session_id)
        )
        "#,
    )
    .execute(pool)
    .await?;

    Ok(())
}

async fn init_sqlite(pool: &AnyPool) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS sessions (
            id TEXT PRIMARY KEY,
            name TEXT,
            storage_path TEXT NOT NULL,
            phone_number TEXT,
            push_name TEXT,
            status TEXT NOT NULL DEFAULT 'disconnected',
            is_logged_in INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            last_connected_at TEXT
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS webhooks (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            url TEXT NOT NULL,
            events TEXT NOT NULL DEFAULT '',
            secret TEXT,
            enabled INTEGER NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_webhooks_session_id ON webhooks(session_id)")
        .execute(pool)
        .await?;

    Ok(())
}
