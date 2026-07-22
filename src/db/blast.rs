//! Persistence for the blast (bulk-send) feature.
//!
//! Two tables back it: `blast_jobs` (one row per campaign: endpoint
//! key, original JSON body, pacing options, counters, lifecycle
//! timestamps) and `blast_recipients` (one row per job/recipient pair,
//! `UNIQUE (job_id, recipient)`, with `session_id` denormalized so
//! cross-job dedup can find every recipient a session ever `sent` to).
//!
//! The background worker in [`crate::handlers::blast`] claims the
//! oldest due job via [`claim_next_due`] (or resumes an interrupted
//! `running` one via [`running_job`]), drains [`pending_recipients`] in
//! batches, settles each row with the `mark_recipient_*` family, bumps
//! job counters with the `incr_*` family, and closes the job with
//! [`finish_job`]. The management endpoints use [`list_jobs`],
//! [`get_job`], [`list_recipients`], [`cancel_job`] and the
//! [`requeue_failed`] / [`reopen_job`] pair behind retry.
//!
//! Timestamps follow the house style: `%Y-%m-%d %H:%M:%S` UTC text on
//! SQLite and MySQL, `TIMESTAMPTZ` on Postgres, formatted to the same
//! text on the way out so rows are backend-agnostic.

use std::collections::HashSet;

use crate::db::session::{sqlite_blocking, DbPool};
use crate::db::sqlite_raw::{self, Value as SQ};

const JOB_COLS: &str = "id, session_id, endpoint, body, options, status, total, sent_count, failed_count, dlq_count, skipped_dup_count, send_at, created_at, started_at, finished_at";

const REC_COLS: &str =
    "id, job_id, session_id, recipient, status, attempts, last_error, message_id, updated_at";

