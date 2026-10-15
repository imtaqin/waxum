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
