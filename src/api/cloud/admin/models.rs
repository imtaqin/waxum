//! Request types for the WhatsApp Cloud API account-administration and
//! BSP surface: phone number registration and verification, business
//! profile, templates, QR codes, webhook subscriptions, blocked users,
//! India business compliance, analytics, payments messages, Flow extras,
//! and the Embedded Signup system-user / credit-line-sharing steps.
//!
//! Responses from these routes are Meta's own JSON, returned unchanged,
//! so only request bodies and query strings are modeled here.

use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

/// `POST /sessions/{id}/cloud/register`, per "Register Phone" and
/// "OnPrem Account Migration".
#[derive(Debug, Deserialize, ToSchema)]
pub struct RegisterPhoneRequest {
    /// The number's 6-digit two-step verification PIN (sets it if none
    /// exists yet).
    #[schema(example = "123456")]
    pub pin: String,
    /// Local storage region (e.g. `ID`, `IN`, `SG`), for Cloud API local
    /// storage.
    pub data_localization_region: Option<String>,
    /// Only when migrating a number off the On-Premises API.
    pub backup: Option<OnPremBackup>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct OnPremBackup {
    pub data: String,
    pub password: String,
}

/// `POST /sessions/{id}/cloud/request-code`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RequestCodeRequest {
    /// `SMS` or `VOICE`.
    #[schema(example = "SMS")]
    pub code_method: String,
    #[schema(example = "en_US")]
    pub locale: Option<String>,
}

/// `POST /sessions/{id}/cloud/verify-code`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct VerifyCodeRequest {
    #[schema(example = "123456")]
    pub code: String,
}

/// `POST /sessions/{id}/cloud/two-step-pin`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct TwoStepPinRequest {
    #[schema(example = "123456")]
    pub pin: String,
}

/// `POST /sessions/{id}/cloud/business-profile`. Omitted fields are left
/// unchanged.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateBusinessProfileRequest {
    pub about: Option<String>,
    pub address: Option<String>,
    pub description: Option<String>,
    pub email: Option<String>,
    /// Meta's industry enum, e.g. `RETAIL`, `EDU`, `PROF_SERVICES`.
    pub vertical: Option<String>,
    pub websites: Option<Vec<String>>,
    /// Handle from `POST /cloud/business-profile/photo`'s upload, when
    /// setting the photo by hand.
    pub profile_picture_handle: Option<String>,
}

/// `POST /sessions/{id}/cloud/typing` -- shows the typing indicator and
/// marks the customer's message as read.
#[derive(Debug, Deserialize, ToSchema)]
pub struct TypingIndicatorRequest {
    #[schema(example = "wamid.HBgLMTU1NTEyMzQ1NjcVAgASGBQzQTZ")]
    pub message_id: String,
}

/// `POST /sessions/{id}/cloud/subscribed-apps`. With both fields set this
/// is "Override Callback URL"; with neither it's a plain subscribe.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SubscribeAppRequest {
    pub override_callback_uri: Option<String>,
    pub verify_token: Option<String>,
}

/// `POST /sessions/{id}/cloud/templates`, per the "Create template ..."
/// requests. `components` is Meta's component array as-is (HEADER, BODY,
/// FOOTER, BUTTONS, with `example` values).
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateTemplateRequest {
    #[schema(example = "seasonal_promotion")]
    pub name: String,
    #[schema(example = "en_US")]
    pub language: String,
    /// `MARKETING`, `UTILITY` or `AUTHENTICATION`.
    #[schema(example = "MARKETING")]
    pub category: String,
    pub components: serde_json::Value,
    pub allow_category_change: Option<bool>,
    pub parameter_format: Option<String>,
    pub message_send_ttl_seconds: Option<i64>,
}

/// `POST /sessions/{id}/cloud/templates/{template_id}`, per "Edit
/// template". At least one field is required.
#[derive(Debug, Deserialize, ToSchema)]
pub struct EditTemplateRequest {
    pub category: Option<String>,
    pub components: Option<serde_json::Value>,
    pub message_send_ttl_seconds: Option<i64>,
}

/// Filters for `GET /sessions/{id}/cloud/templates`.
#[derive(Debug, Deserialize, IntoParams)]
pub struct ListTemplatesQuery {
    pub name: Option<String>,
    pub status: Option<String>,
    pub category: Option<String>,
    pub language: Option<String>,
    pub limit: Option<u32>,
    /// Paging cursor from a previous response's `paging.cursors.after`.
    pub after: Option<String>,
    pub before: Option<String>,
}

/// `DELETE /sessions/{id}/cloud/templates` -- by name (every language),
/// or one language version when `hsm_id` is also given.
#[derive(Debug, Deserialize, IntoParams)]
pub struct DeleteTemplateQuery {
    pub name: String,
    pub hsm_id: Option<String>,
}

/// `POST /sessions/{id}/cloud/qr-codes`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateQrCodeRequest {
    #[schema(example = "Hi, I'd like to know more about your promo")]
    pub prefilled_message: String,
    /// `SVG` (default) or `PNG`.
    pub generate_qr_image: Option<String>,
}

/// `POST /sessions/{id}/cloud/qr-codes/{code}`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateQrCodeRequest {
    pub prefilled_message: String,
}

/// `GET /sessions/{id}/cloud/qr-codes` -- `format` adds `qr_image_url` in
/// that format to each code.
#[derive(Debug, Deserialize, IntoParams)]
pub struct QrCodeQuery {
    /// `SVG` or `PNG`.
    pub format: Option<String>,
}

