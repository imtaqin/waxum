use sqlx::any::AnyRow;
use sqlx::{AnyPool, Row};

use crate::models::sessions::{SessionInfo, SessionStatus};
use crate::models::webhooks::{WebhookConfig, WebhookEvent};

#[derive(Clone)]
pub struct SessionManager {
    pool: AnyPool,
    backend: DbBackend,
}

#[derive(Clone, Copy, Debug)]
enum DbBackend {
    Postgres,
    MySQL,
    SQLite,
}

fn detect_backend() -> DbBackend {
    let url = std::env::var("DATABASE_URL").unwrap_or_default();
    if url.starts_with("postgres") {
        DbBackend::Postgres
    } else if url.starts_with("mysql") {
        DbBackend::MySQL
    } else {
        DbBackend::SQLite
    }
}

impl SessionManager {
    pub fn new(pool: AnyPool) -> Self {
        Self {
            pool,
            backend: detect_backend(),
        }
    }

    pub async fn create_session(
        &self,
        id: &str,
        name: Option<&str>,
        storage_path: &str,
    ) -> anyhow::Result<SessionInfo> {
        let name_str = name.unwrap_or("");

        sqlx::query(
            "INSERT INTO sessions (id, name, storage_path, status, is_logged_in) VALUES (?, ?, ?, 'disconnected', 0)",
        )
        .bind(id)
        .bind(name_str)
        .bind(storage_path)
        .execute(&self.pool)
        .await?;

        // Fetch back
        let session = self.get_session(id).await?;
        session.ok_or_else(|| anyhow::anyhow!("Failed to fetch created session"))
    }

