use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChatStateType {
    Composing,

    Recording,

    Paused,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SendChatStateRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    pub state: ChatStateType,
}
