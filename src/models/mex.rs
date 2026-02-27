use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
pub struct MexQueryRequest {
    /// GraphQL document ID
    #[schema(example = "1234567890")]
    pub doc_id: String,

    /// Query variables as JSON
    pub variables: Value,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct MexMutateRequest {
    /// GraphQL document ID
    #[schema(example = "1234567890")]
    pub doc_id: String,

    /// Mutation variables as JSON
    pub variables: Value,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MexApiResponse {
    pub data: Option<Value>,
    pub errors: Option<Vec<MexGraphQLErrorItem>>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MexGraphQLErrorItem {
    pub message: String,
    pub error_code: Option<i32>,
    pub is_retryable: Option<bool>,
    pub severity: Option<String>,
}
