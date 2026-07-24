use axum::{
    extract::{
        ws::{Message as WsMessage, WebSocket, WebSocketUpgrade},
        Path, Query, State,
    },
    response::Response as AxumResponse,
    Json,
};
use futures::{sink::SinkExt, stream::StreamExt};
use serde::Deserialize;
use std::sync::Arc;
use wacore_binary::jid::Jid;

use crate::error::ApiError;
use crate::handlers::messages::parse_jid;
use crate::models::calls::{
    AcceptCallRequest, PlayCallRequest, PlayCallResponse, RejectCallRequest, RingCallRequest,
    RingCallResponse, TerminateCallRequest, TtsCallRequest, TtsCallResponse, VoiceEntry,
};
use crate::models::common::SuccessResponse;
use crate::state::{ActiveCallAudio, AppState};

type Pcm = Vec<i16>;
type DummyAudio = (
    async_channel::Receiver<Pcm>,
    async_channel::Sender<Pcm>,
    async_channel::Sender<Pcm>,
    async_channel::Receiver<Pcm>,
);

fn make_dummy_audio() -> DummyAudio {
    let (mic_tx, mic_rx) = async_channel::unbounded::<Pcm>();
    let (spk_tx, spk_rx) = async_channel::unbounded::<Pcm>();
    (mic_rx, spk_tx, mic_tx, spk_rx)
}

fn build_silence(ms: usize) -> Vec<i16> {
    let samples = ms * 16;
    vec![0i16; samples]
}

/// Encode a silence prefix + speech PCM to a stream of native MLOW frames.
///
/// First iteration used `WaOpusEncoder::new_mlow_escape()` (Opus carried
/// through MLOW's RTP profile). Peer received the packets cleanly (RTCP
/// showed zero loss, sequence advancing) but the WhatsApp client on the
/// callee side never emitted audio to the speaker — the decoder path
/// there is native MLOW, not the Opus-escape codec. Encoding directly with
/// `wacore::voip::mlow::encode::MlowEncoder` and using the
/// `AudioFormat::MLOW_16KHZ_60MS` profile matches the codec the peer's
/// receiver expects on 1:1 companion calls.
///
/// PCM is padded to a full 60 ms boundary with zeros and split into
/// 960-sample frames. `MlowEncoder::encode` takes normalised f32 samples in
/// [-1.0, 1.0]; the i16 → f32 conversion happens once per frame.
fn encode_pcm_to_mlow_native(silence: &[i16], pcm: &[i16]) -> anyhow::Result<Vec<bytes::Bytes>> {
    use wacore::voip::MlowEncoder;
    use whatsapp_rust::voip::audio::WA_FRAME_SAMPLES;

    let mut full: Vec<i16> = Vec::with_capacity(silence.len() + pcm.len());
    full.extend_from_slice(silence);
    full.extend_from_slice(pcm);
    let rem = full.len() % WA_FRAME_SAMPLES;
    if rem != 0 {
        full.extend(std::iter::repeat_n(0i16, WA_FRAME_SAMPLES - rem));
    }

    let mut enc = MlowEncoder::new();
    let mut frames: Vec<bytes::Bytes> = Vec::with_capacity(full.len() / WA_FRAME_SAMPLES);
    let mut total_bytes: usize = 0;
    let mut buf: Vec<f32> = vec![0.0; WA_FRAME_SAMPLES];
    for chunk in full.chunks_exact(WA_FRAME_SAMPLES) {
        for (dst, &src) in buf.iter_mut().zip(chunk.iter()) {
            *dst = (src as f32) / 32768.0;
        }
        let bytes = enc
            .encode(&buf)
            .map_err(|e| anyhow::anyhow!("mlow encode: {e:?}"))?;
        total_bytes += bytes.len();
        frames.push(bytes::Bytes::from(bytes));
    }
    tracing::info!(
        target: "waxum::call_audio",
        frames = frames.len(),
        total_bytes,
        pcm_samples = full.len(),
        duration_ms = full.len() / 16,
        "encoded PCM to native MLOW"
    );
    Ok(frames)
}