fn now_str() -> String {
    chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

fn fmt_utc(dt: chrono::DateTime<chrono::Utc>) -> String {
    dt.format("%Y-%m-%d %H:%M:%S").to_string()
}

/// A `blast_jobs` row with all timestamps rendered as
/// `%Y-%m-%d %H:%M:%S` UTC text regardless of backend.
#[derive(Debug, Clone)]
pub struct BlastJobRow {
    pub id: String,
    pub session_id: String,
    pub endpoint: String,
    pub body: String,
    pub options: String,
    pub status: String,
    pub total: i64,
    pub sent_count: i64,
    pub failed_count: i64,
    pub dlq_count: i64,
    pub skipped_dup_count: i64,
    pub send_at: Option<String>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

/// A `blast_recipients` row.
#[derive(Debug, Clone)]
pub struct BlastRecipientRow {
    pub id: i64,
    pub job_id: String,
    pub session_id: String,
    pub recipient: String,
    pub status: String,
    pub attempts: i64,
    pub last_error: Option<String>,
    pub message_id: Option<String>,
    pub updated_at: String,
}

/// Persist a freshly created job in `pending` status. Not best-effort:
/// the caller answers the client with the job id, so a failed insert
/// must surface as an API error.
#[allow(clippy::too_many_arguments)]
pub async fn insert_job(
    pool: &DbPool,
    id: &str,
    session_id: &str,
    endpoint: &str,
    body: &str,
    options: &str,
    total: i64,
    skipped_dup: i64,
    send_at: Option<chrono::DateTime<chrono::Utc>>,
) -> anyhow::Result<()> {
    match pool {
        DbPool::Postgres(pg) => {
            let client = pg.get().await?;
            client
                .execute(
                    "INSERT INTO blast_jobs (id, session_id, endpoint, body, options, status, total, skipped_dup_count, send_at, created_at) VALUES ($1,$2,$3,$4,$5,'pending',$6,$7,$8,NOW())",
                    &[
                        &id,
                        &session_id,
                        &endpoint,
                        &body,
                        &options,
                        &total,
                        &skipped_dup,
                        &send_at,
                    ],
                )
                .await?;
        }
        DbPool::MySQL(my) => {
            use mysql_async::prelude::*;
            let mut conn = my.get_conn().await?;
            let send_at_s = send_at.map(fmt_utc);
            let now = now_str();
            conn.exec_drop(
                "INSERT INTO blast_jobs (id, session_id, endpoint, body, options, status, total, skipped_dup_count, send_at, created_at) VALUES (?,?,?,?,?,'pending',?,?,?,?)",
                (
                    id,
                    session_id,
                    endpoint,
                    body,
                    options,
                    total,
                    skipped_dup,
                    send_at_s,
                    now,
                ),
            )
            .await?;
        }
        DbPool::SQLite(handle) => {
            let id = id.to_string();
            let sid = session_id.to_string();
            let ep = endpoint.to_string();
            let body = body.to_string();
            let opts = options.to_string();
            let send_at_s = send_at.map(fmt_utc);
            let now = now_str();
            sqlite_blocking(handle, move |conn| {
                sqlite_raw::execute(
                    conn,
                    "INSERT INTO blast_jobs (id, session_id, endpoint, body, options, status, total, skipped_dup_count, send_at, created_at) VALUES (?,?,?,?,?,'pending',?,?,?,?)",
                    &[
                        SQ::Text(id),
                        SQ::Text(sid),
                        SQ::Text(ep),
                        SQ::Text(body),
                        SQ::Text(opts),
                        SQ::Int(total),
                        SQ::Int(skipped_dup),
                        SQ::from_opt_str(send_at_s.as_deref()),
                        SQ::Text(now),
                    ],
                )?;
                Ok(())
            })
            .await?;
        }
    }
    Ok(())
}

/// Bulk-insert the recipient rows of a job. `rows` pairs the canonical
/// recipient JID with its initial status (`pending` or `skipped_dup`).
pub async fn insert_recipients(
    pool: &DbPool,
    job_id: &str,
    session_id: &str,
    rows: &[(String, String)],
) -> anyhow::Result<()> {
    match pool {
        DbPool::Postgres(pg) => {
            let client = pg.get().await?;
            for (recipient, status) in rows {
                client
                    .execute(
                        "INSERT INTO blast_recipients (job_id, session_id, recipient, status, updated_at) VALUES ($1,$2,$3,$4,NOW()) ON CONFLICT (job_id, recipient) DO NOTHING",
                        &[&job_id, &session_id, &recipient, &status],
                    )
                    .await?;
            }
        }
        DbPool::MySQL(my) => {
            use mysql_async::prelude::*;
            let mut conn = my.get_conn().await?;
            let now = now_str();
            for (recipient, status) in rows {
                conn.exec_drop(
                    "INSERT IGNORE INTO blast_recipients (job_id, session_id, recipient, status, updated_at) VALUES (?,?,?,?,?)",
                    (job_id, session_id, recipient, status, &now),
                )
                .await?;
            }
        }
        DbPool::SQLite(handle) => {
            let jid = job_id.to_string();
            let sid = session_id.to_string();
            let rows = rows.to_vec();
            let now = now_str();
            sqlite_blocking(handle, move |conn| {
                for (recipient, status) in rows {
                    sqlite_raw::execute(
                        conn,
                        "INSERT OR IGNORE INTO blast_recipients (job_id, session_id, recipient, status, updated_at) VALUES (?,?,?,?,?)",
                        &[
                            SQ::Text(jid.clone()),
                            SQ::Text(sid.clone()),
                            SQ::Text(recipient),
                            SQ::Text(status),
                            SQ::Text(now.clone()),
                        ],
                    )?;
                }
                Ok(())
            })
            .await?;
        }
    }
    Ok(())
}

/// Recipients of `session_id` already delivered to by ANY blast job
/// (status `sent`), restricted to `candidates`. Backs the
/// `dedup_across_jobs` create-time filter. Queried in chunks so the IN
/// list stays well under driver parameter limits.
pub async fn already_sent(
    pool: &DbPool,
    session_id: &str,
    candidates: &[String],
) -> anyhow::Result<HashSet<String>> {
    let mut out = HashSet::new();
    for chunk in candidates.chunks(500) {
        let marks: Vec<String> = (0..chunk.len()).map(|i| placeholder(pool, i + 1)).collect();
        let sql = format!(
            "SELECT recipient FROM blast_recipients WHERE session_id = {} AND status = 'sent' AND recipient IN ({})",
            placeholder(pool, 0),
            marks.join(", ")
        );
        let mut values: Vec<String> = Vec::with_capacity(chunk.len() + 1);
        values.push(session_id.to_string());
        values.extend(chunk.iter().cloned());
        match pool {
            DbPool::Postgres(pg) => {
                let client = pg.get().await?;
                let params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = values
                    .iter()
                    .map(|v| v as &(dyn tokio_postgres::types::ToSql + Sync))
                    .collect();
                let rows = client.query(&sql, &params).await?;
                for row in rows {
                    out.insert(row.get::<_, String>("recipient"));
                }
            }
            DbPool::MySQL(my) => {
                use mysql_async::prelude::*;
                let mut conn = my.get_conn().await?;
                let rows: Vec<mysql_async::Row> = conn.exec(sql, values).await?;
                for row in rows {
                    if let Some(r) = my_get_string(&row, "recipient") {
                        out.insert(r);
                    }
                }
            }
            DbPool::SQLite(handle) => {
                let params: Vec<SQ> = values.into_iter().map(SQ::Text).collect();
                let found = sqlite_blocking(handle, move |conn| {
                    sqlite_raw::query(conn, &sql, &params, |row| {
                        row.get_string(0).unwrap_or_default()
                    })
                })
                .await?;
                out.extend(found);
            }
        }
    }
    Ok(out)
}

/// Atomically claim the oldest due `pending` job (send_at null or
/// already past) by flipping it to `running`. Returns the claimed row,
/// or `None` when nothing is due or a concurrent claimer won the race.
pub async fn claim_next_due(pool: &DbPool) -> anyhow::Result<Option<BlastJobRow>> {
    let candidate = match pool {
        DbPool::Postgres(pg) => {
            let client = pg.get().await?;
            let row = client
                .query_opt(
                    &format!(
                        "SELECT {JOB_COLS} FROM blast_jobs WHERE status = 'pending' AND (send_at IS NULL OR send_at <= NOW()) ORDER BY created_at ASC LIMIT 1"
                    ),
                    &[],
                )
                .await?;
            row.as_ref().map(pg_row_to_job)
        }
        DbPool::MySQL(my) => {
            use mysql_async::prelude::*;
            let mut conn = my.get_conn().await?;
            let now = now_str();
            let row: Option<mysql_async::Row> = conn
                .exec_first(
                    format!(
                        "SELECT {JOB_COLS} FROM blast_jobs WHERE status = 'pending' AND (send_at IS NULL OR send_at <= ?) ORDER BY created_at ASC LIMIT 1"
                    ),
                    (now,),
                )
                .await?;
            row.as_ref().map(my_row_to_job)
        }
        DbPool::SQLite(handle) => {
            let now = now_str();
            sqlite_blocking(handle, move |conn| {
                let mut out = sqlite_raw::query(
                    conn,
                    &format!(
                        "SELECT {JOB_COLS} FROM blast_jobs WHERE status = 'pending' AND (send_at IS NULL OR send_at <= ?) ORDER BY created_at ASC LIMIT 1"
                    ),
                    &[SQ::Text(now)],
                    sqlite_row_to_job,
                )?;
                Ok(out.pop())
            })
            .await?
        }
    };
    let Some(job) = candidate else {
        return Ok(None);
    };
    let now = now_str();
    let changed = match pool {
        DbPool::Postgres(pg) => {
            let client = pg.get().await?;
            client
                .execute(
                    "UPDATE blast_jobs SET status = 'running', started_at = NOW() WHERE id = $1 AND status = 'pending'",
                    &[&job.id],
                )
                .await?
        }
        DbPool::MySQL(my) => {
            use mysql_async::prelude::*;
            let mut conn = my.get_conn().await?;
            conn.exec_drop(
                "UPDATE blast_jobs SET status = 'running', started_at = ? WHERE id = ? AND status = 'pending'",
                (now, &job.id),
            )
            .await?;
            conn.affected_rows()
        }
        DbPool::SQLite(handle) => {
            let id = job.id.clone();
            sqlite_blocking(handle, move |conn| {
                sqlite_raw::execute(
                    conn,
                    "UPDATE blast_jobs SET status = 'running', started_at = ? WHERE id = ? AND status = 'pending'",
                    &[SQ::Text(now), SQ::Text(id)],
                )
            })
            .await?
        }
    };
    if changed == 0 {
        return Ok(None);
    }
    get_job(pool, &job.id).await
}

/// The oldest `running` job, if any. Used by the worker to continue an
/// in-flight job across ticks — and across process restarts, since a
/// job only leaves `running` through [`finish_job`] or [`cancel_job`].
pub async fn running_job(pool: &DbPool) -> anyhow::Result<Option<BlastJobRow>> {
    match pool {
        DbPool::Postgres(pg) => {
            let client = pg.get().await?;
            let row = client
                .query_opt(
                    &format!(
                        "SELECT {JOB_COLS} FROM blast_jobs WHERE status = 'running' ORDER BY started_at ASC LIMIT 1"
                    ),
                    &[],
                )
                .await?;
            Ok(row.as_ref().map(pg_row_to_job))
        }
        DbPool::MySQL(my) => {
            use mysql_async::prelude::*;
            let mut conn = my.get_conn().await?;
            let row: Option<mysql_async::Row> = conn
                .exec_first(
                    format!(
                        "SELECT {JOB_COLS} FROM blast_jobs WHERE status = 'running' ORDER BY started_at ASC LIMIT 1"
                    ),
                    (),
                )
                .await?;
            Ok(row.as_ref().map(my_row_to_job))
        }
        DbPool::SQLite(handle) => {
            sqlite_blocking(handle, move |conn| {
                let mut out = sqlite_raw::query(
                    conn,
                    &format!(
                        "SELECT {JOB_COLS} FROM blast_jobs WHERE status = 'running' ORDER BY started_at ASC LIMIT 1"
                    ),
                    &[],
                    sqlite_row_to_job,
                )?;
                Ok(out.pop())
            })
            .await
        }
    }
}

/// Move a job's `sending` recipients back to `pending`. Called when the
/// worker (re)starts a job: any row stuck in `sending` was in flight
/// when the process died and never got settled, so it is retried.
pub async fn reset_sending_recipients(pool: &DbPool, job_id: &str) -> anyhow::Result<()> {
    match pool {
        DbPool::Postgres(pg) => {
            let client = pg.get().await?;
            client
                .execute(
                    "UPDATE blast_recipients SET status = 'pending', updated_at = NOW() WHERE job_id = $1 AND status = 'sending'",
                    &[&job_id],
                )
                .await?;
        }
        DbPool::MySQL(my) => {
            use mysql_async::prelude::*;
            let mut conn = my.get_conn().await?;
            let now = now_str();
            conn.exec_drop(
                "UPDATE blast_recipients SET status = 'pending', updated_at = ? WHERE job_id = ? AND status = 'sending'",
                (now, job_id),
            )
            .await?;
        }
        DbPool::SQLite(handle) => {
            let jid = job_id.to_string();
            let now = now_str();
            sqlite_blocking(handle, move |conn| {
                sqlite_raw::execute(
                    conn,
                    "UPDATE blast_recipients SET status = 'pending', updated_at = ? WHERE job_id = ? AND status = 'sending'",
                    &[SQ::Text(now), SQ::Text(jid)],
                )?;
                Ok(())
            })
            .await?;
        }
    }
    Ok(())
}

/// Next batch of a job's `pending` recipients, in insertion order.
pub async fn pending_recipients(
    pool: &DbPool,
    job_id: &str,
    limit: i64,
) -> anyhow::Result<Vec<BlastRecipientRow>> {
    match pool {
        DbPool::Postgres(pg) => {
            let client = pg.get().await?;
            let rows = client
                .query(
                    &format!(
                        "SELECT {REC_COLS} FROM blast_recipients WHERE job_id = $1 AND status = 'pending' ORDER BY id ASC LIMIT $2"
                    ),
                    &[&job_id, &limit],
                )
                .await?;
            Ok(rows.iter().map(pg_row_to_recipient).collect())
        }
        DbPool::MySQL(my) => {
            use mysql_async::prelude::*;
            let mut conn = my.get_conn().await?;
            let rows: Vec<mysql_async::Row> = conn
                .exec(
                    format!(
                        "SELECT {REC_COLS} FROM blast_recipients WHERE job_id = ? AND status = 'pending' ORDER BY id ASC LIMIT ?"
                    ),
                    (job_id, limit),
                )
                .await?;
            Ok(rows.iter().map(my_row_to_recipient).collect())
        }
        DbPool::SQLite(handle) => {
            let jid = job_id.to_string();
            sqlite_blocking(handle, move |conn| {
                sqlite_raw::query(
                    conn,
                    &format!(
                        "SELECT {REC_COLS} FROM blast_recipients WHERE job_id = ? AND status = 'pending' ORDER BY id ASC LIMIT ?"
                    ),
                    &[SQ::Text(jid), SQ::Int(limit)],
                    sqlite_row_to_recipient,
                )
            })
            .await
        }
    }
}

/// Mark one recipient as mid-dispatch.
pub async fn mark_recipient_sending(
    pool: &DbPool,
    job_id: &str,
    recipient: &str,
) -> anyhow::Result<()> {
    update_recipient(pool, job_id, recipient, "status = 'sending'", None, None).await
}

/// Settle one recipient as delivered, recording the WhatsApp message id.
pub async fn mark_recipient_sent(
    pool: &DbPool,
    job_id: &str,
    recipient: &str,
    message_id: &str,
) -> anyhow::Result<()> {
    update_recipient(
        pool,
        job_id,
        recipient,
        "status = 'sent'",
        Some(message_id),
        None,
    )
    .await
}

/// Record a failed attempt and put the recipient back to `pending` so
/// the worker retries it on a later batch pass.
pub async fn mark_recipient_retry(
    pool: &DbPool,
    job_id: &str,
    recipient: &str,
    error: &str,
) -> anyhow::Result<()> {
    update_recipient(
        pool,
        job_id,
        recipient,
        "status = 'pending', attempts = attempts + 1",
        None,
        Some(error),
    )
    .await
}

/// Record a failed attempt and park the recipient in the dead-letter
/// state; only the retry endpoint requeues it from here.
pub async fn mark_recipient_dlq(
    pool: &DbPool,
    job_id: &str,
    recipient: &str,
    error: &str,
) -> anyhow::Result<()> {
    update_recipient(
        pool,
        job_id,
        recipient,
        "status = 'dlq', attempts = attempts + 1",
        None,
        Some(error),
    )
    .await
}

/// Shared row-update behind the `mark_recipient_*` family. `set` is an
/// internal hardcoded SET fragment (never user input); `message_id` and
/// `error` ride along as bound parameters.
async fn update_recipient(
    pool: &DbPool,
    job_id: &str,
    recipient: &str,
    set: &str,
    message_id: Option<&str>,
    error: Option<&str>,
) -> anyhow::Result<()> {
    match pool {
        DbPool::Postgres(pg) => {
            let client = pg.get().await?;
            client
                .execute(
                    &format!(
                        "UPDATE blast_recipients SET {set}, message_id = COALESCE($1, message_id), last_error = $2, updated_at = NOW() WHERE job_id = $3 AND recipient = $4"
                    ),
                    &[&message_id, &error, &job_id, &recipient],
                )
                .await?;
        }
        DbPool::MySQL(my) => {
            use mysql_async::prelude::*;
            let mut conn = my.get_conn().await?;
            let now = now_str();
            conn.exec_drop(
                format!(
                    "UPDATE blast_recipients SET {set}, message_id = COALESCE(?, message_id), last_error = ?, updated_at = ? WHERE job_id = ? AND recipient = ?"
                ),
                (message_id, error, now, job_id, recipient),
            )
            .await?;
        }
        DbPool::SQLite(handle) => {
            let jid = job_id.to_string();
            let rec = recipient.to_string();
            let mid = message_id.map(str::to_string);
            let err = error.map(str::to_string);
            let now = now_str();
            let sql = format!(
                "UPDATE blast_recipients SET {set}, message_id = COALESCE(?, message_id), last_error = ?, updated_at = ? WHERE job_id = ? AND recipient = ?"
            );
            sqlite_blocking(handle, move |conn| {
                sqlite_raw::execute(
                    conn,
                    &sql,
                    &[
                        SQ::from_opt_str(mid.as_deref()),
                        SQ::from_opt_str(err.as_deref()),
                        SQ::Text(now),
                        SQ::Text(jid),
                        SQ::Text(rec),
                    ],
                )?;
                Ok(())
            })
            .await?;
        }
    }
    Ok(())
}

/// Increment a job's `sent_count` by one.
pub async fn incr_sent(pool: &DbPool, job_id: &str) -> anyhow::Result<()> {
    bump_counter(pool, job_id, "sent_count = sent_count + 1").await
}

/// Increment a job's `failed_count` (failed attempts, retries included)
/// by one.
pub async fn incr_failed(pool: &DbPool, job_id: &str) -> anyhow::Result<()> {
    bump_counter(pool, job_id, "failed_count = failed_count + 1").await
}

/// Increment a job's `dlq_count` by one.
pub async fn incr_dlq(pool: &DbPool, job_id: &str) -> anyhow::Result<()> {
    bump_counter(pool, job_id, "dlq_count = dlq_count + 1").await
}

/// Shared job-counter bump behind the `incr_*` family. `set` is an
/// internal hardcoded fragment (never user input).
async fn bump_counter(pool: &DbPool, job_id: &str, set: &str) -> anyhow::Result<()> {
    match pool {
        DbPool::Postgres(pg) => {
            let client = pg.get().await?;
            client
                .execute(
                    &format!("UPDATE blast_jobs SET {set} WHERE id = $1"),
                    &[&job_id],
                )
                .await?;
        }
        DbPool::MySQL(my) => {
            use mysql_async::prelude::*;
            let mut conn = my.get_conn().await?;
            conn.exec_drop(
                format!("UPDATE blast_jobs SET {set} WHERE id = ?"),
                (job_id,),
            )
            .await?;
        }
        DbPool::SQLite(handle) => {
            let jid = job_id.to_string();
            let sql = format!("UPDATE blast_jobs SET {set} WHERE id = ?");
            sqlite_blocking(handle, move |conn| {
                sqlite_raw::execute(conn, &sql, &[SQ::Text(jid)])?;
                Ok(())
            })
            .await?;
        }
    }
    Ok(())
}

/// Fetch one job by id (worker path, not session-scoped).
pub async fn get_job(pool: &DbPool, id: &str) -> anyhow::Result<Option<BlastJobRow>> {
    match pool {
        DbPool::Postgres(pg) => {
            let client = pg.get().await?;
            let row = client
                .query_opt(
                    &format!("SELECT {JOB_COLS} FROM blast_jobs WHERE id = $1"),
                    &[&id],
                )
                .await?;
            Ok(row.as_ref().map(pg_row_to_job))
        }
        DbPool::MySQL(my) => {
            use mysql_async::prelude::*;
            let mut conn = my.get_conn().await?;
            let row: Option<mysql_async::Row> = conn
                .exec_first(
                    format!("SELECT {JOB_COLS} FROM blast_jobs WHERE id = ?"),
                    (id,),
                )
                .await?;
            Ok(row.as_ref().map(my_row_to_job))
        }
        DbPool::SQLite(handle) => {
            let id_s = id.to_string();
            sqlite_blocking(handle, move |conn| {
                let mut out = sqlite_raw::query(
                    conn,
                    &format!("SELECT {JOB_COLS} FROM blast_jobs WHERE id = ?"),
                    &[SQ::Text(id_s)],
                    sqlite_row_to_job,
                )?;
                Ok(out.pop())
            })
            .await
        }
    }
}

/// Fetch one job scoped to a session (management endpoints).
pub async fn get_job_scoped(
    pool: &DbPool,
    session_id: &str,
    id: &str,
) -> anyhow::Result<Option<BlastJobRow>> {
    match pool {
        DbPool::Postgres(pg) => {
            let client = pg.get().await?;
            let row = client
                .query_opt(
                    &format!("SELECT {JOB_COLS} FROM blast_jobs WHERE session_id = $1 AND id = $2"),
                    &[&session_id, &id],
                )
                .await?;
            Ok(row.as_ref().map(pg_row_to_job))
        }
        DbPool::MySQL(my) => {
            use mysql_async::prelude::*;
            let mut conn = my.get_conn().await?;
            let row: Option<mysql_async::Row> = conn
                .exec_first(
                    format!("SELECT {JOB_COLS} FROM blast_jobs WHERE session_id = ? AND id = ?"),
                    (session_id, id),
                )
                .await?;
            Ok(row.as_ref().map(my_row_to_job))
        }
        DbPool::SQLite(handle) => {
            let sid = session_id.to_string();
            let id_s = id.to_string();
            sqlite_blocking(handle, move |conn| {
                let mut out = sqlite_raw::query(
                    conn,
                    &format!("SELECT {JOB_COLS} FROM blast_jobs WHERE session_id = ? AND id = ?"),
                    &[SQ::Text(sid), SQ::Text(id_s)],
                    sqlite_row_to_job,
                )?;
                Ok(out.pop())
            })
            .await
        }
    }
}

/// Current status string of a job, or `None` when the id is unknown.
/// The worker polls this between batches to honor cancellation.
pub async fn job_status(pool: &DbPool, id: &str) -> anyhow::Result<Option<String>> {
    match pool {
        DbPool::Postgres(pg) => {
            let client = pg.get().await?;
            let row = client
                .query_opt("SELECT status FROM blast_jobs WHERE id = $1", &[&id])
                .await?;
            Ok(row.map(|r| r.get("status")))
        }
        DbPool::MySQL(my) => {
            use mysql_async::prelude::*;
            let mut conn = my.get_conn().await?;
            let row: Option<mysql_async::Row> = conn
                .exec_first("SELECT status FROM blast_jobs WHERE id = ?", (id,))
                .await?;
            Ok(row.as_ref().and_then(|r| my_get_string(r, "status")))
        }
        DbPool::SQLite(handle) => {
            let id_s = id.to_string();
            sqlite_blocking(handle, move |conn| {
                let mut out = sqlite_raw::query(
                    conn,
                    "SELECT status FROM blast_jobs WHERE id = ?",
                    &[SQ::Text(id_s)],
                    |row| row.get_string(0).unwrap_or_default(),
                )?;
                Ok(out.pop())
            })
            .await
        }
    }
}

/// Close a job with a terminal status, stamping `finished_at`.
pub async fn finish_job(pool: &DbPool, id: &str, status: &str) -> anyhow::Result<()> {
    match pool {
        DbPool::Postgres(pg) => {
            let client = pg.get().await?;
            client
                .execute(
                    "UPDATE blast_jobs SET status = $1, finished_at = NOW() WHERE id = $2",
                    &[&status, &id],
                )
                .await?;
        }
        DbPool::MySQL(my) => {
            use mysql_async::prelude::*;
            let mut conn = my.get_conn().await?;
            let now = now_str();
            conn.exec_drop(
                "UPDATE blast_jobs SET status = ?, finished_at = ? WHERE id = ?",
                (status, now, id),
            )
            .await?;
        }
        DbPool::SQLite(handle) => {
            let id_s = id.to_string();
            let st = status.to_string();
            let now = now_str();
            sqlite_blocking(handle, move |conn| {
                sqlite_raw::execute(
                    conn,
                    "UPDATE blast_jobs SET status = ?, finished_at = ? WHERE id = ?",
                    &[SQ::Text(st), SQ::Text(now), SQ::Text(id_s)],
                )?;
                Ok(())
            })
            .await?;
        }
    }
    Ok(())
}

/// Cancel a job that is still `pending` or `running`. Returns false
/// when the job is absent or already terminal (the handler
/// distinguishes 404 vs 400 via [`get_job_scoped`]).
pub async fn cancel_job(pool: &DbPool, session_id: &str, id: &str) -> anyhow::Result<bool> {
    let changed = match pool {
        DbPool::Postgres(pg) => {
            let client = pg.get().await?;
            client
                .execute(
                    "UPDATE blast_jobs SET status = 'cancelled', finished_at = NOW() WHERE session_id = $1 AND id = $2 AND status IN ('pending','running')",
                    &[&session_id, &id],
                )
                .await?
        }
        DbPool::MySQL(my) => {
            use mysql_async::prelude::*;
            let mut conn = my.get_conn().await?;
            let now = now_str();
            conn.exec_drop(
                "UPDATE blast_jobs SET status = 'cancelled', finished_at = ? WHERE session_id = ? AND id = ? AND status IN ('pending','running')",
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
                    "UPDATE blast_jobs SET status = 'cancelled', finished_at = ? WHERE session_id = ? AND id = ? AND status IN ('pending','running')",
                    &[SQ::Text(now), SQ::Text(sid), SQ::Text(id_s)],
                )
            })
            .await?
        }
    };
    Ok(changed > 0)
}

