use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema)]
pub struct PrivacySettingsResponse {
    pub settings: Vec<PrivacySettingItem>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PrivacySettingItem {
    pub category: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub enum PrivacyCategory {
    Last,
    Online,
    Profile,
    Status,
    GroupAdd,
    ReadReceipts,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub enum PrivacyValue {
    All,
    Contacts,
    None,
    ContactBlacklist,
    MatchLastSeen,
}
