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

/// Body for `POST /sessions/{id}/cloud/embedded-signup/exchange`: the
/// OAuth `code` the client-side Embedded Signup JS SDK callback hands
/// back, plus the app credentials needed to exchange it.
#[derive(Debug, Deserialize, ToSchema)]
pub struct EmbeddedSignupExchangeRequest {
    pub code: String,
    pub app_id: String,
    pub app_secret: String,
    /// WABA ID the Embedded Signup JS SDK's `message` event handed back
    /// alongside the `code` -- needed here to subscribe the app and list
    /// phone numbers; the token exchange itself doesn't return it.
    pub waba_id: String,
}

/// The exchanged access token plus the WABA's phone numbers, so the
/// caller can pick one and finish onboarding with `POST
/// /sessions/{id}/cloud/connect`. `access_token` is returned here (unlike
/// `SessionInfo`, which never echoes a stored one back) because it has
/// not been persisted yet at this point -- the caller needs it to
/// complete the following `connect` call.
#[derive(Debug, Serialize, ToSchema)]
pub struct EmbeddedSignupExchangeResponse {
    pub access_token: String,
    pub waba_id: String,
    pub phone_numbers: serde_json::Value,
}

/// `POST /sessions/{id}/messages/template` -- sends an approved WhatsApp
/// message template. Cloud API only; a `whatsapp_web` session rejects
/// this with 400, same as any other cloud-only endpoint hit the other
/// way around. `components` is the already-shaped Cloud API
/// template-component array (see `src/cloud/client.rs::send_template`
/// doc comment for why it isn't modeled field-by-field).
#[derive(Debug, Deserialize, ToSchema)]
pub struct SendTemplateRequest {
    #[schema(example = "559999999999")]
    pub to: String,
    #[schema(example = "order_confirmation")]
    pub name: String,
    #[schema(example = "en_US")]
    pub language_code: String,
    #[serde(default)]
    pub components: serde_json::Value,
}
