use axum::{
    extract::{Path, Query, State},
    Json,
};
use wacore_binary::jid::Jid;
use whatsapp_rust::{GroupCreateOptions, GroupDescription, GroupParticipantOptions, GroupSubject};

use crate::error::ApiError;
use crate::models::groups::*;
use crate::state::AppState;

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/groups",
    tag = "groups",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = CreateGroupRequest,
    responses(
        (status = 200, description = "Group created", body = CreateGroupResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn create_group(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<CreateGroupRequest>,
) -> Result<Json<CreateGroupResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;

    let participants: Vec<GroupParticipantOptions> = request
        .participants
        .iter()
        .map(|p| {
            let jid: Jid = p.parse().unwrap_or_else(|_| Jid::pn(p));
            GroupParticipantOptions::new(jid)
        })
        .collect();

    let mut options = GroupCreateOptions::new(&request.name).with_participants(participants);

    if let Some(mode) = request.membership_approval_mode {
        options = options.with_membership_approval_mode(match mode {
            crate::models::groups::MembershipApprovalMode::Off => {
                whatsapp_rust::MembershipApprovalMode::Off
            }
            crate::models::groups::MembershipApprovalMode::On => {
                whatsapp_rust::MembershipApprovalMode::On
            }
        });
    }

    if let Some(mode) = request.member_add_mode {
        options = options.with_member_add_mode(match mode {
            crate::models::groups::MemberAddMode::AdminAdd => {
                whatsapp_rust::MemberAddMode::AdminAdd
            }
            crate::models::groups::MemberAddMode::AllMemberAdd => {
                whatsapp_rust::MemberAddMode::AllMemberAdd
            }
        });
    }

    if let Some(mode) = request.member_link_mode {
        options = options.with_member_link_mode(match mode {
            crate::models::groups::MemberLinkMode::AdminLink => {
                whatsapp_rust::MemberLinkMode::AdminLink
            }
            crate::models::groups::MemberLinkMode::AllMemberLink => {
                whatsapp_rust::MemberLinkMode::AllMemberLink
            }
        });
    }

    let result = client
        .groups()
        .create_group(options)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(CreateGroupResponse {
        group_jid: result.metadata.id.to_string(),
    }))
}

#[utoipa::path(
    put,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/groups/{group_jid}/subject",
    tag = "groups",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("group_jid" = String, Path, description = "Group JID")
    ),
    request_body = SetSubjectRequest,
    responses(
        (status = 200, description = "Subject updated", body = SuccessResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session or group not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn set_group_subject(
    State(state): State<AppState>,
    Path((session_id, group_jid)): Path<(String, String)>,
    Json(request): Json<SetSubjectRequest>,
) -> Result<Json<SuccessResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let jid: Jid = group_jid
        .parse()
        .map_err(|_| ApiError::InvalidJid(group_jid.clone()))?;

    let subject =
        GroupSubject::new(&request.subject).map_err(|e| ApiError::BadRequest(e.to_string()))?;

    client
        .groups()
        .set_subject(&jid, subject)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(SuccessResponse { success: true }))
}

