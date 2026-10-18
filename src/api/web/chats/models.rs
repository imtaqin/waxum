use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
pub struct ArchiveChatRequest {
    pub archived: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PinChatRequest {
    pub pinned: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct MuteChatRequest {
    pub muted: bool,
    /// Unix seconds when the mute ends; omit to mute forever.
    pub until: Option<i64>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct MarkChatReadRequest {
    /// `false` marks the chat unread.
    pub read: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ClearChatRequest {
    /// Keep starred messages (WhatsApp's default).
    #[serde(default = "default_true")]
    pub keep_starred: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct DisappearingRequest {
    /// Timer in seconds: 0 (off), 86400, 604800 or 7776000.
    pub seconds: u32,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct StarMessageRequest {
    #[schema(example = "628123456789@s.whatsapp.net")]
    pub chat: String,
    pub message_id: String,
    pub starred: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct DeleteForMeRequest {
    #[schema(example = "628123456789@s.whatsapp.net")]
    pub chat: String,
    pub message_id: String,
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ChatListQuery {
    /// Include archived chats (default false).
    pub archived: Option<bool>,
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ChatSummary {
    pub jid: String,
    pub name: Option<String>,
    pub last_message_at: Option<i64>,
    pub last_message_preview: Option<String>,
    pub last_message_kind: Option<String>,
    /// `-1` means manually marked unread.
    pub unread_count: i32,
    pub pinned: bool,
    pub muted_until: Option<i64>,
    pub archived: bool,
    pub ephemeral_expiration: Option<u32>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ChatListResponse {
    pub chats: Vec<ChatSummary>,
    pub count: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct StarredMessage {
    pub chat_jid: String,
    pub message_id: String,
    pub sender_jid: String,
    pub from_me: bool,
    pub timestamp: i64,
    pub kind: String,
    pub text: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct StarredResponse {
    pub messages: Vec<StarredMessage>,
    pub count: usize,
}

fn default_true() -> bool {
    true
}