/// Requeue a job's `dlq` and `failed` recipients back to `pending` with
/// their attempts reset. Returns the number of recipients requeued.
pub async fn requeue_failed(pool: &DbPool, job_id: &str) -> anyhow::Result<u64> {
    let changed = match pool {
        DbPool::Postgres(pg) => {
            let client = pg.get().await?;
            client
                .execute(
                    "UPDATE blast_recipients SET status = 'pending', attempts = 0, last_error = NULL, updated_at = NOW() WHERE job_id = $1 AND status IN ('dlq','failed')",
                    &[&job_id],
                )
                .await?
        }
        DbPool::MySQL(my) => {
            use mysql_async::prelude::*;
            let mut conn = my.get_conn().await?;
            let now = now_str();
            conn.exec_drop(
                "UPDATE blast_recipients SET status = 'pending', attempts = 0, last_error = NULL, updated_at = ? WHERE job_id = ? AND status IN ('dlq','failed')",
                (now, job_id),
            )
            .await?;
            conn.affected_rows()
        }
        DbPool::SQLite(handle) => {
            let jid = job_id.to_string();
            let now = now_str();
            sqlite_blocking(handle, move |conn| {
                sqlite_raw::execute(
                    conn,
                    "UPDATE blast_recipients SET status = 'pending', attempts = 0, last_error = NULL, updated_at = ? WHERE job_id = ? AND status IN ('dlq','failed')",
                    &[SQ::Text(now), SQ::Text(jid)],
                )
            })
            .await?
        }
    };
    Ok(changed)
}

