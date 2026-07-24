use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
#[allow(dead_code)]
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
#[allow(dead_code)]
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

/// Accept an incoming call via `client.voip().accept(&incoming)`. If
/// `text` or `audio_url` is supplied, the accepted call also plays that
/// audio to the caller after `answer_grace_ms` of silent padding, then
/// hangs up. Both silent by default.
#[derive(Debug, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct AcceptCallRequest {
    /// Caller JID as reported in `IncomingCall.from`.
    #[schema(example = "6285117822731@s.whatsapp.net")]
    pub from: String,
    /// Call id from `IncomingCall.action.call_id()`.
    #[schema(example = "2E3F4A5B6C7D")]
    pub call_id: String,
    /// Optional TTS text to speak to the caller once accepted.
    #[serde(default)]
    pub text: Option<String>,
    /// Optional audio URL to play to the caller once accepted (mp3/wav/ogg).
    #[serde(default)]
    pub audio_url: Option<String>,
    /// edge-tts voice id when `text` is set. Defaults to `id-ID-ArdiNeural`.
    #[serde(default)]
    pub voice: Option<String>,
    /// Silent padding before playback starts. Defaults to 1500 ms.
    #[serde(default)]
    pub answer_grace_ms: Option<u64>,
}

/// Ring a peer and, once the media relay is up, speak `text` at them via
/// `edge-tts` (Microsoft Neural voices, free). Terminates the call after
/// the last PCM chunk is flushed.
#[derive(Debug, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct TtsCallRequest {
    #[schema(example = "6285117822731")]
    pub to: String,
    #[schema(example = "Halo dari MAUBLAST, pesan otomatis.")]
    pub text: String,
    /// edge-tts voice id. Defaults to `id-ID-ArdiNeural` (male Indonesian).
    /// See `edge-tts --list-voices` for the full list.
    #[schema(example = "id-ID-ArdiNeural")]
    #[serde(default)]
    pub voice: Option<String>,
    /// Grace period in milliseconds to wait before the first PCM chunk is
    /// pushed, giving the peer a moment to answer. Defaults to 4000 ms.
    #[schema(example = 4000)]
    #[serde(default)]
    pub answer_grace_ms: Option<u64>,
    /// If true, waxum decodes the peer's incoming MLOW frames back to
    /// 16 kHz mono PCM and writes them as a WAV file under
    /// `{WHATSAPP_STORAGE_PATH}/{session_id}/recordings/{call_id}.wav`.
    /// The file is served over
    /// `GET /api/v1/sessions/{session_id}/calls/{call_id}/recording.wav`
    /// once the call ends. Defaults to `false`.
    #[serde(default)]
    pub record: Option<bool>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TtsCallResponse {
    pub call_id: String,
    pub to: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recording_url: Option<String>,
}

/// Ring a peer and play back an audio file (mp3, wav, ogg — anything ffmpeg
/// can decode) once the media relay is up. Terminates the call after the
/// last PCM chunk is flushed.
#[derive(Debug, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct PlayCallRequest {
    #[schema(example = "6285117822731")]
    pub to: String,
    /// URL of the audio file to fetch and play. Must be reachable from the
    /// waxum process. Any format ffmpeg can demux (mp3, wav, ogg, m4a, opus).
    #[schema(example = "https://example.com/greeting.mp3")]
    pub audio_url: String,
    /// Grace period in ms before playback starts, giving the peer time to
    /// answer. Defaults to 4000 ms.
    #[schema(example = 4000)]
    #[serde(default)]
    pub answer_grace_ms: Option<u64>,
    /// See `TtsCallRequest::record`. Same behaviour.
    #[serde(default)]
    pub record: Option<bool>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PlayCallResponse {
    pub call_id: String,
    pub to: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recording_url: Option<String>,
}

/// End a call the session is currently in.
#[derive(Debug, Deserialize, ToSchema)]
#[allow(dead_code)]
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

/// One Edge-TTS voice, as returned by `GET /api/v1/voices`. Field
/// subset mirrors upstream `msedge_tts::voice::Voice` — just what a
/// caller needs to pick a voice (short name, locale, gender, display
/// name).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct VoiceEntry {
    pub name: String,
    pub short_name: Option<String>,
    pub locale: Option<String>,
    pub gender: Option<String>,
    pub friendly_name: Option<String>,
}

/// Transcript of a call recording, returned by
/// `POST /api/v1/sessions/{session_id}/calls/{call_id}/transcript`.
/// Produced by an external whisper.cpp-compatible HTTP server (see
/// `WHISPER_API_URL`) — waxum only forwards the recording and relays
/// the `text` field back, so the shape stays deliberately minimal.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TranscriptResponse {
    pub text: String,
}
