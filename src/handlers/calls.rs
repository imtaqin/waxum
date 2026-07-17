use axum::{
    extract::{Path, State},
    Json,
};
use std::sync::Arc;
use wacore_binary::jid::Jid;

use crate::error::ApiError;
use crate::models::calls::{
    AcceptCallRequest, PlayCallRequest, PlayCallResponse, RejectCallRequest, RingCallRequest,
    RingCallResponse, TerminateCallRequest, TtsCallRequest, TtsCallResponse,
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

    let to: Jid = if request.to.contains('@') {
        request
            .to
            .parse()
            .map_err(|_| ApiError::InvalidJid(request.to.clone()))?
    } else {
        Jid::pn(&request.to)
    };

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
            const CHUNK: usize = 320;
            let mut full = silence_prefix;
            full.extend_from_slice(&pcm);
            for chunk in full.chunks(CHUNK) {
                if mic_tx.send(chunk.to_vec()).await.is_err() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
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

    let to: Jid = if request.to.contains('@') {
        request
            .to
            .parse()
            .map_err(|_| ApiError::InvalidJid(request.to.clone()))?
    } else {
        Jid::pn(&request.to)
    };

    let voice = request
        .voice
        .unwrap_or_else(|| "id-ID-ArdiNeural".to_string());
    let pcm = generate_tts_pcm(&request.text, &voice)
        .await
        .map_err(|e| ApiError::Internal(format!("tts generation failed: {e}")))?;
    let grace_ms = request.answer_grace_ms.unwrap_or(6000);
    let silence_prefix = build_silence(grace_ms as usize);

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

    let call_id_bg = call_id.clone();
    let state_bg = state.clone();

    tokio::spawn(async move {
        const CHUNK: usize = 320;
        let mut full = silence_prefix;
        full.extend_from_slice(&pcm);
        for chunk in full.chunks(CHUNK) {
            if mic_tx.send(chunk.to_vec()).await.is_err() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        handle_arc.hangup().await;
        state_bg.active_calls().remove(&call_id_bg);
        state_bg.call_audio_channels().remove(&call_id_bg);
    });

    Ok(Json(TtsCallResponse {
        call_id,
        to: to.to_string(),
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

    let to: Jid = if request.to.contains('@') {
        request
            .to
            .parse()
            .map_err(|_| ApiError::InvalidJid(request.to.clone()))?
    } else {
        Jid::pn(&request.to)
    };

    let pcm = decode_url_to_pcm(&request.audio_url)
        .await
        .map_err(|e| ApiError::Internal(format!("audio decode failed: {e}")))?;
    let grace_ms = request.answer_grace_ms.unwrap_or(6000);
    let silence_prefix = build_silence(grace_ms as usize);

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

    let call_id_bg = call_id.clone();
    let state_bg = state.clone();

    tokio::spawn(async move {
        const CHUNK: usize = 320;
        let mut full = silence_prefix;
        full.extend_from_slice(&pcm);
        for chunk in full.chunks(CHUNK) {
            if mic_tx.send(chunk.to_vec()).await.is_err() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        handle_arc.hangup().await;
        state_bg.active_calls().remove(&call_id_bg);
        state_bg.call_audio_channels().remove(&call_id_bg);
    });

    Ok(Json(PlayCallResponse {
        call_id,
        to: to.to_string(),
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
async fn generate_tts_pcm(text: &str, voice: &str) -> anyhow::Result<Vec<i16>> {
    use std::process::Stdio;
    use tokio::io::AsyncWriteExt;

    let text_owned = text.to_string();
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
        let mut client = connect().map_err(|e| anyhow::anyhow!("edge tts connect: {e}"))?;
        let audio = client
            .synthesize(&text_owned, &config)
            .map_err(|e| anyhow::anyhow!("edge tts synth: {e}"))?;
        Ok(audio.audio_bytes)
    })
    .await??;

    let mut ffmpeg = tokio::process::Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
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
        .stderr(Stdio::null())
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
        anyhow::bail!("ffmpeg exited with status {}", out.status);
    }
    let buf = out.stdout;

    let mut samples = Vec::with_capacity(buf.len() / 2);
    for chunk in buf.chunks_exact(2) {
        samples.push(i16::from_le_bytes([chunk[0], chunk[1]]));
    }
    Ok(samples)
}

fn get_client(state: &AppState, session_id: &str) -> Result<Arc<whatsapp_rust::Client>, ApiError> {
    let runtime = state
        .get_session(session_id)
        .ok_or(ApiError::NotConnected)?;

    runtime.get_live_client().ok_or(ApiError::NotConnected)
}
