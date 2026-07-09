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
    /// Call kind: `"audio"` (default) or `"video"`. Video adds a `<video>`
    /// codec child so the peer's phone shows the video-call incoming UI.
    #[schema(example = "audio")]
    #[serde(default)]
    pub kind: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RingCallResponse {
    pub call_id: String,
    pub to: String,
}

/// Accept an incoming call by writing back the `<call><accept/>` stanza.
/// Signalling only — no media stack, so audio will not flow. Pair with
/// `terminate` immediately if you just want the call to show up in the
/// recipient's call log as "answered".
#[derive(Debug, Deserialize, ToSchema)]
pub struct AcceptCallRequest {
    /// Caller JID as reported in `IncomingCall.from`.
    #[schema(example = "6285117822731@s.whatsapp.net")]
    pub from: String,
    /// Call id from `IncomingCall.action.call_id()`.
    #[schema(example = "2E3F4A5B6C7D")]
    pub call_id: String,
}

/// End a call the session is currently in.
#[derive(Debug, Deserialize, ToSchema)]
pub struct TerminateCallRequest {
    /// Peer JID (caller for incoming, callee for outgoing).
    #[schema(example = "6285117822731@s.whatsapp.net")]
    pub peer: String,
    #[schema(example = "2E3F4A5B6C7D")]
    pub call_id: String,
    /// Optional termination reason string (e.g. "hangup", "busy"). Sent as
    /// the `reason` attr on the `<terminate>` child. Defaults to
    /// `"hangup"` when omitted.
    #[serde(default)]
    pub reason: Option<String>,
}
