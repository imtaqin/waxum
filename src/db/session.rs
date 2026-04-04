use crate::models::sessions::{SessionInfo, SessionStatus};
use crate::models::webhooks::{WebhookConfig, WebhookEvent};
use chrono::{DateTime, Utc};
use deadpool_postgres::Pool as PgPool;
use mysql_async::prelude::*;
use mysql_async::Pool as MyPool;

#[derive(Clone)]
pub enum DbPool {
    Postgres(PgPool),
    MySQL(MyPool),
}

#[derive(Clone)]
pub struct SessionManager {
    pool: DbPool,
}

impl SessionManager {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn create_session(
        &self,
        id: &str,
        name: Option<&str>,
        storage_path: &str,
    ) -> anyhow::Result<SessionInfo> {
        let name_str = name.unwrap_or("");

        match &self.pool {
            DbPool::Postgres(pool) => {
                let client = pool.get().await?;
                let row = client
                    .query_one(
                        "INSERT INTO sessions (id, name, storage_path, status, is_logged_in) VALUES ($1, $2, $3, 'disconnected', FALSE) RETURNING *",
                        &[&id, &name_str, &storage_path],
                    )
                    .await?;
                Ok(pg_row_to_session(&row))
            }
            DbPool::MySQL(pool) => {
                let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
                let mut conn = pool.get_conn().await?;
                conn.exec_drop(
                    "INSERT INTO sessions (id, name, storage_path, status, is_logged_in, created_at, updated_at) VALUES (?, ?, ?, 'disconnected', 0, ?, ?)",
                    (id, name_str, storage_path, &now, &now),
                ).await?;
                drop(conn);
                let session = self.get_session(id).await?;
                session.ok_or_else(|| anyhow::anyhow!("Failed to fetch created session"))
            }
        }
    }

    pub async fn get_session(&self, id: &str) -> anyhow::Result<Option<SessionInfo>> {
        match &self.pool {
            DbPool::Postgres(pool) => {
                let client = pool.get().await?;
                let row = client
                    .query_opt("SELECT * FROM sessions WHERE id = $1", &[&id])
                    .await?;
                Ok(row.map(|r| pg_row_to_session(&r)))
            }
            DbPool::MySQL(pool) => {
                let mut conn = pool.get_conn().await?;
                let row: Option<mysql_async::Row> = conn
                    .exec_first("SELECT * FROM sessions WHERE id = ?", (id,))
                    .await?;
                Ok(row.map(|r| my_row_to_session(&r)))
            }
        }
    }

    pub async fn get_storage_path(&self, id: &str) -> anyhow::Result<Option<String>> {
        match &self.pool {
            DbPool::Postgres(pool) => {
                let client = pool.get().await?;
                let row = client
                    .query_opt("SELECT storage_path FROM sessions WHERE id = $1", &[&id])
                    .await?;
                Ok(row.map(|r| r.get::<_, String>("storage_path")))
            }
            DbPool::MySQL(pool) => {
                let mut conn = pool.get_conn().await?;
                let row: Option<String> = conn
                    .exec_first("SELECT storage_path FROM sessions WHERE id = ?", (id,))
                    .await?;
                Ok(row)
            }
        }
    }

    pub async fn list_sessions(&self) -> anyhow::Result<Vec<SessionInfo>> {
        match &self.pool {
            DbPool::Postgres(pool) => {
                let client = pool.get().await?;
                let rows = client
                    .query("SELECT * FROM sessions ORDER BY created_at DESC", &[])
                    .await?;
                Ok(rows.iter().map(pg_row_to_session).collect())
            }
            DbPool::MySQL(pool) => {
                let mut conn = pool.get_conn().await?;
                let rows: Vec<mysql_async::Row> = conn
                    .exec("SELECT * FROM sessions ORDER BY created_at DESC", ())
                    .await?;
                Ok(rows.iter().map(my_row_to_session).collect())
            }
        }
    }

    pub async fn update_session_status(
        &self,
        id: &str,
        status: SessionStatus,
        is_logged_in: bool,
    ) -> anyhow::Result<()> {
        match &self.pool {
            DbPool::Postgres(pool) => {
                let client = pool.get().await?;
                client
                    .execute(
                        "UPDATE sessions SET status = $1, is_logged_in = $2, updated_at = NOW() WHERE id = $3",
                        &[&status.as_str(), &is_logged_in, &id],
                    )
                    .await?;
            }
            DbPool::MySQL(pool) => {
                let logged_in: i32 = if is_logged_in { 1 } else { 0 };
                let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
                let mut conn = pool.get_conn().await?;
                conn.exec_drop(
                    "UPDATE sessions SET status = ?, is_logged_in = ?, updated_at = ? WHERE id = ?",
                    (status.as_str(), logged_in, &now, id),
                )
                .await?;
            }
        }
        Ok(())
    }

