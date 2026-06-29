use crate::db::session::{sqlite_blocking, DbPool};
use crate::db::sqlite_raw::{self, Value as SQ};

/// Persist a failed webhook delivery so an operator can replay it later.
/// Best-effort: a failure to write the DLQ row is logged but never
/// propagated, since the original event has already been lost — there's
/// nothing useful the caller can do about a DLQ insert failing.
pub async fn record_failure(
    pool: &DbPool,
    session_id: &str,
    webhook_url: &str,
    event_type: &str,
    payload: &str,
    last_error: &str,
    attempts: i32,
) {
    let res: anyhow::Result<()> = async {
        match pool {
            DbPool::Postgres(pg) => {
                let client = pg.get().await?;
                client
                    .execute(
                        "INSERT INTO webhook_dlq (session_id, webhook_url, event_type, payload, last_error, attempts, last_attempt_at) VALUES ($1,$2,$3,$4,$5,$6, NOW())",
                        &[&session_id, &webhook_url, &event_type, &payload, &last_error, &attempts],
                    )
                    .await?;
            }
            DbPool::MySQL(my) => {
                use mysql_async::prelude::*;
                let mut conn = my.get_conn().await?;
                let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
                conn.exec_drop(
                    "INSERT INTO webhook_dlq (session_id, webhook_url, event_type, payload, last_error, attempts, created_at, last_attempt_at) VALUES (?,?,?,?,?,?,?,?)",
                    (
                        session_id,
                        webhook_url,
                        event_type,
                        payload,
                        last_error,
                        attempts,
                        &now,
                        &now,
                    ),
                )
                .await?;
            }
            DbPool::SQLite(handle) => {
                let sid = session_id.to_string();
                let url = webhook_url.to_string();
                let et = event_type.to_string();
                let pl = payload.to_string();
                let err = last_error.to_string();
                let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
                let now2 = now.clone();
                let att = attempts as i64;
                sqlite_blocking(handle, move |conn| {
                    sqlite_raw::execute(
                        conn,
                        "INSERT INTO webhook_dlq (session_id, webhook_url, event_type, payload, last_error, attempts, created_at, last_attempt_at) VALUES (?,?,?,?,?,?,?,?)",
                        &[
                            SQ::Text(sid),
                            SQ::Text(url),
                            SQ::Text(et),
                            SQ::Text(pl),
                            SQ::Text(err),
                            SQ::Int(att),
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
    .await;
    if let Err(e) = res {
        tracing::warn!("webhook DLQ insert failed for {}: {}", webhook_url, e);
    }
}
