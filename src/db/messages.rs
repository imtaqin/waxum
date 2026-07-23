//! Persistence for message history + full-text search.
//!
//! One `messages` row per ingested message, BOTH directions: incoming
//! rows are captured from the event stream in
//! [`crate::handlers::sessions`], outgoing rows from the `execute_*`
//! send core in [`crate::handlers::messages`]. Ingestion is best-effort
//! everywhere — a failed insert must never break send or receive, so
//! callers log and continue.
//!
//! Search is tri-backend with a best-effort degrade ladder per
//! backend, mirroring the house style:
//!
//! - **SQLite** — FTS5 virtual table `messages_fts` (external content
//!   synced manually on insert, no triggers; the row is written right
//!   after the main insert in the same `sqlite_blocking` call) with
//!   `snippet()` highlights. If FTS5 is unavailable or the MATCH query
//!   errors, falls back to a plain `LIKE` scan.
//! - **Postgres** — stored generated `body_tsv` column
//!   (`to_tsvector('simple', …)`, the `simple` config because chats mix
//!   languages and stemming would hurt) with a GIN index, queried via
//!   `plainto_tsquery`, snippets via `ts_headline`. Falls back to
//!   `ILIKE` on error.
//! - **MySQL** — `FULLTEXT` index + `MATCH … AGAINST` in natural
//!   language mode, no cheap snippet (plain rows). Falls back to
//!   `LIKE` on error. Note: MySQL's default `ft_min_word_len` of 4
//!   silently drops shorter tokens from the index — the LIKE fallback
//!   only triggers on ERRORS, not empty results.
//!
//! Timestamps follow the house convention: `%Y-%m-%d %H:%M:%S` UTC
//! text on SQLite/MySQL (lexicographically sortable), `TIMESTAMPTZ` on
//! Postgres, normalized to text on read so [`MessageRow`] is
//! backend-agnostic.

use crate::db::session::{sqlite_blocking, DbPool};
use crate::db::sqlite_raw::{self, Value as SQ};

const COLS: &str =
    "id, message_id, session_id, chat_jid, sender_jid, direction, msg_type, body, msg_timestamp";

