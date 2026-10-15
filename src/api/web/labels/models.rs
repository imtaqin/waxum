use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreateLabelRequest {
    #[schema(example = "my-label-id")]
    pub label_id: String,
    #[schema(example = "My Label")]
    pub name: String,
    /// Hex color without #, e.g. "ff0000"
    #[serde(default)]
    pub color: Option<String>,
    /// Optional color id (0-19) as used by WA Web
    #[serde(default)]
    pub color_id: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ChatLabelRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub chat_jid: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct MessageLabelRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub chat_jid: String,
    #[schema(example = "ABCD1234")]
    pub message_id: String,
    #[serde(default = "default_true")]
    pub from_me: bool,
    #[serde(default)]
    pub participant: Option<String>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct QuickReplyRequest {
    #[schema(example = "qr-id-1")]
    pub id: String,
    #[schema(example = "/hello")]
    pub shortcut: String,
    #[schema(example = "Hello! How can I help?")]
    pub message: String,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub count: i32,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct LinkPreviewsRequest {
    pub disabled: bool,
}
