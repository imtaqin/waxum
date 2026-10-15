//! Session export and import as a zip of the session directory.

use axum::{
    extract::{Multipart, Path, State},
    response::Response as AxumResponse,
    Json,
};

use crate::api::common::SuccessResponse;
use crate::api::shared::sessions::models::SessionStatus;
use crate::error::ApiError;
use crate::state::AppState;

/// Package a session's local storage directory (device identity, Signal
/// protocol keys, noise handshake state — everything `whatsapp-rust`
/// itself persists) as a zip, so it can be moved to another waxum
/// instance. Disconnects the session first: the same device credentials
/// must never be live on two instances at once, so export always leaves
/// the source side stopped.
#[utoipa::path(
    post,
    path = "/api/v1/sessions/{session_id}/export",
    tag = "sessions",
    security(("bearer_auth" = [])),
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    responses(
        (status = 200, description = "Zip archive of the session's local storage directory"),
        (status = 404, description = "Session not found")
    )
)]
pub async fn export_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<AxumResponse, ApiError> {
    let _ = state
        .session_manager()
        .get_session(&session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::SessionNotFound(session_id.clone()))?;

    if let Some(runtime) = state.get_session(&session_id) {
        if let Some(client) = runtime.get_client() {
            client.disconnect().await;
        }
        runtime.set_status(SessionStatus::Disconnected);
        runtime.set_client(None);
        runtime.clear_reconnecting();
    }
    let _ = state
        .session_manager()
        .update_session_status(&session_id, SessionStatus::Disconnected, false)
        .await;

    let storage_path = state
        .session_manager()
        .get_storage_path(&session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .unwrap_or_else(|| format!("{}/{}", state.base_storage_path(), session_id));

    let sid = session_id.clone();
    let zip_bytes = tokio::task::spawn_blocking(move || zip_directory(&storage_path))
        .await
        .map_err(|e| ApiError::Internal(format!("export task panicked: {e}")))?
        .map_err(|e| ApiError::Internal(format!("export failed: {e}")))?;

    AxumResponse::builder()
        .status(200)
        .header("content-type", "application/zip")
        .header(
            "content-disposition",
            format!("attachment; filename=\"{sid}.waxum-session.zip\""),
        )
        .body(axum::body::Body::from(zip_bytes))
        .map_err(|e| ApiError::Internal(format!("response build: {e}")))
}

/// Restore a session's local storage directory from an [`export_session`]
/// zip, e.g. after copying it to a different waxum instance. Refuses to
/// run over a session that is currently connected on this instance —
/// disconnect it first (or export it here, which does that
/// automatically). Does not reconnect automatically; call
/// `POST /sessions/{id}/connect` afterwards.
#[utoipa::path(
    post,
    path = "/api/v1/sessions/{session_id}/import",
    tag = "sessions",
    security(("bearer_auth" = [])),
    params(
        ("session_id" = String, Path, description = "Session ID — must already exist (create it first if needed)")
    ),
    responses(
        (status = 200, description = "Storage restored", body = SuccessResponse),
        (status = 400, description = "Invalid zip upload"),
        (status = 404, description = "Session not found"),
        (status = 409, description = "Session is currently connected on this instance")
    )
)]
pub async fn import_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    mut multipart: Multipart,
) -> Result<Json<SuccessResponse>, ApiError> {
    let _ = state
        .session_manager()
        .get_session(&session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::SessionNotFound(session_id.clone()))?;

    if let Some(runtime) = state.get_session(&session_id) {
        if runtime.is_alive() {
            return Err(ApiError::AlreadyConnected);
        }
    }

    let mut zip_bytes: Option<Vec<u8>> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::BadRequest(e.to_string()))?
    {
        if field.name() == Some("file") {
            zip_bytes = Some(
                field
                    .bytes()
                    .await
                    .map_err(|e| ApiError::BadRequest(e.to_string()))?
                    .to_vec(),
            );
        }
    }
    let zip_bytes = zip_bytes.ok_or_else(|| ApiError::BadRequest("No file provided".into()))?;

    let storage_path = state
        .session_manager()
        .get_storage_path(&session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .unwrap_or_else(|| format!("{}/{}", state.base_storage_path(), session_id));

    tokio::task::spawn_blocking(move || unzip_directory(&storage_path, &zip_bytes))
        .await
        .map_err(|e| ApiError::Internal(format!("import task panicked: {e}")))?
        .map_err(|e| ApiError::BadRequest(format!("import failed: {e}")))?;

    Ok(Json(SuccessResponse::with_message(
        "Session storage imported — call /connect to bring it online",
    )))
}

