use chrono::{DateTime, Utc};
use deadpool_postgres::Pool;

use crate::models::sessions::{SessionInfo, SessionStatus};
use crate::models::webhooks::{WebhookConfig, WebhookEvent};

#[derive(Clone)]
pub struct SessionManager {
    pool: Pool,
}

impl SessionManager {
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }

    pub async fn create_session(
        &self,
        id: &str,
        name: Option<&str>,
        storage_path: &str,
    ) -> anyhow::Result<SessionInfo> {
        let client = self.pool.get().await?;

        let row = client
            .query_one(
                r#"
                INSERT INTO sessions (id, name, storage_path, status, is_logged_in)
                VALUES ($1, $2, $3, 'disconnected', FALSE)
                RETURNING id, name, storage_path, phone_number, push_name, status, is_logged_in, created_at, updated_at, last_connected_at
                "#,
                &[&id, &name, &storage_path],
            )
            .await?;

        Ok(self.row_to_session_info(&row))
    }

    pub async fn get_session(&self, id: &str) -> anyhow::Result<Option<SessionInfo>> {
        let client = self.pool.get().await?;

        let row = client
            .query_opt("SELECT * FROM sessions WHERE id = $1", &[&id])
            .await?;

        Ok(row.map(|r| self.row_to_session_info(&r)))
    }

    pub async fn get_storage_path(&self, id: &str) -> anyhow::Result<Option<String>> {
        let client = self.pool.get().await?;

        let row = client
            .query_opt("SELECT storage_path FROM sessions WHERE id = $1", &[&id])
            .await?;

        Ok(row.map(|r| r.get::<_, String>("storage_path")))
    }

    pub async fn list_sessions(&self) -> anyhow::Result<Vec<SessionInfo>> {
        let client = self.pool.get().await?;

        let rows = client
            .query("SELECT * FROM sessions ORDER BY created_at DESC", &[])
            .await?;

        Ok(rows.iter().map(|r| self.row_to_session_info(r)).collect())
    }

    pub async fn update_session_status(
        &self,
        id: &str,
        status: SessionStatus,
        is_logged_in: bool,
    ) -> anyhow::Result<()> {
        let client = self.pool.get().await?;

        client
            .execute(
                r#"
                UPDATE sessions
                SET status = $1, is_logged_in = $2, updated_at = NOW()
                WHERE id = $3
                "#,
                &[&status.as_str(), &is_logged_in, &id],
            )
            .await?;

        Ok(())
    }

    pub async fn update_session_info(
        &self,
        id: &str,
        phone_number: Option<&str>,
        push_name: Option<&str>,
    ) -> anyhow::Result<()> {
        let client = self.pool.get().await?;

        client
            .execute(
                r#"
                UPDATE sessions
                SET phone_number = COALESCE($1, phone_number),
                    push_name = COALESCE($2, push_name),
                    updated_at = NOW()
                WHERE id = $3
                "#,
                &[&phone_number, &push_name, &id],
            )
            .await?;

        Ok(())
    }

    pub async fn update_last_connected(&self, id: &str) -> anyhow::Result<()> {
        let client = self.pool.get().await?;

        client
            .execute(
                r#"
                UPDATE sessions
                SET last_connected_at = NOW(), updated_at = NOW()
                WHERE id = $1
                "#,
                &[&id],
            )
            .await?;

        Ok(())
    }

    pub async fn delete_session(&self, id: &str) -> anyhow::Result<bool> {
        let client = self.pool.get().await?;

        let result = client
            .execute("DELETE FROM sessions WHERE id = $1", &[&id])
            .await?;

        Ok(result > 0)
    }

    pub async fn create_webhook(
        &self,
        id: &str,
        session_id: &str,
        config: &WebhookConfig,
    ) -> anyhow::Result<()> {
        let client = self.pool.get().await?;

        let events: Vec<String> = config
            .events
            .iter()
            .map(|e| e.as_str().to_string())
            .collect();

        client
            .execute(
                r#"
                INSERT INTO webhooks (id, session_id, url, events, secret, enabled)
                VALUES ($1, $2, $3, $4, $5, $6)
                "#,
                &[
                    &id,
                    &session_id,
                    &config.url,
                    &events,
                    &config.secret,
                    &config.enabled,
                ],
            )
            .await?;

        Ok(())
    }

    #[allow(dead_code)]
    pub async fn get_webhooks(
        &self,
        session_id: &str,
    ) -> anyhow::Result<Vec<(String, WebhookConfig)>> {
        let client = self.pool.get().await?;

        let rows = client
            .query(
                "SELECT * FROM webhooks WHERE session_id = $1",
                &[&session_id],
            )
            .await?;

        Ok(rows
            .iter()
            .map(|r| {
                let id: String = r.get("id");
                let url: String = r.get("url");
                let events_raw: Vec<String> = r.get("events");
                let secret: Option<String> = r.get("secret");
                let enabled: bool = r.get("enabled");

                let events: Vec<WebhookEvent> = events_raw
                    .iter()
                    .filter_map(|s| WebhookEvent::from_str(s))
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
        let client = self.pool.get().await?;

        let result = client
            .execute("DELETE FROM webhooks WHERE id = $1", &[&id])
            .await?;

        Ok(result > 0)
    }

    fn row_to_session_info(&self, row: &tokio_postgres::Row) -> SessionInfo {
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
}
