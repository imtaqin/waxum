//! Persistence for the new-outgoing-chat counter and limit (#157).
//!
//! WhatsApp restricts an account that starts too many conversations with
//! numbers it has never talked to, and unlinks every companion device when
//! it does. The threshold is not published and differs per account, so
//! waxum records what it can observe and lets the operator set a ceiling:
//!
//! - `new_chats`: one row the first time a session sends to a direct chat
//!   it has no history with. Counting rows by `created_at` gives the
//!   rolling 3/6/12/24 h figures.
//! - `new_chat_limits`: an optional per-session ceiling.
//! - `new_chat_incidents`: the rolling figures at the moment WhatsApp
//!   logged the session out, so the threshold can be read off real
//!   incidents.
//!
//! Every timestamp is unix seconds in a `BIGINT`, and every query is plain
//! SQL written once with `?` placeholders (rewritten to `$n` for
//! Postgres), so the three backends cannot drift apart here.

use crate::db::session::{sqlite_blocking, DbPool};
use crate::db::sqlite_raw::{self, Value as SQ};

#[derive(Clone)]
enum P {
    Text(String),
    Int(i64),
}

#[derive(Debug, Clone, PartialEq)]
enum Cell {
    Text(Option<String>),
    Int(i64),
}

impl Cell {
    fn int(&self) -> i64 {
        match self {
            Cell::Int(i) => *i,
            Cell::Text(_) => 0,
        }
    }
    fn text(&self) -> String {
        match self {
            Cell::Text(t) => t.clone().unwrap_or_default(),
            Cell::Int(i) => i.to_string(),
        }
    }
}

/// `?` placeholders to Postgres' `$1..$n`.
fn pg_sql(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len() + 8);
    let mut n = 0;
    for ch in sql.chars() {
        if ch == '?' {
            n += 1;
            out.push('$');
            out.push_str(&n.to_string());
        } else {
            out.push(ch);
        }
    }
    out
}

fn pg_params(params: &[P]) -> Vec<&(dyn tokio_postgres::types::ToSql + Sync)> {
    params
        .iter()
        .map(|p| match p {
            P::Text(t) => t as &(dyn tokio_postgres::types::ToSql + Sync),
            P::Int(i) => i as &(dyn tokio_postgres::types::ToSql + Sync),
        })
        .collect()
}

fn my_params(params: &[P]) -> mysql_async::Params {
    if params.is_empty() {
        return mysql_async::Params::Empty;
    }
    mysql_async::Params::Positional(
        params
            .iter()
            .map(|p| match p {
                P::Text(t) => mysql_async::Value::from(t.as_str()),
                P::Int(i) => mysql_async::Value::from(*i),
            })
            .collect(),
    )
}

fn sq_params(params: &[P]) -> Vec<SQ> {
    params
        .iter()
        .map(|p| match p {
            P::Text(t) => SQ::Text(t.clone()),
            P::Int(i) => SQ::Int(*i),
        })
        .collect()
}

async fn exec(pool: &DbPool, sql: &str, params: Vec<P>) -> anyhow::Result<()> {
    match pool {
        DbPool::Postgres(pg) => {
            let client = pg.get().await?;
            client.execute(&pg_sql(sql), &pg_params(&params)).await?;
        }
        DbPool::MySQL(my) => {
            use mysql_async::prelude::*;
            let mut conn = my.get_conn().await?;
            conn.exec_drop(sql, my_params(&params)).await?;
        }
        DbPool::SQLite(handle) => {
            let sql = sql.to_string();
            let params = sq_params(&params);
            sqlite_blocking(handle, move |conn| {
                sqlite_raw::execute(conn, &sql, &params)?;
                Ok(())
            })
            .await?;
        }
    }
    Ok(())
}

