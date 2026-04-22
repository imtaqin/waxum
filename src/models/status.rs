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
