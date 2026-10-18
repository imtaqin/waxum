//! waxum running inside another process, for desktop frontends.
//!
//! [`Embedded::start`] builds the same state, background workers and
//! router the server binary runs, on a SQLite database and session
//! storage under one data directory. There is no TCP listener and no JWT
//! gate: requests go straight into the router with [`Embedded::call`],
//! so every REST endpoint is available in-process without opening a
//! port.

use std::path::PathBuf;

use axum::body::Body;
use axum::http::{Request, Response};
use tower::ServiceExt;

use crate::api::router::create_router;
use crate::db;
use crate::preflight::{self, InstanceLock};
use crate::state::AppState;
use crate::storage::RecordingStore;

/// Where an embedded instance keeps its data.
#[derive(Clone, Debug)]
pub struct EmbedConfig {
    /// Holds `waxum.db` and the `sessions/` storage directory.
    pub data_dir: PathBuf,
}

/// A running in-process waxum. Dropping it releases the instance lock;
/// background tasks live as long as the tokio runtime they were started on.
pub struct Embedded {
    state: AppState,
    router: axum::Router,
    _lock: InstanceLock,
}

impl Embedded {
    /// Opens the database, initialises the schema, takes the instance
    /// lock and spawns the background workers on the current runtime.
    pub async fn start(config: EmbedConfig) -> anyhow::Result<Self> {
        let storage = config.data_dir.join("sessions");
        std::fs::create_dir_all(&storage)?;
        let storage = storage.to_string_lossy().into_owned();
        let db_path = config.data_dir.join("waxum.db");

        let pool = db::connect(&db_path.to_string_lossy(), db::DbBackend::SQLite).await?;
        db::schema::init_schema(&pool).await?;

        let recordings = RecordingStore::local(&storage);
        let state = AppState::with_storage_path(pool, None, recordings, storage).await;
        let lock = preflight::acquire_instance_lock(state.base_storage_path())
            .map_err(anyhow::Error::msg)?;

        crate::api::shared::bulk::handlers::touch_uptime_anchor();
        crate::workers::spawn(&state);

        let router = create_router().with_state(state.clone());
        Ok(Self {
            state,
            router,
            _lock: lock,
        })
    }

    pub fn state(&self) -> &AppState {
        &self.state
    }

    /// Runs one request through the full API router, as if it had arrived
    /// over HTTP with superadmin rights.
    pub async fn call(&self, request: Request<Body>) -> Response<Body> {
        match self.router.clone().oneshot(request).await {
            Ok(response) => response,
            Err(never) => match never {},
        }
    }
}
