use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreateNewsletterRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub picture_b64: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct UpdateNewsletterRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct NewsletterJidQuery {
    #[schema(example = "120363000000000000@newsletter")]
    pub jid: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ChangeOwnerRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub user: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct MuteRequest {
    pub muted: bool,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct InviteMetadataRequest {
    #[schema(example = "INVITE_CODE")]
    pub invite_code: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct NewsletterMetadataResponse {
    pub metadata: serde_json::Value,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct NewsletterListResponse {
    pub newsletters: serde_json::Value,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct NewsletterAdminInfoResponse {
    pub admin_info: serde_json::Value,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct NewsletterFollowersResponse {
    pub followers: serde_json::Value,
}

#[derive(Debug, serde::Deserialize, utoipa::IntoParams)]
pub struct NewsletterMessagesQuery {
    /// How many messages to return (default 50, at most 100).
    pub count: Option<u32>,
    /// Server id to page back from: only messages older than it.
    pub before: Option<u64>,
}

#[derive(Debug, serde::Serialize, ToSchema)]
pub struct NewsletterReaction {
    pub emoji: String,
    pub count: u64,
}

#[derive(Debug, serde::Serialize, ToSchema)]
pub struct NewsletterMessageItem {
    pub message_id: String,
    pub server_id: u64,
    pub timestamp: u64,
    pub text: Option<String>,
    pub views: Option<u64>,
    pub reactions: Vec<NewsletterReaction>,
    /// The message protobuf, base64. Absent for messages the server sent
    /// without content (revoked ones).
    pub message_proto: Option<String>,
}

#[derive(Debug, serde::Serialize, ToSchema)]
pub struct NewsletterMessagesResponse {
    pub messages: Vec<NewsletterMessageItem>,
}
