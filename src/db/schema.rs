use crate::db::session::{sqlite_blocking, DbPool};

pub async fn init_schema(pool: &DbPool) -> anyhow::Result<()> {
    match pool {
        DbPool::Postgres(pg) => init_postgres(pg).await,
        DbPool::MySQL(my) => init_mysql(my).await,
        DbPool::SQLite(s) => init_sqlite(s).await,
    }
}

async fn init_sqlite(pool: &crate::db::session::SqlitePool) -> anyhow::Result<()> {
    use crate::db::sqlite_raw;
    sqlite_blocking(pool, |conn| {
        sqlite_raw::exec_batch(
            conn,
            "PRAGMA journal_mode=WAL; \
             PRAGMA foreign_keys=ON; \
             CREATE TABLE IF NOT EXISTS sessions ( \
                id TEXT PRIMARY KEY, \
                name TEXT, \
                storage_path TEXT NOT NULL, \
                phone_number TEXT, \
                push_name TEXT, \
                status TEXT NOT NULL DEFAULT 'disconnected', \
                is_logged_in INTEGER NOT NULL DEFAULT 0, \
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%d %H:%M:%S','now')), \
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%d %H:%M:%S','now')), \
                last_connected_at TEXT \
             ); \
             CREATE TABLE IF NOT EXISTS webhooks ( \
                id TEXT PRIMARY KEY, \
                session_id TEXT NOT NULL, \
                url TEXT NOT NULL, \
                events TEXT NOT NULL DEFAULT '', \
                secret TEXT, \
                enabled INTEGER NOT NULL DEFAULT 1, \
                disabled_at TEXT, \
                disabled_reason TEXT, \
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%d %H:%M:%S','now')), \
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE \
             ); \
             CREATE INDEX IF NOT EXISTS idx_webhooks_session_id ON webhooks(session_id); \
             CREATE TABLE IF NOT EXISTS contacts ( \
                session_id TEXT NOT NULL, \
                jid TEXT NOT NULL, \
                phone TEXT, \
                lid_jid TEXT, \
                full_name TEXT, \
                first_name TEXT, \
                push_name TEXT, \
                business_name TEXT, \
                source TEXT NOT NULL DEFAULT 'unknown', \
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%d %H:%M:%S','now')), \
                PRIMARY KEY (session_id, jid), \
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE \
             ); \
             CREATE INDEX IF NOT EXISTS idx_contacts_phone ON contacts(session_id, phone); \
             CREATE TABLE IF NOT EXISTS webhook_dlq ( \
                id INTEGER PRIMARY KEY AUTOINCREMENT, \
                session_id TEXT NOT NULL, \
                webhook_url TEXT NOT NULL, \
                event_type TEXT NOT NULL, \
                payload TEXT NOT NULL, \
                last_error TEXT, \
                attempts INTEGER NOT NULL DEFAULT 0, \
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%d %H:%M:%S','now')), \
                last_attempt_at TEXT \
             ); \
             CREATE INDEX IF NOT EXISTS idx_webhook_dlq_session ON webhook_dlq(session_id); \
             CREATE TABLE IF NOT EXISTS scheduled_messages ( \
                id TEXT PRIMARY KEY, \
                session_id TEXT NOT NULL, \
                endpoint TEXT NOT NULL, \
                body TEXT NOT NULL, \
                send_at TEXT NOT NULL, \
                status TEXT NOT NULL DEFAULT 'pending', \
                error TEXT, \
                message_id TEXT, \
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%d %H:%M:%S','now')), \
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%d %H:%M:%S','now')) \
             ); \
             CREATE INDEX IF NOT EXISTS idx_scheduled_messages_due ON scheduled_messages(status, send_at); \
             CREATE INDEX IF NOT EXISTS idx_scheduled_messages_session ON scheduled_messages(session_id); \
             CREATE TABLE IF NOT EXISTS blast_jobs ( \
                id TEXT PRIMARY KEY, \
                session_id TEXT NOT NULL, \
                endpoint TEXT NOT NULL, \
                body TEXT NOT NULL, \
                options TEXT NOT NULL, \
                status TEXT NOT NULL DEFAULT 'pending', \
                total INTEGER NOT NULL DEFAULT 0, \
                sent_count INTEGER NOT NULL DEFAULT 0, \
                failed_count INTEGER NOT NULL DEFAULT 0, \
                dlq_count INTEGER NOT NULL DEFAULT 0, \
                skipped_dup_count INTEGER NOT NULL DEFAULT 0, \
                send_at TEXT, \
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%d %H:%M:%S','now')), \
                started_at TEXT, \
                finished_at TEXT \
             ); \
             CREATE INDEX IF NOT EXISTS idx_blast_jobs_runnable ON blast_jobs(status, send_at); \
             CREATE INDEX IF NOT EXISTS idx_blast_jobs_session ON blast_jobs(session_id); \
             CREATE TABLE IF NOT EXISTS blast_recipients ( \
                id INTEGER PRIMARY KEY AUTOINCREMENT, \
                job_id TEXT NOT NULL, \
                session_id TEXT NOT NULL, \
                recipient TEXT NOT NULL, \
                status TEXT NOT NULL DEFAULT 'pending', \
                attempts INTEGER NOT NULL DEFAULT 0, \
                last_error TEXT, \
                message_id TEXT, \
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%d %H:%M:%S','now')), \
                UNIQUE (job_id, recipient) \
             ); \
             CREATE INDEX IF NOT EXISTS idx_blast_recipients_dedup ON blast_recipients(session_id, recipient, status); \
             CREATE INDEX IF NOT EXISTS idx_blast_recipients_job ON blast_recipients(job_id, status); \
             CREATE TABLE IF NOT EXISTS messages ( \
                id INTEGER PRIMARY KEY AUTOINCREMENT, \
                message_id TEXT NOT NULL, \
                session_id TEXT NOT NULL, \
                chat_jid TEXT NOT NULL, \
                sender_jid TEXT NOT NULL, \
                direction TEXT NOT NULL, \
                msg_type TEXT NOT NULL, \
                body TEXT, \
                msg_timestamp TEXT NOT NULL, \
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%d %H:%M:%S','now')), \
                UNIQUE (session_id, message_id) \
             ); \
             CREATE INDEX IF NOT EXISTS idx_messages_session_ts ON messages(session_id, msg_timestamp); \
             CREATE INDEX IF NOT EXISTS idx_messages_chat ON messages(session_id, chat_jid);",
        )?;
        if let Err(e) = sqlite_raw::exec_batch(
            conn,
            "CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(body, session_id UNINDEXED, message_id UNINDEXED);",
        ) {
            tracing::warn!(
                "FTS5 unavailable, message search will use LIKE fallback: {}",
                e
            );
        }
        Ok(())
    })
    .await
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
                disabled_at TIMESTAMPTZ,
                disabled_reason TEXT,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )
            "#,
            &[],
        )
        .await?;

    let _ = client
        .execute(
            "ALTER TABLE webhooks ADD COLUMN IF NOT EXISTS disabled_at TIMESTAMPTZ",
            &[],
        )
        .await;
    let _ = client
        .execute(
            "ALTER TABLE webhooks ADD COLUMN IF NOT EXISTS disabled_reason TEXT",
            &[],
        )
        .await;

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

    client
        .execute(
            r#"
            CREATE TABLE IF NOT EXISTS webhook_dlq (
                id BIGSERIAL PRIMARY KEY,
                session_id VARCHAR(255) NOT NULL,
                webhook_url TEXT NOT NULL,
                event_type VARCHAR(64) NOT NULL,
                payload TEXT NOT NULL,
                last_error TEXT,
                attempts INT NOT NULL DEFAULT 0,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                last_attempt_at TIMESTAMPTZ
            )
            "#,
            &[],
        )
        .await?;
    client
        .execute(
            "CREATE INDEX IF NOT EXISTS idx_webhook_dlq_session ON webhook_dlq(session_id)",
            &[],
        )
        .await?;

    client
        .execute(
            r#"
            CREATE TABLE IF NOT EXISTS scheduled_messages (
                id VARCHAR(64) PRIMARY KEY,
                session_id VARCHAR(255) NOT NULL,
                endpoint VARCHAR(64) NOT NULL,
                body TEXT NOT NULL,
                send_at TIMESTAMPTZ NOT NULL,
                status VARCHAR(16) NOT NULL DEFAULT 'pending',
                error TEXT,
                message_id VARCHAR(255),
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )
            "#,
            &[],
        )
        .await?;
    client
        .execute(
            "CREATE INDEX IF NOT EXISTS idx_scheduled_messages_due ON scheduled_messages(status, send_at)",
            &[],
        )
        .await?;
    client
        .execute(
            "CREATE INDEX IF NOT EXISTS idx_scheduled_messages_session ON scheduled_messages(session_id)",
            &[],
        )
        .await?;

    client
        .execute(
            r#"
            CREATE TABLE IF NOT EXISTS blast_jobs (
                id VARCHAR(64) PRIMARY KEY,
                session_id VARCHAR(255) NOT NULL,
                endpoint VARCHAR(64) NOT NULL,
                body TEXT NOT NULL,
                options TEXT NOT NULL,
                status VARCHAR(32) NOT NULL DEFAULT 'pending',
                total BIGINT NOT NULL DEFAULT 0,
                sent_count BIGINT NOT NULL DEFAULT 0,
                failed_count BIGINT NOT NULL DEFAULT 0,
                dlq_count BIGINT NOT NULL DEFAULT 0,
                skipped_dup_count BIGINT NOT NULL DEFAULT 0,
                send_at TIMESTAMPTZ,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                started_at TIMESTAMPTZ,
                finished_at TIMESTAMPTZ
            )
            "#,
            &[],
        )
        .await?;
    client
        .execute(
            "CREATE INDEX IF NOT EXISTS idx_blast_jobs_runnable ON blast_jobs(status, send_at)",
            &[],
        )
        .await?;
    client
        .execute(
            "CREATE INDEX IF NOT EXISTS idx_blast_jobs_session ON blast_jobs(session_id)",
            &[],
        )
        .await?;

    client
        .execute(
            r#"
            CREATE TABLE IF NOT EXISTS blast_recipients (
                id BIGSERIAL PRIMARY KEY,
                job_id VARCHAR(64) NOT NULL,
                session_id VARCHAR(255) NOT NULL,
                recipient VARCHAR(255) NOT NULL,
                status VARCHAR(16) NOT NULL DEFAULT 'pending',
                attempts BIGINT NOT NULL DEFAULT 0,
                last_error TEXT,
                message_id VARCHAR(255),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                UNIQUE (job_id, recipient)
            )
            "#,
            &[],
        )
        .await?;
    client
        .execute(
            "CREATE INDEX IF NOT EXISTS idx_blast_recipients_dedup ON blast_recipients(session_id, recipient, status)",
            &[],
        )
        .await?;
    client
        .execute(
            "CREATE INDEX IF NOT EXISTS idx_blast_recipients_job ON blast_recipients(job_id, status)",
            &[],
        )
        .await?;

    client
        .execute(
            r#"
            CREATE TABLE IF NOT EXISTS messages (
                id BIGSERIAL PRIMARY KEY,
                message_id VARCHAR(255) NOT NULL,
                session_id VARCHAR(255) NOT NULL,
                chat_jid VARCHAR(255) NOT NULL,
                sender_jid VARCHAR(255) NOT NULL,
                direction VARCHAR(8) NOT NULL,
                msg_type VARCHAR(32) NOT NULL,
                body TEXT,
                msg_timestamp TIMESTAMPTZ NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                body_tsv TSVECTOR GENERATED ALWAYS AS (to_tsvector('simple', coalesce(body, ''))) STORED,
                UNIQUE (session_id, message_id)
            )
            "#,
            &[],
        )
        .await?;
    client
        .execute(
            "CREATE INDEX IF NOT EXISTS idx_messages_session_ts ON messages(session_id, msg_timestamp)",
            &[],
        )
        .await?;
    client
        .execute(
            "CREATE INDEX IF NOT EXISTS idx_messages_chat ON messages(session_id, chat_jid)",
            &[],
        )
        .await?;
    client
        .execute(
            "CREATE INDEX IF NOT EXISTS idx_messages_fts ON messages USING GIN (body_tsv)",
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
            disabled_at VARCHAR(30) NULL,
            disabled_reason TEXT NULL,
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
        ) DEFAULT CHARSET=utf8mb4
        "#,
    )
    .await?;

    conn.query_drop(
        r#"
        CREATE TABLE IF NOT EXISTS webhook_dlq (
            id BIGINT AUTO_INCREMENT PRIMARY KEY,
            session_id VARCHAR(255) NOT NULL,
            webhook_url VARCHAR(2000) NOT NULL,
            event_type VARCHAR(64) NOT NULL,
            payload LONGTEXT NOT NULL,
            last_error TEXT,
            attempts INT NOT NULL DEFAULT 0,
            created_at VARCHAR(30) NOT NULL DEFAULT '1970-01-01 00:00:00',
            last_attempt_at VARCHAR(30) NULL,
            INDEX idx_webhook_dlq_session (session_id)
        ) DEFAULT CHARSET=utf8mb4
        "#,
    )
    .await?;

    conn.query_drop(
        r#"
        CREATE TABLE IF NOT EXISTS scheduled_messages (
            id VARCHAR(64) PRIMARY KEY,
            session_id VARCHAR(255) NOT NULL,
            endpoint VARCHAR(64) NOT NULL,
            body LONGTEXT NOT NULL,
            send_at VARCHAR(30) NOT NULL,
            status VARCHAR(16) NOT NULL DEFAULT 'pending',
            error TEXT NULL,
            message_id VARCHAR(255) NULL,
            created_at VARCHAR(30) NOT NULL DEFAULT '1970-01-01 00:00:00',
            updated_at VARCHAR(30) NOT NULL DEFAULT '1970-01-01 00:00:00',
            INDEX idx_scheduled_messages_due (status, send_at),
            INDEX idx_scheduled_messages_session (session_id)
        ) DEFAULT CHARSET=utf8mb4
        "#,
    )
    .await?;

    conn.query_drop(
        r#"
        CREATE TABLE IF NOT EXISTS blast_jobs (
            id VARCHAR(64) PRIMARY KEY,
            session_id VARCHAR(255) NOT NULL,
            endpoint VARCHAR(64) NOT NULL,
            body LONGTEXT NOT NULL,
            options TEXT NOT NULL,
            status VARCHAR(32) NOT NULL DEFAULT 'pending',
            total BIGINT NOT NULL DEFAULT 0,
            sent_count BIGINT NOT NULL DEFAULT 0,
            failed_count BIGINT NOT NULL DEFAULT 0,
            dlq_count BIGINT NOT NULL DEFAULT 0,
            skipped_dup_count BIGINT NOT NULL DEFAULT 0,
            send_at VARCHAR(30) NULL,
            created_at VARCHAR(30) NOT NULL DEFAULT '1970-01-01 00:00:00',
            started_at VARCHAR(30) NULL,
            finished_at VARCHAR(30) NULL,
            INDEX idx_blast_jobs_runnable (status, send_at),
            INDEX idx_blast_jobs_session (session_id)
        ) DEFAULT CHARSET=utf8mb4
        "#,
    )
    .await?;

    conn.query_drop(
        r#"
        CREATE TABLE IF NOT EXISTS blast_recipients (
            id BIGINT AUTO_INCREMENT PRIMARY KEY,
            job_id VARCHAR(64) NOT NULL,
            session_id VARCHAR(255) NOT NULL,
            recipient VARCHAR(255) NOT NULL,
            status VARCHAR(16) NOT NULL DEFAULT 'pending',
            attempts BIGINT NOT NULL DEFAULT 0,
            last_error TEXT NULL,
            message_id VARCHAR(255) NULL,
            updated_at VARCHAR(30) NOT NULL DEFAULT '1970-01-01 00:00:00',
            UNIQUE KEY uniq_blast_recipient (job_id, recipient),
            INDEX idx_blast_recipients_dedup (session_id, recipient, status),
            INDEX idx_blast_recipients_job (job_id, status)
        ) DEFAULT CHARSET=utf8mb4
        "#,
    )
    .await?;

    conn.query_drop(
        r#"
        CREATE TABLE IF NOT EXISTS messages (
            id BIGINT AUTO_INCREMENT PRIMARY KEY,
            message_id VARCHAR(255) NOT NULL,
            session_id VARCHAR(255) NOT NULL,
            chat_jid VARCHAR(255) NOT NULL,
            sender_jid VARCHAR(255) NOT NULL,
            direction VARCHAR(8) NOT NULL,
            msg_type VARCHAR(32) NOT NULL,
            body TEXT NULL,
            msg_timestamp VARCHAR(30) NOT NULL,
            created_at VARCHAR(30) NOT NULL DEFAULT '1970-01-01 00:00:00',
            UNIQUE KEY uniq_messages_id (session_id, message_id),
            INDEX idx_messages_session_ts (session_id, msg_timestamp),
            INDEX idx_messages_chat (session_id, chat_jid),
            FULLTEXT INDEX ft_messages_body (body)
        ) DEFAULT CHARSET=utf8mb4
        "#,
    )
    .await?;

    let migrations = [
        "ALTER TABLE webhooks ADD COLUMN disabled_at VARCHAR(30) NULL",
        "ALTER TABLE webhooks ADD COLUMN disabled_reason TEXT NULL",
        "ALTER TABLE sessions MODIFY COLUMN is_logged_in INT NOT NULL DEFAULT 0",
        "ALTER TABLE sessions MODIFY COLUMN storage_path VARCHAR(500) NOT NULL",
        "ALTER TABLE sessions MODIFY COLUMN created_at VARCHAR(30) NOT NULL DEFAULT '1970-01-01 00:00:00'",
        "ALTER TABLE sessions MODIFY COLUMN updated_at VARCHAR(30) NOT NULL DEFAULT '1970-01-01 00:00:00'",
        "ALTER TABLE sessions MODIFY COLUMN last_connected_at VARCHAR(30) NULL",
        "ALTER TABLE webhooks MODIFY COLUMN enabled INT NOT NULL DEFAULT 1",
        "ALTER TABLE webhooks MODIFY COLUMN url VARCHAR(2000) NOT NULL",
        "ALTER TABLE webhooks MODIFY COLUMN created_at VARCHAR(30) NOT NULL DEFAULT '1970-01-01 00:00:00'",
        "ALTER TABLE contacts MODIFY COLUMN jid VARCHAR(255) CHARACTER SET utf8mb4 COLLATE utf8mb4_general_ci NOT NULL",
        "ALTER TABLE contacts MODIFY COLUMN phone VARCHAR(50) CHARACTER SET utf8mb4 COLLATE utf8mb4_general_ci NULL",
        "ALTER TABLE contacts MODIFY COLUMN lid_jid VARCHAR(255) CHARACTER SET utf8mb4 COLLATE utf8mb4_general_ci NULL",
        "ALTER TABLE contacts MODIFY COLUMN full_name VARCHAR(255) CHARACTER SET utf8mb4 COLLATE utf8mb4_general_ci NULL",
        "ALTER TABLE contacts MODIFY COLUMN first_name VARCHAR(255) CHARACTER SET utf8mb4 COLLATE utf8mb4_general_ci NULL",
        "ALTER TABLE contacts MODIFY COLUMN push_name VARCHAR(255) CHARACTER SET utf8mb4 COLLATE utf8mb4_general_ci NULL",
        "ALTER TABLE contacts MODIFY COLUMN business_name VARCHAR(255) CHARACTER SET utf8mb4 COLLATE utf8mb4_general_ci NULL",
        "ALTER TABLE contacts MODIFY COLUMN source VARCHAR(40) CHARACTER SET utf8mb4 COLLATE utf8mb4_general_ci NOT NULL DEFAULT 'unknown'",
    ];
    for sql in &migrations {
        let _ = conn.query_drop(*sql).await;
    }

    Ok(())
}
