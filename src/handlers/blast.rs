//! Blast (bulk-send): fan one message payload out to many recipients
//! with pacing, dedup, retry, and a dead-letter queue.
//!
//! `POST /api/v1/sessions/{sid}/blast` stores a job row (endpoint key +
//! original JSON body + pacing options) plus one recipient row per
//! accepted recipient, then answers immediately. Delivery is entirely
//! asynchronous: [`run_blast_worker`] is a background
//! `tokio::time::interval` loop (period from `BLAST_POLL_MS`, default
//! 1000 ms) that claims the oldest due `pending` job — or resumes an
//! interrupted `running` one — and drains its recipients in batches of
//! [`BATCH_SIZE`].
//!
//! The worker is deliberately SINGLE and SEQUENTIAL: WhatsApp
//! rate-limits and bans aggressive senders, so parallel blast workers
//! would only multiply ban risk. One send at a time, per-job
//! `delay_ms` plus a random `0..=jitter_ms` pause between sends.
//!
//! Per recipient the worker replays the stored body through
//! [`crate::handlers::schedule::dispatch`] — the same dispatch the
//! scheduler uses — after rewriting `to` to the recipient and forcing
//! `send_at` to `null` (a blast never round-trips through the
//! scheduler). Outcomes:
//!
//! - success → `sent`, WhatsApp message id recorded, `sent_count` + 1;
//! - error with attempts left → attempts + 1, back to `pending`,
//!   retried on a later batch pass of the same job (same per-send
//!   delay; no extra backoff — the attempts count is the only cap);
//! - error on the last attempt → `dlq`, `dlq_count` + 1; only the
//!   retry endpoint requeues it.
//!
//! Cancellation is cooperative: the worker re-reads the job status
//! before each batch and stops when it is no longer `running`, leaving
//! unprocessed recipients `pending` (the job status is the source of
//! truth). When no `pending` recipients remain the job closes as
//! `completed` (nothing failed) or `completed_with_failures` (at least
//! one recipient in the DLQ) and a `blast_completed` webhook fires;
//! every [`PROGRESS_EVERY`] sends a `blast_progress` event fires with
//! the current counters.
//!
//! Management endpoints:
//! - `GET  /api/v1/sessions/{sid}/blasts` — list jobs for one session.
//! - `GET  /api/v1/sessions/{sid}/blasts/{id}` — job detail + counters.
//! - `GET  /api/v1/sessions/{sid}/blasts/{id}/recipients` — paginated.
//! - `POST /api/v1/sessions/{sid}/blasts/{id}/cancel` — stop sending.
//! - `POST /api/v1/sessions/{sid}/blasts/{id}/retry` — requeue DLQ.
//! - `GET  /api/v1/blasts` — fleet-wide list.

use std::collections::HashSet;
use std::time::Duration;

use axum::{
    extract::{Path, Query, State},
    Json,
};
use chrono::{DateTime, Utc};
use rand::Rng;

use crate::db::blast::{self, BlastJobRow};
use crate::error::ApiError;
use crate::models::blast::{
    BlastFleetQuery, BlastJob, BlastJobListResponse, BlastJobStatus, BlastOptions, BlastRecipient,
    BlastRecipientListResponse, BlastRecipientStatus, BlastRecipientsQuery, BlastSessionQuery,
    CreateBlastRequest, CreateBlastResponse,
};
use crate::models::webhooks::WebhookEvent;
use crate::state::AppState;

/// Recipients fetched and processed per batch pass.
const BATCH_SIZE: i64 = 50;

