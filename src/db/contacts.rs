use crate::db::session::{sqlite_blocking, DbPool};
use crate::db::sqlite_raw::{self, Value as SQ};
use serde::Serialize;

#[derive(Debug, Clone, Default)]
pub struct ContactUpsert<'a> {
    pub session_id: &'a str,
    pub jid: &'a str,
    pub phone: Option<&'a str>,
    pub lid_jid: Option<&'a str>,
    pub full_name: Option<&'a str>,
    pub first_name: Option<&'a str>,
    pub push_name: Option<&'a str>,
    pub business_name: Option<&'a str>,
    /// "appstate" | "notification" | "message" | "push_name" | "manual" ...
    pub source: &'a str,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContactRecord {
    pub jid: String,
    pub phone: Option<String>,
    pub lid_jid: Option<String>,
    pub full_name: Option<String>,
    pub first_name: Option<String>,
    pub push_name: Option<String>,
    pub business_name: Option<String>,
    pub source: String,
    pub updated_at: Option<String>,
}

pub struct ContactStore<'a> {
    pool: &'a DbPool,
}

impl<'a> ContactStore<'a> {
    pub fn new(pool: &'a DbPool) -> Self {
        Self { pool }
    }

    pub async fn upsert(&self, c: &ContactUpsert<'_>) -> anyhow::Result<()> {
        match self.pool {
            DbPool::Postgres(pg) => {
                let client = pg.get().await?;
                client.execute(
                    r#"
                    INSERT INTO contacts (session_id, jid, phone, lid_jid, full_name, first_name, push_name, business_name, source, updated_at)
                    VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9, NOW())
                    ON CONFLICT (session_id, jid) DO UPDATE SET
                        phone = COALESCE(EXCLUDED.phone, contacts.phone),
                        lid_jid = COALESCE(EXCLUDED.lid_jid, contacts.lid_jid),
                        full_name = COALESCE(EXCLUDED.full_name, contacts.full_name),
                        first_name = COALESCE(EXCLUDED.first_name, contacts.first_name),
                        push_name = COALESCE(EXCLUDED.push_name, contacts.push_name),
                        business_name = COALESCE(EXCLUDED.business_name, contacts.business_name),
                        source = EXCLUDED.source,
                        updated_at = NOW()
                    "#,
                    &[
                        &c.session_id,
                        &c.jid,
                        &c.phone,
                        &c.lid_jid,
                        &c.full_name,
                        &c.first_name,
                        &c.push_name,
                        &c.business_name,
                        &c.source,
                    ],
                ).await?;
            }
            DbPool::MySQL(my) => {
                use mysql_async::prelude::*;
                let mut conn = my.get_conn().await?;
                let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
                conn.exec_drop(
                    r#"
                    INSERT INTO contacts (session_id, jid, phone, lid_jid, full_name, first_name, push_name, business_name, source, updated_at)
                    VALUES (?,?,?,?,?,?,?,?,?,?)
                    ON DUPLICATE KEY UPDATE
                        phone = COALESCE(VALUES(phone), phone),
                        lid_jid = COALESCE(VALUES(lid_jid), lid_jid),
                        full_name = COALESCE(VALUES(full_name), full_name),
                        first_name = COALESCE(VALUES(first_name), first_name),
                        push_name = COALESCE(VALUES(push_name), push_name),
                        business_name = COALESCE(VALUES(business_name), business_name),
                        source = VALUES(source),
                        updated_at = VALUES(updated_at)
                    "#,
                    (
                        c.session_id, c.jid, c.phone, c.lid_jid, c.full_name,
                        c.first_name, c.push_name, c.business_name, c.source, &now,
                    ),
                ).await?;
            }
            DbPool::SQLite(pool) => {
                let sid = c.session_id.to_string();
                let jid = c.jid.to_string();
                let phone = SQ::from_opt_str(c.phone);
                let lid = SQ::from_opt_str(c.lid_jid);
                let fname = SQ::from_opt_str(c.full_name);
                let first = SQ::from_opt_str(c.first_name);
                let push = SQ::from_opt_str(c.push_name);
                let biz = SQ::from_opt_str(c.business_name);
                let src = c.source.to_string();
                let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
                sqlite_blocking(pool, move |conn| {
                    sqlite_raw::execute(
                        conn,
                        "INSERT INTO contacts (session_id, jid, phone, lid_jid, full_name, first_name, push_name, business_name, source, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(session_id, jid) DO UPDATE SET phone = COALESCE(excluded.phone, contacts.phone), lid_jid = COALESCE(excluded.lid_jid, contacts.lid_jid), full_name = COALESCE(excluded.full_name, contacts.full_name), first_name = COALESCE(excluded.first_name, contacts.first_name), push_name = COALESCE(excluded.push_name, contacts.push_name), business_name = COALESCE(excluded.business_name, contacts.business_name), source = excluded.source, updated_at = excluded.updated_at",
                        &[SQ::Text(sid), SQ::Text(jid), phone, lid, fname, first, push, biz, SQ::Text(src), SQ::Text(now)],
                    )?;
                    Ok(())
                })
                .await?;
            }
        }
        Ok(())
    }

    #[allow(clippy::type_complexity)]
    pub async fn list(
        &self,
        session_id: &str,
        search: Option<&str>,
        limit: u32,
        offset: u32,
    ) -> anyhow::Result<Vec<ContactRecord>> {
        let limit = limit.clamp(1, 1000);
        match self.pool {
            DbPool::Postgres(pg) => {
                let client = pg.get().await?;
                let rows = if let Some(q) = search {
                    let q_like = format!("%{}%", q);
                    client.query(
                        "SELECT jid, phone, lid_jid, full_name, first_name, push_name, business_name, source, to_char(updated_at, 'YYYY-MM-DD HH24:MI:SS')
                         FROM contacts WHERE session_id = $1
                         AND (full_name ILIKE $2 OR first_name ILIKE $2 OR push_name ILIKE $2 OR phone ILIKE $2 OR business_name ILIKE $2)
                         ORDER BY COALESCE(full_name, push_name, jid) ASC LIMIT $3 OFFSET $4",
                        &[&session_id, &q_like, &(limit as i64), &(offset as i64)],
                    ).await?
                } else {
                    client.query(
                        "SELECT jid, phone, lid_jid, full_name, first_name, push_name, business_name, source, to_char(updated_at, 'YYYY-MM-DD HH24:MI:SS')
                         FROM contacts WHERE session_id = $1
                         ORDER BY COALESCE(full_name, push_name, jid) ASC LIMIT $2 OFFSET $3",
                        &[&session_id, &(limit as i64), &(offset as i64)],
                    ).await?
                };
                Ok(rows
                    .into_iter()
                    .map(|r| ContactRecord {
                        jid: r.get(0),
                        phone: r.get(1),
                        lid_jid: r.get(2),
                        full_name: r.get(3),
                        first_name: r.get(4),
                        push_name: r.get(5),
                        business_name: r.get(6),
                        source: r.get(7),
                        updated_at: r.get(8),
                    })
                    .collect())
            }
            DbPool::MySQL(my) => {
                use mysql_async::prelude::*;
                let mut conn = my.get_conn().await?;
                let rows: Vec<(
                    String,
                    Option<String>,
                    Option<String>,
                    Option<String>,
                    Option<String>,
                    Option<String>,
                    Option<String>,
                    String,
                    Option<String>,
                )> = if let Some(q) = search {
                    let q_like = format!("%{}%", q);
                    conn.exec(
                        "SELECT jid, phone, lid_jid, full_name, first_name, push_name, business_name, source, updated_at
                         FROM contacts WHERE session_id = ?
                         AND (full_name LIKE ? OR first_name LIKE ? OR push_name LIKE ? OR phone LIKE ? OR business_name LIKE ?)
                         ORDER BY COALESCE(full_name, push_name, jid) ASC LIMIT ? OFFSET ?",
                        (session_id, &q_like, &q_like, &q_like, &q_like, &q_like, limit as i64, offset as i64),
                    ).await?
                } else {
                    conn.exec(
                        "SELECT jid, phone, lid_jid, full_name, first_name, push_name, business_name, source, updated_at
                         FROM contacts WHERE session_id = ?
                         ORDER BY COALESCE(full_name, push_name, jid) ASC LIMIT ? OFFSET ?",
                        (session_id, limit as i64, offset as i64),
                    ).await?
                };
                Ok(rows
                    .into_iter()
                    .map(|r| ContactRecord {
                        jid: r.0,
                        phone: r.1,
                        lid_jid: r.2,
                        full_name: r.3,
                        first_name: r.4,
                        push_name: r.5,
                        business_name: r.6,
                        source: r.7,
                        updated_at: r.8,
                    })
                    .collect())
            }
            DbPool::SQLite(pool) => {
                let sid = session_id.to_string();
                let q = search.map(|s| format!("%{}%", s));
                let limit_i = limit as i64;
                let offset_i = offset as i64;
                sqlite_blocking(pool, move |conn| {
                    let mapper = |r: &sqlite_raw::Row| ContactRecord {
                        jid: r.get_string(0).unwrap_or_default(),
                        phone: r.get_string(1),
                        lid_jid: r.get_string(2),
                        full_name: r.get_string(3),
                        first_name: r.get_string(4),
                        push_name: r.get_string(5),
                        business_name: r.get_string(6),
                        source: r.get_string(7).unwrap_or_else(|| "unknown".to_string()),
                        updated_at: r.get_string(8),
                    };
                    if let Some(qq) = q {
                        sqlite_raw::query(
                            conn,
                            "SELECT jid, phone, lid_jid, full_name, first_name, push_name, business_name, source, updated_at FROM contacts WHERE session_id = ? AND (COALESCE(full_name,'') LIKE ? OR COALESCE(first_name,'') LIKE ? OR COALESCE(push_name,'') LIKE ? OR COALESCE(phone,'') LIKE ? OR COALESCE(business_name,'') LIKE ?) ORDER BY COALESCE(full_name, push_name, jid) ASC LIMIT ? OFFSET ?",
                            &[SQ::Text(sid), SQ::Text(qq.clone()), SQ::Text(qq.clone()), SQ::Text(qq.clone()), SQ::Text(qq.clone()), SQ::Text(qq), SQ::Int(limit_i), SQ::Int(offset_i)],
                            mapper,
                        )
                    } else {
                        sqlite_raw::query(
                            conn,
                            "SELECT jid, phone, lid_jid, full_name, first_name, push_name, business_name, source, updated_at FROM contacts WHERE session_id = ? ORDER BY COALESCE(full_name, push_name, jid) ASC LIMIT ? OFFSET ?",
                            &[SQ::Text(sid), SQ::Int(limit_i), SQ::Int(offset_i)],
                            mapper,
                        )
                    }
                })
                .await
            }
        }
    }

    pub async fn count(&self, session_id: &str) -> anyhow::Result<u64> {
        match self.pool {
            DbPool::Postgres(pg) => {
                let client = pg.get().await?;
                let row = client
                    .query_one(
                        "SELECT COUNT(*) FROM contacts WHERE session_id = $1",
                        &[&session_id],
                    )
                    .await?;
                let n: i64 = row.get(0);
                Ok(n as u64)
            }
            DbPool::MySQL(my) => {
                use mysql_async::prelude::*;
                let mut conn = my.get_conn().await?;
                let row: Option<(i64,)> = conn
                    .exec_first(
                        "SELECT COUNT(*) FROM contacts WHERE session_id = ?",
                        (session_id,),
                    )
                    .await?;
                Ok(row.map(|r| r.0 as u64).unwrap_or(0))
            }
            DbPool::SQLite(pool) => {
                let sid = session_id.to_string();
                let rows = sqlite_blocking(pool, move |conn| {
                    sqlite_raw::query(
                        conn,
                        "SELECT COUNT(*) FROM contacts WHERE session_id = ?",
                        &[SQ::Text(sid)],
                        |r| r.get_int(0),
                    )
                })
                .await?;
                Ok(rows.first().copied().unwrap_or(0) as u64)
            }
        }
    }
}
