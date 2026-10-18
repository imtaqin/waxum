use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema)]
pub struct CommunitySummary {
    pub jid: String,
    pub name: String,
    pub description: Option<String>,
    pub participants: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CommunityListResponse {
    pub communities: Vec<CommunitySummary>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CommunitySubgroupItem {
    pub jid: String,
    pub subject: String,
    pub participants: Option<u32>,
    /// The community's announcement group.
    pub announcement: bool,
    pub general_chat: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CommunitySubgroupsResponse {
    pub subgroups: Vec<CommunitySubgroupItem>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateCommunityRequest {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CreateCommunityResponse {
    pub jid: String,
}