async fn place_encoded_audio_call(
    state: AppState,
    to: wacore_binary::jid::Jid,
    client: Arc<whatsapp_rust::Client>,
    opus_frames: Vec<bytes::Bytes>,
    session_id: String,
    record: bool,
) -> Result<(String, wacore_binary::jid::Jid, Option<String>), ApiError> {
    use wacore::voip::AudioFormat;

    let (opus_tx, opus_rx) = async_channel::bounded::<bytes::Bytes>(32);
    let (peer_tx, peer_rx) = async_channel::bounded::<wacore::voip::EncodedAudioFrame>(64);

    let handle = client
        .voip()
        .call(&to)
        .encoded_audio(AudioFormat::MLOW_16KHZ_60MS, opus_rx, peer_tx)
        .start()
        .await
        .map_err(|e| ApiError::Internal(format!("place_call failed: {e}")))?;

    let call_id = handle.call_id().to_string();
    let handle_arc = Arc::new(handle);
    state
        .active_calls()
        .insert(call_id.clone(), handle_arc.clone());

    let recording_url: Option<String> = if record {
        let base = state.base_storage_path().to_string();
        let sid = session_id.clone();
        let cid = call_id.clone();
        tokio::spawn(async move {
            if let Err(e) = record_peer_task(peer_rx, base, sid, cid).await {
                tracing::warn!(target: "waxum::call_audio", "record task ended: {e}");
            }
        });
        Some(format!(
            "/api/v1/sessions/{}/calls/{}/recording.wav",
            session_id, call_id
        ))
    } else {
        None
    };

    let call_id_bg = call_id.clone();
    let state_bg = state.clone();
    let events = handle_arc.events();
    let call_id_events = call_id.clone();
    tokio::spawn(async move {
        while let Ok(ev) = events.recv().await {
            let is_rtcp = matches!(ev, whatsapp_rust::voip::CallEvent::RtcpReceived { .. });
            if is_rtcp {
                tracing::debug!(
                    target: "waxum::call_audio",
                    call_id = %call_id_events,
                    "call event: {:?}", ev
                );
            } else {
                tracing::info!(
                    target: "waxum::call_audio",
                    call_id = %call_id_events,
                    "call event: {:?}", ev
                );
            }
        }
    });

    tokio::spawn(async move {
        let total = opus_frames.len();
        for (i, frame) in opus_frames.into_iter().enumerate() {
            if opus_tx.send(frame).await.is_err() {
                tracing::warn!(
                    target: "waxum::call_audio",
                    call_id = %call_id_bg,
                    sent = i,
                    total,
                    "opus source channel closed early — engine dropped receiver"
                );
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(60)).await;
        }
        tracing::info!(
            target: "waxum::call_audio",
            call_id = %call_id_bg,
            total_frames = total,
            "opus stream complete, hanging up"
        );
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        handle_arc.hangup().await;
        state_bg.active_calls().remove(&call_id_bg);
    });

    Ok((call_id, to, recording_url))
}

/// Consume peer-received MLOW frames off the sink, decode each to f32 PCM,
/// convert to i16, and write out as a 16 kHz mono WAV file once the sink
/// closes (i.e. the call has ended).
async fn record_peer_task(
    rx: async_channel::Receiver<wacore::voip::EncodedAudioFrame>,
    base_storage: String,
    session_id: String,
    call_id: String,
) -> anyhow::Result<()> {
    use wacore::voip::MlowDecoder;

    let mut decoder = MlowDecoder::new();
    let mut pcm_i16: Vec<i16> = Vec::new();

    while let Ok(frame) = rx.recv().await {
        let pcm_f32 = decoder.decode(&frame.data);
        pcm_i16.reserve(pcm_f32.len());
        for &s in &pcm_f32 {
            let clamped = s.clamp(-1.0, 1.0);
            pcm_i16.push((clamped * 32767.0) as i16);
        }
    }

    let dir = std::path::Path::new(&base_storage)
        .join(&session_id)
        .join("recordings");
    tokio::fs::create_dir_all(&dir).await?;
    let path = dir.join(format!("{}.wav", call_id));

    let wav_bytes = build_wav_pcm16_mono_16khz(&pcm_i16);
    tokio::fs::write(&path, &wav_bytes).await?;

    if pcm_i16.is_empty() {
        tracing::info!(
            target: "waxum::call_audio",
            call_id = %call_id,
            "peer sent no audio (call never fully answered) — wrote 0-sample WAV placeholder"
        );
        return Ok(());
    }

    tracing::info!(
        target: "waxum::call_audio",
        call_id = %call_id,
        samples = pcm_i16.len(),
        duration_ms = pcm_i16.len() / 16,
        path = %path.display(),
        "peer audio recorded"
    );
    Ok(())
}

