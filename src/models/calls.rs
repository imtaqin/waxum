use serde::Deserialize;
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
pub struct RejectCallRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub from: String,
    #[schema(example = "2E3F4A5B6C7D")]
    pub call_id: String,
}
