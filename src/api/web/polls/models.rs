use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
pub struct PollVoteRequest {
    /// Names of the chosen options; an empty list withdraws the vote.
    pub options: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PollOptionTally {
    pub name: String,
    pub voters: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PollResultsResponse {
    pub question: String,
    /// How many options a voter may pick; 0 means any number.
    pub selectable_count: u32,
    pub options: Vec<PollOptionTally>,
    /// Options this account currently has selected.
    pub my_votes: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PollVoteResponse {
    pub message_id: String,
}