/// Build a minimal RIFF/WAV container for 16-bit mono PCM at 16 kHz. Written
/// by hand because the codec pipeline is already fixed to that format — a
/// full crate like `hound` would only bloat the binary for this one call
/// site.
fn build_wav_pcm16_mono_16khz(samples: &[i16]) -> Vec<u8> {
    let sample_rate: u32 = 16_000;
    let channels: u16 = 1;
    let bits_per_sample: u16 = 16;
    let byte_rate = sample_rate * (bits_per_sample as u32 / 8) * (channels as u32);
    let block_align = channels * bits_per_sample / 8;
    let data_len = (samples.len() * 2) as u32;
    let riff_len = 36 + data_len;

    let mut buf = Vec::with_capacity(44 + samples.len() * 2);
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&riff_len.to_le_bytes());
    buf.extend_from_slice(b"WAVE");
    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.extend_from_slice(&channels.to_le_bytes());
    buf.extend_from_slice(&sample_rate.to_le_bytes());
    buf.extend_from_slice(&byte_rate.to_le_bytes());
    buf.extend_from_slice(&block_align.to_le_bytes());
    buf.extend_from_slice(&bits_per_sample.to_le_bytes());
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_len.to_le_bytes());
    for &s in samples {
        buf.extend_from_slice(&s.to_le_bytes());
    }
    buf
}

