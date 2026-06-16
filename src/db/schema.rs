use crate::db::session::DbPool;

pub async fn init_schema(pool: &DbPool) -> anyhow::Result<()> {
    match pool {
        DbPool::Postgres(pg) => init_postgres(pg).await,
        DbPool::MySQL(my) => init_mysql(my).await,
    }
}

async fn init_postgres(pool: &deadpool_postgres::Pool) -> anyhow::Result<()> {
    let client = pool.get().await?;

    client
        .execute(
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
            &[],
        )
        .await?;

    client
        .execute(
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
            &[],
        )
        .await?;

    client
        .execute(
            "CREATE INDEX IF NOT EXISTS idx_webhooks_session_id ON webhooks(session_id)",
            &[],
        )
        .await?;

    client
        .execute(
            r#"
            CREATE TABLE IF NOT EXISTS contacts (
                session_id VARCHAR(255) NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                jid VARCHAR(255) NOT NULL,
                phone VARCHAR(50),
                lid_jid VARCHAR(255),
                full_name VARCHAR(255),
                first_name VARCHAR(255),
                push_name VARCHAR(255),
                business_name VARCHAR(255),
                source VARCHAR(40) NOT NULL DEFAULT 'unknown',
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                PRIMARY KEY (session_id, jid)
            )
            "#,
            &[],
        )
        .await?;

    client
        .execute(
            "CREATE INDEX IF NOT EXISTS idx_contacts_phone ON contacts(session_id, phone)",
            &[],
        )
        .await?;

    Ok(())
}

async fn init_mysql(pool: &mysql_async::Pool) -> anyhow::Result<()> {
    use mysql_async::prelude::*;

    let mut conn = pool.get_conn().await?;

    conn.query_drop(
        r#"
        CREATE TABLE IF NOT EXISTS sessions (
            id VARCHAR(255) PRIMARY KEY,
            name VARCHAR(255),
            storage_path VARCHAR(500) NOT NULL,
            phone_number VARCHAR(50),
            push_name VARCHAR(255),
            status VARCHAR(50) NOT NULL DEFAULT 'disconnected',
            is_logged_in INT NOT NULL DEFAULT 0,
            created_at VARCHAR(30) NOT NULL DEFAULT '1970-01-01 00:00:00',
            updated_at VARCHAR(30) NOT NULL DEFAULT '1970-01-01 00:00:00',
            last_connected_at VARCHAR(30) NULL
        )
        "#,
    )
    .await?;

    conn.query_drop(
        r#"
        CREATE TABLE IF NOT EXISTS webhooks (
            id VARCHAR(255) PRIMARY KEY,
            session_id VARCHAR(255) NOT NULL,
            url VARCHAR(2000) NOT NULL,
            events VARCHAR(2000) NOT NULL DEFAULT '',
            secret VARCHAR(255),
            enabled INT NOT NULL DEFAULT 1,
            created_at VARCHAR(30) NOT NULL DEFAULT '1970-01-01 00:00:00',
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE,
            INDEX idx_webhooks_session_id (session_id)
        )
        "#,
    )
    .await?;

    conn.query_drop(
        r#"
        CREATE TABLE IF NOT EXISTS contacts (
            session_id VARCHAR(255) NOT NULL,
            jid VARCHAR(255) NOT NULL,
            phone VARCHAR(50),
            lid_jid VARCHAR(255),
            full_name VARCHAR(255),
            first_name VARCHAR(255),
            push_name VARCHAR(255),
            business_name VARCHAR(255),
            source VARCHAR(40) NOT NULL DEFAULT 'unknown',
            updated_at VARCHAR(30) NOT NULL DEFAULT '1970-01-01 00:00:00',
            PRIMARY KEY (session_id, jid),
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE,
            INDEX idx_contacts_phone (session_id, phone)
        )
        "#,
    )
    .await?;

    // Auto-migrate existing tables with incompatible types
    let migrations = [
        "ALTER TABLE sessions MODIFY COLUMN is_logged_in INT NOT NULL DEFAULT 0",
        "ALTER TABLE sessions MODIFY COLUMN storage_path VARCHAR(500) NOT NULL",
        "ALTER TABLE sessions MODIFY COLUMN created_at VARCHAR(30) NOT NULL DEFAULT '1970-01-01 00:00:00'",
        "ALTER TABLE sessions MODIFY COLUMN updated_at VARCHAR(30) NOT NULL DEFAULT '1970-01-01 00:00:00'",
        "ALTER TABLE sessions MODIFY COLUMN last_connected_at VARCHAR(30) NULL",
        "ALTER TABLE webhooks MODIFY COLUMN enabled INT NOT NULL DEFAULT 1",
        "ALTER TABLE webhooks MODIFY COLUMN url VARCHAR(2000) NOT NULL",
        "ALTER TABLE webhooks MODIFY COLUMN created_at VARCHAR(30) NOT NULL DEFAULT '1970-01-01 00:00:00'",
    ];
    for sql in &migrations {
        let _ = conn.query_drop(*sql).await;
    }

    Ok(())
}