/// Put a non-running job back to `pending` so the worker claims it
/// again. Clears `finished_at`; `started_at` is refreshed on claim.
pub async fn reopen_job(pool: &DbPool, job_id: &str) -> anyhow::Result<()> {
    match pool {
        DbPool::Postgres(pg) => {
            let client = pg.get().await?;
            client
                .execute(
                    "UPDATE blast_jobs SET status = 'pending', finished_at = NULL WHERE id = $1 AND status != 'running'",
                    &[&job_id],
                )
                .await?;
        }
        DbPool::MySQL(my) => {
            use mysql_async::prelude::*;
            let mut conn = my.get_conn().await?;
            conn.exec_drop(
                "UPDATE blast_jobs SET status = 'pending', finished_at = NULL WHERE id = ? AND status != 'running'",
                (job_id,),
            )
            .await?;
        }
        DbPool::SQLite(handle) => {
            let jid = job_id.to_string();
            sqlite_blocking(handle, move |conn| {
                sqlite_raw::execute(
                    conn,
                    "UPDATE blast_jobs SET status = 'pending', finished_at = NULL WHERE id = ? AND status != 'running'",
                    &[SQ::Text(jid)],
                )?;
                Ok(())
            })
            .await?;
        }
    }
    Ok(())
}

/// List jobs with optional session/status filters, newest first. Both
/// filters absent = fleet-wide listing.
pub async fn list_jobs(
    pool: &DbPool,
    session_id: Option<&str>,
    status: Option<&str>,
) -> anyhow::Result<Vec<BlastJobRow>> {
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
    let mut sql = format!("SELECT {JOB_COLS} FROM blast_jobs");
    if !clauses.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&clauses.join(" AND "));
    }
    sql.push_str(" ORDER BY created_at DESC");

    match pool {
        DbPool::Postgres(pg) => {
            let client = pg.get().await?;
            let params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = values
                .iter()
                .map(|v| v as &(dyn tokio_postgres::types::ToSql + Sync))
                .collect();
            let rows = client.query(&sql, &params).await?;
            Ok(rows.iter().map(pg_row_to_job).collect())
        }
        DbPool::MySQL(my) => {
            use mysql_async::prelude::*;
            let mut conn = my.get_conn().await?;
            let rows: Vec<mysql_async::Row> = conn.exec(sql, values).await?;
            Ok(rows.iter().map(my_row_to_job).collect())
        }
        DbPool::SQLite(handle) => {
            let params: Vec<SQ> = values.into_iter().map(SQ::Text).collect();
            sqlite_blocking(handle, move |conn| {
                sqlite_raw::query(conn, &sql, &params, sqlite_row_to_job)
            })
            .await
        }
    }
}