pub async fn get_recording(
    State(state): State<AppState>,
    Path((session_id, call_id)): Path<(String, String)>,
) -> Result<AxumResponse, ApiError> {
    let base = state.base_storage_path().to_string();
    let path = std::path::Path::new(&base)
        .join(&session_id)
        .join("recordings")
        .join(format!("{}.wav", call_id));
    let bytes = match tokio::fs::read(&path).await {
        Ok(b) => b,
        Err(_) => {
            let mut r = AxumResponse::new(axum::body::Body::from(
                "{\"success\":false,\"error\":{\"code\":404,\"message\":\"recording not ready yet — wait until peer hangs up, then reload\"}}",
            ));
            *r.status_mut() = axum::http::StatusCode::NOT_FOUND;
            r.headers_mut().insert(
                axum::http::header::CONTENT_TYPE,
                axum::http::HeaderValue::from_static("application/json"),
            );
            return Ok(r);
        }
    };
    let mut r = AxumResponse::new(axum::body::Body::from(bytes));
    r.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("audio/wav"),
    );
    r.headers_mut().insert(
        axum::http::header::CONTENT_DISPOSITION,
        axum::http::HeaderValue::from_str(&format!("attachment; filename=\"{}.wav\"", call_id))
            .unwrap_or_else(|_| axum::http::HeaderValue::from_static("attachment")),
    );
    Ok(r)
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/calls/reject",
    tag = "calls",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = RejectCallRequest,
    responses(
        (status = 200, description = "Call rejected", body = SuccessResponse),
        (status = 400, description = "Invalid JID or empty call_id"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn reject_call(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<RejectCallRequest>,
) -> Result<Json<SuccessResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    if request.call_id.is_empty() {
        return Err(ApiError::BadRequest("call_id is empty".to_string()));
    }
    let incoming = state
        .incoming_calls()
        .remove(&request.call_id)
        .map(|(_, v)| v)
        .ok_or_else(|| ApiError::BadRequest("call_id not found in registry".to_string()))?;

    client
        .voip()
        .reject(&incoming)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(SuccessResponse::with_message("Call rejected")))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/calls/ring",
    tag = "calls",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = RingCallRequest,
    responses(
        (status = 200, description = "Ring signal sent", body = RingCallResponse),
        (status = 400, description = "Invalid recipient"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn ring_call(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<RingCallRequest>,
) -> Result<Json<RingCallResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;

    let to = resolve_call_recipient(client.clone(), parse_jid(&request.to)?).await;

    let (mic_rx, spk_tx, mic_tx, spk_rx) = make_dummy_audio();

    let handle = client
        .voip()
        .call(&to)
        .audio(mic_rx, spk_tx)
        .start()
        .await
        .map_err(|e| ApiError::Internal(format!("place_call failed: {e}")))?;

    let call_id = handle.call_id().to_string();
    let handle_arc = Arc::new(handle);
    state.active_calls().insert(call_id.clone(), handle_arc);
    state
        .call_audio_channels()
        .insert(call_id.clone(), ActiveCallAudio { mic_tx, spk_rx });

    Ok(Json(RingCallResponse {
        call_id,
        to: to.to_string(),
    }))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/calls/accept",
    tag = "calls",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = AcceptCallRequest,
    responses(
        (status = 200, description = "Accept signal sent", body = SuccessResponse),
        (status = 400, description = "Invalid caller JID or empty call_id"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn accept_call(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<AcceptCallRequest>,
) -> Result<Json<SuccessResponse>, ApiError> {
    if request.call_id.is_empty() {
        return Err(ApiError::BadRequest("call_id is empty".to_string()));
    }
    let client = get_client(&state, &session_id)?;
    let incoming = state
        .incoming_calls()
        .remove(&request.call_id)
        .map(|(_, v)| v)
        .ok_or_else(|| ApiError::BadRequest("call_id not found in registry".to_string()))?;

    let payload_pcm: Option<Vec<i16>> = if let Some(text) = request.text.as_ref() {
        let voice = request
            .voice
            .clone()
            .unwrap_or_else(|| "id-ID-ArdiNeural".to_string());
        Some(
            generate_tts_pcm(text, &voice)
                .await
                .map_err(|e| ApiError::Internal(format!("tts generation failed: {e}")))?,
        )
    } else if let Some(url) = request.audio_url.as_ref() {
        Some(
            decode_url_to_pcm(url)
                .await
                .map_err(|e| ApiError::Internal(format!("audio decode failed: {e}")))?,
        )
    } else {
        None
    };
    let grace_ms = request.answer_grace_ms.unwrap_or(1500);
    let silence_prefix = build_silence(grace_ms as usize);

    let (mic_rx, spk_tx, mic_tx, spk_rx) = make_dummy_audio();

    let handle = client
        .voip()
        .accept(&incoming)
        .audio(mic_rx, spk_tx)
        .start()
        .await
        .map_err(|e| ApiError::Internal(format!("accept failed: {e}")))?;

    let call_id = handle.call_id().to_string();
    let handle_arc = Arc::new(handle);
    state
        .active_calls()
        .insert(call_id.clone(), handle_arc.clone());
    state.call_audio_channels().insert(
        call_id.clone(),
        ActiveCallAudio {
            mic_tx: mic_tx.clone(),
            spk_rx,
        },
    );

    if let Some(pcm) = payload_pcm {
        let call_id_bg = call_id.clone();
        let state_bg = state.clone();
        tokio::spawn(async move {
            const CHUNK: usize = 960;
            let mut full = silence_prefix;
            full.extend_from_slice(&pcm);
            for chunk in full.chunks(CHUNK) {
                if mic_tx.send(chunk.to_vec()).await.is_err() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(60)).await;
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            handle_arc.hangup().await;
            state_bg.active_calls().remove(&call_id_bg);
            state_bg.call_audio_channels().remove(&call_id_bg);
        });
    }

    Ok(Json(SuccessResponse::with_message("Call accepted")))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/calls/terminate",
    tag = "calls",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = TerminateCallRequest,
    responses(
        (status = 200, description = "Terminate signal sent", body = SuccessResponse),
        (status = 400, description = "Invalid peer JID or empty call_id"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn terminate_call(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<TerminateCallRequest>,
) -> Result<Json<SuccessResponse>, ApiError> {
    if request.call_id.is_empty() {
        return Err(ApiError::BadRequest("call_id is empty".to_string()));
    }
    let client = get_client(&state, &session_id)?;

    if let Some((_, handle)) = state.active_calls().remove(&request.call_id) {
        handle.hangup().await;
        state.call_audio_channels().remove(&request.call_id);
        return Ok(Json(SuccessResponse::with_message("Call terminated")));
    }

    let peer: Jid = request
        .peer
        .parse()
        .map_err(|_| ApiError::InvalidJid(request.peer.clone()))?;

    client
        .voip()
        .terminate(&request.call_id, &peer, &peer)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(SuccessResponse::with_message("Call terminated")))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/calls/tts",
    tag = "calls",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = TtsCallRequest,
    responses(
        (status = 200, description = "TTS call started", body = TtsCallResponse),
        (status = 400, description = "Invalid recipient or empty text"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn tts_call(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<TtsCallRequest>,
) -> Result<Json<TtsCallResponse>, ApiError> {
    if request.text.trim().is_empty() {
        return Err(ApiError::BadRequest("text is empty".to_string()));
    }
    let client = get_client(&state, &session_id)?;

    let to = resolve_call_recipient(client.clone(), parse_jid(&request.to)?).await;

    let voice = request
        .voice
        .unwrap_or_else(|| "id-ID-ArdiNeural".to_string());
    let pcm = generate_tts_pcm(&request.text, &voice)
        .await
        .map_err(|e| ApiError::Internal(format!("tts generation failed: {e}")))?;
    let grace_ms = request.answer_grace_ms.unwrap_or(6000);
    let silence_prefix = build_silence(grace_ms as usize);

    let opus_frames = encode_pcm_to_mlow_native(&silence_prefix, &pcm)
        .map_err(|e| ApiError::Internal(format!("mlow encode failed: {e}")))?;
    let record = request.record.unwrap_or(false);
    let (call_id, to_jid, recording_url) =
        place_encoded_audio_call(state, to, client, opus_frames, session_id, record).await?;
    Ok(Json(TtsCallResponse {
        call_id,
        to: to_jid.to_string(),
        recording_url,
    }))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/calls/play",
    tag = "calls",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = PlayCallRequest,
    responses(
        (status = 200, description = "Audio call started", body = PlayCallResponse),
        (status = 400, description = "Invalid recipient or url"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn play_call(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<PlayCallRequest>,
) -> Result<Json<PlayCallResponse>, ApiError> {
    if request.audio_url.trim().is_empty() {
        return Err(ApiError::BadRequest("audio_url is empty".to_string()));
    }
    let client = get_client(&state, &session_id)?;

    let to = resolve_call_recipient(client.clone(), parse_jid(&request.to)?).await;

    let pcm = decode_url_to_pcm(&request.audio_url)
        .await
        .map_err(|e| ApiError::Internal(format!("audio decode failed: {e}")))?;
    let grace_ms = request.answer_grace_ms.unwrap_or(6000);
    let silence_prefix = build_silence(grace_ms as usize);

    let opus_frames = encode_pcm_to_mlow_native(&silence_prefix, &pcm)
        .map_err(|e| ApiError::Internal(format!("mlow encode failed: {e}")))?;
    let record = request.record.unwrap_or(false);
    let (call_id, to_jid, recording_url) =
        place_encoded_audio_call(state, to, client, opus_frames, session_id, record).await?;
    Ok(Json(PlayCallResponse {
        call_id,
        to: to_jid.to_string(),
        recording_url,
    }))
}

async fn decode_url_to_pcm(url: &str) -> anyhow::Result<Vec<i16>> {
    use std::process::Stdio;

    let ffmpeg = tokio::process::Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-i",
            url,
            "-f",
            "s16le",
            "-acodec",
            "pcm_s16le",
            "-ar",
            "16000",
            "-ac",
            "1",
            "pipe:1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;

    let out = ffmpeg.wait_with_output().await?;
    if !out.status.success() {
        anyhow::bail!("ffmpeg exited with status {}", out.status);
    }
    let buf = out.stdout;
    let mut samples = Vec::with_capacity(buf.len() / 2);
    for chunk in buf.chunks_exact(2) {
        samples.push(i16::from_le_bytes([chunk[0], chunk[1]]));
    }
    Ok(samples)
}

/// Synthesise `text` with Microsoft Edge's neural voice engine and return
/// the resulting audio as 16 kHz mono PCM ready to feed into a WhatsApp
/// call.
///
/// Uses the `msedge-tts` crate — a native Rust WebSocket client for the
/// Edge readaloud endpoint. Nothing external is required on the server
/// (no `edge-tts` CLI, no Python interpreter), just `ffmpeg` for the
/// MP3 → PCM decode step.
/// Prepare TTS text for embedding into SSML.
///
/// `msedge-tts` builds its SSML by string-formatting the caller's text
/// straight into the `<prosody>…</prosody>` payload with no escaping. Any
/// `<`, `>`, `&`, `"`, or `'` in the text corrupts the SSML XML and the
/// Microsoft backend responds with an empty audio blob (0 bytes) — the
/// failure we saw in the field. Escape those characters here so the SSML
/// stays well-formed regardless of what the operator pasted in.
///
/// Also strip characters that Microsoft's neural voices choke on silently
/// (control chars, U+FEFF byte-order marks) and normalise runs of
/// whitespace to a single space.
fn sanitize_ssml_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_was_space = false;
    for ch in text.chars() {
        if ch == '\u{FEFF}' || (ch.is_control() && ch != '\n' && ch != '\r' && ch != '\t') {
            continue;
        }
        let mapped = match ch {
            '<' => Some("&lt;"),
            '>' => Some("&gt;"),
            '&' => Some("&amp;"),
            '"' => Some("&quot;"),
            '\'' => Some("&apos;"),
            _ => None,
        };
        if let Some(entity) = mapped {
            out.push_str(entity);
            last_was_space = false;
        } else if ch.is_whitespace() {
            if !last_was_space {
                out.push(' ');
                last_was_space = true;
            }
        } else {
            out.push(ch);
            last_was_space = false;
        }
    }
    out.trim().to_string()
}

async fn generate_tts_pcm(text: &str, voice: &str) -> anyhow::Result<Vec<i16>> {
    use std::process::Stdio;
    use tokio::io::AsyncWriteExt;

    let text_owned = sanitize_ssml_text(text);
    let voice_owned = voice.to_string();

    let mp3 = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<u8>> {
        use msedge_tts::tts::client::connect;
        use msedge_tts::tts::SpeechConfig;
        use msedge_tts::voice::get_voices_list;

        let voices = get_voices_list().map_err(|e| anyhow::anyhow!("edge tts voice list: {e}"))?;
        let matched = voices
            .iter()
            .find(|v| v.short_name.as_deref() == Some(voice_owned.as_str()))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "edge tts voice '{}' not found. Try id-ID-ArdiNeural, \
                     en-US-JennyNeural, ja-JP-NanamiNeural, etc.",
                    voice_owned
                )
            })?;

        let config = SpeechConfig::from(matched);

        let mut last_err: Option<anyhow::Error> = None;
        for attempt in 1..=3 {
            let mut client = match connect() {
                Ok(c) => c,
                Err(e) => {
                    last_err = Some(anyhow::anyhow!("edge tts connect (attempt {attempt}): {e}"));
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    continue;
                }
            };
            match client.synthesize(&text_owned, &config) {
                Ok(audio) if !audio.audio_bytes.is_empty() => {
                    if attempt > 1 {
                        tracing::info!(
                            target: "waxum::tts",
                            attempt,
                            bytes = audio.audio_bytes.len(),
                            "edge tts synth recovered after retry"
                        );
                    }
                    return Ok(audio.audio_bytes);
                }
                Ok(_) => {
                    last_err = Some(anyhow::anyhow!(
                        "edge tts returned 0 audio bytes on attempt {attempt} — probably transient upstream"
                    ));
                }
                Err(e) => {
                    last_err = Some(anyhow::anyhow!("edge tts synth (attempt {attempt}): {e}"));
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("edge tts failed after 3 attempts")))
    })
    .await??;

    if mp3.len() < 32 {
        anyhow::bail!(
            "edge-tts returned an unusually small audio blob ({} bytes) — probably an upstream error response, retry",
            mp3.len()
        );
    }
    let looks_like_mp3 =
        mp3.starts_with(b"ID3") || (mp3.len() >= 2 && mp3[0] == 0xFF && (mp3[1] & 0xE0) == 0xE0);
    if !looks_like_mp3 {
        anyhow::bail!(
            "edge-tts payload does not look like MP3 (first bytes {:02x?}) — Microsoft may have returned an error page. Retry, and if it persists, check that the voice ID exists in `edge-tts --list-voices`.",
            &mp3[..mp3.len().min(16)]
        );
    }

    let mut ffmpeg = tokio::process::Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "mp3",
            "-i",
            "pipe:0",
            "-f",
            "s16le",
            "-acodec",
            "pcm_s16le",
            "-ar",
            "16000",
            "-ac",
            "1",
            "pipe:1",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            anyhow::anyhow!(
                "ffmpeg not found on PATH: {e}. Install ffmpeg to enable /calls/tts and /calls/play."
            )
        })?;

    if let Some(mut stdin) = ffmpeg.stdin.take() {
        stdin.write_all(&mp3).await?;
        drop(stdin);
    }
    let out = ffmpeg.wait_with_output().await?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        anyhow::bail!(
            "ffmpeg exited with status {} (mp3 was {} bytes). stderr: {}",
            out.status,
            mp3.len(),
            if stderr.is_empty() {
                "<empty>".into()
            } else {
                stderr
            }
        );
    }
    let buf = out.stdout;

    let mut samples = Vec::with_capacity(buf.len() / 2);
    for chunk in buf.chunks_exact(2) {
        samples.push(i16::from_le_bytes([chunk[0], chunk[1]]));
    }
    Ok(samples)
}

/// Resolve a recipient JID to a form that the VoIP media pipeline can
/// derive keys for. WhatsApp VoIP requires the LID of the peer; a raw
/// `phone@s.whatsapp.net` fails at the media-offer step with
/// "no known LID for the PN callee".
///
/// This walks two lookups in order:
///
/// 1. `Contacts::is_on_whatsapp` — the same call used by
///    `/contacts/check`. It hits WA's usync graph, learns the PN↔LID
///    mapping, and persists it into the client's `lid_pn_cache` so the
///    next call skips this step.
/// 2. Existing local cache via `get_user_info`, as a fallback when the
///    server response is missing the LID.
///
/// Non-PN inputs (`@lid`, `@g.us`, already-resolved) are returned as
/// is. A JID that has no LID on WhatsApp's side is returned unchanged —
/// the caller then surfaces the same "no LID" error to the user, but
/// with an accurate cause.
pub(crate) async fn resolve_call_recipient(
    client: Arc<whatsapp_rust::Client>,
    jid: wacore_binary::jid::Jid,
) -> wacore_binary::jid::Jid {
    use wacore_binary::jid::SERVER_JID;
    if jid.server != SERVER_JID {
        return jid;
    }
    let probe = vec![jid.clone()];
    struct AssertSend<F>(F);
    unsafe impl<F: std::future::Future> Send for AssertSend<F> {}
    impl<F: std::future::Future> std::future::Future for AssertSend<F> {
        type Output = F::Output;
        fn poll(
            self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Self::Output> {
            unsafe { self.map_unchecked_mut(|s| &mut s.0) }.poll(cx)
        }
    }

    let client_for_check = client.clone();
    let probe_for_check = probe.clone();
    let via_usync = tokio::time::timeout(
        std::time::Duration::from_secs(8),
        AssertSend(async move {
            client_for_check
                .contacts()
                .is_on_whatsapp(&probe_for_check)
                .await
        }),
    )
    .await;
    if let Ok(Ok(results)) = via_usync {
        if let Some(r) = results.iter().find(|r| r.jid == jid) {
            if let Some(lid) = r.lid.clone() {
                return lid;
            }
        }
    }

    let via_info = tokio::time::timeout(
        std::time::Duration::from_secs(4),
        AssertSend(async move { client.contacts().get_user_info(&probe).await }),
    )
    .await;
    if let Ok(Ok(map)) = via_info {
        if let Some(info) = map.get(&jid) {
            if let Some(lid) = info.lid.clone() {
                return lid;
            }
        }
    }

    jid
}

fn get_client(state: &AppState, session_id: &str) -> Result<Arc<whatsapp_rust::Client>, ApiError> {
    let runtime = state
        .get_session(session_id)
        .ok_or(ApiError::NotConnected)?;

    runtime.get_live_client().ok_or(ApiError::NotConnected)
}

#[derive(Debug, Deserialize)]
pub struct MediaWsQuery {
    pub to: String,
    #[serde(default)]
    pub kind: Option<String>,
}

pub async fn media_stream_ws(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(q): Query<MediaWsQuery>,
    ws: WebSocketUpgrade,
) -> Result<AxumResponse, ApiError> {
    let client = get_client(&state, &session_id)?;
    let to = resolve_call_recipient(client.clone(), parse_jid(&q.to)?).await;
    let kind = q.kind.as_deref().unwrap_or("audio").to_string();
    if kind != "audio" {
        return Err(ApiError::BadRequest(
            "only kind=audio is supported over the media WebSocket for now".into(),
        ));
    }

    Ok(ws.on_upgrade(move |socket| async move {
        if let Err(e) = drive_media_socket(state, session_id, client, to, socket).await {
            tracing::warn!("media ws terminated: {e}");
        }
    }))
}

async fn drive_media_socket(
    state: AppState,
    session_id: String,
    client: Arc<whatsapp_rust::Client>,
    to: Jid,
    socket: WebSocket,
) -> anyhow::Result<()> {
    let (mut sink, mut stream) = socket.split();

    let (mic_tx, mic_rx) = async_channel::bounded::<Vec<i16>>(64);
    let (spk_tx, spk_rx) = async_channel::bounded::<Vec<i16>>(64);

    let handle = client
        .voip()
        .call(&to)
        .audio(mic_rx, spk_tx)
        .start()
        .await
        .map_err(|e| anyhow::anyhow!("place_call failed: {e}"))?;

    let call_id = handle.call_id().to_string();
    let handle_arc = Arc::new(handle);
    state
        .active_calls()
        .insert(call_id.clone(), handle_arc.clone());
    state.call_audio_channels().insert(
        call_id.clone(),
        ActiveCallAudio {
            mic_tx: mic_tx.clone(),
            spk_rx: spk_rx.clone(),
        },
    );

    let sess_meta = serde_json::json!({
        "type": "call_started",
        "call_id": call_id,
        "session_id": session_id,
        "to": to.to_string(),
        "sample_rate": whatsapp_rust::voip::audio::WA_SAMPLE_RATE,
        "frame_samples": whatsapp_rust::voip::audio::WA_FRAME_SAMPLES,
        "encoding": "pcm_s16le_mono_16khz",
    });
    let _ = sink
        .send(WsMessage::Text(sess_meta.to_string().into()))
        .await;

    let mut peer_task = tokio::spawn(async move {
        while let Ok(pcm) = spk_rx.recv().await {
            let mut bytes = Vec::with_capacity(pcm.len() * 2);
            for s in &pcm {
                bytes.extend_from_slice(&s.to_le_bytes());
            }
            if sink.send(WsMessage::Binary(bytes.into())).await.is_err() {
                break;
            }
        }
        let _ = sink.close().await;
    });

    let inbound = async {
        while let Some(msg) = stream.next().await {
            match msg {
                Ok(WsMessage::Binary(bytes)) => {
                    if bytes.len() < 2 {
                        continue;
                    }
                    let mut samples = Vec::with_capacity(bytes.len() / 2);
                    for chunk in bytes.chunks_exact(2) {
                        samples.push(i16::from_le_bytes([chunk[0], chunk[1]]));
                    }
                    if mic_tx.send(samples).await.is_err() {
                        break;
                    }
                }
                Ok(WsMessage::Close(_)) | Err(_) => break,
                _ => {}
            }
        }
    };

    tokio::select! {
        _ = inbound => {}
        _ = &mut peer_task => {}
    }

    peer_task.abort();
    handle_arc.hangup().await;
    state.active_calls().remove(&call_id);
    state.call_audio_channels().remove(&call_id);
    Ok(())
}

/// Preview a TTS voice without placing a call — synthesize `text` with
/// `voice` and return the MP3 that Edge-TTS produced, before the ffmpeg
/// resample step. Cheap way for a caller to audition voices in the
/// browser before wiring one to `tts_call`.
#[derive(Debug, serde::Deserialize)]
pub struct TtsPreviewQuery {
    pub text: String,
    pub voice: String,
}

pub async fn tts_preview(Query(q): Query<TtsPreviewQuery>) -> Result<AxumResponse, ApiError> {
    let text = q.text.trim();
    if text.is_empty() {
        return Err(ApiError::BadRequest("text is empty".into()));
    }
    if text.chars().count() > 500 {
        return Err(ApiError::BadRequest(
            "text is over 500 chars — not a preview, use /calls/tts".into(),
        ));
    }
    let text_owned = sanitize_ssml_text(text);
    let voice_owned = q.voice.clone();

    let mp3 = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, ApiError> {
        use msedge_tts::tts::client::connect;
        use msedge_tts::tts::SpeechConfig;
        use msedge_tts::voice::get_voices_list;
        let voices = get_voices_list()
            .map_err(|e| ApiError::Internal(format!("edge tts voice list: {e}")))?;
        let matched = voices
            .iter()
            .find(|v| v.short_name.as_deref() == Some(voice_owned.as_str()))
            .ok_or_else(|| ApiError::BadRequest(format!("voice '{voice_owned}' not found")))?;
        let config = SpeechConfig::from(matched);
        let mut client =
            connect().map_err(|e| ApiError::Internal(format!("edge tts connect: {e}")))?;
        let audio = client
            .synthesize(&text_owned, &config)
            .map_err(|e| ApiError::Internal(format!("edge tts synth: {e}")))?;
        if audio.audio_bytes.is_empty() {
            return Err(ApiError::Internal("edge tts returned 0 bytes".into()));
        }
        Ok(audio.audio_bytes)
    })
    .await
    .map_err(|e| ApiError::Internal(format!("tts task join: {e}")))??;

    let resp = axum::response::Response::builder()
        .status(200)
        .header("content-type", "audio/mpeg")
        .header("cache-control", "public, max-age=3600")
        .header("x-waxum-tts-voice", q.voice)
        .body(axum::body::Body::from(mp3))
        .map_err(|e| ApiError::Internal(format!("response build: {e}")))?;
    Ok(resp)
}

/// List every voice Edge-TTS exposes. Response shape mirrors what the
/// upstream `get_voices_list` returns, with just the fields a caller
/// actually needs (short name, locale, gender, display name). The list
/// is stable per Edge-TTS release, so it is fetched once and cached in
/// [`AppState`] instead of re-querying Edge-TTS on every request.
pub async fn list_voices(State(state): State<AppState>) -> Result<AxumResponse, ApiError> {
    if let Some(cached) = state.cached_voices() {
        return voices_response(cached);
    }

    let voices = tokio::task::spawn_blocking(|| -> anyhow::Result<Vec<VoiceEntry>> {
        use msedge_tts::voice::get_voices_list;
        let raw = get_voices_list().map_err(|e| anyhow::anyhow!("voice list: {e}"))?;
        Ok(raw
            .into_iter()
            .map(|v| VoiceEntry {
                name: v.name,
                short_name: v.short_name,
                locale: v.locale,
                gender: v.gender,
                friendly_name: v.friendly_name,
            })
            .collect())
    })
    .await
    .map_err(|e| ApiError::Internal(format!("voice task join: {e}")))?
    .map_err(|e| ApiError::Internal(format!("voice list failed: {e}")))?;

    state.set_cached_voices(voices.clone());
    voices_response(voices)
}

fn voices_response(voices: Vec<VoiceEntry>) -> Result<AxumResponse, ApiError> {
    let body = serde_json::to_vec(&voices)
        .map_err(|e| ApiError::Internal(format!("voice list encode: {e}")))?;
    axum::response::Response::builder()
        .status(200)
        .header("content-type", "application/json")
        .header("cache-control", "public, max-age=3600")
        .body(axum::body::Body::from(body))
        .map_err(|e| ApiError::Internal(format!("response build: {e}")))
}
