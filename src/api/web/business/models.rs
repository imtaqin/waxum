use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CatalogQuery {
    /// Business JID that owns the catalog (e.g. 559999999999@s.whatsapp.net)
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub jid: String,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CollectionsQuery {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub jid: String,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub collection_limit: Option<u32>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct OrderQuery {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub jid: String,
    #[schema(example = "ORDER_ID_123")]
    pub order_id: String,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct BusinessProfileUpdateRequest {
    /// Partial business profile fields — all optional, only sent fields are patched.
    /// Mirrors `BusinessProfileUpdate` in wacore::iq::business.
    pub description: Option<String>,
    pub email: Option<String>,
    pub websites: Option<Vec<String>>,
    pub address: Option<String>,
    pub category: Option<String>,
    pub business_hours: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SetCoverPhotoRequest {
    /// Upload spec — base64 or URL handled by caller; here we accept raw bytes as base64 string
    pub image_b64: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BusinessCatalogResponse {
    pub products: serde_json::Value,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BusinessCollectionsResponse {
    pub collections: serde_json::Value,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BusinessOrderResponse {
    pub order: serde_json::Value,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BusinessProfileResponse {
    pub result: serde_json::Value,
}
