use crate::db::sqlite_raw::{self, SqliteHandle, Value as SQ};
use crate::models::sessions::{SessionInfo, SessionStatus};
use crate::models::webhooks::{WebhookConfig, WebhookEvent};
use chrono::{DateTime, Utc};
use deadpool_postgres::Pool as PgPool;
use mysql_async::prelude::*;
use mysql_async::Pool as MyPool;

pub type SqlitePool = SqliteHandle;

#[derive(Clone)]
pub enum DbPool {
    Postgres(PgPool),
    MySQL(MyPool),
    SQLite(SqlitePool),
}

/// Run a synchronous SQLite block on the blocking thread pool. The closure
/// receives an exclusive guard on the connection.
pub async fn sqlite_blocking<F, T>(handle: &SqliteHandle, f: F) -> anyhow::Result<T>
where
    F: FnOnce(&sqlite_raw::Connection) -> anyhow::Result<T> + Send + 'static,
    T: Send + 'static,
{
    let handle = handle.clone();
    let res = tokio::task::spawn_blocking(move || -> anyhow::Result<T> {
        let guard = handle.lock();
        f(&guard)
    })
    .await??;
    Ok(res)
}

fn now_str() -> String {
    chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

#[derive(Clone)]
pub struct SessionManager {
    pool: DbPool,
}

impl SessionManager {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &DbPool {
        &self.pool
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
                let now = now_str();
                let mut conn = pool.get_conn().await?;
                conn.exec_drop(
                    "INSERT INTO sessions (id, name, storage_path, status, is_logged_in, created_at, updated_at) VALUES (?, ?, ?, 'disconnected', 0, ?, ?)",
                    (id, name_str, storage_path, &now, &now),
                ).await?;
                drop(conn);
                let session = self.get_session(id).await?;
                session.ok_or_else(|| anyhow::anyhow!("Failed to fetch created session"))
            }
            DbPool::SQLite(pool) => {
                let (id_s, name_s, sp_s) = (
                    id.to_string(),
                    name_str.to_string(),
                    storage_path.to_string(),
                );
                let now = now_str();
                let now2 = now.clone();
                sqlite_blocking(pool, move |conn| {
                    sqlite_raw::execute(
                        conn,
                        "INSERT INTO sessions (id, name, storage_path, status, is_logged_in, created_at, updated_at) VALUES (?, ?, ?, 'disconnected', 0, ?, ?)",
                        &[SQ::Text(id_s), SQ::Text(name_s), SQ::Text(sp_s), SQ::Text(now), SQ::Text(now2)],
                    )?;
                    Ok(())
                }).await?;
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
            DbPool::SQLite(pool) => {
                let id_s = id.to_string();
                sqlite_blocking(pool, move |conn| {
                    let mut out = sqlite_raw::query(
                        conn,
                        "SELECT id, name, phone_number, push_name, status, is_logged_in, created_at, updated_at, last_connected_at FROM sessions WHERE id = ?",
                        &[SQ::Text(id_s)],
                        sqlite_row_to_session,
                    )?;
                    Ok(out.pop())
                }).await
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
            DbPool::SQLite(pool) => {
                let id_s = id.to_string();
                sqlite_blocking(pool, move |conn| {
                    let mut out = sqlite_raw::query(
                        conn,
                        "SELECT storage_path FROM sessions WHERE id = ?",
                        &[SQ::Text(id_s)],
                        |r| r.get_string(0).unwrap_or_default(),
                    )?;
                    Ok(out.pop())
                })
                .await
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
            DbPool::SQLite(pool) => sqlite_blocking(pool, |conn| {
                sqlite_raw::query(
                    conn,
                    "SELECT id, name, phone_number, push_name, status, is_logged_in, created_at, updated_at, last_connected_at FROM sessions ORDER BY created_at DESC",
                    &[],
                    sqlite_row_to_session,
                )
            })
            .await,
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
                let now = now_str();
                let mut conn = pool.get_conn().await?;
                conn.exec_drop(
                    "UPDATE sessions SET status = ?, is_logged_in = ?, updated_at = ? WHERE id = ?",
                    (status.as_str(), logged_in, &now, id),
                )
                .await?;
            }
            DbPool::SQLite(pool) => {
                let id_s = id.to_string();
                let status_s = status.as_str().to_string();
                let now = now_str();
                let logged: i64 = if is_logged_in { 1 } else { 0 };
                sqlite_blocking(pool, move |conn| {
                    sqlite_raw::execute(
                        conn,
                        "UPDATE sessions SET status = ?, is_logged_in = ?, updated_at = ? WHERE id = ?",
                        &[SQ::Text(status_s), SQ::Int(logged), SQ::Text(now), SQ::Text(id_s)],
                    )?;
                    Ok(())
                })
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
                let now = now_str();
                let mut conn = pool.get_conn().await?;
                conn.exec_drop(
                    "UPDATE sessions SET phone_number = COALESCE(?, phone_number), push_name = COALESCE(?, push_name), updated_at = ? WHERE id = ?",
                    (phone_number, push_name, &now, id),
                )
                .await?;
            }
            DbPool::SQLite(pool) => {
                let id_s = id.to_string();
                let pv = SQ::from_opt_str(phone_number);
                let nv = SQ::from_opt_str(push_name);
                let now = now_str();
                sqlite_blocking(pool, move |conn| {
                    sqlite_raw::execute(
                        conn,
                        "UPDATE sessions SET phone_number = COALESCE(?, phone_number), push_name = COALESCE(?, push_name), updated_at = ? WHERE id = ?",
                        &[pv, nv, SQ::Text(now), SQ::Text(id_s)],
                    )?;
                    Ok(())
                })
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
                let now = now_str();
                let mut conn = pool.get_conn().await?;
                conn.exec_drop(
                    "UPDATE sessions SET last_connected_at = ?, updated_at = ? WHERE id = ?",
                    (&now, &now, id),
                )
                .await?;
            }
            DbPool::SQLite(pool) => {
                let id_s = id.to_string();
                let now = now_str();
                let now2 = now.clone();
                sqlite_blocking(pool, move |conn| {
                    sqlite_raw::execute(
                        conn,
                        "UPDATE sessions SET last_connected_at = ?, updated_at = ? WHERE id = ?",
                        &[SQ::Text(now), SQ::Text(now2), SQ::Text(id_s)],
                    )?;
                    Ok(())
                })
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
            DbPool::SQLite(pool) => {
                let id_s = id.to_string();
                let n = sqlite_blocking(pool, move |conn| {
                    sqlite_raw::execute(
                        conn,
                        "DELETE FROM sessions WHERE id = ?",
                        &[SQ::Text(id_s)],
                    )
                })
                .await?;
                Ok(n > 0)
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
                let now = now_str();
                let mut conn = pool.get_conn().await?;
                conn.exec_drop(
                    "INSERT INTO webhooks (id, session_id, url, events, secret, enabled, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
                    (id, session_id, &config.url, &events_str, &config.secret, enabled, &now),
                )
                .await?;
            }
            DbPool::SQLite(pool) => {
                let id_s = id.to_string();
                let sid = session_id.to_string();
                let url = config.url.clone();
                let secret_v = match config.secret.as_deref() {
                    Some(x) => SQ::Text(x.to_string()),
                    None => SQ::Null,
                };
                let enabled: i64 = if config.enabled { 1 } else { 0 };
                let evs = events_str.clone();
                let now = now_str();
                sqlite_blocking(pool, move |conn| {
                    sqlite_raw::execute(
                        conn,
                        "INSERT INTO webhooks (id, session_id, url, events, secret, enabled, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
                        &[SQ::Text(id_s), SQ::Text(sid), SQ::Text(url), SQ::Text(evs), secret_v, SQ::Int(enabled), SQ::Text(now)],
                    )?;
                    Ok(())
                })
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
            DbPool::SQLite(pool) => {
                let sid = session_id.to_string();
                sqlite_blocking(pool, move |conn| {
                    sqlite_raw::query(
                        conn,
                        "SELECT id, url, events, secret, enabled FROM webhooks WHERE session_id = ?",
                        &[SQ::Text(sid)],
                        sqlite_row_to_webhook,
                    )
                })
                .await
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
            DbPool::SQLite(pool) => {
                let id_s = id.to_string();
                let n = sqlite_blocking(pool, move |conn| {
                    sqlite_raw::execute(
                        conn,
                        "DELETE FROM webhooks WHERE id = ?",
                        &[SQ::Text(id_s)],
                    )
                })
                .await?;
                Ok(n > 0)
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

fn my_get_string(row: &mysql_async::Row, col: &str) -> Option<String> {
    use mysql_async::Value;
    let idx = row.columns_ref().iter().position(|c| c.name_str() == col)?;
    match row.as_ref(idx)? {
        Value::NULL => None,
        Value::Bytes(b) => Some(String::from_utf8_lossy(b).to_string()),
        v => Some(format!("{:?}", v)),
    }
}

fn my_get_int(row: &mysql_async::Row, col: &str) -> i32 {
    use mysql_async::Value;
    let idx = match row.columns_ref().iter().position(|c| c.name_str() == col) {
        Some(i) => i,
        None => return 0,
    };
    match row.as_ref(idx) {
        Some(Value::Int(i)) => *i as i32,
        Some(Value::UInt(u)) => *u as i32,
        _ => 0,
    }
}

fn my_row_to_session(row: &mysql_async::Row) -> SessionInfo {
    let status_str = my_get_string(row, "status").unwrap_or_else(|| "disconnected".to_string());
    let is_logged_in = my_get_int(row, "is_logged_in");

    let created_at = my_get_string(row, "created_at");
    let updated_at = my_get_string(row, "updated_at");
    let last_connected_at = my_get_string(row, "last_connected_at");

    SessionInfo {
        id: my_get_string(row, "id").unwrap_or_default(),
        name: my_get_string(row, "name"),
        phone_number: my_get_string(row, "phone_number"),
        push_name: my_get_string(row, "push_name"),
        status: SessionStatus::from_str(&status_str),
        created_at: parse_mysql_timestamp(created_at.as_deref()).unwrap_or(0),
        updated_at: parse_mysql_timestamp(updated_at.as_deref()).unwrap_or(0),
        last_connected_at: parse_mysql_timestamp(last_connected_at.as_deref()),
        is_logged_in: is_logged_in != 0,
    }
}

fn my_row_to_webhook(row: &mysql_async::Row) -> (String, WebhookConfig) {
    let id = my_get_string(row, "id").unwrap_or_default();
    let url = my_get_string(row, "url").unwrap_or_default();
    let events_raw = my_get_string(row, "events").unwrap_or_default();
    let secret = my_get_string(row, "secret");
    let enabled = my_get_int(row, "enabled");

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

// === SQLite row mapping ===

fn sqlite_row_to_session(row: &sqlite_raw::Row) -> SessionInfo {
    let status_str = row
        .get_string(4)
        .unwrap_or_else(|| "disconnected".to_string());
    let logged = row.get_int(5);
    let created_at = row.get_string(6).unwrap_or_default();
    let updated_at = row.get_string(7).unwrap_or_default();
    let last_connected_at = row.get_string(8);
    SessionInfo {
        id: row.get_string(0).unwrap_or_default(),
        name: row.get_string(1),
        phone_number: row.get_string(2),
        push_name: row.get_string(3),
        status: SessionStatus::from_str(&status_str),
        created_at: parse_mysql_timestamp(Some(&created_at)).unwrap_or(0),
        updated_at: parse_mysql_timestamp(Some(&updated_at)).unwrap_or(0),
        last_connected_at: last_connected_at
            .as_deref()
            .and_then(|s| parse_mysql_timestamp(Some(s))),
        is_logged_in: logged != 0,
    }
}

fn sqlite_row_to_webhook(row: &sqlite_raw::Row) -> (String, WebhookConfig) {
    let id = row.get_string(0).unwrap_or_default();
    let url = row.get_string(1).unwrap_or_default();
    let events_raw = row.get_string(2).unwrap_or_default();
    let secret = row.get_string(3);
    let enabled = row.get_int(4);
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