/// A `blast_progress` webhook event fires every this many successful
/// sends.
const PROGRESS_EVERY: i64 = 25;

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/blast",
    tag = "blast",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = CreateBlastRequest,
    responses(
        (status = 200, description = "Blast job created", body = CreateBlastResponse),
        (status = 400, description = "Unknown endpoint, bad body, or invalid recipients"),
        (status = 404, description = "Session not found")
    )
)]
pub async fn create_blast(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<CreateBlastRequest>,
) -> Result<Json<CreateBlastResponse>, ApiError> {
    if state.get_session(&session_id).is_none() {
        return Err(ApiError::SessionNotFound(session_id));
    }
    crate::handlers::schedule::validate_body(&request.endpoint, &request.body)
        .map_err(ApiError::BadRequest)?;
    if request.recipients.is_empty() {
        return Err(ApiError::BadRequest("recipients must not be empty".into()));
    }

    let mut bad: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut accepted: Vec<String> = Vec::new();
    let mut skipped_dup = 0usize;
    for raw in &request.recipients {
        match crate::handlers::messages::parse_jid(raw) {
            Ok(jid) => {
                let canonical = jid.to_string();
                if seen.insert(canonical.clone()) {
                    accepted.push(canonical);
                } else {
                    skipped_dup += 1;
                }
            }
            Err(_) => bad.push(raw.clone()),
        }
    }
    if !bad.is_empty() {
        return Err(ApiError::BadRequest(format!(
            "invalid recipients: {}",
            bad.join(", ")
        )));
    }

    let options = request.options();
    let pool = state.session_manager().pool();
    if options.dedup_across_jobs && !accepted.is_empty() {
        let sent = blast::already_sent(pool, &session_id, &accepted)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        let before = accepted.len();
        accepted.retain(|r| !sent.contains(r));
        skipped_dup += before - accepted.len();
    }

    let id = uuid::Uuid::new_v4().to_string();
    let body_str = request.body.to_string();
    let options_str = serde_json::to_string(&options)
        .map_err(|e| ApiError::Internal(format!("failed to serialize blast options: {e}")))?;
    blast::insert_job(
        pool,
        &id,
        &session_id,
        &request.endpoint,
        &body_str,
        &options_str,
        accepted.len() as i64,
        skipped_dup as i64,
        request.send_at,
    )
    .await
    .map_err(|e| ApiError::Internal(format!("failed to store blast job: {e}")))?;

    let mut rows: Vec<(String, String)> = accepted
        .iter()
        .map(|r| {
            (
                r.clone(),
                BlastRecipientStatus::Pending.as_str().to_string(),
            )
        })
        .collect();
    let accepted_set: HashSet<&String> = accepted.iter().collect();
    rows.extend(seen.iter().filter(|r| !accepted_set.contains(*r)).map(|r| {
        (
            r.clone(),
            BlastRecipientStatus::SkippedDup.as_str().to_string(),
        )
    }));
    blast::insert_recipients(pool, &id, &session_id, &rows)
        .await
        .map_err(|e| ApiError::Internal(format!("failed to store blast recipients: {e}")))?;

    Ok(Json(CreateBlastResponse {
        job_id: id,
        total: accepted.len(),
        skipped_dup,
        status: BlastJobStatus::Pending.as_str().to_string(),
    }))
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/blasts",
    tag = "blast",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        BlastSessionQuery,
    ),
    responses(
        (status = 200, description = "Blast jobs for the session", body = BlastJobListResponse)
    )
)]
pub async fn list_session_blasts(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(q): Query<BlastSessionQuery>,
) -> Result<Json<BlastJobListResponse>, ApiError> {
    let rows = blast::list_jobs(
        state.session_manager().pool(),
        Some(&session_id),
        q.status.as_deref(),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(jobs_to_response(rows)))
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/blasts/{id}",
    tag = "blast",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("id" = String, Path, description = "Blast job ID")
    ),
    responses(
        (status = 200, description = "Blast job detail", body = BlastJob),
        (status = 404, description = "Blast job not found")
    )
)]
pub async fn get_blast(
    State(state): State<AppState>,
    Path((session_id, id)): Path<(String, String)>,
) -> Result<Json<BlastJob>, ApiError> {
    let row = blast::get_job_scoped(state.session_manager().pool(), &session_id, &id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::MessageNotFound(format!("blast job {id}")))?;
    Ok(Json(job_to_model(&row)))
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/blasts/{id}/recipients",
    tag = "blast",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("id" = String, Path, description = "Blast job ID"),
        BlastRecipientsQuery,
    ),
    responses(
        (status = 200, description = "Paginated recipient list", body = BlastRecipientListResponse),
        (status = 404, description = "Blast job not found")
    )
)]
pub async fn list_blast_recipients(
    State(state): State<AppState>,
    Path((session_id, id)): Path<(String, String)>,
    Query(q): Query<BlastRecipientsQuery>,
) -> Result<Json<BlastRecipientListResponse>, ApiError> {
    let pool = state.session_manager().pool();
    blast::get_job_scoped(pool, &session_id, &id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::MessageNotFound(format!("blast job {id}")))?;
    let limit = q.limit.unwrap_or(100).clamp(1, 1000);
    let offset = q.offset.unwrap_or(0).max(0);
    let rows = blast::list_recipients(pool, &id, q.status.as_deref(), limit, offset)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let recipients: Vec<BlastRecipient> = rows
        .iter()
        .map(|r| BlastRecipient {
            id: r.id,
            recipient: r.recipient.clone(),
            status: BlastRecipientStatus::from_str(&r.status),
            attempts: r.attempts,
            last_error: r.last_error.clone(),
            message_id: r.message_id.clone(),
            updated_at: parse_stored_ts(&r.updated_at),
        })
        .collect();
    Ok(Json(BlastRecipientListResponse {
        count: recipients.len(),
        recipients,
    }))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/blasts/{id}/cancel",
    tag = "blast",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("id" = String, Path, description = "Blast job ID")
    ),
    responses(
        (status = 200, description = "Blast job cancelled", body = BlastJob),
        (status = 400, description = "Blast job already finished"),
        (status = 404, description = "Blast job not found")
    )
)]
pub async fn cancel_blast(
    State(state): State<AppState>,
    Path((session_id, id)): Path<(String, String)>,
) -> Result<Json<BlastJob>, ApiError> {
    let pool = state.session_manager().pool();
    let row = blast::get_job_scoped(pool, &session_id, &id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::MessageNotFound(format!("blast job {id}")))?;
    if BlastJobStatus::from_str(&row.status).is_terminal() {
        return Err(ApiError::BadRequest(format!(
            "blast job {id} is {}, only pending or running jobs can be cancelled",
            row.status
        )));
    }
    if !blast::cancel_job(pool, &session_id, &id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        return Err(ApiError::BadRequest(format!(
            "blast job {id} changed state concurrently; try again"
        )));
    }
    let row = blast::get_job_scoped(pool, &session_id, &id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::MessageNotFound(format!("blast job {id}")))?;
    Ok(Json(job_to_model(&row)))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/blasts/{id}/retry",
    tag = "blast",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("id" = String, Path, description = "Blast job ID")
    ),
    responses(
        (status = 200, description = "DLQ recipients requeued, job reopened", body = BlastJob),
        (status = 400, description = "Job still running or nothing to retry"),
        (status = 404, description = "Blast job not found")
    )
)]
pub async fn retry_blast(
    State(state): State<AppState>,
    Path((session_id, id)): Path<(String, String)>,
) -> Result<Json<BlastJob>, ApiError> {
    let pool = state.session_manager().pool();
    let row = blast::get_job_scoped(pool, &session_id, &id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::MessageNotFound(format!("blast job {id}")))?;
    if row.status == BlastJobStatus::Running.as_str() {
        return Err(ApiError::BadRequest(format!(
            "blast job {id} is still running; cancel it first"
        )));
    }
    let requeued = blast::requeue_failed(pool, &id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if requeued == 0 {
        return Err(ApiError::BadRequest(format!(
            "blast job {id} has no dlq/failed recipients to retry"
        )));
    }
    blast::reopen_job(pool, &id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let row = blast::get_job_scoped(pool, &session_id, &id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::MessageNotFound(format!("blast job {id}")))?;
    Ok(Json(job_to_model(&row)))
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/blasts",
    tag = "blast",
    params(
        BlastFleetQuery,
    ),
    responses(
        (status = 200, description = "Blast jobs across all sessions", body = BlastJobListResponse)
    )
)]
pub async fn list_all_blasts(
    State(state): State<AppState>,
    Query(q): Query<BlastFleetQuery>,
) -> Result<Json<BlastJobListResponse>, ApiError> {
    let rows = blast::list_jobs(
        state.session_manager().pool(),
        q.session.as_deref(),
        q.status.as_deref(),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(jobs_to_response(rows)))
}

fn jobs_to_response(rows: Vec<BlastJobRow>) -> BlastJobListResponse {
    let jobs: Vec<BlastJob> = rows.iter().map(job_to_model).collect();
    BlastJobListResponse {
        count: jobs.len(),
        jobs,
    }
}

fn job_to_model(row: &BlastJobRow) -> BlastJob {
    BlastJob {
        id: row.id.clone(),
        session_id: row.session_id.clone(),
        endpoint: row.endpoint.clone(),
        status: BlastJobStatus::from_str(&row.status),
        options: serde_json::from_str(&row.options).unwrap_or_default(),
        total: row.total,
        sent_count: row.sent_count,
        failed_count: row.failed_count,
        dlq_count: row.dlq_count,
        skipped_dup_count: row.skipped_dup_count,
        send_at: row.send_at.as_deref().map(parse_stored_ts),
        created_at: parse_stored_ts(&row.created_at),
        started_at: row.started_at.as_deref().map(parse_stored_ts),
        finished_at: row.finished_at.as_deref().map(parse_stored_ts),
    }
}

/// Parse a timestamp coming out of the DB layer: the canonical
/// `%Y-%m-%d %H:%M:%S` UTC text, its sub-second variant, or RFC 3339.
/// Unparseable values degrade to the Unix epoch rather than failing the
/// whole listing. Mirrors the scheduler's helper.
fn parse_stored_ts(s: &str) -> DateTime<Utc> {
    for fmt in ["%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M:%S%.f"] {
        if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, fmt) {
            return dt.and_utc();
        }
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return dt.with_timezone(&Utc);
    }
    DateTime::UNIX_EPOCH
}

