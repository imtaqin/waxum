use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Blocklist response
#[derive(Debug, Serialize, ToSchema)]
pub struct BlocklistResponse {
    /// List of blocked JIDs
    pub blocked: Vec<String>,
    /// Count of blocked contacts
    pub count: usize,
}

/// Request to block/unblock a contact
#[derive(Debug, Deserialize, ToSchema)]
pub struct BlockRequest {
    /// JID to block/unblock
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub jid: String,
}
