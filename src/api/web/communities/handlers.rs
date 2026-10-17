use axum::{
    extract::{Path, State},
    Json,
};
use wacore_binary::jid::Jid;

use crate::api::common::SuccessResponse;
use crate::api::shared::messages::handlers::{get_client, parse_jid};
use crate::api::web::communities::models::{
    CommunityListResponse, CommunitySubgroupItem, CommunitySubgroupsResponse, CommunitySummary,
    CreateCommunityRequest, CreateCommunityResponse,
};
use crate::error::ApiError;
use crate::state::AppState;

fn internal(e: impl std::fmt::Display) -> ApiError {
    ApiError::Internal(e.to_string())
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/communities",
    tag = "communities",
    params(("session_id" = String, Path, description = "Session ID")),
    responses(
        (status = 200, description = "Communities this account belongs to", body = CommunityListResponse),
        (status = 503, description = "Not connected")
    )
)]
pub async fn list_communities(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<CommunityListResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let metadata = client
        .community()
        .fetch_participating_metadata()
        .await
        .map_err(internal)?;
    let mut communities: Vec<CommunitySummary> = metadata
        .into_values()
        .map(|m| CommunitySummary {
            jid: m.id.to_string(),
            name: m.subject.clone().unwrap_or_default(),
            description: m.description.clone(),
            participants: m.participants.len(),
        })
        .collect();
    communities.sort_by_key(|c| c.name.to_lowercase());
    Ok(Json(CommunityListResponse { communities }))
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/communities/{jid}/subgroups",
    tag = "communities",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("jid" = String, Path, description = "Community JID")
    ),
    responses(
        (status = 200, description = "Groups linked to the community", body = CommunitySubgroupsResponse),
        (status = 503, description = "Not connected")
    )
)]
pub async fn list_subgroups(
    State(state): State<AppState>,
    Path((session_id, jid)): Path<(String, String)>,
) -> Result<Json<CommunitySubgroupsResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let community = parse_jid(&jid)?;
    let subgroups = client
        .community()
        .get_subgroups(&community)
        .await
        .map_err(internal)?
        .into_iter()
        .map(|g| CommunitySubgroupItem {
            jid: g.id.to_string(),
            subject: g.subject,
            participants: g.participant_count,
            announcement: g.is_default_sub_group,
            general_chat: g.is_general_chat,
        })
        .collect();
    Ok(Json(CommunitySubgroupsResponse { subgroups }))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/communities/{jid}/subgroups/{subgroup}/join",
    tag = "communities",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("jid" = String, Path, description = "Community JID"),
        ("subgroup" = String, Path, description = "Linked group JID")
    ),
    responses(
        (status = 200, description = "Joined the linked group", body = SuccessResponse),
        (status = 503, description = "Not connected")
    )
)]
pub async fn join_subgroup(
    State(state): State<AppState>,
    Path((session_id, jid, subgroup)): Path<(String, String, String)>,
) -> Result<Json<SuccessResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let community: Jid = parse_jid(&jid)?;
    let subgroup: Jid = parse_jid(&subgroup)?;
    client
        .community()
        .join_subgroup(community, subgroup)
        .await
        .map_err(internal)?;
    Ok(Json(SuccessResponse::new()))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/communities",
    tag = "communities",
    params(("session_id" = String, Path, description = "Session ID")),
    request_body = CreateCommunityRequest,
    responses(
        (status = 200, description = "Community created", body = CreateCommunityResponse),
        (status = 503, description = "Not connected")
    )
)]
pub async fn create_community(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<CreateCommunityRequest>,
) -> Result<Json<CreateCommunityResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let options = whatsapp_rust::features::CreateCommunityOptions {
        name: request.name,
        description: request.description,
        closed: false,
        allow_non_admin_sub_group_creation: false,
        create_general_chat: false,
    };
    let created = client.community().create(options).await.map_err(internal)?;
    Ok(Json(CreateCommunityResponse {
        jid: created.metadata.id.to_string(),
    }))
}
