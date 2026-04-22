use axum::{
    extract::{Path, State},
    Json,
};
use wacore_binary::jid::Jid;

use crate::error::ApiError;
use crate::models::groups::*;
use crate::state::AppState;

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/groups",
    tag = "groups",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    responses(
        (status = 200, description = "List of groups", body = GroupListResponse),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn list_groups(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<GroupListResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;

    let groups = client
        .groups()
        .get_participating()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let groups: Vec<GroupInfo> = groups
        .into_iter()
        .map(|(id, metadata)| GroupInfo {
            jid: id,
            subject: metadata.subject,
            participants: metadata
                .participants
                .into_iter()
                .map(|p| GroupParticipant {
                    jid: p.jid.to_string(),
                    phone_number: p.phone_number.as_ref().map(|j| j.to_string()),
                    role: if p.is_admin() {
                        ParticipantRole::Admin
                    } else {
                        ParticipantRole::Member
                    },
                })
                .collect(),
            addressing_mode: format!("{:?}", metadata.addressing_mode),
        })
        .collect();

    let total = groups.len();
    Ok(Json(GroupListResponse { groups, total }))
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/groups/{group_jid}",
    tag = "groups",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("group_jid" = String, Path, description = "Group JID")
    ),
    responses(
        (status = 200, description = "Group info", body = GroupInfo),
        (status = 404, description = "Session or group not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn get_group(
    State(state): State<AppState>,
    Path((session_id, group_jid)): Path<(String, String)>,
) -> Result<Json<GroupInfo>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let jid: Jid = group_jid
        .parse()
        .map_err(|_| ApiError::InvalidJid(group_jid.clone()))?;

    let metadata = client
        .groups()
        .get_metadata(&jid)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(GroupInfo {
        jid: metadata.id.to_string(),
        subject: metadata.subject,
        participants: metadata
            .participants
            .into_iter()
            .map(|p| GroupParticipant {
                jid: p.jid.to_string(),
                phone_number: p.phone_number.as_ref().map(|j| j.to_string()),
                role: if p.is_admin() {
                    ParticipantRole::Admin
                } else {
                    ParticipantRole::Member
                },
            })
            .collect(),
        addressing_mode: format!("{:?}", metadata.addressing_mode),
    }))
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/groups/{group_jid}/info",
    tag = "groups",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("group_jid" = String, Path, description = "Group JID")
    ),
    responses(
        (status = 200, description = "Group info", body = GroupInfoCached),
        (status = 404, description = "Session or group not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn get_group_info(
    State(state): State<AppState>,
    Path((session_id, group_jid)): Path<(String, String)>,
) -> Result<Json<GroupInfoCached>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let jid: Jid = group_jid
        .parse()
        .map_err(|_| ApiError::InvalidJid(group_jid.clone()))?;

    let info = client
        .groups()
        .query_info(&jid)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(GroupInfoCached {
        participants: info
            .participants
            .into_iter()
            .map(|p| GroupParticipant {
                jid: p.to_string(),
                phone_number: None,
                role: ParticipantRole::Member,
            })
            .collect(),
        addressing_mode: format!("{:?}", info.addressing_mode),
    }))
}

fn get_client(
    state: &AppState,
    session_id: &str,
) -> Result<std::sync::Arc<whatsapp_rust::Client>, ApiError> {
    let runtime = state
        .get_session(session_id)
        .ok_or(ApiError::NotConnected)?;

    runtime.get_client().ok_or(ApiError::NotConnected)
}
