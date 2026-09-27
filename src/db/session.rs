use crate::db::sqlite_raw::{self, SqliteHandle, Value as SQ};
use crate::models::sessions::{SessionInfo, SessionStatus};
use crate::models::webhooks::{WebhookConfig, WebhookEvent};
use chrono::{DateTime, Utc};
use deadpool_postgres::Pool as PgPool;
use mysql_async::prelude::*;
use mysql_async::Pool as MyPool;

pub type SqlitePool = SqliteHandle;

/// Cloud API credentials for a `whatsapp_cloud` session, read internally by
/// [`crate::cloud::client::CloudClient`] and the webhook verifier. Never
/// serialized into an HTTP response.
#[derive(Debug, Clone)]
pub struct CloudCredentials {
    pub waba_id: String,
    pub phone_number_id: String,
    pub access_token: String,
    pub app_secret: String,
    pub webhook_verify_token: String,
    /// PEM-encoded RSA private key used to unwrap the per-request AES
    /// key in a Flows Data Exchange payload (see
    /// [`crate::cloud::flows_crypto`]). `None` until
    /// [`SessionManager::set_flow_endpoint`] has been called.
    pub flow_private_key: Option<String>,
    /// Where a decrypted Flows Data Exchange request is forwarded so the
    /// business's own backend can pick the next screen. `None` means only
    /// Meta's health-check ping is answered.
    pub flow_forward_url: Option<String>,
}

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
                        "SELECT id, name, phone_number, push_name, status, is_logged_in, created_at, updated_at, last_connected_at, provider, cloud_waba_id, cloud_phone_number_id, cloud_business_id, cloud_app_id FROM sessions WHERE id = ?",
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
                    "SELECT id, name, phone_number, push_name, status, is_logged_in, created_at, updated_at, last_connected_at, provider, cloud_waba_id, cloud_phone_number_id, cloud_business_id, cloud_app_id FROM sessions ORDER BY created_at DESC",
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

    /// Attaches Cloud API credentials to a session and flips its
    /// `provider` to `whatsapp_cloud`. Secrets (`access_token`,
    /// `app_secret`, `webhook_verify_token`) land in the DB here but are
    /// never read back through [`SessionInfo`] -- fetch them via
    /// [`Self::get_cloud_credentials`] instead.
    pub async fn connect_cloud(
        &self,
        id: &str,
        req: &crate::models::cloud::ConnectCloudRequest,
    ) -> anyhow::Result<()> {
        match &self.pool {
            DbPool::Postgres(pool) => {
                let client = pool.get().await?;
                client
                    .execute(
                        "UPDATE sessions SET provider = 'whatsapp_cloud', cloud_waba_id = $1, cloud_phone_number_id = $2, cloud_business_id = $3, cloud_access_token = $4, cloud_app_id = $5, cloud_app_secret = $6, cloud_webhook_verify_token = $7, updated_at = NOW() WHERE id = $8",
                        &[
                            &req.waba_id,
                            &req.phone_number_id,
                            &req.business_id,
                            &req.access_token,
                            &req.app_id,
                            &req.app_secret,
                            &req.webhook_verify_token,
                            &id,
                        ],
                    )
                    .await?;
            }
            DbPool::MySQL(pool) => {
                let now = now_str();
                let mut conn = pool.get_conn().await?;
                conn.exec_drop(
                    "UPDATE sessions SET provider = 'whatsapp_cloud', cloud_waba_id = ?, cloud_phone_number_id = ?, cloud_business_id = ?, cloud_access_token = ?, cloud_app_id = ?, cloud_app_secret = ?, cloud_webhook_verify_token = ?, updated_at = ? WHERE id = ?",
                    (
                        &req.waba_id,
                        &req.phone_number_id,
                        &req.business_id,
                        &req.access_token,
                        &req.app_id,
                        &req.app_secret,
                        &req.webhook_verify_token,
                        &now,
                        id,
                    ),
                )
                .await?;
            }
            DbPool::SQLite(pool) => {
                let id_s = id.to_string();
                let waba_id = req.waba_id.clone();
                let phone_number_id = req.phone_number_id.clone();
                let business_id = SQ::from_opt_str(req.business_id.as_deref());
                let access_token = req.access_token.clone();
                let app_id = SQ::from_opt_str(req.app_id.as_deref());
                let app_secret = req.app_secret.clone();
                let webhook_verify_token = req.webhook_verify_token.clone();
                let now = now_str();
                sqlite_blocking(pool, move |conn| {
                    sqlite_raw::execute(
                        conn,
                        "UPDATE sessions SET provider = 'whatsapp_cloud', cloud_waba_id = ?, cloud_phone_number_id = ?, cloud_business_id = ?, cloud_access_token = ?, cloud_app_id = ?, cloud_app_secret = ?, cloud_webhook_verify_token = ?, updated_at = ? WHERE id = ?",
                        &[
                            SQ::Text(waba_id),
                            SQ::Text(phone_number_id),
                            business_id,
                            SQ::Text(access_token),
                            app_id,
                            SQ::Text(app_secret),
                            SQ::Text(webhook_verify_token),
                            SQ::Text(now),
                            SQ::Text(id_s),
                        ],
                    )?;
                    Ok(())
                })
                .await?;
            }
        }
        Ok(())
    }

    /// Stores the RSA private key used to unwrap a Flows Data Exchange
    /// AES key (see [`crate::cloud::flows_crypto`]) and the URL decrypted
    /// requests are forwarded to. Never read back through [`SessionInfo`]
    /// -- only via [`Self::get_cloud_credentials`] for internal use by the
    /// data-exchange handler.
    pub async fn set_flow_endpoint(
        &self,
        id: &str,
        private_key_pem: &str,
        forward_url: Option<&str>,
    ) -> anyhow::Result<()> {
        match &self.pool {
            DbPool::Postgres(pool) => {
                let client = pool.get().await?;
                client
                    .execute(
                        "UPDATE sessions SET cloud_flow_private_key = $1, cloud_flow_forward_url = $2, updated_at = NOW() WHERE id = $3",
                        &[&private_key_pem, &forward_url, &id],
                    )
                    .await?;
            }
            DbPool::MySQL(pool) => {
                let now = now_str();
                let mut conn = pool.get_conn().await?;
                conn.exec_drop(
                    "UPDATE sessions SET cloud_flow_private_key = ?, cloud_flow_forward_url = ?, updated_at = ? WHERE id = ?",
                    (private_key_pem, forward_url, &now, id),
                )
                .await?;
            }
            DbPool::SQLite(pool) => {
                let id_s = id.to_string();
                let key = private_key_pem.to_string();
                let forward = match forward_url {
                    Some(url) => SQ::Text(url.to_string()),
                    None => SQ::Null,
                };
                let now = now_str();
                sqlite_blocking(pool, move |conn| {
                    sqlite_raw::execute(
                        conn,
                        "UPDATE sessions SET cloud_flow_private_key = ?, cloud_flow_forward_url = ?, updated_at = ? WHERE id = ?",
                        &[SQ::Text(key), forward, SQ::Text(now), SQ::Text(id_s)],
                    )?;
                    Ok(())
                })
                .await?;
            }
        }
        Ok(())
    }

    /// Reads back the Cloud API credentials stored by
    /// [`Self::connect_cloud`], for internal use by
    /// [`crate::cloud::client::CloudClient`] and the webhook verifier.
    /// Never exposed through the HTTP API.
    pub async fn get_cloud_credentials(
        &self,
        id: &str,
    ) -> anyhow::Result<Option<CloudCredentials>> {
        match &self.pool {
            DbPool::Postgres(pool) => {
                let client = pool.get().await?;
                let row = client
                    .query_opt(
                        "SELECT provider, cloud_waba_id, cloud_phone_number_id, cloud_access_token, cloud_app_secret, cloud_webhook_verify_token, cloud_flow_private_key, cloud_flow_forward_url FROM sessions WHERE id = $1",
                        &[&id],
                    )
                    .await?;
                Ok(row.and_then(|r| {
                    let provider: String = r.get("provider");
                    (provider == "whatsapp_cloud").then(|| CloudCredentials {
                        waba_id: r.get("cloud_waba_id"),
                        phone_number_id: r.get("cloud_phone_number_id"),
                        access_token: r.get("cloud_access_token"),
                        app_secret: r.get("cloud_app_secret"),
                        webhook_verify_token: r.get("cloud_webhook_verify_token"),
                        flow_private_key: r.get("cloud_flow_private_key"),
                        flow_forward_url: r.get("cloud_flow_forward_url"),
                    })
                }))
            }
            DbPool::MySQL(pool) => {
                let mut conn = pool.get_conn().await?;
                let row: Option<mysql_async::Row> = conn
                    .exec_first(
                        "SELECT provider, cloud_waba_id, cloud_phone_number_id, cloud_access_token, cloud_app_secret, cloud_webhook_verify_token, cloud_flow_private_key, cloud_flow_forward_url FROM sessions WHERE id = ?",
                        (id,),
                    )
                    .await?;
                Ok(row.and_then(|r| {
                    let provider = my_get_string(&r, "provider").unwrap_or_default();
                    (provider == "whatsapp_cloud").then(|| CloudCredentials {
                        waba_id: my_get_string(&r, "cloud_waba_id").unwrap_or_default(),
                        phone_number_id: my_get_string(&r, "cloud_phone_number_id")
                            .unwrap_or_default(),
                        access_token: my_get_string(&r, "cloud_access_token").unwrap_or_default(),
                        app_secret: my_get_string(&r, "cloud_app_secret").unwrap_or_default(),
                        webhook_verify_token: my_get_string(&r, "cloud_webhook_verify_token")
                            .unwrap_or_default(),
                        flow_private_key: my_get_string(&r, "cloud_flow_private_key"),
                        flow_forward_url: my_get_string(&r, "cloud_flow_forward_url"),
                    })
                }))
            }
            DbPool::SQLite(pool) => {
                let id_s = id.to_string();
                sqlite_blocking(pool, move |conn| {
                    let mut out = sqlite_raw::query(
                        conn,
                        "SELECT provider, cloud_waba_id, cloud_phone_number_id, cloud_access_token, cloud_app_secret, cloud_webhook_verify_token, cloud_flow_private_key, cloud_flow_forward_url FROM sessions WHERE id = ?",
                        &[SQ::Text(id_s)],
                        |row| {
                            let provider = row.get_string(0).unwrap_or_default();
                            (provider == "whatsapp_cloud").then(|| CloudCredentials {
                                waba_id: row.get_string(1).unwrap_or_default(),
                                phone_number_id: row.get_string(2).unwrap_or_default(),
                                access_token: row.get_string(3).unwrap_or_default(),
                                app_secret: row.get_string(4).unwrap_or_default(),
                                webhook_verify_token: row.get_string(5).unwrap_or_default(),
                                flow_private_key: row.get_string(6),
                                flow_forward_url: row.get_string(7),
                            })
                        },
                    )?;
                    Ok(out.pop().flatten())
                })
                .await
            }
        }
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
        self.purge_dependents(id).await?;
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

    /// Explicitly purge every row that points at this session before the
    /// `sessions` row itself is deleted. `ON DELETE CASCADE` on the FKs
    /// covers this in the fresh schema, but production tables migrated
    /// from earlier releases are missing the cascade — those tables
    /// leave orphan webhooks (127.0.0.1:3452 stayed around for months
    /// after the owning session was gone). Doing the delete here makes
    /// the behaviour uniform across every backend, cascade or not.
    async fn purge_dependents(&self, session_id: &str) -> anyhow::Result<()> {
        match &self.pool {
            DbPool::Postgres(pool) => {
                let client = pool.get().await?;
                client
                    .execute("DELETE FROM webhooks WHERE session_id = $1", &[&session_id])
                    .await?;
                client
                    .execute("DELETE FROM contacts WHERE session_id = $1", &[&session_id])
                    .await?;
                client
                    .execute(
                        "DELETE FROM webhook_dlq WHERE session_id = $1",
                        &[&session_id],
                    )
                    .await?;
            }
            DbPool::MySQL(pool) => {
                let mut conn = pool.get_conn().await?;
                conn.exec_drop("DELETE FROM webhooks WHERE session_id = ?", (session_id,))
                    .await?;
                conn.exec_drop("DELETE FROM contacts WHERE session_id = ?", (session_id,))
                    .await?;
                conn.exec_drop(
                    "DELETE FROM webhook_dlq WHERE session_id = ?",
                    (session_id,),
                )
                .await?;
            }
            DbPool::SQLite(pool) => {
                let sid = session_id.to_string();
                sqlite_blocking(pool, move |conn| {
                    sqlite_raw::execute(
                        conn,
                        "DELETE FROM webhooks WHERE session_id = ?",
                        &[SQ::Text(sid.clone())],
                    )?;
                    sqlite_raw::execute(
                        conn,
                        "DELETE FROM contacts WHERE session_id = ?",
                        &[SQ::Text(sid.clone())],
                    )?;
                    sqlite_raw::execute(
                        conn,
                        "DELETE FROM webhook_dlq WHERE session_id = ?",
                        &[SQ::Text(sid)],
                    )?;
                    Ok(())
                })
                .await?;
            }
        }
        Ok(())
    }

    /// Persist the "disabled after N consecutive failures" decision. Sets
    /// `enabled=false` plus optional `disabled_at` / `disabled_reason`
    /// columns (added in 0.6.12) so the operator sees WHY a target got
    /// muted, not just that it stopped receiving events.
    pub async fn disable_webhook_by_url(&self, url: &str, reason: &str) -> anyhow::Result<u64> {
        match &self.pool {
            DbPool::Postgres(pool) => {
                let client = pool.get().await?;
                let n = client
                    .execute(
                        "UPDATE webhooks SET enabled=false, disabled_at=NOW(), disabled_reason=$2 WHERE url=$1 AND enabled=true",
                        &[&url, &reason],
                    )
                    .await?;
                Ok(n)
            }
            DbPool::MySQL(pool) => {
                let mut conn = pool.get_conn().await?;
                let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
                conn.exec_drop(
                    "UPDATE webhooks SET enabled=0, disabled_at=?, disabled_reason=? WHERE url=? AND enabled=1",
                    (now, reason, url),
                )
                .await?;
                Ok(conn.affected_rows())
            }
            DbPool::SQLite(pool) => {
                let url_s = url.to_string();
                let reason_s = reason.to_string();
                let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
                let n = sqlite_blocking(pool, move |conn| {
                    sqlite_raw::execute(
                        conn,
                        "UPDATE webhooks SET enabled=0, disabled_at=?, disabled_reason=? WHERE url=? AND enabled=1",
                        &[SQ::Text(now), SQ::Text(reason_s), SQ::Text(url_s)],
                    )
                })
                .await?;
                Ok(n)
            }
        }
    }

    /// Flip a specific webhook row back to enabled and clear the diagnostic
    /// columns. Used by the manual re-enable REST endpoint.
    pub async fn enable_webhook(&self, id: &str) -> anyhow::Result<bool> {
        match &self.pool {
            DbPool::Postgres(pool) => {
                let client = pool.get().await?;
                let n = client
                    .execute(
                        "UPDATE webhooks SET enabled=true, disabled_at=NULL, disabled_reason=NULL WHERE id=$1",
                        &[&id],
                    )
                    .await?;
                Ok(n > 0)
            }
            DbPool::MySQL(pool) => {
                let mut conn = pool.get_conn().await?;
                conn.exec_drop(
                    "UPDATE webhooks SET enabled=1, disabled_at=NULL, disabled_reason=NULL WHERE id=?",
                    (id,),
                )
                .await?;
                Ok(conn.affected_rows() > 0)
            }
            DbPool::SQLite(pool) => {
                let id_s = id.to_string();
                let n = sqlite_blocking(pool, move |conn| {
                    sqlite_raw::execute(
                        conn,
                        "UPDATE webhooks SET enabled=1, disabled_at=NULL, disabled_reason=NULL WHERE id=?",
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
        provider: row.get("provider"),
        cloud_waba_id: row.get("cloud_waba_id"),
        cloud_phone_number_id: row.get("cloud_phone_number_id"),
        cloud_business_id: row.get("cloud_business_id"),
        cloud_app_id: row.get("cloud_app_id"),
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
        provider: my_get_string(row, "provider").unwrap_or_else(|| "whatsapp_web".to_string()),
        cloud_waba_id: my_get_string(row, "cloud_waba_id"),
        cloud_phone_number_id: my_get_string(row, "cloud_phone_number_id"),
        cloud_business_id: my_get_string(row, "cloud_business_id"),
        cloud_app_id: my_get_string(row, "cloud_app_id"),
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
        provider: row
            .get_string(9)
            .unwrap_or_else(|| "whatsapp_web".to_string()),
        cloud_waba_id: row.get_string(10),
        cloud_phone_number_id: row.get_string(11),
        cloud_business_id: row.get_string(12),
        cloud_app_id: row.get_string(13),
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
