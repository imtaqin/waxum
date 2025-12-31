use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Request to check if phone numbers are on WhatsApp
#[derive(Debug, Deserialize, ToSchema)]
pub struct CheckOnWhatsAppRequest {
    /// List of phone numbers to check
    #[schema(example = json!(["559999999999", "551234567890"]))]
    pub phones: Vec<String>,
}

/// Response with WhatsApp registration status
#[derive(Debug, Serialize, ToSchema)]
pub struct CheckOnWhatsAppResponse {
    /// Check results for each phone
    pub results: Vec<WhatsAppCheckResult>,
}

/// Result of checking a phone number
#[derive(Debug, Serialize, ToSchema)]
pub struct WhatsAppCheckResult {
    /// Phone number queried
    pub phone: String,
    /// JID if registered
    pub jid: Option<String>,
    /// Whether the phone is registered on WhatsApp
    pub is_registered: bool,
}

/// Request to get contact info
#[derive(Debug, Deserialize, ToSchema)]
pub struct GetContactInfoRequest {
    /// List of phone numbers
    pub phones: Vec<String>,
}

/// Response with contact information
#[derive(Debug, Serialize, ToSchema)]
pub struct ContactInfoResponse {
    /// Contact information
    pub contacts: Vec<ContactInfo>,
}

/// Contact information
#[derive(Debug, Serialize, ToSchema)]
pub struct ContactInfo {
    /// Contact JID
    pub jid: String,
    /// Linked ID
    pub lid: Option<String>,
    /// Whether registered on WhatsApp
    pub is_registered: bool,
    /// Whether this is a business account
    pub is_business: bool,
    /// Status text
    pub status: Option<String>,
    /// Profile picture ID
    pub picture_id: Option<String>,
}

/// Profile picture response
#[derive(Debug, Serialize, ToSchema)]
pub struct ProfilePictureResponse {
    /// Picture URL
    pub url: Option<String>,
    /// Direct path
    pub direct_path: Option<String>,
    /// Picture ID
    pub picture_id: Option<String>,
}

/// Request to get user info
#[derive(Debug, Deserialize, ToSchema)]
pub struct GetUserInfoRequest {
    /// List of JIDs
    pub jids: Vec<String>,
}

/// Response with user information
#[derive(Debug, Serialize, ToSchema)]
pub struct UserInfoResponse {
    /// User information
    pub users: Vec<UserInfo>,
}

/// User information
#[derive(Debug, Serialize, ToSchema)]
pub struct UserInfo {
    /// User JID
    pub jid: String,
    /// Linked ID
    pub lid: Option<String>,
    /// Status text
    pub status: Option<String>,
    /// Whether this is a business account
    pub is_business: bool,
    /// Profile picture ID
    pub picture_id: Option<String>,
}
