use deadpool_postgres::Pool;

pub async fn init_schema(pool: &Pool) -> anyhow::Result<()> {
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
                events TEXT[] NOT NULL DEFAULT '{}',
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
            r#"
            CREATE INDEX IF NOT EXISTS idx_webhooks_session_id ON webhooks(session_id)
            "#,
            &[],
        )
        .await?;

    Ok(())
}
