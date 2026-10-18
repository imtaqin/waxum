use serde::Deserialize;
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
pub struct StatusReactionRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub status_owner: String,
    #[schema(example = "3EB0C431C26D15B441EE15")]
    pub message_id: String,
    #[schema(example = "\u{1F49A}")]
    pub reaction: String,
}

#[derive(Debug, serde::Deserialize, ToSchema)]
pub struct TextStatusRequest {
    pub text: String,
    /// Background colour as `0xAARRGGBB` (default WhatsApp teal).
    pub background_argb: Option<u32>,
    /// WhatsApp font index: 0 sans, 1 serif, 2 script, 6 bold, 7 condensed.
    pub font: Option<i32>,
}

#[derive(Debug, serde::Deserialize, ToSchema)]
pub struct ImageStatusRequest {
    pub image: crate::api::shared::messages::models::MediaData,
    pub caption: Option<String>,
}

#[derive(Debug, serde::Serialize, ToSchema)]
pub struct StatusPostResponse {
    pub message_id: String,
    /// How many contacts the status was sent to.
    pub recipients: usize,
}