    pub async fn update_session_info(
        &self,
        id: &str,
        phone_number: Option<&str>,
        push_name: Option<&str>,
    ) -> anyhow::Result<()> {
        match &self.pool {
            DbPool::Postgres(pool) => {
                let client = pool.get().await?;
                client
                    .execute(
                        "UPDATE sessions SET phone_number = COALESCE($1, phone_number), push_name = COALESCE($2, push_name), updated_at = NOW() WHERE id = $3",
                        &[&phone_number, &push_name, &id],
                    )
                    .await?;
            }
            DbPool::MySQL(pool) => {
                let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
                let mut conn = pool.get_conn().await?;
                conn.exec_drop(
                    "UPDATE sessions SET phone_number = COALESCE(?, phone_number), push_name = COALESCE(?, push_name), updated_at = ? WHERE id = ?",
                    (phone_number, push_name, &now, id),
                )
                .await?;
            }
        }
        Ok(())
    }

    pub async fn update_last_connected(&self, id: &str) -> anyhow::Result<()> {
        match &self.pool {
            DbPool::Postgres(pool) => {
                let client = pool.get().await?;
                client
                    .execute(
                        "UPDATE sessions SET last_connected_at = NOW(), updated_at = NOW() WHERE id = $1",
                        &[&id],
                    )
                    .await?;
            }
            DbPool::MySQL(pool) => {
                let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
                let mut conn = pool.get_conn().await?;
                conn.exec_drop(
                    "UPDATE sessions SET last_connected_at = ?, updated_at = ? WHERE id = ?",
                    (&now, &now, id),
                )
                .await?;
            }
        }
        Ok(())
    }

    pub async fn delete_session(&self, id: &str) -> anyhow::Result<bool> {
        match &self.pool {
            DbPool::Postgres(pool) => {
                let client = pool.get().await?;
                let result = client
                    .execute("DELETE FROM sessions WHERE id = $1", &[&id])
                    .await?;
                Ok(result > 0)
            }
            DbPool::MySQL(pool) => {
                let mut conn = pool.get_conn().await?;
                conn.exec_drop("DELETE FROM sessions WHERE id = ?", (id,))
                    .await?;
                Ok(conn.affected_rows() > 0)
            }
        }
    }

    pub async fn create_webhook(
        &self,
        id: &str,
        session_id: &str,
        config: &WebhookConfig,
    ) -> anyhow::Result<()> {
        let events_str: String = config
            .events
            .iter()
            .map(|e| e.as_str().to_string())
            .collect::<Vec<_>>()
            .join(",");

        match &self.pool {
            DbPool::Postgres(pool) => {
                let client = pool.get().await?;
                client
                    .execute(
                        "INSERT INTO webhooks (id, session_id, url, events, secret, enabled) VALUES ($1, $2, $3, $4, $5, $6)",
                        &[&id, &session_id, &config.url, &events_str, &config.secret, &config.enabled],
                    )
                    .await?;
            }
            DbPool::MySQL(pool) => {
                let enabled: i32 = if config.enabled { 1 } else { 0 };
                let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
                let mut conn = pool.get_conn().await?;
                conn.exec_drop(
                    "INSERT INTO webhooks (id, session_id, url, events, secret, enabled, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
                    (id, session_id, &config.url, &events_str, &config.secret, enabled, &now),
                )
                .await?;
            }
        }
        Ok(())
    }

    #[allow(dead_code)]
    pub async fn get_webhooks(
        &self,
        session_id: &str,
    ) -> anyhow::Result<Vec<(String, WebhookConfig)>> {
        match &self.pool {
            DbPool::Postgres(pool) => {
                let client = pool.get().await?;
                let rows = client
                    .query(
                        "SELECT * FROM webhooks WHERE session_id = $1",
                        &[&session_id],
                    )
                    .await?;
                Ok(rows.iter().map(pg_row_to_webhook).collect())
            }
            DbPool::MySQL(pool) => {
                let mut conn = pool.get_conn().await?;
                let rows: Vec<mysql_async::Row> = conn
                    .exec("SELECT * FROM webhooks WHERE session_id = ?", (session_id,))
                    .await?;
                Ok(rows.iter().map(my_row_to_webhook).collect())
            }
        }
    }

