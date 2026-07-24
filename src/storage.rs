//! Optional S3-compatible object storage for call recordings.
//!
//! Recordings (`{base_storage_path}/{session_id}/recordings/{call_id}.wav`)
//! are the only "media" waxum itself ever writes to disk — message media
//! (images, video, documents) flows straight through to WhatsApp's own CDN
//! via [`whatsapp_rust::Client::upload`]/`download_from_params` and is
//! never persisted locally, so there is nothing else here to back with
//! object storage.
//!
//! Local filesystem is the zero-config default, matching the rest of
//! waxum. Setting `S3_BUCKET` switches [`RecordingStore`] to an
//! S3-compatible backend (real AWS S3, MinIO, R2, Wasabi, …) — `S3_ENDPOINT`
//! and `S3_REGION` pick the provider, `AWS_ACCESS_KEY_ID` /
//! `AWS_SECRET_ACCESS_KEY` are the standard credential env vars the `s3`
//! crate's [`s3::Auth::from_env`] already reads. A connection failure at
//! startup logs an error and falls back to local storage rather than
//! aborting the process — recordings are a best-effort feature, same as
//! webhook fan-out and message-search indexing elsewhere in waxum.

use s3::{Auth, Client};

/// Parsed from `S3_BUCKET` / `S3_ENDPOINT` / `S3_REGION`. `None` when
/// `S3_BUCKET` is unset (S3 disabled, local filesystem is used).
pub struct S3Config {
    pub bucket: String,
    pub endpoint: String,
    pub region: String,
}

impl S3Config {
    pub fn from_env() -> Option<Self> {
        let bucket = std::env::var("S3_BUCKET").ok()?;
        Some(Self {
            bucket,
            endpoint: std::env::var("S3_ENDPOINT")
                .unwrap_or_else(|_| "https://s3.amazonaws.com".into()),
            region: std::env::var("S3_REGION").unwrap_or_else(|_| "us-east-1".into()),
        })
    }
}

/// Where call recordings are read from / written to.
pub enum RecordingStore {
    Local { base: String },
    S3 { client: Client, bucket: String },
}

impl RecordingStore {
    pub fn local(base_storage_path: &str) -> Self {
        Self::Local {
            base: base_storage_path.to_string(),
        }
    }

    pub fn connect_s3(config: S3Config) -> anyhow::Result<Self> {
        let auth = Auth::from_env()?;
        let client = Client::builder(&config.endpoint)?
            .region(config.region)
            .auth(auth)
            .build()?;
        Ok(Self::S3 {
            client,
            bucket: config.bucket,
        })
    }

    fn key(session_id: &str, call_id: &str) -> String {
        format!("{session_id}/recordings/{call_id}.wav")
    }

    fn local_path(base: &str, session_id: &str, call_id: &str) -> std::path::PathBuf {
        std::path::Path::new(base)
            .join(session_id)
            .join("recordings")
            .join(format!("{call_id}.wav"))
    }

    pub async fn write(
        &self,
        session_id: &str,
        call_id: &str,
        bytes: Vec<u8>,
    ) -> anyhow::Result<()> {
        match self {
            Self::Local { base } => {
                let path = Self::local_path(base, session_id, call_id);
                if let Some(parent) = path.parent() {
                    tokio::fs::create_dir_all(parent).await?;
                }
                tokio::fs::write(&path, bytes).await?;
                Ok(())
            }
            Self::S3 { client, bucket } => {
                client
                    .objects()
                    .put(bucket, Self::key(session_id, call_id))
                    .body_bytes(bytes)
                    .send()
                    .await?;
                Ok(())
            }
        }
    }

    /// `Ok(None)` when the recording does not exist (not yet finished, or
    /// never made). Any other failure to reach the store is an `Err`.
    pub async fn read(&self, session_id: &str, call_id: &str) -> anyhow::Result<Option<Vec<u8>>> {
        match self {
            Self::Local { base } => {
                let path = Self::local_path(base, session_id, call_id);
                match tokio::fs::read(&path).await {
                    Ok(bytes) => Ok(Some(bytes)),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                    Err(e) => Err(e.into()),
                }
            }
            Self::S3 { client, bucket } => {
                match client
                    .objects()
                    .get(bucket, Self::key(session_id, call_id))
                    .send()
                    .await
                {
                    Ok(obj) => Ok(Some(obj.bytes().await?.to_vec())),
                    Err(e) if e.status() == Some(reqwest::StatusCode::NOT_FOUND) => Ok(None),
                    Err(e) => Err(e.into()),
                }
            }
        }
    }
}
