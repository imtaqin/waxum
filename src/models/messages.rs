use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

/// Fake reply config — makes the outgoing message look like it's replying to a
/// fictional previous message from a random JID. Used for blast to appear more
/// natural / human-like.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct FakeReplyConfig {
    /// text | product | order | location | video | document | contact
    #[serde(rename = "type")]
    pub reply_type: String,

    pub title: Option<String>,

    pub body: Option<String>,

    /// Override random participant JID (optional). Format: 628xxx@s.whatsapp.net
    pub participant: Option<String>,

    /// Override random stanza id (optional).
    pub stanza_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SendTextRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    #[schema(example = "Hello, World!")]
    pub text: String,

    pub reply_to: Option<String>,

    pub fake_reply: Option<FakeReplyConfig>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct SendImageRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    pub image: MediaData,

    pub caption: Option<String>,

    pub reply_to: Option<String>,

    pub fake_reply: Option<FakeReplyConfig>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct SendVideoRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    pub video: MediaData,

    pub caption: Option<String>,

    pub reply_to: Option<String>,

    pub fake_reply: Option<FakeReplyConfig>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct SendAudioRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    pub audio: MediaData,

    #[serde(default)]
    pub ptt: bool,

    pub reply_to: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct SendDocumentRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    pub document: MediaData,

    #[schema(example = "document.pdf")]
    pub filename: String,

    pub caption: Option<String>,

    pub reply_to: Option<String>,

    pub fake_reply: Option<FakeReplyConfig>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct SendStickerRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    pub sticker: MediaData,

    pub reply_to: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct SendLocationRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    #[schema(example = -23.5505)]
    pub latitude: f64,

    #[schema(example = -46.6333)]
    pub longitude: f64,

    pub name: Option<String>,

    pub address: Option<String>,

    pub reply_to: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct SendContactRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    pub contact: ContactCard,

    pub reply_to: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ContactCard {
    pub display_name: String,

    pub phones: Vec<ContactPhone>,

    pub organization: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ContactPhone {
    pub number: String,

    #[serde(default = "default_phone_type")]
    pub phone_type: String,
}

fn default_phone_type() -> String {
    "CELL".to_string()
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct EditMessageRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    pub message_id: String,

    pub text: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SendReactionRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    pub message_id: String,

    #[schema(example = "👍")]
    pub emoji: String,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(untagged)]
#[allow(dead_code)]
pub enum MediaData {
    Url {
        url: String,
    },

    Base64 {
        data: String,
        mimetype: String,
    },

    Uploaded {
        url: String,
        direct_path: String,
        media_key: String,
        file_sha256: String,
        file_enc_sha256: String,
        file_length: u64,
        mimetype: String,
    },
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct MessageResponse {
    pub message_id: String,

    pub timestamp: i64,

    pub to: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct SendMessageRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    #[schema(example = "Hello, World!")]
    pub text: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RevokeMessageRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    pub message_id: String,

    #[schema(example = "559888888888@s.whatsapp.net")]
    pub original_sender: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct MarkAsReadRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub chat_jid: String,

    #[schema(example = "559888888888@s.whatsapp.net")]
    pub sender: Option<String>,

    pub message_ids: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SuccessResponse {
    pub success: bool,
}

// --- Poll Messages ---

#[derive(Debug, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct SendPollRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    #[schema(example = "What's your favorite color?")]
    pub name: String,

    #[schema(example = json!(["Red", "Blue", "Green"]))]
    pub options: Vec<String>,

    /// Max number of selectable options (0 = unlimited)
    #[serde(default)]
    pub selectable_count: u32,

    pub reply_to: Option<String>,
}

// --- Buttons Messages ---

#[derive(Debug, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct SendButtonsRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    #[schema(example = "Please choose an option")]
    pub content_text: String,

    pub footer: Option<String>,

    pub buttons: Vec<ButtonItem>,

    pub header_text: Option<String>,

    pub reply_to: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ButtonItem {
    pub button_id: String,
    pub display_text: String,
}

// --- List Messages ---

#[derive(Debug, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct SendListRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    #[schema(example = "Main Menu")]
    pub title: String,

    #[schema(example = "Please select an option")]
    pub description: String,

    #[schema(example = "View Options")]
    pub button_text: String,

    pub sections: Vec<ListSection>,

    pub footer: Option<String>,

    pub reply_to: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ListSection {
    pub title: String,
    pub rows: Vec<ListRow>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ListRow {
    pub row_id: String,
    pub title: String,
    pub description: Option<String>,
}

// --- Interactive Messages (Native Flow) ---

#[derive(Debug, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct SendInteractiveRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    pub body_text: String,

    pub footer_text: Option<String>,

    pub buttons: Vec<NativeFlowButtonItem>,

    pub reply_to: Option<String>,

    /// Optional fake-reply quoted message context. Adds a fabricated "quoted"
    /// header above the message — some consumer WhatsApp builds only render
    /// native_flow buttons as clickable when wrapped in a quoted context.
    pub fake_reply: Option<FakeReplyConfig>,

    /// Wrap the interactive payload in `viewOnceMessageV2`. Empirically the
    /// only reliable way to get native_flow quick-reply buttons clickable on
    /// consumer WhatsApp accounts (unverified business). Defaults to true.
    #[serde(default = "default_true")]
    pub view_once: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct NativeFlowButtonItem {
    pub name: String,
    pub button_params_json: String,
}

// --- CTA URL button (single call-to-action that opens a URL) ---

#[derive(Debug, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct SendCtaUrlRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    /// Body text shown above the button.
    pub body_text: String,

    /// Optional footer text shown beneath the body.
    pub footer_text: Option<String>,

    /// Visible label on the button (e.g. "Open website").
    pub display_text: String,

    /// Public URL the button opens.
    pub url: String,

    /// Optional merchant URL — falls back to `url` when omitted. Some
    /// clients show this in the link preview UI.
    pub merchant_url: Option<String>,

    /// Optional image to show above the CTA body. Accepts either an
    /// `{ "url": "https://…" }` or `{ "data": "<base64>", "mimetype": "image/jpeg" }`
    /// object. When set, waxum uploads the image to WhatsApp and
    /// attaches it as the interactive header media, so the button
    /// appears with an image thumbnail on the recipient side.
    pub image: Option<MediaData>,

    pub reply_to: Option<String>,
}

// --- Quick Reply buttons (modern native-flow replacement for legacy ButtonsMessage) ---

#[derive(Debug, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct SendQuickReplyRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    pub body_text: String,

    pub footer_text: Option<String>,

    /// 1-3 reply buttons. WhatsApp clients clip beyond 3.
    pub buttons: Vec<QuickReplyButtonItem>,

    pub reply_to: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct QuickReplyButtonItem {
    /// Internal ID returned to your webhook when the user taps the button.
    pub id: String,

    /// Visible label on the button.
    pub display_text: String,
}

// --- Template Messages ---

#[derive(Debug, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct SendTemplateRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    /// Raw template message content as JSON (passed through to protobuf)
    pub content: Value,

    pub reply_to: Option<String>,
}

// --- Newsletter Messages ---

#[derive(Debug, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct SendNewsletterAdminInviteRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    pub newsletter_jid: String,

    pub newsletter_name: String,

    pub caption: Option<String>,

    pub invite_expiration: Option<i64>,

    pub reply_to: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct SendNewsletterFollowerInviteRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    pub newsletter_jid: String,

    pub newsletter_name: String,

    pub caption: Option<String>,

    pub reply_to: Option<String>,
}

