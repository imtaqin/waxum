use serde::Serialize;
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema)]
pub struct BotListResponse {
    pub bots: serde_json::Value,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CappingResponse {
    pub capping: serde_json::Value,
}
