use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
pub struct CheckOnWhatsAppRequest {
    #[schema(example = json!(["559999999999", "551234567890"]))]
    pub phones: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CheckOnWhatsAppResponse {
    pub results: Vec<WhatsAppCheckResult>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WhatsAppCheckResult {
    pub phone: String,

    pub jid: Option<String>,

    pub is_registered: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct GetContactInfoRequest {
    pub phones: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ContactInfoResponse {
    pub contacts: Vec<ContactInfo>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ContactInfo {
    pub jid: String,

    pub lid: Option<String>,

    pub is_registered: bool,

    pub is_business: bool,

    pub status: Option<String>,

    pub picture_id: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ProfilePictureResponse {
    pub url: Option<String>,

    pub direct_path: Option<String>,

    pub picture_id: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct GetUserInfoRequest {
    pub jids: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UserInfoResponse {
    pub users: Vec<UserInfo>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UserInfo {
    pub jid: String,

    pub lid: Option<String>,

    pub status: Option<String>,

    pub is_business: bool,

    pub picture_id: Option<String>,
}