fn now_str() -> String {
    chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

fn fmt_utc(dt: chrono::DateTime<chrono::Utc>) -> String {
    dt.format("%Y-%m-%d %H:%M:%S").to_string()
}

/// One message offered for ingestion.
#[derive(Debug, Clone)]
pub struct NewMessage {
    pub message_id: String,
    pub session_id: String,
    pub chat_jid: String,
    pub sender_jid: String,
    /// `in` or `out`.
    pub direction: String,
    /// Type slug as produced by the content extractor: `text`,
    /// `image`, `video`, `audio`, `ptt`, `document`, `sticker`,
    /// `location`, `contact`, …
    pub msg_type: String,
    /// Searchable text: message body, or the caption for media.
    pub body: Option<String>,
    pub msg_timestamp: chrono::DateTime<chrono::Utc>,
}

/// A `messages` row as returned by search, timestamps rendered as
/// `%Y-%m-%d %H:%M:%S` UTC text regardless of backend. `snippet` is
/// only populated by backends with cheap highlight support (SQLite
/// FTS5, Postgres).
#[derive(Debug, Clone)]
pub struct MessageRow {
    pub id: i64,
    pub message_id: String,
    pub session_id: String,
    pub chat_jid: String,
    pub sender_jid: String,
    pub direction: String,
    pub msg_type: String,
    pub body: Option<String>,
    pub msg_timestamp: String,
    pub snippet: Option<String>,
}

/// Store one message, ignoring duplicates on `(session_id,
/// message_id)` (history-sync replays and event re-deliveries repeat
/// ids). On SQLite the FTS index is synced in the same blocking call
/// when the row was actually new; FTS-insert errors are swallowed so a
/// broken index never blocks ingestion.
pub async fn insert(pool: &DbPool, msg: &NewMessage) -> anyhow::Result<()> {
    match pool {
        DbPool::Postgres(pg) => {
            let client = pg.get().await?;
            client
                .execute(
                    "INSERT INTO messages (message_id, session_id, chat_jid, sender_jid, direction, msg_type, body, msg_timestamp) VALUES ($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT (session_id, message_id) DO NOTHING",
                    &[
                        &msg.message_id,
                        &msg.session_id,
                        &msg.chat_jid,
                        &msg.sender_jid,
                        &msg.direction,
                        &msg.msg_type,
                        &msg.body,
                        &msg.msg_timestamp,
                    ],
                )
                .await?;
        }
        DbPool::MySQL(my) => {
            use mysql_async::prelude::*;
            let mut conn = my.get_conn().await?;
            let ts = fmt_utc(msg.msg_timestamp);
            let now = now_str();
            conn.exec_drop(
                "INSERT IGNORE INTO messages (message_id, session_id, chat_jid, sender_jid, direction, msg_type, body, msg_timestamp, created_at) VALUES (?,?,?,?,?,?,?,?,?)",
                (
                    &msg.message_id,
                    &msg.session_id,
                    &msg.chat_jid,
                    &msg.sender_jid,
                    &msg.direction,
                    &msg.msg_type,
                    msg.body.as_deref(),
                    ts,
                    now,
                ),
            )
            .await?;
        }
        DbPool::SQLite(handle) => {
            let m = msg.clone();
            let ts = fmt_utc(m.msg_timestamp);
            let now = now_str();
            sqlite_blocking(handle, move |conn| {
                let changed = sqlite_raw::execute(
                    conn,
                    "INSERT OR IGNORE INTO messages (message_id, session_id, chat_jid, sender_jid, direction, msg_type, body, msg_timestamp, created_at) VALUES (?,?,?,?,?,?,?,?,?)",
                    &[
                        SQ::Text(m.message_id.clone()),
                        SQ::Text(m.session_id.clone()),
                        SQ::Text(m.chat_jid.clone()),
                        SQ::Text(m.sender_jid.clone()),
                        SQ::Text(m.direction.clone()),
                        SQ::Text(m.msg_type.clone()),
                        SQ::from_opt_str(m.body.as_deref()),
                        SQ::Text(ts),
                        SQ::Text(now),
                    ],
                )?;
                if changed > 0 {
                    if let Some(body) = m.body.as_deref() {
                        let _ = sqlite_raw::execute(
                            conn,
                            "INSERT INTO messages_fts (body, session_id, message_id) VALUES (?,?,?)",
                            &[
                                SQ::Text(body.to_string()),
                                SQ::Text(m.session_id.clone()),
                                SQ::Text(m.message_id.clone()),
                            ],
                        );
                    }
                }
                Ok(())
            })
            .await?;
        }
    }
    Ok(())
}

/// Full-text search over stored message bodies, newest first.
/// `session_id = None` searches fleet-wide. Empty-after-sanitizing
/// queries return an empty vec rather than erroring. See the module
/// docs for the per-backend strategy and fallback ladder.
pub async fn search(
    pool: &DbPool,
    session_id: Option<&str>,
    query: &str,
    limit: i64,
    offset: i64,
) -> anyhow::Result<Vec<MessageRow>> {
    match pool {
        DbPool::Postgres(pg) => {
            let client = pg.get().await?;
            let mut values: Vec<String> = vec![query.to_string()];
            let mut where_sql = "body_tsv @@ plainto_tsquery('simple', $1)".to_string();
            if let Some(sid) = session_id {
                values.push(sid.to_string());
                where_sql.push_str(&format!(" AND session_id = ${}", values.len()));
            }
            let (limit_ph, offset_ph) = (values.len() + 1, values.len() + 2);
            let sql = format!(
                "SELECT {COLS}, ts_headline('simple', coalesce(body, ''), plainto_tsquery('simple', $1)) AS snippet FROM messages WHERE {where_sql} ORDER BY msg_timestamp DESC, id DESC LIMIT ${limit_ph} OFFSET ${offset_ph}"
            );
            let mut params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = values
                .iter()
                .map(|v| v as &(dyn tokio_postgres::types::ToSql + Sync))
                .collect();
            params.push(&limit);
            params.push(&offset);
            match client.query(&sql, &params).await {
                Ok(rows) => Ok(rows.iter().map(pg_row_to_message).collect()),
                Err(e) => {
                    tracing::warn!("postgres FTS query failed, falling back to ILIKE: {}", e);
                    pg_search_ilike(&client, session_id, query, limit, offset).await
                }
            }
        }
        DbPool::MySQL(my) => {
            use mysql_async::prelude::*;
            let mut conn = my.get_conn().await?;
            let mut values: Vec<String> = vec![query.to_string()];
            let mut where_sql = "MATCH(body) AGAINST(? IN NATURAL LANGUAGE MODE)".to_string();
            if let Some(sid) = session_id {
                values.push(sid.to_string());
                where_sql.push_str(" AND session_id = ?");
            }
            let sql = format!(
                "SELECT {COLS}, NULL AS snippet FROM messages WHERE {where_sql} ORDER BY msg_timestamp DESC, id DESC LIMIT ? OFFSET ?"
            );
            let mut params: Vec<mysql_async::Value> = values.into_iter().map(Into::into).collect();
            params.push(limit.into());
            params.push(offset.into());
            match conn.exec::<mysql_async::Row, _, _>(sql, params).await {
                Ok(rows) => Ok(rows.iter().map(my_row_to_message).collect()),
                Err(e) => {
                    tracing::warn!("mysql FULLTEXT query failed, falling back to LIKE: {}", e);
                    let (like, like_params) = like_query(pool, session_id, query, limit, offset);
                    let like_params: Vec<mysql_async::Value> = like_params
                        .into_iter()
                        .map(|v| match v {
                            LikeValue::Text(s) => s.into(),
                            LikeValue::Int(i) => i.into(),
                        })
                        .collect();
                    let rows: Vec<mysql_async::Row> = conn.exec(like, like_params).await?;
                    Ok(rows.iter().map(my_row_to_message).collect())
                }
            }
        }
        DbPool::SQLite(handle) => {
            let fts_query = fts5_match_query(query);
            if !fts_query.is_empty() {
                let mut values: Vec<SQ> = vec![SQ::Text(fts_query)];
                let mut where_sql = "messages_fts MATCH ?".to_string();
                if let Some(sid) = session_id {
                    values.push(SQ::Text(sid.to_string()));
                    where_sql.push_str(" AND f.session_id = ?");
                }
                let m_cols = "m.id, m.message_id, m.session_id, m.chat_jid, m.sender_jid, m.direction, m.msg_type, m.body, m.msg_timestamp";
                let sql = format!(
                    "SELECT {m_cols}, snippet(messages_fts, 0, '<b>', '</b>', '…', 16) FROM messages_fts f JOIN messages m ON m.session_id = f.session_id AND m.message_id = f.message_id WHERE {where_sql} ORDER BY m.msg_timestamp DESC, m.id DESC LIMIT ? OFFSET ?"
                );
                values.push(SQ::Int(limit));
                values.push(SQ::Int(offset));
                let attempt = sqlite_blocking(handle, move |conn| {
                    sqlite_raw::query(conn, &sql, &values, sqlite_row_to_message)
                })
                .await;
                match attempt {
                    Ok(rows) => return Ok(rows),
                    Err(e) => {
                        tracing::warn!("sqlite FTS5 query failed, falling back to LIKE: {}", e);
                    }
                }
            }
            let (sql, values) = like_query(pool, session_id, query, limit, offset);
            let values: Vec<SQ> = values
                .into_iter()
                .map(|v| match v {
                    LikeValue::Text(s) => SQ::Text(s),
                    LikeValue::Int(i) => SQ::Int(i),
                })
                .collect();
            sqlite_blocking(handle, move |conn| {
                sqlite_raw::query(conn, &sql, &values, sqlite_row_to_message)
            })
            .await
        }
    }
}

/// Postgres ILIKE fallback, broken out because the primary path borrows
/// the pooled client already.
async fn pg_search_ilike(
    client: &deadpool_postgres::Client,
    session_id: Option<&str>,
    query: &str,
    limit: i64,
    offset: i64,
) -> anyhow::Result<Vec<MessageRow>> {
    let pattern = like_pattern(query);
    let mut values: Vec<String> = vec![pattern];
    let mut where_sql = "body ILIKE $1 ESCAPE '\\'".to_string();
    if let Some(sid) = session_id {
        values.push(sid.to_string());
        where_sql.push_str(&format!(" AND session_id = ${}", values.len()));
    }
    let (limit_ph, offset_ph) = (values.len() + 1, values.len() + 2);
    let sql = format!(
        "SELECT {COLS}, NULL AS snippet FROM messages WHERE {where_sql} ORDER BY msg_timestamp DESC, id DESC LIMIT ${limit_ph} OFFSET ${offset_ph}"
    );
    let mut params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = values
        .iter()
        .map(|v| v as &(dyn tokio_postgres::types::ToSql + Sync))
        .collect();
    params.push(&limit);
    params.push(&offset);
    let rows = client.query(&sql, &params).await?;
    Ok(rows.iter().map(pg_row_to_message).collect())
}

/// Values for a LIKE fallback query, kept backend-neutral so SQLite
/// and MySQL can share the builder.
enum LikeValue {
    Text(String),
    Int(i64),
}

/// Build the LIKE fallback SQL + params for SQLite/MySQL (`?`
/// placeholders). Caller converts [`LikeValue`] into driver values.
fn like_query(
    pool: &DbPool,
    session_id: Option<&str>,
    query: &str,
    limit: i64,
    offset: i64,
) -> (String, Vec<LikeValue>) {
    let mut values: Vec<LikeValue> = vec![LikeValue::Text(like_pattern(query))];
    let mut where_sql = match pool {
        DbPool::MySQL(_) | DbPool::SQLite(_) => "body LIKE ? ESCAPE '\\'".to_string(),
        DbPool::Postgres(_) => unreachable!("postgres fallback uses pg_search_ilike"),
    };
    if let Some(sid) = session_id {
        values.push(LikeValue::Text(sid.to_string()));
        where_sql.push_str(" AND session_id = ?");
    }
    values.push(LikeValue::Int(limit));
    values.push(LikeValue::Int(offset));
    let sql = format!(
        "SELECT {COLS}, NULL AS snippet FROM messages WHERE {where_sql} ORDER BY msg_timestamp DESC, id DESC LIMIT ? OFFSET ?"
    );
    (sql, values)
}

/// Escape a user query for `LIKE … ESCAPE '\'` and wrap it in `%…%`.
fn like_pattern(query: &str) -> String {
    let escaped = query
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("%{escaped}%")
}

/// Turn a free-form user query into a safe FTS5 MATCH expression:
/// whitespace-split tokens, double-quotes stripped, each token
/// phrase-quoted, implicitly ANDed. Empty result means "nothing safe
/// to match" and the caller should go straight to the LIKE path.
fn fts5_match_query(query: &str) -> String {
    query
        .split_whitespace()
        .map(|t| t.replace('"', ""))
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{t}\""))
        .collect::<Vec<_>>()
        .join(" ")
}

