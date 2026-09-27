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

/// `POST /sessions/{id}/cloud/flows` -- creates a Flow in draft status.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateFlowRequest {
    #[schema(example = "Appointment booking")]
    pub name: String,
    /// At least one of: `SIGN_UP`, `SIGN_IN`, `APPOINTMENT_BOOKING`,
    /// `LEAD_GENERATION`, `CONTACT_US`, `CUSTOMER_SUPPORT`, `SURVEY`,
    /// `OTHER`.
    #[schema(example = json!(["APPOINTMENT_BOOKING"]))]
    pub categories: Vec<String>,
    pub clone_flow_id: Option<String>,
}

/// `POST /sessions/{id}/cloud/flows/{flow_id}` -- updates a Flow's
/// name/categories/endpoint URI. Every field is optional; only the ones
/// present are sent to Meta.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateFlowMetadataRequest {
    pub name: Option<String>,
    pub categories: Option<Vec<String>>,
    pub endpoint_uri: Option<String>,
}

/// `POST /sessions/{id}/cloud/flow-endpoint` -- configures this session
/// as a Flows Data Exchange endpoint. The public half of the RSA keypair
/// is registered with Meta; the private half is stored on the session and
/// never returned by any GET.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ConfigureFlowEndpointRequest {
    /// PEM RSA private key (PKCS#1 or PKCS#8, 2048-bit or larger). Omit it
    /// to have waxum generate a fresh 2048-bit keypair, so the private key
    /// never has to leave this server at all.
    #[schema(example = "-----BEGIN PRIVATE KEY-----\n...\n-----END PRIVATE KEY-----")]
    pub private_key: Option<String>,
    /// Public `http`/`https` URL that decrypted `INIT`/`data_exchange`/
    /// `BACK` requests are POSTed to as JSON; its JSON reply is encrypted
    /// and returned to Meta as the next screen. Omit it to answer only
    /// Meta's health-check ping.
    #[schema(example = "https://example.com/flows/handler")]
    pub forward_url: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ConfigureFlowEndpointResponse {
    /// The public key that was registered with Meta.
    pub public_key: String,
    /// The URL to set as the Flow's `endpoint_uri`.
    #[schema(
        example = "https://waxum.example.com/api/v1/sessions/cloud-1/cloud/flow-endpoint/exchange"
    )]
    pub endpoint_path: String,
    /// Meta's raw response to the key registration.
    pub meta_response: serde_json::Value,
}

/// `POST /sessions/{id}/cloud/flows/{flow_id}/send` -- sends a Flow as an
/// interactive message, per "Send Flow".
#[derive(Debug, Deserialize, ToSchema)]
pub struct SendFlowRequest {
    #[schema(example = "6281234567890")]
    pub to: String,
    #[schema(example = "Book now")]
    pub flow_cta: String,
    #[schema(example = "Pick a time slot for your appointment")]
    pub body: String,
    pub header: Option<String>,
    pub footer: Option<String>,
    /// Opaque token echoed back in every data-exchange request and in the
    /// completion webhook, for correlating a Flow run with your own state.
    pub flow_token: Option<String>,
    /// `published` (default) or `draft` -- draft Flows can only be sent to
    /// the WABA's own test numbers.
    pub mode: Option<String>,
    /// `navigate` (default) opens `screen` directly; `data_exchange` asks
    /// the Flow endpoint for the first screen instead.
    pub flow_action: Option<String>,
    /// First screen ID, required when `flow_action` is `navigate`.
    pub screen: Option<String>,
    /// Initial data for `screen`.
    pub data: Option<serde_json::Value>,
}