/// `POST`/`DELETE /sessions/{id}/cloud/blocked-users`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct BlockUsersRequest {
    #[schema(example = json!(["6281234567890"]))]
    pub users: Vec<String>,
}

/// `POST /sessions/{id}/cloud/business-compliance`, per "Add India-based
/// business compliance info".
#[derive(Debug, Deserialize, ToSchema)]
pub struct BusinessComplianceRequest {
    pub entity_name: String,
    /// e.g. `LIMITED_LIABILITY_PARTNERSHIP`, `PRIVATE_COMPANY`,
    /// `SOLE_PROPRIETORSHIP`, `OTHER`.
    pub entity_type: String,
    pub is_registered: bool,
    pub other_entity_type: Option<String>,
    pub grievance_officer_details: Option<serde_json::Value>,
    pub customer_care_details: Option<serde_json::Value>,
}

/// `GET /sessions/{id}/cloud/analytics`, per "Get analytics".
#[derive(Debug, Deserialize, IntoParams)]
pub struct AnalyticsQuery {
    /// Unix seconds.
    pub start: i64,
    /// Unix seconds.
    pub end: i64,
    /// `HALF_HOUR`, `DAY` or `MONTH`.
    pub granularity: String,
    /// Comma-separated phone numbers; all when omitted.
    pub phone_numbers: Option<String>,
    /// Comma-separated ISO country codes.
    pub country_codes: Option<String>,
}

/// `GET /sessions/{id}/cloud/conversation-analytics`, per "Get
/// conversation analytics".
#[derive(Debug, Deserialize, IntoParams)]
pub struct ConversationAnalyticsQuery {
    pub start: i64,
    pub end: i64,
    /// `HALF_HOUR`, `DAILY` or `MONTHLY`.
    pub granularity: String,
    /// Comma-separated, e.g. `business_initiated,user_initiated`.
    pub conversation_directions: Option<String>,
    /// Comma-separated, e.g. `conversation_type,conversation_direction`.
    pub dimensions: Option<String>,
    pub conversation_categories: Option<String>,
    pub conversation_types: Option<String>,
    pub phone_numbers: Option<String>,
    pub country_codes: Option<String>,
}

/// `GET /sessions/{id}/cloud/flows/{flow_id}/metrics`, per "Get Endpoint
/// Metrics".
#[derive(Debug, Deserialize, IntoParams)]
pub struct FlowMetricsQuery {
    /// `ENDPOINT_REQUEST_COUNT`, `ENDPOINT_REQUEST_ERROR`,
    /// `ENDPOINT_REQUEST_ERROR_RATE`, `ENDPOINT_REQUEST_LATENCY_SECONDS_CEIL`
    /// or `ENDPOINT_AVAILABILITY`.
    pub metric: String,
    /// `DAY`, `HOUR` or `LIFETIME`.
    pub granularity: String,
    /// `YYYY-MM-DD`.
    pub since: Option<String>,
    /// `YYYY-MM-DD`.
    pub until: Option<String>,
}

/// `POST /sessions/{id}/cloud/flows-migrate`, per "Migrate Flows".
#[derive(Debug, Deserialize, ToSchema)]
pub struct MigrateFlowsRequest {
    pub source_waba_id: String,
    /// Flow names to copy; all Flows when omitted.
    pub source_flow_names: Option<Vec<String>>,
}

/// `POST /sessions/{id}/messages/order-details`, per the Payments API
/// "Send Order Details Message". `parameters` is Meta's payment object
/// as-is (`reference_id`, `type`, `payment_type`, `payment_configuration`,
/// `currency`, `total_amount`, `order`, ...).
#[derive(Debug, Deserialize, ToSchema)]
pub struct SendOrderDetailsRequest {
    pub to: String,
    /// `IN` (default, UPI and payment gateways) or `SG`; the two regions
    /// nest the message differently.
    pub region: Option<String>,
    /// Optional header object, e.g. `{"type":"image","image":{"link":"..."}}`.
    pub header: Option<serde_json::Value>,
    pub body: String,
    pub footer: Option<String>,
    pub parameters: serde_json::Value,
}

/// `POST /sessions/{id}/messages/order-status`, per "Send Order Status
/// Message".
#[derive(Debug, Deserialize, ToSchema)]
pub struct SendOrderStatusRequest {
    pub to: String,
    pub body: String,
    /// The `reference_id` of the order-details message being updated.
    pub reference_id: String,
    /// e.g. `processing`, `partially_shipped`, `shipped`, `completed`,
    /// `canceled`.
    pub status: String,
    pub description: Option<String>,
}

/// `POST /sessions/{id}/cloud/assigned-users`, per Embedded Signup "Add
/// System User to WABA".
#[derive(Debug, Deserialize, ToSchema)]
pub struct AssignUserRequest {
    pub user_id: String,
    /// e.g. `["MANAGE"]`, `["DEVELOP"]`, `["MANAGE_TEMPLATES"]`.
    pub tasks: Vec<String>,
}

/// `POST /sessions/{id}/cloud/credit-sharing`, per Embedded Signup
/// "Attach Your Credit Line to the client's WABA".
#[derive(Debug, Deserialize, ToSchema)]
pub struct ShareCreditLineRequest {
    pub credit_line_id: String,
    /// The client WABA's billing currency, e.g. `USD`, `IDR`.
    #[schema(example = "USD")]
    pub waba_currency: String,
}