fn pg_row_to_message(row: &tokio_postgres::Row) -> MessageRow {
    MessageRow {
        id: row.get("id"),
        message_id: row.get("message_id"),
        session_id: row.get("session_id"),
        chat_jid: row.get("chat_jid"),
        sender_jid: row.get("sender_jid"),
        direction: row.get("direction"),
        msg_type: row.get("msg_type"),
        body: row.get("body"),
        msg_timestamp: fmt_utc(row.get::<_, chrono::DateTime<chrono::Utc>>("msg_timestamp")),
        snippet: row.get("snippet"),
    }
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

fn my_get_i64(row: &mysql_async::Row, col: &str) -> i64 {
    use mysql_async::Value;
    let Some(idx) = row.columns_ref().iter().position(|c| c.name_str() == col) else {
        return 0;
    };
    match row.as_ref(idx) {
        Some(Value::Int(i)) => *i,
        Some(Value::UInt(u)) => *u as i64,
        _ => 0,
    }
}

fn my_row_to_message(row: &mysql_async::Row) -> MessageRow {
    MessageRow {
        id: my_get_i64(row, "id"),
        message_id: my_get_string(row, "message_id").unwrap_or_default(),
        session_id: my_get_string(row, "session_id").unwrap_or_default(),
        chat_jid: my_get_string(row, "chat_jid").unwrap_or_default(),
        sender_jid: my_get_string(row, "sender_jid").unwrap_or_default(),
        direction: my_get_string(row, "direction").unwrap_or_default(),
        msg_type: my_get_string(row, "msg_type").unwrap_or_default(),
        body: my_get_string(row, "body"),
        msg_timestamp: my_get_string(row, "msg_timestamp").unwrap_or_default(),
        snippet: my_get_string(row, "snippet"),
    }
}

fn sqlite_row_to_message(row: &sqlite_raw::Row) -> MessageRow {
    MessageRow {
        id: row.get_int(0),
        message_id: row.get_string(1).unwrap_or_default(),
        session_id: row.get_string(2).unwrap_or_default(),
        chat_jid: row.get_string(3).unwrap_or_default(),
        sender_jid: row.get_string(4).unwrap_or_default(),
        direction: row.get_string(5).unwrap_or_default(),
        msg_type: row.get_string(6).unwrap_or_default(),
        body: row.get_string(7),
        msg_timestamp: row.get_string(8).unwrap_or_default(),
        snippet: row.get_string(9),
    }
}
