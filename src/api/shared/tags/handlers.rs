//! Session tag CRUD.
//!
//! Tags are free-form short strings an operator attaches to a session
//! to organise a fleet — e.g. `cs`, `blast-campaign-2`, `client:acme`,
//! `region:jkt`. They are held in-memory on [`AppState`] and snapshotted
//! to `{WHATSAPP_STORAGE_PATH}/session_tags.json` on every mutation, so
//! restarts do not wipe organisation. Not on the send hot path — the
//! session listing consults them only when `?tag=` is present, and the
//! console reads them for its overview grouping.
//!
//! Endpoints:
//! - `GET    /api/v1/sessions/{sid}/tags` — list tags on a session.
//! - `PUT    /api/v1/sessions/{sid}/tags` — replace the full set.
//! - `POST   /api/v1/sessions/{sid}/tags` — add a single tag.
//! - `DELETE /api/v1/sessions/{sid}/tags/{tag}` — remove one tag.
//! - `GET    /api/v1/tags` — list every distinct tag with counts.

use axum::{
    extract::{Path, State},
    Json,
};
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::state::AppState;

#[derive(Serialize)]
pub struct TagListResponse {
    pub session_id: String,
    pub tags: Vec<String>,
}

pub async fn list_session_tags(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<TagListResponse>, ApiError> {
    let tags = state.list_tags(&session_id);
    Ok(Json(TagListResponse { session_id, tags }))
}

#[derive(Deserialize)]
pub struct ReplaceTagsBody {
    pub tags: Vec<String>,
}

pub async fn replace_session_tags(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(body): Json<ReplaceTagsBody>,
) -> Result<Json<TagListResponse>, ApiError> {
    state.set_tags(&session_id, body.tags).await;
    let tags = state.list_tags(&session_id);
    Ok(Json(TagListResponse { session_id, tags }))
}

#[derive(Deserialize)]
pub struct AddTagBody {
    pub tag: String,
}

#[derive(Serialize)]
pub struct TagMutateResponse {
    pub session_id: String,
    pub tag: String,
    pub changed: bool,
    pub tags: Vec<String>,
}

pub async fn add_session_tag(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(body): Json<AddTagBody>,
) -> Result<Json<TagMutateResponse>, ApiError> {
    if body.tag.trim().is_empty() {
        return Err(ApiError::BadRequest("tag is empty".into()));
    }
    let changed = state.add_tag(&session_id, &body.tag).await;
    Ok(Json(TagMutateResponse {
        session_id: session_id.clone(),
        tag: body.tag,
        changed,
        tags: state.list_tags(&session_id),
    }))
}

pub async fn remove_session_tag(
    State(state): State<AppState>,
    Path((session_id, tag)): Path<(String, String)>,
) -> Result<Json<TagMutateResponse>, ApiError> {
    let changed = state.remove_tag(&session_id, &tag).await;
    Ok(Json(TagMutateResponse {
        session_id: session_id.clone(),
        tag,
        changed,
        tags: state.list_tags(&session_id),
    }))
}

#[derive(Serialize)]
pub struct TagCount {
    pub tag: String,
    pub session_count: usize,
}

pub async fn list_all_tags(State(state): State<AppState>) -> Result<Json<Vec<TagCount>>, ApiError> {
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for sid in state.all_session_ids_with_tags() {
        for tag in state.list_tags(&sid) {
            *counts.entry(tag).or_insert(0) += 1;
        }
    }
    let mut out: Vec<TagCount> = counts
        .into_iter()
        .map(|(tag, session_count)| TagCount { tag, session_count })
        .collect();
    out.sort_by(|a, b| {
        b.session_count
            .cmp(&a.session_count)
            .then(a.tag.cmp(&b.tag))
    });
    Ok(Json(out))
}