/// Recursively zip a directory's contents, entry paths relative to
/// `dir`. Blocking (file I/O + deflate); run inside `spawn_blocking`.
pub(super) fn zip_directory(dir: &str) -> anyhow::Result<Vec<u8>> {
    let mut buf = std::io::Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(&mut buf);
    let options = zip::write::SimpleFileOptions::default();

    fn add_dir(
        writer: &mut zip::ZipWriter<&mut std::io::Cursor<Vec<u8>>>,
        options: zip::write::SimpleFileOptions,
        base: &std::path::Path,
        dir: &std::path::Path,
    ) -> anyhow::Result<()> {
        for entry in std::fs::read_dir(dir)?.flatten() {
            let path = entry.path();
            let rel = path.strip_prefix(base)?.to_string_lossy().to_string();
            if path.is_dir() {
                add_dir(writer, options, base, &path)?;
            } else {
                writer.start_file(rel, options)?;
                std::io::Write::write_all(writer, &std::fs::read(&path)?)?;
            }
        }
        Ok(())
    }

    let base = std::path::Path::new(dir);
    if base.is_dir() {
        add_dir(&mut writer, options, base, base)?;
    }
    writer.finish()?;
    Ok(buf.into_inner())
}

/// Caps for [`unzip_directory`]: an import is caller-supplied and untrusted,
/// so a crafted archive with a tiny compressed size but a huge (or
/// unbounded) decompressed size -- a zip bomb -- must not be able to
/// exhaust disk. `MAX_IMPORT_ENTRY_UNCOMPRESSED_BYTES` is enforced twice:
/// once from the entry's declared size (fast rejection for the well-formed
/// case) and once as a hard `Read::take` cap on the actual bytes copied,
/// since the declared size in a zip's local/central header is
/// attacker-controlled metadata, not a guarantee.
pub(super) const MAX_IMPORT_ENTRIES: usize = 20_000;

pub(super) const MAX_IMPORT_ENTRY_UNCOMPRESSED_BYTES: u64 = 512 * 1024 * 1024;

pub(super) const MAX_IMPORT_TOTAL_UNCOMPRESSED_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Unzip into `dir`, creating it if needed. Rejects entries whose path
/// would escape `dir` (zip-slip) instead of writing them, and enforces the
/// zip-bomb caps above. Blocking; run inside `spawn_blocking`.
pub(super) fn unzip_directory(dir: &str, zip_bytes: &[u8]) -> anyhow::Result<()> {
    let base = std::path::Path::new(dir);
    std::fs::create_dir_all(base)?;

    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes))?;
    if archive.len() > MAX_IMPORT_ENTRIES {
        anyhow::bail!(
            "zip has {} entries, exceeding the {} entry limit",
            archive.len(),
            MAX_IMPORT_ENTRIES
        );
    }

    let mut total_uncompressed: u64 = 0;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let Some(rel) = file.enclosed_name() else {
            anyhow::bail!(
                "zip entry {:?} has an unsafe path, refusing to extract",
                file.name()
            );
        };
        if file.is_dir() {
            continue;
        }

        if file.size() > MAX_IMPORT_ENTRY_UNCOMPRESSED_BYTES {
            anyhow::bail!(
                "zip entry {:?} declares {} uncompressed bytes, exceeding the per-file limit",
                rel,
                file.size()
            );
        }
        total_uncompressed = total_uncompressed.saturating_add(file.size());
        if total_uncompressed > MAX_IMPORT_TOTAL_UNCOMPRESSED_BYTES {
            anyhow::bail!(
                "zip's total declared uncompressed size exceeds the {} byte limit",
                MAX_IMPORT_TOTAL_UNCOMPRESSED_BYTES
            );
        }

        let out_path = base.join(&rel);
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = std::fs::File::create(&out_path)?;
        let mut limited = std::io::Read::take(&mut file, MAX_IMPORT_ENTRY_UNCOMPRESSED_BYTES + 1);
        let copied = std::io::copy(&mut limited, &mut out)?;
        if copied > MAX_IMPORT_ENTRY_UNCOMPRESSED_BYTES {
            anyhow::bail!(
                "zip entry {:?} exceeded the per-file uncompressed size limit while extracting",
                rel
            );
        }
    }
    Ok(())
}
