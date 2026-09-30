//! Persistence for session-scoped bearer tokens (`tokens` +
//! `token_sessions` + `token_session_prefixes`). A minted token's JWT carries only a `jti`; whether
//! it's still valid and which sessions it may touch both live here, looked
//! up on every authenticated request in [`crate::middleware::jwt`] --
//! that's what makes revocation and rebinding possible without reissuing
//! the token itself.

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::db::session::{sqlite_blocking, DbPool};
use crate::db::sqlite_raw::{self, Value as SQ};

#[derive(Debug, Clone, Serialize)]
pub struct TokenRecord {
    pub id: String,
    pub name: Option<String>,
    pub session_ids: Vec<String>,
    pub session_prefixes: Vec<String>,
    pub created_at: String,
    pub expires_at: Option<String>,
    pub revoked_at: Option<String>,
}

/// Just enough to answer "is this token still usable, and for which
/// sessions" -- the hot-path query run once per authenticated request for
/// any token that carries a `jti`.
#[derive(Debug, Clone, Default)]
pub struct TokenStatus {
    pub revoked: bool,
    pub session_ids: Vec<String>,
    /// Session-id prefixes this token may use (and create sessions under).
    pub session_prefixes: Vec<String>,
}

impl TokenStatus {
    /// Whether `session_id` is inside this token's scope: listed exactly,
    /// or starting with one of its prefixes.
    pub fn covers(&self, session_id: &str) -> bool {
        self.session_ids.iter().any(|s| s == session_id)
            || self
                .session_prefixes
                .iter()
                .any(|p| session_id.starts_with(p.as_str()))
    }
}

pub struct TokenStore<'a> {
    pool: &'a DbPool,
}

impl<'a> TokenStore<'a> {
    pub fn new(pool: &'a DbPool) -> Self {
        Self { pool }
    }