    pub async fn get_session(&self, id: &str) -> anyhow::Result<Option<SessionInfo>> {
        let row: Option<AnyRow> =
            sqlx::query("SELECT * FROM sessions WHERE id = ?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?;

        Ok(row.map(|r| self.row_to_session_info(&r)))
    }

    pub async fn get_storage_path(&self, id: &str) -> anyhow::Result<Option<String>> {
        let row: Option<AnyRow> =
            sqlx::query("SELECT storage_path FROM sessions WHERE id = ?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?;

        Ok(row.map(|r| r.get::<String, _>("storage_path")))
    }

    pub async fn list_sessions(&self) -> anyhow::Result<Vec<SessionInfo>> {
        let rows: Vec<AnyRow> =
            sqlx::query("SELECT * FROM sessions ORDER BY created_at DESC")
                .fetch_all(&self.pool)
                .await?;

        Ok(rows.iter().map(|r| self.row_to_session_info(r)).collect())
    }

    pub async fn update_session_status(
        &self,
        id: &str,
        status: SessionStatus,
        is_logged_in: bool,
    ) -> anyhow::Result<()> {
        let logged_in_val: i32 = if is_logged_in { 1 } else { 0 };

        match self.backend {
            DbBackend::SQLite => {
                sqlx::query(
                    "UPDATE sessions SET status = ?, is_logged_in = ?, updated_at = datetime('now') WHERE id = ?",
                )
                .bind(status.as_str())
                .bind(logged_in_val)
                .bind(id)
                .execute(&self.pool)
                .await?;
            }
            _ => {
                sqlx::query(
                    "UPDATE sessions SET status = ?, is_logged_in = ?, updated_at = NOW() WHERE id = ?",
                )
                .bind(status.as_str())
                .bind(logged_in_val)
                .bind(id)
                .execute(&self.pool)
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
        match self.backend {
            DbBackend::SQLite => {
                sqlx::query(
                    "UPDATE sessions SET phone_number = COALESCE(?, phone_number), push_name = COALESCE(?, push_name), updated_at = datetime('now') WHERE id = ?",
                )
                .bind(phone_number)
                .bind(push_name)
                .bind(id)
                .execute(&self.pool)
                .await?;
            }
            _ => {
                sqlx::query(
                    "UPDATE sessions SET phone_number = COALESCE(?, phone_number), push_name = COALESCE(?, push_name), updated_at = NOW() WHERE id = ?",
                )
                .bind(phone_number)
                .bind(push_name)
                .bind(id)
                .execute(&self.pool)
                .await?;
            }
        }

        Ok(())
    }

    pub async fn update_last_connected(&self, id: &str) -> anyhow::Result<()> {
        match self.backend {
            DbBackend::SQLite => {
                sqlx::query(
                    "UPDATE sessions SET last_connected_at = datetime('now'), updated_at = datetime('now') WHERE id = ?",
                )
                .bind(id)
                .execute(&self.pool)
                .await?;
            }
            _ => {
                sqlx::query(
                    "UPDATE sessions SET last_connected_at = NOW(), updated_at = NOW() WHERE id = ?",
                )
                .bind(id)
                .execute(&self.pool)
                .await?;
            }
        }

        Ok(())
    }

    pub async fn delete_session(&self, id: &str) -> anyhow::Result<bool> {
        let result = sqlx::query("DELETE FROM sessions WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
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

        let enabled_val: i32 = if config.enabled { 1 } else { 0 };

        sqlx::query(
            "INSERT INTO webhooks (id, session_id, url, events, secret, enabled) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(session_id)
        .bind(&config.url)
        .bind(&events_str)
        .bind(&config.secret)
        .bind(enabled_val)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    #[allow(dead_code)]
    pub async fn get_webhooks(
        &self,
        session_id: &str,
    ) -> anyhow::Result<Vec<(String, WebhookConfig)>> {
        let rows: Vec<AnyRow> =
            sqlx::query("SELECT * FROM webhooks WHERE session_id = ?")
                .bind(session_id)
                .fetch_all(&self.pool)
                .await?;

        Ok(rows
            .iter()
            .map(|r| {
                let id: String = r.get("id");
                let url: String = r.get("url");
                let events_raw: String = r.get("events");
                let secret: Option<String> = r.try_get("secret").ok().flatten();
                let enabled: bool = r.try_get::<bool, _>("enabled").unwrap_or(true);

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
            })
            .collect())
    }

    pub async fn delete_webhook(&self, id: &str) -> anyhow::Result<bool> {
        let result = sqlx::query("DELETE FROM webhooks WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    fn row_to_session_info(&self, row: &AnyRow) -> SessionInfo {
        let status_str: String = row.get("status");

        // Handle timestamps - different backends store differently
        let created_at = self.get_timestamp(row, "created_at").unwrap_or(0);
        let updated_at = self.get_timestamp(row, "updated_at").unwrap_or(0);
        let last_connected_at = self.get_timestamp(row, "last_connected_at");

        // Handle bool - SQLite uses integer
        let is_logged_in: bool = row.try_get::<bool, _>("is_logged_in")
            .unwrap_or_else(|_| {
                row.try_get::<i32, _>("is_logged_in")
                    .map(|v| v != 0)
                    .unwrap_or(false)
            });

        SessionInfo {
            id: row.get("id"),
            name: row.try_get("name").ok(),
            phone_number: row.try_get("phone_number").ok().flatten(),
            push_name: row.try_get("push_name").ok().flatten(),
            status: SessionStatus::from_str(&status_str),
            created_at,
            updated_at,
            last_connected_at,
            is_logged_in,
        }
    }

    fn get_timestamp(&self, row: &AnyRow, col: &str) -> Option<i64> {
        use sqlx::ValueRef;

        // Check if column is null first
        let raw = row.try_get_raw(col).ok()?;
        if raw.is_null() {
            return None;
        }

        // Try string representation (works for all backends via Display/text)
        if let Ok(s) = row.try_get::<String, _>(col) {
            // Try various datetime formats
            for fmt in &[
                "%Y-%m-%d %H:%M:%S%.f%:z", // Postgres TIMESTAMPTZ
                "%Y-%m-%dT%H:%M:%S%.f%:z", // ISO 8601 with tz
                "%Y-%m-%d %H:%M:%S%.f",    // MySQL/SQLite with fractional
                "%Y-%m-%d %H:%M:%S",        // Basic datetime
                "%Y-%m-%dT%H:%M:%S%.f",    // ISO 8601 no tz
                "%Y-%m-%dT%H:%M:%S",       // ISO 8601 basic
            ] {
                if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(&s, fmt) {
                    return Some(dt.and_utc().timestamp());
                }
                if let Ok(dt) = chrono::DateTime::parse_from_str(&s, fmt) {
                    return Some(dt.timestamp());
                }
            }
            return None;
        }

        // Try i64 directly (epoch)
        if let Ok(ts) = row.try_get::<i64, _>(col) {
            return Some(ts);
        }

        None
    }
}
