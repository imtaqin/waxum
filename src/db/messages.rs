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

const COLS: &str = "id, message_id, session_id, chat_jid, sender_jid, direction, msg_type, body, msg_timestamp, quoted_message_id, quoted_sender_jid";

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
    /// Download pointers for media message types, `None` for
    /// text/location/contact/etc. All base64-encoded except
    /// `direct_path`, `media_type`, and `mimetype` — the same shapes
    /// [`crate::models::media::DownloadMediaRequest`] expects, so a
    /// caller can round-trip these fields straight into
    /// `POST /media/download` with no re-encoding.
    pub media: Option<MediaPointer>,
    /// `ContextInfo.stanzaId` when this message is a reply, `None`
    /// otherwise. See [`crate::handlers::sessions::extract_quoted_context`].
    pub quoted_message_id: Option<String>,
    /// `ContextInfo.participant` — the quoted message's sender — when
    /// this message is a reply and the field was present on the wire.
    pub quoted_sender_jid: Option<String>,
}

/// Download pointer for one media message, captured at ingestion time
/// since `/media/download` needs the encryption key and hashes that
/// only exist on the original message — they are never re-derivable
/// from a stored history row otherwise.
#[derive(Debug, Clone, Default)]
pub struct MediaPointer {
    pub media_key: String,
    pub file_sha256: String,
    pub file_enc_sha256: String,
    pub direct_path: String,
    pub file_length: i64,
    /// One of `image`, `video`, `audio`, `document`, `sticker` —
    /// matches `crate::models::media::MediaType`'s snake_case wire
    /// form.
    pub media_type: String,
    pub mimetype: String,
}

/// A `messages` row as returned by search or chat listing, timestamps
/// rendered as `%Y-%m-%d %H:%M:%S` UTC text regardless of backend.
/// `snippet` is only populated by backends with cheap highlight
/// support (SQLite FTS5, Postgres) and only by [`search`]. `media` and
/// `push_name` are only populated by [`list_by_chat`] — `search`'s
/// queries don't select those columns, so its rows always carry `None`
/// there. `quoted_message_id`/`quoted_sender_jid` are populated by
/// both.
#[derive(Debug, Clone, Default)]
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
    pub media: Option<MediaPointer>,
    pub push_name: Option<String>,
    /// `ContextInfo.stanzaId` when this message is a reply.
    pub quoted_message_id: Option<String>,
    /// `ContextInfo.participant` — the quoted message's sender.
    pub quoted_sender_jid: Option<String>,
}