    pub async fn create(
        &self,
        id: &str,
        name: Option<&str>,
        expires_at: Option<DateTime<Utc>>,
        session_ids: &[String],
        session_prefixes: &[String],
    ) -> anyhow::Result<()> {
        match self.pool {
            DbPool::Postgres(pg) => {
                let client = pg.get().await?;
                client
                    .execute(
                        "INSERT INTO tokens (id, name, expires_at) VALUES ($1, $2, $3)",
                        &[&id, &name, &expires_at],
                    )
                    .await?;
                for sid in session_ids {
                    client
                        .execute(
                            "INSERT INTO token_sessions (token_id, session_id) VALUES ($1, $2)",
                            &[&id, sid],
                        )
                        .await?;
                }
                for prefix in session_prefixes {
                    client
                        .execute(
                            "INSERT INTO token_session_prefixes (token_id, prefix) VALUES ($1, $2)",
                            &[&id, prefix],
                        )
                        .await?;
                }
            }
            DbPool::MySQL(my) => {
                use mysql_async::prelude::*;
                let mut conn = my.get_conn().await?;
                let expires_str = expires_at.map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string());
                conn.exec_drop(
                    "INSERT INTO tokens (id, name, expires_at) VALUES (?, ?, ?)",
                    (id, name, &expires_str),
                )
                .await?;
                for sid in session_ids {
                    conn.exec_drop(
                        "INSERT INTO token_sessions (token_id, session_id) VALUES (?, ?)",
                        (id, sid),
                    )
                    .await?;
                }
                for prefix in session_prefixes {
                    conn.exec_drop(
                        "INSERT INTO token_session_prefixes (token_id, prefix) VALUES (?, ?)",
                        (id, prefix),
                    )
                    .await?;
                }
            }
            DbPool::SQLite(pool) => {
                let id = id.to_string();
                let name = SQ::from_opt_str(name);
                let expires_str = expires_at.map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string());
                let session_ids = session_ids.to_vec();
                let session_prefixes = session_prefixes.to_vec();
                sqlite_blocking(pool, move |conn| {
                    sqlite_raw::execute(
                        conn,
                        "INSERT INTO tokens (id, name, expires_at) VALUES (?, ?, ?)",
                        &[
                            SQ::Text(id.clone()),
                            name,
                            SQ::from_opt_str(expires_str.as_deref()),
                        ],
                    )?;
                    for sid in &session_ids {
                        sqlite_raw::execute(
                            conn,
                            "INSERT INTO token_sessions (token_id, session_id) VALUES (?, ?)",
                            &[SQ::Text(id.clone()), SQ::Text(sid.clone())],
                        )?;
                    }
                    for prefix in &session_prefixes {
                        sqlite_raw::execute(
                            conn,
                            "INSERT INTO token_session_prefixes (token_id, prefix) VALUES (?, ?)",
                            &[SQ::Text(id.clone()), SQ::Text(prefix.clone())],
                        )?;
                    }
                    Ok(())
                })
                .await?;
            }
        }
        Ok(())
    }

    pub async fn status(&self, id: &str) -> anyhow::Result<Option<TokenStatus>> {
        match self.pool {
            DbPool::Postgres(pg) => {
                let client = pg.get().await?;
                let row = client
                    .query_opt("SELECT revoked_at FROM tokens WHERE id = $1", &[&id])
                    .await?;
                let Some(row) = row else {
                    return Ok(None);
                };
                let revoked: Option<DateTime<Utc>> = row.get(0);
                let session_rows = client
                    .query(
                        "SELECT session_id FROM token_sessions WHERE token_id = $1",
                        &[&id],
                    )
                    .await?;
                let prefix_rows = client
                    .query(
                        "SELECT prefix FROM token_session_prefixes WHERE token_id = $1",
                        &[&id],
                    )
                    .await?;
                Ok(Some(TokenStatus {
                    revoked: revoked.is_some(),
                    session_ids: session_rows.iter().map(|r| r.get(0)).collect(),
                    session_prefixes: prefix_rows.iter().map(|r| r.get(0)).collect(),
                }))
            }
            DbPool::MySQL(my) => {
                use mysql_async::prelude::*;
                let mut conn = my.get_conn().await?;
                let revoked: Option<Option<String>> = conn
                    .exec_first("SELECT revoked_at FROM tokens WHERE id = ?", (id,))
                    .await?;
                let Some(revoked_at) = revoked else {
                    return Ok(None);
                };
                let session_ids: Vec<String> = conn
                    .exec(
                        "SELECT session_id FROM token_sessions WHERE token_id = ?",
                        (id,),
                    )
                    .await?;
                let session_prefixes: Vec<String> = conn
                    .exec(
                        "SELECT prefix FROM token_session_prefixes WHERE token_id = ?",
                        (id,),
                    )
                    .await?;
                Ok(Some(TokenStatus {
                    revoked: revoked_at.is_some(),
                    session_ids,
                    session_prefixes,
                }))
            }
            DbPool::SQLite(pool) => {
                let id = id.to_string();
                sqlite_blocking(pool, move |conn| {
                    let revoked = sqlite_raw::query(
                        conn,
                        "SELECT revoked_at FROM tokens WHERE id = ?",
                        &[SQ::Text(id.clone())],
                        |r| r.get_string(0),
                    )?;
                    let Some(revoked_at) = revoked.into_iter().next() else {
                        return Ok(None);
                    };
                    let session_ids = sqlite_raw::query(
                        conn,
                        "SELECT session_id FROM token_sessions WHERE token_id = ?",
                        &[SQ::Text(id.clone())],
                        |r| r.get_string(0).unwrap_or_default(),
                    )?;
                    let session_prefixes = sqlite_raw::query(
                        conn,
                        "SELECT prefix FROM token_session_prefixes WHERE token_id = ?",
                        &[SQ::Text(id.clone())],
                        |r| r.get_string(0).unwrap_or_default(),
                    )?;
                    Ok(Some(TokenStatus {
                        revoked: revoked_at.is_some(),
                        session_ids,
                        session_prefixes,
                    }))
                })
                .await
            }
        }
    }

    #[allow(clippy::type_complexity)]
    pub async fn list(&self, limit: u32, offset: u32) -> anyhow::Result<Vec<TokenRecord>> {
        let limit = limit.clamp(1, 1000) as i64;
        let offset = offset as i64;
        let mut records: Vec<TokenRecord> = match self.pool {
            DbPool::Postgres(pg) => {
                let client = pg.get().await?;
                let rows = client
                    .query(
                        "SELECT id, name, to_char(created_at, 'YYYY-MM-DD HH24:MI:SS'), \
                         to_char(expires_at, 'YYYY-MM-DD HH24:MI:SS'), \
                         to_char(revoked_at, 'YYYY-MM-DD HH24:MI:SS') \
                         FROM tokens ORDER BY created_at DESC LIMIT $1 OFFSET $2",
                        &[&limit, &offset],
                    )
                    .await?;
                rows.into_iter()
                    .map(|r| TokenRecord {
                        id: r.get(0),
                        name: r.get(1),
                        session_ids: Vec::new(),
                        session_prefixes: Vec::new(),
                        created_at: r.get(2),
                        expires_at: r.get(3),
                        revoked_at: r.get(4),
                    })
                    .collect()
            }
            DbPool::MySQL(my) => {
                use mysql_async::prelude::*;
                let mut conn = my.get_conn().await?;
                let rows: Vec<(
                    String,
                    Option<String>,
                    String,
                    Option<String>,
                    Option<String>,
                )> = conn
                    .exec(
                        "SELECT id, name, created_at, expires_at, revoked_at \
                         FROM tokens ORDER BY created_at DESC LIMIT ? OFFSET ?",
                        (limit, offset),
                    )
                    .await?;
                rows.into_iter()
                    .map(|r| TokenRecord {
                        id: r.0,
                        name: r.1,
                        session_ids: Vec::new(),
                        session_prefixes: Vec::new(),
                        created_at: r.2,
                        expires_at: r.3,
                        revoked_at: r.4,
                    })
                    .collect()
            }
            DbPool::SQLite(pool) => {
                sqlite_blocking(pool, move |conn| {
                    sqlite_raw::query(
                        conn,
                        "SELECT id, name, created_at, expires_at, revoked_at \
                         FROM tokens ORDER BY created_at DESC LIMIT ? OFFSET ?",
                        &[SQ::Int(limit), SQ::Int(offset)],
                        |r| TokenRecord {
                            id: r.get_string(0).unwrap_or_default(),
                            name: r.get_string(1),
                            session_ids: Vec::new(),
                            session_prefixes: Vec::new(),
                            created_at: r.get_string(2).unwrap_or_default(),
                            expires_at: r.get_string(3),
                            revoked_at: r.get_string(4),
                        },
                    )
                })
                .await?
            }
        };

        for record in &mut records {
            if let Some(status) = self.status(&record.id).await? {
                record.session_ids = status.session_ids;
                record.session_prefixes = status.session_prefixes;
            }
        }
        Ok(records)
    }

    /// Returns `true` if a not-already-revoked token with this id was found
    /// and revoked; `false` if it didn't exist or was already revoked.
    pub async fn revoke(&self, id: &str) -> anyhow::Result<bool> {
        match self.pool {
            DbPool::Postgres(pg) => {
                let client = pg.get().await?;
                let n = client
                    .execute(
                        "UPDATE tokens SET revoked_at = NOW() WHERE id = $1 AND revoked_at IS NULL",
                        &[&id],
                    )
                    .await?;
                Ok(n > 0)
            }
            DbPool::MySQL(my) => {
                use mysql_async::prelude::*;
                let mut conn = my.get_conn().await?;
                let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
                conn.exec_drop(
                    "UPDATE tokens SET revoked_at = ? WHERE id = ? AND revoked_at IS NULL",
                    (&now, id),
                )
                .await?;
                Ok(conn.affected_rows() > 0)
            }
            DbPool::SQLite(pool) => {
                let id = id.to_string();
                sqlite_blocking(pool, move |conn| {
                    let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
                    let changed = sqlite_raw::execute(
                        conn,
                        "UPDATE tokens SET revoked_at = ? WHERE id = ? AND revoked_at IS NULL",
                        &[SQ::Text(now), SQ::Text(id)],
                    )?;
                    Ok(changed > 0)
                })
                .await
            }
        }
    }
}