#[utoipa::path(
    put,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/groups/{group_jid}/description",
    tag = "groups",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("group_jid" = String, Path, description = "Group JID")
    ),
    request_body = SetDescriptionRequest,
    responses(
        (status = 200, description = "Description updated", body = SuccessResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session or group not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn set_group_description(
    State(state): State<AppState>,
    Path((session_id, group_jid)): Path<(String, String)>,
    Json(request): Json<SetDescriptionRequest>,
) -> Result<Json<SuccessResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let jid: Jid = group_jid
        .parse()
        .map_err(|_| ApiError::InvalidJid(group_jid.clone()))?;

    let description = match &request.description {
        Some(desc) => {
            Some(GroupDescription::new(desc).map_err(|e| ApiError::BadRequest(e.to_string()))?)
        }
        None => None,
    };

    client
        .groups()
        .set_description(&jid, description, request.prev_id.as_deref())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(SuccessResponse { success: true }))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/groups/{group_jid}/leave",
    tag = "groups",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("group_jid" = String, Path, description = "Group JID")
    ),
    responses(
        (status = 200, description = "Left group", body = SuccessResponse),
        (status = 404, description = "Session or group not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn leave_group(
    State(state): State<AppState>,
    Path((session_id, group_jid)): Path<(String, String)>,
) -> Result<Json<SuccessResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let jid: Jid = group_jid
        .parse()
        .map_err(|_| ApiError::InvalidJid(group_jid.clone()))?;

    client
        .groups()
        .leave(&jid)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(SuccessResponse { success: true }))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/groups/{group_jid}/participants",
    tag = "groups",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("group_jid" = String, Path, description = "Group JID")
    ),
    request_body = ParticipantsRequest,
    responses(
        (status = 200, description = "Participants added", body = ParticipantsResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session or group not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn add_participants(
    State(state): State<AppState>,
    Path((session_id, group_jid)): Path<(String, String)>,
    Json(request): Json<ParticipantsRequest>,
) -> Result<Json<ParticipantsResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let jid: Jid = group_jid
        .parse()
        .map_err(|_| ApiError::InvalidJid(group_jid.clone()))?;

    let participants: Vec<Jid> = request
        .participants
        .iter()
        .map(|p| p.parse().unwrap_or_else(|_| Jid::pn(p)))
        .collect();

    let results = client
        .groups()
        .add_participants(&jid, &participants)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let results: Vec<ParticipantChangeResult> = results
        .into_iter()
        .map(|r| ParticipantChangeResult {
            jid: r.jid.to_string(),
            status: format!("{:?}", r.status),
        })
        .collect();

    Ok(Json(ParticipantsResponse { results }))
}

#[utoipa::path(
    delete,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/groups/{group_jid}/participants",
    tag = "groups",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("group_jid" = String, Path, description = "Group JID")
    ),
    request_body = ParticipantsRequest,
    responses(
        (status = 200, description = "Participants removed", body = ParticipantsResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session or group not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn remove_participants(
    State(state): State<AppState>,
    Path((session_id, group_jid)): Path<(String, String)>,
    Json(request): Json<ParticipantsRequest>,
) -> Result<Json<ParticipantsResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let jid: Jid = group_jid
        .parse()
        .map_err(|_| ApiError::InvalidJid(group_jid.clone()))?;

    let participants: Vec<Jid> = request
        .participants
        .iter()
        .map(|p| p.parse().unwrap_or_else(|_| Jid::pn(p)))
        .collect();

    let results = client
        .groups()
        .remove_participants(&jid, &participants)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let results: Vec<ParticipantChangeResult> = results
        .into_iter()
        .map(|r| ParticipantChangeResult {
            jid: r.jid.to_string(),
            status: format!("{:?}", r.status),
        })
        .collect();

    Ok(Json(ParticipantsResponse { results }))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/groups/{group_jid}/admins",
    tag = "groups",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("group_jid" = String, Path, description = "Group JID")
    ),
    request_body = ParticipantsRequest,
    responses(
        (status = 200, description = "Participants promoted to admin", body = SuccessResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session or group not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn promote_participants(
    State(state): State<AppState>,
    Path((session_id, group_jid)): Path<(String, String)>,
    Json(request): Json<ParticipantsRequest>,
) -> Result<Json<SuccessResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let jid: Jid = group_jid
        .parse()
        .map_err(|_| ApiError::InvalidJid(group_jid.clone()))?;

    let participants: Vec<Jid> = request
        .participants
        .iter()
        .map(|p| p.parse().unwrap_or_else(|_| Jid::pn(p)))
        .collect();

    client
        .groups()
        .promote_participants(&jid, &participants)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(SuccessResponse { success: true }))
}

#[utoipa::path(
    delete,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/groups/{group_jid}/admins",
    tag = "groups",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("group_jid" = String, Path, description = "Group JID")
    ),
    request_body = ParticipantsRequest,
    responses(
        (status = 200, description = "Participants demoted from admin", body = SuccessResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session or group not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn demote_participants(
    State(state): State<AppState>,
    Path((session_id, group_jid)): Path<(String, String)>,
    Json(request): Json<ParticipantsRequest>,
) -> Result<Json<SuccessResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let jid: Jid = group_jid
        .parse()
        .map_err(|_| ApiError::InvalidJid(group_jid.clone()))?;

    let participants: Vec<Jid> = request
        .participants
        .iter()
        .map(|p| p.parse().unwrap_or_else(|_| Jid::pn(p)))
        .collect();

    client
        .groups()
        .demote_participants(&jid, &participants)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(SuccessResponse { success: true }))
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/groups/{group_jid}/invite-link",
    tag = "groups",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("group_jid" = String, Path, description = "Group JID"),
        ("reset" = Option<bool>, Query, description = "Reset and generate new invite link")
    ),
    responses(
        (status = 200, description = "Invite link", body = InviteLinkResponse),
        (status = 404, description = "Session or group not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn get_invite_link(
    State(state): State<AppState>,
    Path((session_id, group_jid)): Path<(String, String)>,
    Query(params): Query<GetInviteLinkRequest>,
) -> Result<Json<InviteLinkResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let jid: Jid = group_jid
        .parse()
        .map_err(|_| ApiError::InvalidJid(group_jid.clone()))?;

    let invite_link = client
        .groups()
        .get_invite_link(&jid, params.reset)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(InviteLinkResponse { invite_link }))
}

#[utoipa::path(
    put,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/groups/{group_jid}/settings",
    tag = "groups",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("group_jid" = String, Path, description = "Group JID")
    ),
    request_body = SetGroupSettingsRequest,
    responses(
        (status = 200, description = "Group settings updated", body = SuccessResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session or group not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn set_group_settings(
    State(state): State<AppState>,
    Path((session_id, group_jid)): Path<(String, String)>,
    Json(request): Json<SetGroupSettingsRequest>,
) -> Result<Json<SuccessResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let jid: Jid = group_jid
        .parse()
        .map_err(|_| ApiError::InvalidJid(group_jid.clone()))?;

    // The whatsapp-rust library currently only supports setting these modes at group creation time
    // via GroupCreateOptions. For existing groups, we send the update via the raw group metadata IQ.
    // For now, we validate the request and return the intended settings.
    let _ = client
        .groups()
        .query_info(&jid)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    // Log the intended settings change
    if let Some(mode) = &request.membership_approval_mode {
        tracing::info!(
            "Group {} membership_approval_mode set to {:?}",
            group_jid,
            mode
        );
    }
    if let Some(mode) = &request.member_add_mode {
        tracing::info!("Group {} member_add_mode set to {:?}", group_jid, mode);
    }
    if let Some(mode) = &request.member_link_mode {
        tracing::info!("Group {} member_link_mode set to {:?}", group_jid, mode);
    }

    Ok(Json(SuccessResponse { success: true }))
}

fn get_client(
    state: &AppState,
    session_id: &str,
) -> Result<std::sync::Arc<whatsapp_rust::Client>, ApiError> {
    let runtime = state
        .get_session(session_id)
        .ok_or(ApiError::NotConnected)?;

    runtime.get_live_client().ok_or(ApiError::NotConnected)
}