/// Runs a `SELECT`. `text_cols[i]` says whether column `i` is text; every
/// other column is a `BIGINT`.
async fn rows(
    pool: &DbPool,
    sql: &str,
    params: Vec<P>,
    text_cols: &'static [bool],
) -> anyhow::Result<Vec<Vec<Cell>>> {
    match pool {
        DbPool::Postgres(pg) => {
            let client = pg.get().await?;
            let found = client.query(&pg_sql(sql), &pg_params(&params)).await?;
            Ok(found
                .iter()
                .map(|r| {
                    text_cols
                        .iter()
                        .enumerate()
                        .map(|(i, is_text)| {
                            if *is_text {
                                Cell::Text(r.get::<_, Option<String>>(i))
                            } else {
                                Cell::Int(r.get::<_, i64>(i))
                            }
                        })
                        .collect()
                })
                .collect())
        }
        DbPool::MySQL(my) => {
            use mysql_async::prelude::*;
            let mut conn = my.get_conn().await?;
            let found: Vec<mysql_async::Row> = conn.exec(sql, my_params(&params)).await?;
            Ok(found
                .iter()
                .map(|r| {
                    text_cols
                        .iter()
                        .enumerate()
                        .map(|(i, is_text)| {
                            if *is_text {
                                Cell::Text(r.get::<Option<String>, _>(i).flatten())
                            } else {
                                Cell::Int(r.get::<i64, _>(i).unwrap_or(0))
                            }
                        })
                        .collect()
                })
                .collect())
        }
        DbPool::SQLite(handle) => {
            let sql = sql.to_string();
            let params = sq_params(&params);
            sqlite_blocking(handle, move |conn| {
                sqlite_raw::query(conn, &sql, &params, |r| {
                    text_cols
                        .iter()
                        .enumerate()
                        .map(|(i, is_text)| {
                            if *is_text {
                                Cell::Text(r.get_string(i as std::os::raw::c_int))
                            } else {
                                Cell::Int(r.get_int(i as std::os::raw::c_int))
                            }
                        })
                        .collect()
                })
            })
            .await
        }
    }
}

/// Creates the three tables. Called from [`crate::db::schema::init_schema`].
pub async fn init_schema(pool: &DbPool) -> anyhow::Result<()> {
    let ddl = [
        "CREATE TABLE IF NOT EXISTS new_chats ( \
            session_id VARCHAR(191) NOT NULL, \
            chat_jid VARCHAR(191) NOT NULL, \
            created_at BIGINT NOT NULL, \
            PRIMARY KEY (session_id, chat_jid) \
         )",
        "CREATE TABLE IF NOT EXISTS new_chat_limits ( \
            session_id VARCHAR(191) NOT NULL PRIMARY KEY, \
            max_new_chats BIGINT NOT NULL, \
            window_hours BIGINT NOT NULL \
         )",
        "CREATE TABLE IF NOT EXISTS new_chat_incidents ( \
            session_id VARCHAR(191) NOT NULL, \
            occurred_at BIGINT NOT NULL, \
            reason VARCHAR(191) NOT NULL, \
            last_3h BIGINT NOT NULL, \
            last_6h BIGINT NOT NULL, \
            last_12h BIGINT NOT NULL, \
            last_24h BIGINT NOT NULL, \
            PRIMARY KEY (session_id, occurred_at) \
         )",
    ];
    for statement in ddl {
        exec(pool, statement, vec![]).await?;
    }
    Ok(())
}

