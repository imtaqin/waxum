use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
pub struct RejectCallRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub from: String,
    #[schema(example = "2E3F4A5B6C7D")]
    pub call_id: String,
}

/// Send a signalling-only ring to a recipient.
///
/// The upstream `whatsapp-rust` client has no media stack (opus/RTP), so
/// this endpoint sends only the `<call><offer>` signalling stanza. The
/// recipient's WhatsApp phone will ring for the usual timeout, then drop
/// with "call not connected" once no audio flow follows. Useful for
/// number verification, missed-call triggers, or attention pings.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RingCallRequest {
    /// Recipient. Bare phone number ("6285117822731") or full JID
    /// ("6285117822731@s.whatsapp.net"). LID is not supported.
    #[schema(example = "6285117822731")]
    pub to: String,
    /// Optional custom `call-id`. If omitted a UUIDv4 is generated. Return
    /// it back to the caller so they can later `POST /calls/reject`.
    #[schema(example = "2E3F4A5B6C7D")]
    #[serde(default)]
    pub call_id: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RingCallResponse {
    pub call_id: String,
    pub to: String,
}
