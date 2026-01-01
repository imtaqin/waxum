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
