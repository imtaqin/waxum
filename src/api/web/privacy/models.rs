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

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetPrivacyRequest {
    /// `last`, `online`, `profile`, `status`, `groupadd`, `readreceipts`,
    /// `calladd` or `messages`.
    #[schema(example = "last")]
    pub category: String,
    /// `all`, `contacts`, `none`, `contact_blacklist`, `match_last_seen`
    /// or `known`, as the category allows.
    #[schema(example = "contacts")]
    pub value: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct DefaultDisappearingRequest {
    /// Timer for new chats in seconds: 0 (off), 86400, 604800 or 7776000.
    pub seconds: u32,
}
