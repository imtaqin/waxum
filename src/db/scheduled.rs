//! Persistence for the scheduled-send feature.
//!
//! One row per parked send request: the endpoint key (e.g. `text`,
//! `cta-url`), the original JSON body, and the UTC due time. The
//! background scheduler in [`crate::handlers::schedule`] polls
//! [`due_pending`], claims a row via [`claim`] (pending → sending), and
//! settles it with [`mark_sent`] / [`mark_failed`]. The management
//! endpoints use [`list`], [`get`] and [`cancel_pending`].
//!
//! Timestamps are stored as `%Y-%m-%d %H:%M:%S` UTC text on SQLite and
//! MySQL (lexicographically comparable, matching the house style) and
//! as `TIMESTAMPTZ` on Postgres, formatted to the same text on the way
//! out so [`ScheduledRow`] is backend-agnostic.

use crate::db::session::{sqlite_blocking, DbPool};
use crate::db::sqlite_raw::{self, Value as SQ};

const COLS: &str =
    "id, session_id, endpoint, body, send_at, status, error, message_id, created_at, updated_at";

fn now_str() -> String {
    chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

fn fmt_utc(dt: chrono::DateTime<chrono::Utc>) -> String {
    dt.format("%Y-%m-%d %H:%M:%S").to_string()
}

/// A `scheduled_messages` row with all timestamps rendered as
/// `%Y-%m-%d %H:%M:%S` UTC text regardless of backend.
#[derive(Debug, Clone)]
pub struct ScheduledRow {
    pub id: String,
    pub session_id: String,
    pub endpoint: String,
    pub body: String,
    pub send_at: String,
    pub status: String,
    pub error: Option<String>,
    pub message_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Park a send request for later dispatch. Unlike the webhook DLQ this
/// is NOT best-effort: the caller has promised the client a schedule,
/// so a failed insert must surface as an API error.
pub async fn insert(
    pool: &DbPool,
    id: &str,
    session_id: &str,
    endpoint: &str,
    body: &str,
    send_at: chrono::DateTime<chrono::Utc>,
) -> anyhow::Result<()> {
    match pool {
        DbPool::Postgres(pg) => {
            let client = pg.get().await?;
            client
                .execute(
                    "INSERT INTO scheduled_messages (id, session_id, endpoint, body, send_at, status, created_at, updated_at) VALUES ($1,$2,$3,$4,$5,'pending',NOW(),NOW())",
                    &[&id, &session_id, &endpoint, &body, &send_at],
                )
                .await?;
        }
        DbPool::MySQL(my) => {
            use mysql_async::prelude::*;
            let mut conn = my.get_conn().await?;
            let send_at_s = fmt_utc(send_at);
            let now = now_str();
            conn.exec_drop(
                "INSERT INTO scheduled_messages (id, session_id, endpoint, body, send_at, status, created_at, updated_at) VALUES (?,?,?,?,?,'pending',?,?)",
                (id, session_id, endpoint, body, send_at_s, &now, &now),
            )
            .await?;
        }
        DbPool::SQLite(handle) => {
            let id = id.to_string();
            let sid = session_id.to_string();
            let ep = endpoint.to_string();
            let body = body.to_string();
            let send_at_s = fmt_utc(send_at);
            let now = now_str();
            let now2 = now.clone();
            sqlite_blocking(handle, move |conn| {
                sqlite_raw::execute(
                    conn,
                    "INSERT INTO scheduled_messages (id, session_id, endpoint, body, send_at, status, created_at, updated_at) VALUES (?,?,?,?,?,'pending',?,?)",
                    &[
                        SQ::Text(id),
                        SQ::Text(sid),
                        SQ::Text(ep),
                        SQ::Text(body),
                        SQ::Text(send_at_s),
                        SQ::Text(now),
                        SQ::Text(now2),
                    ],
                )?;
                Ok(())
            })
            .await?;
        }
    }
    Ok(())
}

/// Rows due for dispatch: status `pending` and `send_at` at or before
/// now, oldest first, capped at `limit`.
pub async fn due_pending(pool: &DbPool, limit: i64) -> anyhow::Result<Vec<ScheduledRow>> {
    match pool {
        DbPool::Postgres(pg) => {
            let client = pg.get().await?;
            let rows = client
                .query(
                    &format!(
                        "SELECT {COLS} FROM scheduled_messages WHERE status = 'pending' AND send_at <= NOW() ORDER BY send_at ASC LIMIT $1"
                    ),
                    &[&limit],
                )
                .await?;
            Ok(rows.iter().map(pg_row_to_scheduled).collect())
        }
        DbPool::MySQL(my) => {
            use mysql_async::prelude::*;
            let mut conn = my.get_conn().await?;
            let now = now_str();
            let rows: Vec<mysql_async::Row> = conn
                .exec(
                    format!(
                        "SELECT {COLS} FROM scheduled_messages WHERE status = 'pending' AND send_at <= ? ORDER BY send_at ASC LIMIT ?"
                    ),
                    (now, limit),
                )
                .await?;
            Ok(rows.iter().map(my_row_to_scheduled).collect())
        }
        DbPool::SQLite(handle) => {
            let now = now_str();
            sqlite_blocking(handle, move |conn| {
                sqlite_raw::query(
                    conn,
                    &format!(
                        "SELECT {COLS} FROM scheduled_messages WHERE status = 'pending' AND send_at <= ? ORDER BY send_at ASC LIMIT ?"
                    ),
                    &[SQ::Text(now), SQ::Int(limit)],
                    sqlite_row_to_scheduled,
                )
            })
            .await
        }
    }
}

/// Atomically move a row from `pending` to `sending`. Returns false
/// when the row was concurrently cancelled or already claimed, in
/// which case the caller must skip it.
pub async fn claim(pool: &DbPool, id: &str) -> anyhow::Result<bool> {
    let changed = match pool {
        DbPool::Postgres(pg) => {
            let client = pg.get().await?;
            client
                .execute(
                    "UPDATE scheduled_messages SET status = 'sending', updated_at = NOW() WHERE id = $1 AND status = 'pending'",
                    &[&id],
                )
                .await?
        }
        DbPool::MySQL(my) => {
            use mysql_async::prelude::*;
            let mut conn = my.get_conn().await?;
            let now = now_str();
            conn.exec_drop(
                "UPDATE scheduled_messages SET status = 'sending', updated_at = ? WHERE id = ? AND status = 'pending'",
                (now, id),
            )
            .await?;
            conn.affected_rows()
        }
        DbPool::SQLite(handle) => {
            let id_s = id.to_string();
            let now = now_str();
            sqlite_blocking(handle, move |conn| {
                sqlite_raw::execute(
                    conn,
                    "UPDATE scheduled_messages SET status = 'sending', updated_at = ? WHERE id = ? AND status = 'pending'",
                    &[SQ::Text(now), SQ::Text(id_s)],
                )
            })
            .await?
        }
    };
    Ok(changed > 0)
}

/// Settle a claimed row as delivered, recording the WhatsApp message id.
pub async fn mark_sent(pool: &DbPool, id: &str, message_id: &str) -> anyhow::Result<()> {
    match pool {
        DbPool::Postgres(pg) => {
            let client = pg.get().await?;
            client
                .execute(
                    "UPDATE scheduled_messages SET status = 'sent', message_id = $1, error = NULL, updated_at = NOW() WHERE id = $2",
                    &[&message_id, &id],
                )
                .await?;
        }
        DbPool::MySQL(my) => {
            use mysql_async::prelude::*;
            let mut conn = my.get_conn().await?;
            let now = now_str();
            conn.exec_drop(
                "UPDATE scheduled_messages SET status = 'sent', message_id = ?, error = NULL, updated_at = ? WHERE id = ?",
                (message_id, now, id),
            )
            .await?;
        }
        DbPool::SQLite(handle) => {
            let id_s = id.to_string();
            let mid = message_id.to_string();
            let now = now_str();
            sqlite_blocking(handle, move |conn| {
                sqlite_raw::execute(
                    conn,
                    "UPDATE scheduled_messages SET status = 'sent', message_id = ?, error = NULL, updated_at = ? WHERE id = ?",
                    &[SQ::Text(mid), SQ::Text(now), SQ::Text(id_s)],
                )?;
                Ok(())
            })
            .await?;
        }
    }
    Ok(())
}

/// Settle a claimed row as failed, recording the dispatch error.
pub async fn mark_failed(pool: &DbPool, id: &str, error: &str) -> anyhow::Result<()> {
    match pool {
        DbPool::Postgres(pg) => {
            let client = pg.get().await?;
            client
                .execute(
                    "UPDATE scheduled_messages SET status = 'failed', error = $1, updated_at = NOW() WHERE id = $2",
                    &[&error, &id],
                )
                .await?;
        }
        DbPool::MySQL(my) => {
            use mysql_async::prelude::*;
            let mut conn = my.get_conn().await?;
            let now = now_str();
            conn.exec_drop(
                "UPDATE scheduled_messages SET status = 'failed', error = ?, updated_at = ? WHERE id = ?",
                (error, now, id),
            )
            .await?;
        }
        DbPool::SQLite(handle) => {
            let id_s = id.to_string();
            let err = error.to_string();
            let now = now_str();
            sqlite_blocking(handle, move |conn| {
                sqlite_raw::execute(
                    conn,
                    "UPDATE scheduled_messages SET status = 'failed', error = ?, updated_at = ? WHERE id = ?",
                    &[SQ::Text(err), SQ::Text(now), SQ::Text(id_s)],
                )?;
                Ok(())
            })
            .await?;
        }
    }
    Ok(())
}

/// Fetch one row scoped to a session (management endpoints).
pub async fn get(
    pool: &DbPool,
    session_id: &str,
    id: &str,
) -> anyhow::Result<Option<ScheduledRow>> {
    match pool {
        DbPool::Postgres(pg) => {
            let client = pg.get().await?;
            let row = client
                .query_opt(
                    &format!(
                        "SELECT {COLS} FROM scheduled_messages WHERE session_id = $1 AND id = $2"
                    ),
                    &[&session_id, &id],
                )
                .await?;
            Ok(row.as_ref().map(pg_row_to_scheduled))
        }
        DbPool::MySQL(my) => {
            use mysql_async::prelude::*;
            let mut conn = my.get_conn().await?;
            let row: Option<mysql_async::Row> = conn
                .exec_first(
                    format!(
                        "SELECT {COLS} FROM scheduled_messages WHERE session_id = ? AND id = ?"
                    ),
                    (session_id, id),
                )
                .await?;
            Ok(row.as_ref().map(my_row_to_scheduled))
        }
        DbPool::SQLite(handle) => {
            let sid = session_id.to_string();
            let id_s = id.to_string();
            sqlite_blocking(handle, move |conn| {
                let mut out = sqlite_raw::query(
                    conn,
                    &format!(
                        "SELECT {COLS} FROM scheduled_messages WHERE session_id = ? AND id = ?"
                    ),
                    &[SQ::Text(sid), SQ::Text(id_s)],
                    sqlite_row_to_scheduled,
                )?;
                Ok(out.pop())
            })
            .await
        }
    }
}

/// Cancel a row that is still `pending`. Returns false when the row is
/// absent or no longer pending (caller distinguishes 404 vs 400 via
/// [`get`]).
pub async fn cancel_pending(pool: &DbPool, session_id: &str, id: &str) -> anyhow::Result<bool> {
    let changed = match pool {
        DbPool::Postgres(pg) => {
            let client = pg.get().await?;
            client
                .execute(
                    "UPDATE scheduled_messages SET status = 'cancelled', updated_at = NOW() WHERE session_id = $1 AND id = $2 AND status = 'pending'",
                    &[&session_id, &id],
                )
                .await?
        }
        DbPool::MySQL(my) => {
            use mysql_async::prelude::*;
            let mut conn = my.get_conn().await?;
            let now = now_str();
            conn.exec_drop(
                "UPDATE scheduled_messages SET status = 'cancelled', updated_at = ? WHERE session_id = ? AND id = ? AND status = 'pending'",
                (now, session_id, id),
            )
            .await?;
            conn.affected_rows()
        }
        DbPool::SQLite(handle) => {
            let sid = session_id.to_string();
            let id_s = id.to_string();
            let now = now_str();
            sqlite_blocking(handle, move |conn| {
                sqlite_raw::execute(
                    conn,
                    "UPDATE scheduled_messages SET status = 'cancelled', updated_at = ? WHERE session_id = ? AND id = ? AND status = 'pending'",
                    &[SQ::Text(now), SQ::Text(sid), SQ::Text(id_s)],
                )
            })
            .await?
        }
    };
    Ok(changed > 0)
}

/// List rows with optional session/status filters, oldest `send_at`
/// first. Both filters absent = fleet-wide listing.
pub async fn list(
    pool: &DbPool,
    session_id: Option<&str>,
    status: Option<&str>,
) -> anyhow::Result<Vec<ScheduledRow>> {
    let mut clauses: Vec<String> = Vec::new();
    let mut values: Vec<String> = Vec::new();
    if let Some(s) = session_id {
        clauses.push(format!("session_id = {}", placeholder(pool, values.len())));
        values.push(s.to_string());
    }
    if let Some(s) = status {
        clauses.push(format!("status = {}", placeholder(pool, values.len())));
        values.push(s.to_string());
    }
    let mut sql = format!("SELECT {COLS} FROM scheduled_messages");
    if !clauses.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&clauses.join(" AND "));
    }
    sql.push_str(" ORDER BY send_at ASC");

    match pool {
        DbPool::Postgres(pg) => {
            let client = pg.get().await?;
            let params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = values
                .iter()
                .map(|v| v as &(dyn tokio_postgres::types::ToSql + Sync))
                .collect();
            let rows = client.query(&sql, &params).await?;
            Ok(rows.iter().map(pg_row_to_scheduled).collect())
        }
        DbPool::MySQL(my) => {
            use mysql_async::prelude::*;
            let mut conn = my.get_conn().await?;
            let rows: Vec<mysql_async::Row> = conn.exec(sql, values).await?;
            Ok(rows.iter().map(my_row_to_scheduled).collect())
        }
        DbPool::SQLite(handle) => {
            let params: Vec<SQ> = values.into_iter().map(SQ::Text).collect();
            sqlite_blocking(handle, move |conn| {
                sqlite_raw::query(conn, &sql, &params, sqlite_row_to_scheduled)
            })
            .await
        }
    }
}

