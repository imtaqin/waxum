use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SetPresenceRequest {

    pub status: PresenceStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PresenceStatus {

    Available,

    Unavailable,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct ChatStateRequest {

    #[schema(example = "559999999999@s.whatsapp.net")]
    pub chat: String,

    pub state: ChatStateType,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub enum ChatStateType {

    Composing,

    Recording,

    Paused,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct PresenceUpdate {

    pub jid: String,

    pub status: PresenceStatus,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct ChatStateUpdate {

    pub jid: String,

    pub chat: String,

    pub state: ChatStateType,
}