/// Store one message, ignoring duplicates on `(session_id,
/// message_id)` (history-sync replays and event re-deliveries repeat
/// ids). On SQLite the FTS index is synced in the same blocking call
/// when the row was actually new; FTS-insert errors are swallowed so a
/// broken index never blocks ingestion.
pub async fn insert(pool: &DbPool, msg: &NewMessage) -> anyhow::Result<()> {
    let media = msg.media.as_ref();
    let media_key = media.map(|m| m.media_key.as_str());
    let file_sha256 = media.map(|m| m.file_sha256.as_str());
    let file_enc_sha256 = media.map(|m| m.file_enc_sha256.as_str());
    let direct_path = media.map(|m| m.direct_path.as_str());
    let file_length = media.map(|m| m.file_length);
    let media_type = media.map(|m| m.media_type.as_str());
    let mimetype = media.map(|m| m.mimetype.as_str());

    match pool {
        DbPool::Postgres(pg) => {
            let client = pg.get().await?;
            client
                .execute(
                    "INSERT INTO messages (message_id, session_id, chat_jid, sender_jid, direction, msg_type, body, msg_timestamp, media_key, file_sha256, file_enc_sha256, direct_path, file_length, media_type, mimetype, quoted_message_id, quoted_sender_jid) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17) ON CONFLICT (session_id, message_id) DO NOTHING",
                    &[
                        &msg.message_id,
                        &msg.session_id,
                        &msg.chat_jid,
                        &msg.sender_jid,
                        &msg.direction,
                        &msg.msg_type,
                        &msg.body,
                        &msg.msg_timestamp,
                        &media_key,
                        &file_sha256,
                        &file_enc_sha256,
                        &direct_path,
                        &file_length,
                        &media_type,
                        &mimetype,
                        &msg.quoted_message_id,
                        &msg.quoted_sender_jid,
                    ],
                )
                .await?;
        }
        DbPool::MySQL(my) => {
            use mysql_async::prelude::*;
            use mysql_async::Value as MyValue;
            let mut conn = my.get_conn().await?;
            let ts = fmt_utc(msg.msg_timestamp);
            let now = now_str();
            let params: Vec<MyValue> = vec![
                msg.message_id.clone().into(),
                msg.session_id.clone().into(),
                msg.chat_jid.clone().into(),
                msg.sender_jid.clone().into(),
                msg.direction.clone().into(),
                msg.msg_type.clone().into(),
                msg.body.clone().into(),
                ts.into(),
                now.into(),
                media_key.map(str::to_string).into(),
                file_sha256.map(str::to_string).into(),
                file_enc_sha256.map(str::to_string).into(),
                direct_path.map(str::to_string).into(),
                file_length.into(),
                media_type.map(str::to_string).into(),
                mimetype.map(str::to_string).into(),
                msg.quoted_message_id.clone().into(),
                msg.quoted_sender_jid.clone().into(),
            ];
            conn.exec_drop(
                "INSERT IGNORE INTO messages (message_id, session_id, chat_jid, sender_jid, direction, msg_type, body, msg_timestamp, created_at, media_key, file_sha256, file_enc_sha256, direct_path, file_length, media_type, mimetype, quoted_message_id, quoted_sender_jid) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                mysql_async::Params::Positional(params),
            )
            .await?;
        }
        DbPool::SQLite(handle) => {
            let m = msg.clone();
            let ts = fmt_utc(m.msg_timestamp);
            let now = now_str();
            let media_key = media_key.map(str::to_string);
            let file_sha256 = file_sha256.map(str::to_string);
            let file_enc_sha256 = file_enc_sha256.map(str::to_string);
            let direct_path = direct_path.map(str::to_string);
            let media_type = media_type.map(str::to_string);
            let mimetype = mimetype.map(str::to_string);
            sqlite_blocking(handle, move |conn| {
                let file_length_value = match file_length {
                    Some(n) => SQ::Int(n),
                    None => SQ::Null,
                };
                let changed = sqlite_raw::execute(
                    conn,
                    "INSERT OR IGNORE INTO messages (message_id, session_id, chat_jid, sender_jid, direction, msg_type, body, msg_timestamp, created_at, media_key, file_sha256, file_enc_sha256, direct_path, file_length, media_type, mimetype, quoted_message_id, quoted_sender_jid) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
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
                        SQ::from_opt_str(media_key.as_deref()),
                        SQ::from_opt_str(file_sha256.as_deref()),
                        SQ::from_opt_str(file_enc_sha256.as_deref()),
                        SQ::from_opt_str(direct_path.as_deref()),
                        file_length_value,
                        SQ::from_opt_str(media_type.as_deref()),
                        SQ::from_opt_str(mimetype.as_deref()),
                        SQ::from_opt_str(m.quoted_message_id.as_deref()),
                        SQ::from_opt_str(m.quoted_sender_jid.as_deref()),
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
                let m_cols = "m.id, m.message_id, m.session_id, m.chat_jid, m.sender_jid, m.direction, m.msg_type, m.body, m.msg_timestamp, m.quoted_message_id, m.quoted_sender_jid";
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

/// List one chat's history, newest first, no search term required —
/// the counterpart to [`search`] for a plain "show me this chat"
/// read. Rows carry the media download pointer (when the message was
/// media) and the sender's `push_name` via a `LEFT JOIN` against
/// `contacts`, both `None`/absent from [`search`]'s rows.
///
/// Every column across the join is qualified (`m.`/`c.`) even though
/// today's `contacts`/`messages` schemas don't collide outside
/// `session_id` — the FTS join already bit this project once with an
/// "ambiguous column name" failure (see `tests/fts_probe.rs`), so new
/// joins qualify unconditionally rather than relying on the current
/// column set staying disjoint.
pub async fn list_by_chat(
    pool: &DbPool,
    session_id: &str,
    chat_jid: &str,
    limit: i64,
    offset: i64,
) -> anyhow::Result<Vec<MessageRow>> {
    let session_id = session_id.to_string();
    let chat_jid = chat_jid.to_string();
    match pool {
        DbPool::Postgres(pg) => {
            let client = pg.get().await?;
            let sql = "SELECT m.id, m.message_id, m.session_id, m.chat_jid, m.sender_jid, m.direction, m.msg_type, m.body, m.msg_timestamp, m.media_key, m.file_sha256, m.file_enc_sha256, m.direct_path, m.file_length, m.media_type, m.mimetype, m.quoted_message_id, m.quoted_sender_jid, c.push_name FROM messages m LEFT JOIN contacts c ON c.session_id = m.session_id AND c.jid = m.sender_jid WHERE m.session_id = $1 AND m.chat_jid = $2 ORDER BY m.msg_timestamp DESC, m.id DESC LIMIT $3 OFFSET $4";
            let rows = client
                .query(sql, &[&session_id, &chat_jid, &limit, &offset])
                .await?;
            Ok(rows.iter().map(pg_row_to_chat_message).collect())
        }
        DbPool::MySQL(my) => {
            use mysql_async::prelude::*;
            let mut conn = my.get_conn().await?;
            let sql = "SELECT m.id, m.message_id, m.session_id, m.chat_jid, m.sender_jid, m.direction, m.msg_type, m.body, m.msg_timestamp, m.media_key, m.file_sha256, m.file_enc_sha256, m.direct_path, m.file_length, m.media_type, m.mimetype, m.quoted_message_id, m.quoted_sender_jid, c.push_name FROM messages m LEFT JOIN contacts c ON c.session_id = m.session_id AND c.jid = m.sender_jid WHERE m.session_id = ? AND m.chat_jid = ? ORDER BY m.msg_timestamp DESC, m.id DESC LIMIT ? OFFSET ?";
            let rows: Vec<mysql_async::Row> = conn
                .exec(sql, (session_id, chat_jid, limit, offset))
                .await?;
            Ok(rows.iter().map(my_row_to_chat_message).collect())
        }
        DbPool::SQLite(handle) => {
            let sql = "SELECT m.id, m.message_id, m.session_id, m.chat_jid, m.sender_jid, m.direction, m.msg_type, m.body, m.msg_timestamp, m.media_key, m.file_sha256, m.file_enc_sha256, m.direct_path, m.file_length, m.media_type, m.mimetype, m.quoted_message_id, m.quoted_sender_jid, c.push_name FROM messages m LEFT JOIN contacts c ON c.session_id = m.session_id AND c.jid = m.sender_jid WHERE m.session_id = ? AND m.chat_jid = ? ORDER BY m.msg_timestamp DESC, m.id DESC LIMIT ? OFFSET ?";
            let values = vec![
                SQ::Text(session_id),
                SQ::Text(chat_jid),
                SQ::Int(limit),
                SQ::Int(offset),
            ];
            sqlite_blocking(handle, move |conn| {
                sqlite_raw::query(conn, sql, &values, sqlite_row_to_chat_message)
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
///
/// The `ESCAPE` clause's backslash differs per dialect: SQLite's string
/// literals don't process backslash escapes at all, so the SQL text's
/// single `\` is already the one literal backslash character
/// [`like_pattern`] escaped the value with. MySQL *does* interpret
/// backslash escapes inside string literals by default -- a lone `\`
/// immediately before the closing quote would escape that quote instead
/// of terminating the string, so its SQL text needs `\\` (MySQL's own
/// escape sequence for one literal backslash) instead.
fn like_query(
    pool: &DbPool,
    session_id: Option<&str>,
    query: &str,
    limit: i64,
    offset: i64,
) -> (String, Vec<LikeValue>) {
    let mut values: Vec<LikeValue> = vec![LikeValue::Text(like_pattern(query))];
    let mut where_sql = match pool {
        DbPool::SQLite(_) => "body LIKE ? ESCAPE '\\'".to_string(),
        DbPool::MySQL(_) => "body LIKE ? ESCAPE '\\\\'".to_string(),
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
pub(crate) fn like_pattern(query: &str) -> String {
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
        media: None,
        push_name: None,
        quoted_message_id: row.get("quoted_message_id"),
        quoted_sender_jid: row.get("quoted_sender_jid"),
    }
}

fn pg_row_to_chat_message(row: &tokio_postgres::Row) -> MessageRow {
    let media = row
        .get::<_, Option<String>>("media_key")
        .map(|media_key| MediaPointer {
            media_key,
            file_sha256: row.get("file_sha256"),
            file_enc_sha256: row.get("file_enc_sha256"),
            direct_path: row.get("direct_path"),
            file_length: row.get::<_, Option<i64>>("file_length").unwrap_or(0),
            media_type: row.get("media_type"),
            mimetype: row.get("mimetype"),
        });
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
        snippet: None,
        media,
        push_name: row.get("push_name"),
        quoted_message_id: row.get("quoted_message_id"),
        quoted_sender_jid: row.get("quoted_sender_jid"),
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
    my_get_opt_i64(row, col).unwrap_or(0)
}

fn my_get_opt_i64(row: &mysql_async::Row, col: &str) -> Option<i64> {
    use mysql_async::Value;
    let idx = row.columns_ref().iter().position(|c| c.name_str() == col)?;
    match row.as_ref(idx)? {
        Value::Int(i) => Some(*i),
        Value::UInt(u) => Some(*u as i64),
        _ => None,
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
        media: None,
        push_name: None,
        quoted_message_id: my_get_string(row, "quoted_message_id"),
        quoted_sender_jid: my_get_string(row, "quoted_sender_jid"),
    }
}

fn my_row_to_chat_message(row: &mysql_async::Row) -> MessageRow {
    let media = my_get_string(row, "media_key").map(|media_key| MediaPointer {
        media_key,
        file_sha256: my_get_string(row, "file_sha256").unwrap_or_default(),
        file_enc_sha256: my_get_string(row, "file_enc_sha256").unwrap_or_default(),
        direct_path: my_get_string(row, "direct_path").unwrap_or_default(),
        file_length: my_get_opt_i64(row, "file_length").unwrap_or(0),
        media_type: my_get_string(row, "media_type").unwrap_or_default(),
        mimetype: my_get_string(row, "mimetype").unwrap_or_default(),
    });
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
        snippet: None,
        media,
        push_name: my_get_string(row, "push_name"),
        quoted_message_id: my_get_string(row, "quoted_message_id"),
        quoted_sender_jid: my_get_string(row, "quoted_sender_jid"),
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
        quoted_message_id: row.get_string(9),
        quoted_sender_jid: row.get_string(10),
        snippet: row.get_string(11),
        media: None,
        push_name: None,
    }
}

/// Row shape for [`list_by_chat`]: the plain message columns (0-8),
/// the seven media pointer columns (9-15), the quote columns (16-17),
/// then `push_name` from the `contacts` join (18).
fn sqlite_row_to_chat_message(row: &sqlite_raw::Row) -> MessageRow {
    let media = row.get_string(9).map(|media_key| MediaPointer {
        media_key,
        file_sha256: row.get_string(10).unwrap_or_default(),
        file_enc_sha256: row.get_string(11).unwrap_or_default(),
        direct_path: row.get_string(12).unwrap_or_default(),
        file_length: row.get_int(13),
        media_type: row.get_string(14).unwrap_or_default(),
        mimetype: row.get_string(15).unwrap_or_default(),
    });
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
        snippet: None,
        media,
        quoted_message_id: row.get_string(16),
        quoted_sender_jid: row.get_string(17),
        push_name: row.get_string(18),
    }
}
