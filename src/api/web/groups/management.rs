use axum::{
    extract::{Path, Query, State},
    Json,
};
use wacore_binary::jid::Jid;
use whatsapp_rust::{GroupCreateOptions, GroupDescription, GroupParticipantOptions, GroupSubject};

use crate::api::web::groups::models::*;
use crate::error::ApiError;
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
        options.membership_approval_mode = Some(match mode {
            crate::api::web::groups::models::MembershipApprovalMode::Off => {
                whatsapp_rust::MembershipApprovalMode::Off
            }
            crate::api::web::groups::models::MembershipApprovalMode::On => {
                whatsapp_rust::MembershipApprovalMode::On
            }
        });
    }

    if let Some(mode) = request.member_add_mode {
        options = options.with_member_add_mode(match mode {
            crate::api::web::groups::models::MemberAddMode::AdminAdd => {
                whatsapp_rust::MemberAddMode::AdminAdd
            }
            crate::api::web::groups::models::MemberAddMode::AllMemberAdd => {
                whatsapp_rust::MemberAddMode::AllMemberAdd
            }
        });
    }

    if let Some(mode) = request.member_link_mode {
        options = options.with_member_link_mode(match mode {
            crate::api::web::groups::models::MemberLinkMode::AdminLink => {
                whatsapp_rust::MemberLinkMode::AdminLink
            }
            crate::api::web::groups::models::MemberLinkMode::AllMemberLink => {
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
        .set_description(&jid, description, request.prev_id.as_deref().into())
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

    let groups = client.groups();
    let internal = |e: whatsapp_rust::GroupError| ApiError::Internal(e.to_string());
    if let Some(mode) = request.membership_approval_mode {
        let mode = match mode {
            MembershipApprovalMode::On => whatsapp_rust::MembershipApprovalMode::On,
            MembershipApprovalMode::Off => whatsapp_rust::MembershipApprovalMode::Off,
        };
        groups
            .set_membership_approval(jid.clone(), mode)
            .await
            .map_err(internal)?;
    }
    if let Some(mode) = request.member_add_mode {
        let mode = match mode {
            MemberAddMode::AdminAdd => whatsapp_rust::MemberAddMode::AdminAdd,
            MemberAddMode::AllMemberAdd => whatsapp_rust::MemberAddMode::AllMemberAdd,
        };
        groups
            .set_member_add_mode(jid.clone(), mode)
            .await
            .map_err(internal)?;
    }
    if let Some(mode) = request.member_link_mode {
        let mode = match mode {
            MemberLinkMode::AdminLink => whatsapp_rust::MemberLinkMode::AdminLink,
            MemberLinkMode::AllMemberLink => whatsapp_rust::MemberLinkMode::AllMemberLink,
        };
        groups
            .set_member_link_mode(&jid, mode)
            .await
            .map_err(internal)?;
    }
    if let Some(announce) = request.announce {
        groups
            .set_announce(jid.clone(), announce)
            .await
            .map_err(internal)?;
    }
    if let Some(locked) = request.locked {
        groups
            .set_locked(jid.clone(), locked)
            .await
            .map_err(internal)?;
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

#[utoipa::path(
    put,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/groups/{group_jid}/picture",
    tag = "groups",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("group_jid" = String, Path, description = "Group JID")
    ),
    request_body = SetGroupPictureRequest,
    responses(
        (status = 200, description = "Group picture changed", body = crate::api::common::SuccessResponse),
        (status = 503, description = "Not connected")
    )
)]
pub async fn set_group_picture(
    State(state): State<AppState>,
    Path((session_id, group_jid)): Path<(String, String)>,
    Json(request): Json<SetGroupPictureRequest>,
) -> Result<Json<crate::api::common::SuccessResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let jid: Jid = group_jid
        .parse()
        .map_err(|_| ApiError::InvalidJid(group_jid.clone()))?;
    let (bytes, _) = crate::api::shared::messages::handlers::get_media_data(&request.image).await?;
    let jpeg = crate::api::web::profile::handlers::square_jpeg(bytes)?;
    client
        .groups()
        .set_profile_picture(jid, jpeg)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(crate::api::common::SuccessResponse::new()))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/groups/join",
    tag = "groups",
    params(("session_id" = String, Path, description = "Session ID")),
    request_body = JoinGroupRequest,
    responses(
        (status = 200, description = "Joined, or join request sent for approval", body = JoinGroupResponse),
        (status = 503, description = "Not connected")
    )
)]
pub async fn join_group(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<JoinGroupRequest>,
) -> Result<Json<JoinGroupResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let result = client
        .groups()
        .join_with_invite_code(&request.code)
        .await
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    let pending_approval = matches!(result, whatsapp_rust::JoinGroupResult::PendingApproval(_));
    Ok(Json(JoinGroupResponse {
        group_jid: result.group_jid().to_string(),
        pending_approval,
    }))
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/groups/invite/{code}",
    tag = "groups",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("code" = String, Path, description = "Invite code")
    ),
    responses(
        (status = 200, description = "What the invite leads to, without joining", body = InviteInfoResponse),
        (status = 503, description = "Not connected")
    )
)]
pub async fn get_invite_info(
    State(state): State<AppState>,
    Path((session_id, code)): Path<(String, String)>,
) -> Result<Json<InviteInfoResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let metadata = client
        .groups()
        .get_invite_info(&code)
        .await
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    Ok(Json(InviteInfoResponse {
        group_jid: metadata.id.to_string(),
        subject: metadata.subject,
        participants: metadata.participants.len(),
        creator: metadata.creator.map(|j| j.to_string()),
        created_at: metadata.creation_time,
    }))
}