    pub async fn delete_webhook(&self, id: &str) -> anyhow::Result<bool> {
        match &self.pool {
            DbPool::Postgres(pool) => {
                let client = pool.get().await?;
                let result = client
                    .execute("DELETE FROM webhooks WHERE id = $1", &[&id])
                    .await?;
                Ok(result > 0)
            }
            DbPool::MySQL(pool) => {
                let mut conn = pool.get_conn().await?;
                conn.exec_drop("DELETE FROM webhooks WHERE id = ?", (id,))
                    .await?;
                Ok(conn.affected_rows() > 0)
            }
        }
    }
}

// === PostgreSQL row mapping ===

fn pg_row_to_session(row: &tokio_postgres::Row) -> SessionInfo {
    let created_at: DateTime<Utc> = row.get("created_at");
    let updated_at: DateTime<Utc> = row.get("updated_at");
    let last_connected_at: Option<DateTime<Utc>> = row.get("last_connected_at");
    let status_str: String = row.get("status");

    SessionInfo {
        id: row.get("id"),
        name: row.get("name"),
        phone_number: row.get("phone_number"),
        push_name: row.get("push_name"),
        status: SessionStatus::from_str(&status_str),
        created_at: created_at.timestamp(),
        updated_at: updated_at.timestamp(),
        last_connected_at: last_connected_at.map(|t| t.timestamp()),
        is_logged_in: row.get("is_logged_in"),
    }
}

fn pg_row_to_webhook(row: &tokio_postgres::Row) -> (String, WebhookConfig) {
    let id: String = row.get("id");
    let url: String = row.get("url");
    let events_raw: String = row.get("events");
    let secret: Option<String> = row.get("secret");
    let enabled: bool = row.get("enabled");

    let events: Vec<WebhookEvent> = events_raw
        .split(',')
        .filter(|s| !s.is_empty())
        .filter_map(|s| WebhookEvent::from_str(s.trim()))
        .collect();

    (
        id,
        WebhookConfig {
            url,
            events,
            secret,
            enabled,
        },
    )
}

// === MySQL row mapping ===

fn my_row_to_session(row: &mysql_async::Row) -> SessionInfo {
    let status_str: String = row.get("status").unwrap_or_default();
    let is_logged_in: i32 = row.get("is_logged_in").unwrap_or(0);

    let created_at: Option<String> = row.get("created_at");
    let updated_at: Option<String> = row.get("updated_at");
    let last_connected_at: Option<String> = row.get("last_connected_at");

    SessionInfo {
        id: row.get("id").unwrap_or_default(),
        name: row.get("name"),
        phone_number: row.get("phone_number"),
        push_name: row.get("push_name"),
        status: SessionStatus::from_str(&status_str),
        created_at: parse_mysql_timestamp(created_at.as_deref()).unwrap_or(0),
        updated_at: parse_mysql_timestamp(updated_at.as_deref()).unwrap_or(0),
        last_connected_at: parse_mysql_timestamp(last_connected_at.as_deref()),
        is_logged_in: is_logged_in != 0,
    }
}

fn my_row_to_webhook(row: &mysql_async::Row) -> (String, WebhookConfig) {
    let id: String = row.get("id").unwrap_or_default();
    let url: String = row.get("url").unwrap_or_default();
    let events_raw: String = row.get("events").unwrap_or_default();
    let secret: Option<String> = row.get("secret");
    let enabled: i32 = row.get("enabled").unwrap_or(1);

    let events: Vec<WebhookEvent> = events_raw
        .split(',')
        .filter(|s| !s.is_empty())
        .filter_map(|s| WebhookEvent::from_str(s.trim()))
        .collect();

    (
        id,
        WebhookConfig {
            url,
            events,
            secret,
            enabled: enabled != 0,
        },
    )
}

fn parse_mysql_timestamp(s: Option<&str>) -> Option<i64> {
    let s = s?;
    if s.is_empty() {
        return None;
    }
    for fmt in &[
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S",
    ] {
        if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, fmt) {
            return Some(dt.and_utc().timestamp());
        }
    }
    None
}
