use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema)]
pub struct BlocklistResponse {
    pub blocked: Vec<String>,

    pub count: usize,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct BlockRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub jid: String,
}