/// Background blast worker, spawned once from `main`. The poll period
/// comes from `BLAST_POLL_MS` (default 1000 ms); a failed tick is
/// logged and the loop keeps going. See the module docs for why this
/// is a single sequential worker.
pub async fn run_blast_worker(state: AppState) {
    let poll_ms: u64 = std::env::var("BLAST_POLL_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1000);
    let mut ticker = tokio::time::interval(Duration::from_millis(poll_ms));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    tracing::info!(poll_ms, "blast worker started");
    loop {
        ticker.tick().await;
        if let Err(e) = tick_once(&state).await {
            tracing::warn!("blast worker tick failed: {e}");
        }
    }
}

/// One worker tick: continue the in-flight job when there is one,
/// otherwise claim the oldest due pending job and start it.
async fn tick_once(state: &AppState) -> anyhow::Result<()> {
    let pool = state.session_manager().pool();
    if let Some(job) = blast::running_job(pool).await? {
        return process_job(state, &job).await;
    }
    if let Some(job) = blast::claim_next_due(pool).await? {
        blast::reset_sending_recipients(pool, &job.id).await?;
        return process_job(state, &job).await;
    }
    Ok(())
}

/// Drain a claimed job batch by batch until it is cancelled, finished,
/// or its stored body turns out unparseable (job → `failed`; per
/// recipient dispatch errors never fail the job itself).
async fn process_job(state: &AppState, job: &BlastJobRow) -> anyhow::Result<()> {
    let pool = state.session_manager().pool();
    let options: BlastOptions = serde_json::from_str(&job.options).unwrap_or_default();
    let max_attempts = i64::from(options.max_attempts.max(1));

    if serde_json::from_str::<serde_json::Value>(&job.body).is_err() {
        tracing::warn!(job = %job.id, "blast job body is not valid JSON; failing the job");
        blast::finish_job(pool, &job.id, BlastJobStatus::Failed.as_str()).await?;
        broadcast_completed(state, &job.id, &job.session_id, BlastJobStatus::Failed).await;
        return Ok(());
    }

    let mut sends_since_progress = 0i64;
    loop {
        match blast::job_status(pool, &job.id).await? {
            Some(status) if status == BlastJobStatus::Running.as_str() => {}
            other => {
                tracing::info!(job = %job.id, status = ?other, "blast job no longer running; worker stops");
                return Ok(());
            }
        }

        let batch = blast::pending_recipients(pool, &job.id, BATCH_SIZE).await?;
        if batch.is_empty() {
            finalize_job(state, &job.id, &job.session_id).await?;
            return Ok(());
        }

        for recipient in &batch {
            blast::mark_recipient_sending(pool, &job.id, &recipient.recipient).await?;
            let body = build_recipient_body(&job.body, &recipient.recipient)?;
            match crate::handlers::schedule::dispatch(state, &job.session_id, &job.endpoint, &body)
                .await
            {
                Ok(resp) => {
                    blast::mark_recipient_sent(
                        pool,
                        &job.id,
                        &recipient.recipient,
                        &resp.message_id,
                    )
                    .await?;
                    blast::incr_sent(pool, &job.id).await?;
                    sends_since_progress += 1;
                    if sends_since_progress >= PROGRESS_EVERY {
                        sends_since_progress = 0;
                        broadcast_progress(state, &job.id, &job.session_id).await;
                    }
                }
                Err(err) => {
                    blast::incr_failed(pool, &job.id).await?;
                    if recipient.attempts + 1 < max_attempts {
                        blast::mark_recipient_retry(pool, &job.id, &recipient.recipient, &err)
                            .await?;
                    } else {
                        blast::mark_recipient_dlq(pool, &job.id, &recipient.recipient, &err)
                            .await?;
                        blast::incr_dlq(pool, &job.id).await?;
                    }
                }
            }
            tokio::time::sleep(send_delay(&options)).await;
        }
    }
}

/// Close out a job whose `pending` set is empty: `completed` when
/// nothing landed in the DLQ, `completed_with_failures` otherwise, then
/// broadcast the `blast_completed` event with the final counters.
async fn finalize_job(state: &AppState, job_id: &str, session_id: &str) -> anyhow::Result<()> {
    let pool = state.session_manager().pool();
    let final_status = match blast::get_job(pool, job_id).await? {
        Some(row) if row.dlq_count > 0 => BlastJobStatus::CompletedWithFailures,
        _ => BlastJobStatus::Completed,
    };
    blast::finish_job(pool, job_id, final_status.as_str()).await?;
    broadcast_completed(state, job_id, session_id, final_status).await;
    Ok(())
}

/// Current counters of a job as a webhook payload object.
async fn counters_payload(state: &AppState, job_id: &str) -> serde_json::Value {
    match blast::get_job(state.session_manager().pool(), job_id).await {
        Ok(Some(row)) => serde_json::json!({
            "total": row.total,
            "sent_count": row.sent_count,
            "failed_count": row.failed_count,
            "dlq_count": row.dlq_count,
            "skipped_dup_count": row.skipped_dup_count,
        }),
        _ => serde_json::json!({}),
    }
}

/// Emit a `blast_progress` event with the job's current counters.
async fn broadcast_progress(state: &AppState, job_id: &str, session_id: &str) {
    let counters = counters_payload(state, job_id).await;
    let payload = serde_json::json!({
        "job_id": job_id,
        "session_id": session_id,
        "counters": counters,
    });
    state
        .broadcast_to_webhooks(
            session_id,
            WebhookEvent::BlastProgress.as_str(),
            &payload.to_string(),
        )
        .await;
}

/// Emit the terminal `blast_completed` event with the final status and
/// counters.
async fn broadcast_completed(
    state: &AppState,
    job_id: &str,
    session_id: &str,
    status: BlastJobStatus,
) {
    let counters = counters_payload(state, job_id).await;
    let payload = serde_json::json!({
        "job_id": job_id,
        "session_id": session_id,
        "status": status.as_str(),
        "counters": counters,
    });
    state
        .broadcast_to_webhooks(
            session_id,
            WebhookEvent::BlastCompleted.as_str(),
            &payload.to_string(),
        )
        .await;
}

/// Rewrite a stored job body for one recipient: `to` becomes the
/// recipient's canonical JID and `send_at` is forced to `null` so the
/// dispatch can never park the send in the scheduler.
fn build_recipient_body(body: &str, recipient: &str) -> anyhow::Result<String> {
    let mut value: serde_json::Value = serde_json::from_str(body)?;
    if let Some(obj) = value.as_object_mut() {
        obj.insert(
            "to".to_string(),
            serde_json::Value::String(recipient.to_string()),
        );
        obj.insert("send_at".to_string(), serde_json::Value::Null);
    }
    Ok(serde_json::to_string(&value)?)
}

/// Per-send pause: the job's base delay plus a uniform random
/// `0..=jitter_ms` extra.
fn send_delay(options: &BlastOptions) -> Duration {
    let jitter = if options.jitter_ms > 0 {
        rand::thread_rng().gen_range(0..=options.jitter_ms)
    } else {
        0
    };
    Duration::from_millis(options.delay_ms + jitter)
}