/// Whether the session already has a chat with any of `jids`: one it
/// started through waxum, or any stored message in either direction.
pub async fn chat_known(pool: &DbPool, session_id: &str, jids: &[String]) -> anyhow::Result<bool> {
    for jid in jids {
        let params = vec![P::Text(session_id.to_string()), P::Text(jid.clone())];
        let started = rows(
            pool,
            "SELECT created_at FROM new_chats WHERE session_id = ? AND chat_jid = ? LIMIT 1",
            params.clone(),
            &[false],
        )
        .await?;
        if !started.is_empty() {
            return Ok(true);
        }
        let messaged = rows(
            pool,
            "SELECT id FROM messages WHERE session_id = ? AND chat_jid = ? LIMIT 1",
            params,
            &[false],
        )
        .await?;
        if !messaged.is_empty() {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Records that the session started a chat with `jid` at `now`.
pub async fn record(pool: &DbPool, session_id: &str, jid: &str, now: i64) -> anyhow::Result<()> {
    let sql = match pool {
        DbPool::Postgres(_) => {
            "INSERT INTO new_chats (session_id, chat_jid, created_at) VALUES (?, ?, ?) ON CONFLICT DO NOTHING"
        }
        DbPool::MySQL(_) => {
            "INSERT IGNORE INTO new_chats (session_id, chat_jid, created_at) VALUES (?, ?, ?)"
        }
        DbPool::SQLite(_) => {
            "INSERT OR IGNORE INTO new_chats (session_id, chat_jid, created_at) VALUES (?, ?, ?)"
        }
    };
    exec(
        pool,
        sql,
        vec![
            P::Text(session_id.to_string()),
            P::Text(jid.to_string()),
            P::Int(now),
        ],
    )
    .await
}

/// Un-records a chat whose first send did not go out.
pub async fn forget(pool: &DbPool, session_id: &str, jid: &str) -> anyhow::Result<()> {
    exec(
        pool,
        "DELETE FROM new_chats WHERE session_id = ? AND chat_jid = ?",
        vec![P::Text(session_id.to_string()), P::Text(jid.to_string())],
    )
    .await
}

/// When the session started each new chat since `since`, oldest first.
pub async fn started_since(
    pool: &DbPool,
    session_id: &str,
    since: i64,
) -> anyhow::Result<Vec<i64>> {
    let found = rows(
        pool,
        "SELECT created_at FROM new_chats WHERE session_id = ? AND created_at > ? ORDER BY created_at",
        vec![P::Text(session_id.to_string()), P::Int(since)],
        &[false],
    )
    .await?;
    Ok(found.iter().map(|r| r[0].int()).collect())
}

/// `(max_new_chats, window_hours)`, or `None` when no limit is set.
pub async fn get_limit(pool: &DbPool, session_id: &str) -> anyhow::Result<Option<(i64, i64)>> {
    let found = rows(
        pool,
        "SELECT max_new_chats, window_hours FROM new_chat_limits WHERE session_id = ?",
        vec![P::Text(session_id.to_string())],
        &[false, false],
    )
    .await?;
    Ok(found.first().map(|r| (r[0].int(), r[1].int())))
}

/// Sets or, with `None`, removes the session's limit.
pub async fn set_limit(
    pool: &DbPool,
    session_id: &str,
    limit: Option<(i64, i64)>,
) -> anyhow::Result<()> {
    exec(
        pool,
        "DELETE FROM new_chat_limits WHERE session_id = ?",
        vec![P::Text(session_id.to_string())],
    )
    .await?;
    if let Some((max_new_chats, window_hours)) = limit {
        exec(
            pool,
            "INSERT INTO new_chat_limits (session_id, max_new_chats, window_hours) VALUES (?, ?, ?)",
            vec![
                P::Text(session_id.to_string()),
                P::Int(max_new_chats),
                P::Int(window_hours),
            ],
        )
        .await?;
    }
    Ok(())
}

/// The rolling figures at the moment WhatsApp logged the session out.
#[derive(Debug, Clone, PartialEq)]
pub struct Incident {
    pub occurred_at: i64,
    pub reason: String,
    pub last_3h: i64,
    pub last_6h: i64,
    pub last_12h: i64,
    pub last_24h: i64,
}

pub async fn record_incident(
    pool: &DbPool,
    session_id: &str,
    incident: &Incident,
) -> anyhow::Result<()> {
    exec(
        pool,
        "DELETE FROM new_chat_incidents WHERE session_id = ? AND occurred_at = ?",
        vec![
            P::Text(session_id.to_string()),
            P::Int(incident.occurred_at),
        ],
    )
    .await?;
    exec(
        pool,
        "INSERT INTO new_chat_incidents (session_id, occurred_at, reason, last_3h, last_6h, last_12h, last_24h) VALUES (?, ?, ?, ?, ?, ?, ?)",
        vec![
            P::Text(session_id.to_string()),
            P::Int(incident.occurred_at),
            P::Text(incident.reason.chars().take(180).collect()),
            P::Int(incident.last_3h),
            P::Int(incident.last_6h),
            P::Int(incident.last_12h),
            P::Int(incident.last_24h),
        ],
    )
    .await
}

/// The session's most recent incidents, newest first.
pub async fn incidents(pool: &DbPool, session_id: &str) -> anyhow::Result<Vec<Incident>> {
    let found = rows(
        pool,
        "SELECT occurred_at, reason, last_3h, last_6h, last_12h, last_24h FROM new_chat_incidents WHERE session_id = ? ORDER BY occurred_at DESC LIMIT 50",
        vec![P::Text(session_id.to_string())],
        &[false, true, false, false, false, false],
    )
    .await?;
    Ok(found
        .iter()
        .map(|r| Incident {
            occurred_at: r[0].int(),
            reason: r[1].text(),
            last_3h: r[2].int(),
            last_6h: r[3].int(),
            last_12h: r[4].int(),
            last_24h: r[5].int(),
        })
        .collect())
}

/// Removes everything recorded for a deleted session.
pub async fn purge(pool: &DbPool, session_id: &str) {
    for table in ["new_chats", "new_chat_limits", "new_chat_incidents"] {
        let sql = format!("DELETE FROM {table} WHERE session_id = ?");
        if let Err(e) = exec(pool, &sql, vec![P::Text(session_id.to_string())]).await {
            tracing::warn!("new_chats purge of {table} failed for {session_id}: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::pg_sql;

    #[test]
    fn placeholders_are_numbered_for_postgres() {
        assert_eq!(
            pg_sql("SELECT a FROM t WHERE b = ? AND c > ?"),
            "SELECT a FROM t WHERE b = $1 AND c > $2"
        );
        assert_eq!(pg_sql("DELETE FROM t"), "DELETE FROM t");
    }
}
