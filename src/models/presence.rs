use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Request to set presence status
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SetPresenceRequest {
    /// Presence status to set
    pub status: PresenceStatus,
}

/// Presence status options
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PresenceStatus {
    /// Online/available
    Available,
    /// Offline/unavailable
    Unavailable,
}

/// Request to send chat state (typing indicator)
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct ChatStateRequest {
    /// Chat JID to send state to
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub chat: String,
    /// Chat state type
    pub state: ChatStateType,
}

/// Chat state types (legacy, use chatstate::ChatStateType instead)
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub enum ChatStateType {
    /// User is typing
    Composing,
    /// User is recording audio
    Recording,
    /// User stopped typing/recording
    Paused,
}

/// Presence update event (for webhooks)
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct PresenceUpdate {
    /// JID of the contact
    pub jid: String,
    /// Presence status
    pub status: PresenceStatus,
    /// Last seen timestamp (if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<i64>,
}

/// Chat state update event (for webhooks)
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct ChatStateUpdate {
    /// JID of the contact
    pub jid: String,
    /// Chat JID (for groups)
    pub chat: String,
    /// Chat state
    pub state: ChatStateType,
}
