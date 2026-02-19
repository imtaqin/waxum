use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema)]
pub struct GroupListResponse {
    pub groups: Vec<GroupInfo>,

    pub total: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct GroupInfo {
    pub jid: String,

    pub subject: String,

    pub participants: Vec<GroupParticipant>,

    pub addressing_mode: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct GroupInfoCached {
    pub participants: Vec<GroupParticipant>,

    pub addressing_mode: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct GroupParticipant {
    pub jid: String,

    pub phone_number: Option<String>,

    pub role: ParticipantRole,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ParticipantRole {
    Member,

    Admin,

    SuperAdmin,
}

// Group Management Request/Response Models

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateGroupRequest {
    #[schema(example = "My New Group")]
    pub name: String,

    #[schema(example = json!(["559999999999@s.whatsapp.net"]))]
    pub participants: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CreateGroupResponse {
    pub group_jid: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ParticipantsRequest {
    #[schema(example = json!(["559999999999@s.whatsapp.net"]))]
    pub participants: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ParticipantChangeResult {
    pub jid: String,
    pub status: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ParticipantsResponse {
    pub results: Vec<ParticipantChangeResult>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetSubjectRequest {
    #[schema(example = "New Group Name")]
    pub subject: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetDescriptionRequest {
    #[schema(example = "This is the group description")]
    pub description: Option<String>,

    pub prev_id: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct InviteLinkResponse {
    pub invite_link: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct GetInviteLinkRequest {
    #[serde(default)]
    pub reset: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SuccessResponse {
    pub success: bool,
}