// --- Business Messages ---

#[derive(Debug, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct SendOrderRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    pub order_id: String,

    pub item_count: Option<i32>,

    /// Order status: "inquiry", "accepted", "declined"
    pub status: Option<String>,

    pub message: Option<String>,

    pub order_title: Option<String>,

    pub seller_jid: Option<String>,

    pub token: Option<String>,

    pub total_amount_1000: Option<i64>,

    pub total_currency_code: Option<String>,

    pub reply_to: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct SendInvoiceRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    pub note: Option<String>,

    pub token: Option<String>,

    /// "image" or "pdf"
    pub attachment_type: Option<String>,

    pub attachment_mimetype: Option<String>,

    pub reply_to: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct SendPaymentInviteRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    /// Payment service type (integer)
    pub service_type: Option<i32>,

    pub reply_to: Option<String>,
}

// --- Pin Message ---

#[derive(Debug, Deserialize, ToSchema)]
pub struct SendPinMessageRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub chat: String,

    pub message_id: String,

    /// Pin duration in seconds (0 to unpin, 86400 for 24h, 604800 for 7d, 2592000 for 30d)
    #[serde(default = "default_pin_duration")]
    pub duration_seconds: i64,
}

fn default_pin_duration() -> i64 {
    86400
}

// --- Forward Message ---

#[derive(Debug, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct ForwardMessageRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    pub text: String,

    pub reply_to: Option<String>,
}

// --- Poll Update (Vote) ---

#[derive(Debug, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct SendPollUpdateRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    /// The message ID of the poll creation message
    pub poll_message_id: String,

    /// Selected option hashes (SHA-256 of option text)
    pub selected_options: Vec<String>,

    /// Encryption IV for the vote (base64)
    pub enc_iv: Option<String>,

    /// Encryption key (base64)
    pub enc_payload: Option<String>,
}

// --- Buttons Response ---

#[derive(Debug, Deserialize, ToSchema)]
pub struct SendButtonsResponseRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    /// ID of the selected button
    pub selected_button_id: String,

    /// Display text of the selected button
    pub selected_display_text: String,

    pub reply_to: Option<String>,
}

// --- List Response ---

#[derive(Debug, Deserialize, ToSchema)]
pub struct SendListResponseRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    pub title: String,

    /// ID of the selected row
    pub selected_row_id: String,

    pub description: Option<String>,

    pub reply_to: Option<String>,
}

// --- Interactive Response ---

#[derive(Debug, Deserialize, ToSchema)]
pub struct SendInteractiveResponseRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    /// Body text of the response
    pub body_text: Option<String>,

    /// Native flow response name
    pub name: String,

    /// Native flow response params (JSON string)
    pub params_json: String,

    /// Version of the native flow response
    #[serde(default = "default_version")]
    pub version: i32,

    pub reply_to: Option<String>,
}