fn placeholder(pool: &DbPool, idx: usize) -> String {
    match pool {
        DbPool::Postgres(_) => format!("${}", idx + 1),
        DbPool::MySQL(_) | DbPool::SQLite(_) => "?".to_string(),
    }
}

fn pg_row_to_scheduled(row: &tokio_postgres::Row) -> ScheduledRow {
    ScheduledRow {
        id: row.get("id"),
        session_id: row.get("session_id"),
        endpoint: row.get("endpoint"),
        body: row.get("body"),
        send_at: fmt_utc(row.get::<_, chrono::DateTime<chrono::Utc>>("send_at")),
        status: row.get("status"),
        error: row.get("error"),
        message_id: row.get("message_id"),
        created_at: fmt_utc(row.get::<_, chrono::DateTime<chrono::Utc>>("created_at")),
        updated_at: fmt_utc(row.get::<_, chrono::DateTime<chrono::Utc>>("updated_at")),
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

fn my_row_to_scheduled(row: &mysql_async::Row) -> ScheduledRow {
    ScheduledRow {
        id: my_get_string(row, "id").unwrap_or_default(),
        session_id: my_get_string(row, "session_id").unwrap_or_default(),
        endpoint: my_get_string(row, "endpoint").unwrap_or_default(),
        body: my_get_string(row, "body").unwrap_or_default(),
        send_at: my_get_string(row, "send_at").unwrap_or_default(),
        status: my_get_string(row, "status").unwrap_or_default(),
        error: my_get_string(row, "error"),
        message_id: my_get_string(row, "message_id"),
        created_at: my_get_string(row, "created_at").unwrap_or_default(),
        updated_at: my_get_string(row, "updated_at").unwrap_or_default(),
    }
}

fn sqlite_row_to_scheduled(row: &sqlite_raw::Row) -> ScheduledRow {
    ScheduledRow {
        id: row.get_string(0).unwrap_or_default(),
        session_id: row.get_string(1).unwrap_or_default(),
        endpoint: row.get_string(2).unwrap_or_default(),
        body: row.get_string(3).unwrap_or_default(),
        send_at: row.get_string(4).unwrap_or_default(),
        status: row.get_string(5).unwrap_or_default(),
        error: row.get_string(6),
        message_id: row.get_string(7),
        created_at: row.get_string(8).unwrap_or_default(),
        updated_at: row.get_string(9).unwrap_or_default(),
    }
}