/// Paginated recipient listing for one job, optional status filter, in
/// insertion order.
pub async fn list_recipients(
    pool: &DbPool,
    job_id: &str,
    status: Option<&str>,
    limit: i64,
    offset: i64,
) -> anyhow::Result<Vec<BlastRecipientRow>> {
    match pool {
        DbPool::Postgres(pg) => {
            let client = pg.get().await?;
            let rows = if let Some(st) = status {
                client
                    .query(
                        &format!(
                            "SELECT {REC_COLS} FROM blast_recipients WHERE job_id = $1 AND status = $2 ORDER BY id ASC LIMIT $3 OFFSET $4"
                        ),
                        &[&job_id, &st, &limit, &offset],
                    )
                    .await?
            } else {
                client
                    .query(
                        &format!(
                            "SELECT {REC_COLS} FROM blast_recipients WHERE job_id = $1 ORDER BY id ASC LIMIT $2 OFFSET $3"
                        ),
                        &[&job_id, &limit, &offset],
                    )
                    .await?
            };
            Ok(rows.iter().map(pg_row_to_recipient).collect())
        }
        DbPool::MySQL(my) => {
            use mysql_async::prelude::*;
            let mut conn = my.get_conn().await?;
            let rows: Vec<mysql_async::Row> = if let Some(st) = status {
                conn.exec(
                    format!(
                        "SELECT {REC_COLS} FROM blast_recipients WHERE job_id = ? AND status = ? ORDER BY id ASC LIMIT ? OFFSET ?"
                    ),
                    (job_id, st, limit, offset),
                )
                .await?
            } else {
                conn.exec(
                    format!(
                        "SELECT {REC_COLS} FROM blast_recipients WHERE job_id = ? ORDER BY id ASC LIMIT ? OFFSET ?"
                    ),
                    (job_id, limit, offset),
                )
                .await?
            };
            Ok(rows.iter().map(my_row_to_recipient).collect())
        }
        DbPool::SQLite(handle) => {
            let jid = job_id.to_string();
            let st = status.map(str::to_string);
            sqlite_blocking(handle, move |conn| {
                if let Some(st) = st {
                    sqlite_raw::query(
                        conn,
                        &format!(
                            "SELECT {REC_COLS} FROM blast_recipients WHERE job_id = ? AND status = ? ORDER BY id ASC LIMIT ? OFFSET ?"
                        ),
                        &[SQ::Text(jid), SQ::Text(st), SQ::Int(limit), SQ::Int(offset)],
                        sqlite_row_to_recipient,
                    )
                } else {
                    sqlite_raw::query(
                        conn,
                        &format!(
                            "SELECT {REC_COLS} FROM blast_recipients WHERE job_id = ? ORDER BY id ASC LIMIT ? OFFSET ?"
                        ),
                        &[SQ::Text(jid), SQ::Int(limit), SQ::Int(offset)],
                        sqlite_row_to_recipient,
                    )
                }
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

fn pg_row_to_job(row: &tokio_postgres::Row) -> BlastJobRow {
    BlastJobRow {
        id: row.get("id"),
        session_id: row.get("session_id"),
        endpoint: row.get("endpoint"),
        body: row.get("body"),
        options: row.get("options"),
        status: row.get("status"),
        total: row.get("total"),
        sent_count: row.get("sent_count"),
        failed_count: row.get("failed_count"),
        dlq_count: row.get("dlq_count"),
        skipped_dup_count: row.get("skipped_dup_count"),
        send_at: row
            .get::<_, Option<chrono::DateTime<chrono::Utc>>>("send_at")
            .map(fmt_utc),
        created_at: fmt_utc(row.get::<_, chrono::DateTime<chrono::Utc>>("created_at")),
        started_at: row
            .get::<_, Option<chrono::DateTime<chrono::Utc>>>("started_at")
            .map(fmt_utc),
        finished_at: row
            .get::<_, Option<chrono::DateTime<chrono::Utc>>>("finished_at")
            .map(fmt_utc),
    }
}

fn pg_row_to_recipient(row: &tokio_postgres::Row) -> BlastRecipientRow {
    BlastRecipientRow {
        id: row.get("id"),
        job_id: row.get("job_id"),
        session_id: row.get("session_id"),
        recipient: row.get("recipient"),
        status: row.get("status"),
        attempts: row.get("attempts"),
        last_error: row.get("last_error"),
        message_id: row.get("message_id"),
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

fn my_row_to_job(row: &mysql_async::Row) -> BlastJobRow {
    BlastJobRow {
        id: my_get_string(row, "id").unwrap_or_default(),
        session_id: my_get_string(row, "session_id").unwrap_or_default(),
        endpoint: my_get_string(row, "endpoint").unwrap_or_default(),
        body: my_get_string(row, "body").unwrap_or_default(),
        options: my_get_string(row, "options").unwrap_or_default(),
        status: my_get_string(row, "status").unwrap_or_default(),
        total: my_get_i64(row, "total"),
        sent_count: my_get_i64(row, "sent_count"),
        failed_count: my_get_i64(row, "failed_count"),
        dlq_count: my_get_i64(row, "dlq_count"),
        skipped_dup_count: my_get_i64(row, "skipped_dup_count"),
        send_at: my_get_string(row, "send_at"),
        created_at: my_get_string(row, "created_at").unwrap_or_default(),
        started_at: my_get_string(row, "started_at"),
        finished_at: my_get_string(row, "finished_at"),
    }
}

fn my_row_to_recipient(row: &mysql_async::Row) -> BlastRecipientRow {
    BlastRecipientRow {
        id: my_get_i64(row, "id"),
        job_id: my_get_string(row, "job_id").unwrap_or_default(),
        session_id: my_get_string(row, "session_id").unwrap_or_default(),
        recipient: my_get_string(row, "recipient").unwrap_or_default(),
        status: my_get_string(row, "status").unwrap_or_default(),
        attempts: my_get_i64(row, "attempts"),
        last_error: my_get_string(row, "last_error"),
        message_id: my_get_string(row, "message_id"),
        updated_at: my_get_string(row, "updated_at").unwrap_or_default(),
    }
}

fn sqlite_row_to_job(row: &sqlite_raw::Row) -> BlastJobRow {
    BlastJobRow {
        id: row.get_string(0).unwrap_or_default(),
        session_id: row.get_string(1).unwrap_or_default(),
        endpoint: row.get_string(2).unwrap_or_default(),
        body: row.get_string(3).unwrap_or_default(),
        options: row.get_string(4).unwrap_or_default(),
        status: row.get_string(5).unwrap_or_default(),
        total: row.get_int(6),
        sent_count: row.get_int(7),
        failed_count: row.get_int(8),
        dlq_count: row.get_int(9),
        skipped_dup_count: row.get_int(10),
        send_at: row.get_string(11),
        created_at: row.get_string(12).unwrap_or_default(),
        started_at: row.get_string(13),
        finished_at: row.get_string(14),
    }
}

fn sqlite_row_to_recipient(row: &sqlite_raw::Row) -> BlastRecipientRow {
    BlastRecipientRow {
        id: row.get_int(0),
        job_id: row.get_string(1).unwrap_or_default(),
        session_id: row.get_string(2).unwrap_or_default(),
        recipient: row.get_string(3).unwrap_or_default(),
        status: row.get_string(4).unwrap_or_default(),
        attempts: row.get_int(5),
        last_error: row.get_string(6),
        message_id: row.get_string(7),
        updated_at: row.get_string(8).unwrap_or_default(),
    }
}
