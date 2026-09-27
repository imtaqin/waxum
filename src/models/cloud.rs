use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::sessions::SessionInfo;

/// Attaches WhatsApp Cloud API credentials to a session, switching its
/// `provider` to `whatsapp_cloud`. `access_token`, `app_secret` and
/// `webhook_verify_token` are stored but never echoed back by any GET
/// response — see [`SessionInfo`], which omits them entirely.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ConnectCloudRequest {
    /// WhatsApp Business Account ID (`WABA-ID`)
    #[schema(example = "102290129340398")]
    pub waba_id: String,
    /// Phone number ID to send/receive through (`Phone-Number-ID`)
    #[schema(example = "106540352242922")]
    pub phone_number_id: String,
    /// Meta Business ID that owns the WABA
    #[schema(example = "102290129340399")]
    pub business_id: Option<String>,
    /// System user or Embedded-Signup-exchanged access token
    pub access_token: String,
    /// Meta app ID used to receive this WABA's webhooks
    pub app_id: Option<String>,
    /// Meta app secret, used to verify `X-Hub-Signature-256` on incoming
    /// webhook deliveries
    pub app_secret: String,
    /// Verify token to echo back on the webhook `GET` handshake
    /// (`hub.verify_token`)
    pub webhook_verify_token: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ConnectCloudResponse {
    pub session: SessionInfo,
}
