use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Chat state type
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChatStateType {
    /// Typing indicator
    Composing,
    /// Recording audio indicator
    Recording,
    /// Paused typing
    Paused,
}

/// Request to send chat state
#[derive(Debug, Deserialize, ToSchema)]
pub struct SendChatStateRequest {
    /// Recipient JID
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,
    /// Chat state to send
    pub state: ChatStateType,
}
