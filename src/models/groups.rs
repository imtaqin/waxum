use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Response with list of groups
#[derive(Debug, Serialize, ToSchema)]
pub struct GroupListResponse {
    /// List of groups
    pub groups: Vec<GroupInfo>,
    /// Total count
    pub total: usize,
}

/// Group information
#[derive(Debug, Serialize, ToSchema)]
pub struct GroupInfo {
    /// Group JID
    pub jid: String,
    /// Group subject/name
    pub subject: String,
    /// Group participants
    pub participants: Vec<GroupParticipant>,
    /// Addressing mode (Pn or Lid)
    pub addressing_mode: String,
}

/// Group info with caching
#[derive(Debug, Serialize, ToSchema)]
pub struct GroupInfoCached {
    /// Group participants
    pub participants: Vec<GroupParticipant>,
    /// Addressing mode
    pub addressing_mode: String,
}

/// Group participant
#[derive(Debug, Serialize, ToSchema)]
pub struct GroupParticipant {
    /// Participant JID
    pub jid: String,
    /// Phone number if available
    pub phone_number: Option<String>,
    /// Participant role
    pub role: ParticipantRole,
}

/// Participant role in group
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ParticipantRole {
    /// Regular member
    Member,
    /// Group admin
    Admin,
    /// Group super admin (creator)
    SuperAdmin,
}