fn default_version() -> i32 {
    3
}

// --- Highly Structured Message (HSM) ---

#[derive(Debug, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct SendHighlyStructuredRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    pub namespace: String,

    pub element_name: String,

    #[serde(default)]
    pub params: Vec<String>,

    pub fallback_lg: Option<String>,

    pub fallback_lc: Option<String>,

    pub reply_to: Option<String>,
}

// --- Template Button Reply ---

#[derive(Debug, Deserialize, ToSchema)]
pub struct SendTemplateButtonReplyRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    pub selected_id: String,

    pub selected_display_text: String,

    pub selected_index: Option<u32>,

    pub reply_to: Option<String>,
}

// --- Comment Message (Groups) ---

#[derive(Debug, Deserialize, ToSchema)]
pub struct SendCommentRequest {
    /// JID of the channel / community-announce group the comment lives in
    /// (the same `chat` JID where the parent post was published).
    #[schema(example = "120363000000000000@g.us")]
    pub to: String,

    /// The text content of the comment
    pub text: String,

    /// Message ID of the post being commented on
    pub target_message_id: String,

    /// Optional override for the chat JID embedded in the target key. Defaults
    /// to `to` when omitted.
    pub target_chat_jid: Option<String>,

    /// Optional author JID of the parent post. For encrypted CAG comments the
    /// receivers key decryption off this field; if omitted the lib resolves
    /// it from the locally-stored message secret.
    pub target_participant: Option<String>,
}

// --- Scheduled Call ---

#[derive(Debug, Deserialize, ToSchema)]
pub struct SendScheduledCallRequest {
    #[schema(example = "120363000000000000@g.us")]
    pub to: String,

    /// Scheduled call time as Unix timestamp in milliseconds
    pub scheduled_timestamp_ms: i64,

    /// "voice" or "video"
    #[serde(default = "default_call_type")]
    pub call_type: String,

    pub title: Option<String>,
}

fn default_call_type() -> String {
    "voice".to_string()
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SendScheduledCallEditRequest {
    #[schema(example = "120363000000000000@g.us")]
    pub to: String,

    /// Message ID of the scheduled call creation message
    pub scheduled_call_message_id: String,

    /// "cancel"
    #[serde(default = "default_edit_type")]
    pub edit_type: String,
}

fn default_edit_type() -> String {
    "cancel".to_string()
}

// --- Send Payment ---

#[derive(Debug, Deserialize, ToSchema)]
pub struct SendPaymentRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    /// Optional note message text
    pub note: Option<String>,

    /// Message ID of the payment request being responded to
    pub request_message_id: Option<String>,

    /// Transaction data (JSON string)
    pub transaction_data: Option<String>,
}

// --- Request Payment ---

#[derive(Debug, Deserialize, ToSchema)]
pub struct RequestPaymentRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    /// ISO 4217 currency code
    #[schema(example = "USD")]
    pub currency_code: String,

    /// Amount in smallest unit * 1000 (e.g., 1000 = $0.001)
    pub amount1000: u64,

    /// Optional note message text
    pub note: Option<String>,

    /// Expiration timestamp
    pub expiry_timestamp: Option<i64>,
}

// --- Cancel Payment Request ---

#[derive(Debug, Deserialize, ToSchema)]
pub struct CancelPaymentRequestRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    /// Message ID of the payment request to cancel
    pub request_message_id: String,
}

// --- Decline Payment Request ---

#[derive(Debug, Deserialize, ToSchema)]
pub struct DeclinePaymentRequestRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    /// Message ID of the payment request to decline
    pub request_message_id: String,
}

// --- Newsletter Forward ---

#[derive(Debug, Deserialize, ToSchema)]
pub struct SendNewsletterForwardRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,

    /// The text content to forward
    pub text: String,

    /// Newsletter JID
    pub newsletter_jid: String,

    /// Server message ID from the newsletter
    pub server_message_id: i32,

    /// Newsletter name
    pub newsletter_name: Option<String>,

    /// Content type: "update", "update_card", "link_card"
    pub content_type: Option<String>,
}

// --- Spam Report ---

#[derive(Debug, Deserialize, ToSchema)]
pub struct SpamReportRequest {
    /// The message ID being reported
    pub message_id: String,

    /// The timestamp of the message
    pub message_timestamp: u64,

    /// The JID of the message sender
    pub from_jid: Option<String>,

    /// For group messages, the participant JID
    pub participant_jid: Option<String>,

    /// For group reports, the group JID
    pub group_jid: Option<String>,

    /// For group reports, the group subject/name
    pub group_subject: Option<String>,

    /// Spam flow: "message_menu", "group_spam_banner_report", "group_info_report", "contact_info", "status_report"
    #[serde(default = "default_spam_flow")]
    pub spam_flow: String,

    /// Media type of the message
    pub media_type: Option<String>,
}

fn default_spam_flow() -> String {
    "message_menu".to_string()
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SpamReportResponse {
    pub success: bool,
    pub report_id: Option<String>,
}
