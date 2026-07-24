//! Process-wide shared state.
//!
//! [`AppState`] is cloned into every axum handler. It owns:
//!
//! - the `SessionManager` from [`crate::db`] (DB access),
//! - the in-memory [`DashMap`] of per-session runtimes ([`SessionState`]),
//! - the webhook registry,
//! - the optional NATS handle, and
//! - the per-URL webhook [`CircuitState`] table.
//!
//! [`SessionState`] tracks everything about one live WhatsApp session that
//! doesn't belong on disk: the cached `whatsapp_rust::Client`, current QR
//! frames, pair code, [`SessionStatus`], pair telemetry, rolling logout
//! history (used to decide when to auto-purge), and an event broadcast
//! channel other handlers can subscribe to.

use dashmap::DashMap;
use parking_lot::RwLock;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::broadcast;
use whatsapp_rust::Client;

use crate::db::session::DbPool;
use crate::db::SessionManager;
use crate::models::sessions::SessionStatus;
use crate::models::webhooks::WebhookConfig;
use crate::nats::NatsManager;

/// Shared reqwest client for webhook delivery. Per-call `Client::new()` skips
/// the connection pool and uses the OS-level TCP timeout (~75 s), so a
/// downtime on a webhook target piled tokio tasks faster than they could
/// drain — we observed ~600 threads on a 0 % CPU idle process. A shared
/// client with explicit timeouts keeps each task bounded to ~10 s.
fn webhook_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_secs(5))
            .pool_idle_timeout(Duration::from_secs(90))
            .pool_max_idle_per_host(16)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

pub struct SessionState {
    pub client: RwLock<Option<Arc<Client>>>,

    pub qr_codes: RwLock<Vec<String>>,

    pub pair_code: RwLock<Option<String>>,

    pub status: RwLock<SessionStatus>,

    pub event_tx: broadcast::Sender<String>,

    #[allow(dead_code)]
    pub storage_path: String,

    /// Rolling log of recent LoggedOut event timestamps (unix seconds).
    /// Used by the auto-purge logic so an unstable upstream that flaps
    /// once doesn't blow away the on-disk session — we only purge once
    /// we see N rapid logouts inside a short window. Kept inline so it
    /// shares the same RwLock discipline as the other session fields.
    pub logout_history: RwLock<Vec<i64>>,

    /// Pair-flow telemetry. Surfaced through /status so the backend can
    /// show users meaningful progress and last-error text instead of
    /// guessing from polling QR codes.
    pub pair_state: RwLock<PairState>,
}

/// Snapshot of the latest pair attempt for a session. Lives entirely in
/// memory — cleared on connect_client start, populated as events arrive.
#[derive(Clone, Debug, Default)]
pub struct PairState {
    pub last_qr_at: Option<i64>,
    pub last_pair_code_at: Option<i64>,
    pub pair_code_expires_at: Option<i64>,
    pub last_error: Option<String>,
    pub attempts: u32,
}

impl SessionState {
    pub fn new(storage_path: String) -> Self {
        let (event_tx, _) = broadcast::channel(1000);
        Self {
            client: RwLock::new(None),
            qr_codes: RwLock::new(Vec::new()),
            pair_code: RwLock::new(None),
            status: RwLock::new(SessionStatus::Disconnected),
            event_tx,
            storage_path,
            logout_history: RwLock::new(Vec::new()),
            pair_state: RwLock::new(PairState::default()),
        }
    }

    /// Record a LoggedOut event and return whether the session has crossed
    /// the auto-purge threshold (N events inside WINDOW seconds). The
    /// caller — the LoggedOut event handler — uses the return value to
    /// decide whether to wipe the storage row or just mark the session
    /// disconnected and let the user retry.
    pub fn record_logout_and_should_purge(&self) -> bool {
        const WINDOW_SECS: i64 = 600;
        const THRESHOLD: usize = 3;
        let now = chrono::Utc::now().timestamp();
        let mut hist = self.logout_history.write();
        hist.retain(|t| now - *t < WINDOW_SECS);
        hist.push(now);
        hist.len() >= THRESHOLD
    }

    pub fn get_pair_state(&self) -> PairState {
        self.pair_state.read().clone()
    }

    pub fn update_pair_state(&self, f: impl FnOnce(&mut PairState)) {
        f(&mut self.pair_state.write());
    }

    pub fn clear_pair_state(&self) {
        *self.pair_state.write() = PairState::default();
    }

    pub fn get_client(&self) -> Option<Arc<Client>> {
        self.client.read().clone()
    }

    /// Return the client only if the underlying socket is actually alive and
    /// the device is logged in. Send handlers should use this instead of
    /// `get_client` so a stale Arc left over from a silent disconnect
    /// doesn't accept a write that will never leave the socket.
    pub fn get_live_client(&self) -> Option<Arc<Client>> {
        let c = self.client.read().clone()?;
        if c.is_connected() && c.is_logged_in() {
            Some(c)
        } else {
            None
        }
    }

    /// Single source of truth used by /status and /sessions: only "logged in"
    /// when the cached client agrees it's connected AND authenticated.
    pub fn is_alive(&self) -> bool {
        match self.client.read().as_ref() {
            Some(c) => c.is_connected() && c.is_logged_in(),
            None => false,
        }
    }

    pub fn set_client(&self, client: Option<Arc<Client>>) {
        *self.client.write() = client;
    }

    pub fn get_qr_codes(&self) -> Vec<String> {
        self.qr_codes.read().clone()
    }

    pub fn set_qr_codes(&self, codes: Vec<String>) {
        *self.qr_codes.write() = codes;
    }

    pub fn get_pair_code(&self) -> Option<String> {
        self.pair_code.read().clone()
    }

    pub fn set_pair_code(&self, code: Option<String>) {
        *self.pair_code.write() = code;
    }

    pub fn get_status(&self) -> SessionStatus {
        *self.status.read()
    }

    /// Reconciled view of the session status. Reads the cached
    /// `SessionStatus`, then reality-checks it against the live client
    /// socket via `is_alive()`.
    ///
    /// When the cache says `LoggedIn` but the socket is not currently
    /// alive, the return value degrades to **`Connecting`** — not
    /// `Disconnected` — because the whatsapp-rust client has
    /// auto-reconnect on by default, so a dead socket almost always
    /// means "the peer is rebuilding the WebSocket right now" rather
    /// than "the account is gone". Only an explicit `LoggedOut` event
    /// (which the event loop turns into a cached `Disconnected`)
    /// yields a real `Disconnected` here. This prevents the console
    /// header from flashing a red OFFLINE pill during every network
    /// blip.
    pub fn effective_status(&self) -> SessionStatus {
        let cached = *self.status.read();
        if cached == SessionStatus::LoggedIn && !self.is_alive() {
            SessionStatus::Connecting
        } else {
            cached
        }
    }

    pub fn set_status(&self, status: SessionStatus) {
        *self.status.write() = status;
    }

    pub fn broadcast_event(&self, event: String) {
        let _ = self.event_tx.send(event);
    }

    #[allow(dead_code)]
    pub fn subscribe_events(&self) -> broadcast::Receiver<String> {
        self.event_tx.subscribe()
    }
}

#[derive(Clone)]
pub struct AppState {
    inner: Arc<AppStateInner>,
}

struct AppStateInner {
    pub session_manager: SessionManager,

    pub sessions: DashMap<String, Arc<SessionState>>,

    pub webhooks: DashMap<String, DashMap<String, WebhookConfig>>,

    pub base_storage_path: String,

    pub nats: Option<NatsManager>,

    /// Where call recordings are read from / written to. Local
    /// filesystem by default; S3-compatible object storage when
    /// `S3_BUCKET` is configured. See [`crate::storage`].
    pub recordings: crate::storage::RecordingStore,

    pub webhook_circuits: DashMap<String, CircuitState>,

    pub incoming_calls: DashMap<String, wacore::types::call::IncomingCall>,

    pub active_calls: DashMap<String, Arc<whatsapp_rust::voip::CallHandle>>,

    pub call_audio_channels: DashMap<String, ActiveCallAudio>,

    /// In-memory tag membership per session. `DashMap<session_id, HashSet<tag>>`.
    /// Persisted as `{base_storage_path}/session_tags.json` on every mutation
    /// so restarts do not wipe organisation. Not on the hot path — tags are
    /// only read on the console + session listing filters.
    pub session_tags: DashMap<String, std::collections::HashSet<String>>,

    /// Bounded ring of the last N events crossing `broadcast_to_webhooks`.
    /// Backs the console overview "Live events" panel and also serves as
    /// the source for the terminal event log line.
    pub event_ring: parking_lot::Mutex<std::collections::VecDeque<ConsoleEvent>>,

    /// Edge-TTS voice list, fetched once on first `GET /api/v1/voices`
    /// and reused after that — the list is stable per Edge-TTS release,
    /// so there is no point re-querying it on every request.
    pub voice_cache: RwLock<Option<Vec<crate::models::calls::VoiceEntry>>>,
}

/// A structured event captured from `broadcast_to_webhooks` for both the
/// terminal log line and the console overview panel. `payload_preview` is
/// the first ~120 chars of the JSON payload; full payload is not kept to
/// avoid unbounded memory growth.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ConsoleEvent {
    pub session_id: String,
    pub event_type: String,
    pub payload_preview: String,
    pub at_epoch_ms: i64,
}

pub struct ActiveCallAudio {
    #[allow(dead_code)]
    pub mic_tx: async_channel::Sender<Vec<i16>>,
    #[allow(dead_code)]
    pub spk_rx: async_channel::Receiver<Vec<i16>>,
}

#[derive(Clone, Debug)]
pub struct CircuitState {
    pub failures: u32,
    pub opened_until: Option<std::time::Instant>,
    pub last_event: std::time::Instant,
}

impl Default for CircuitState {
    fn default() -> Self {
        Self {
            failures: 0,
            opened_until: None,
            last_event: std::time::Instant::now(),
        }
    }
}

/// Return code from [`AppState::webhook_record_failure`]: describes the
/// state change the failure just caused, so the caller can log and, in
/// the case of `HardDisable`, persist to the DB + purge in-memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebhookFailureAction {
    Noop,
    Open,
    HardDisable,
}

impl AppState {
    pub async fn new(
        pool: DbPool,
        nats: Option<NatsManager>,
        recordings: crate::storage::RecordingStore,
    ) -> Self {
        let base_storage_path = std::env::var("WHATSAPP_STORAGE_PATH")
            .unwrap_or_else(|_| "./whatsapp_sessions".to_string());

        let _ = tokio::fs::create_dir_all(&base_storage_path).await;

        let session_manager = SessionManager::new(pool);

        let state = Self {
            inner: Arc::new(AppStateInner {
                session_manager,
                sessions: DashMap::new(),
                webhooks: DashMap::new(),
                base_storage_path,
                nats,
                recordings,
                webhook_circuits: DashMap::new(),
                incoming_calls: DashMap::new(),
                active_calls: DashMap::new(),
                call_audio_channels: DashMap::new(),
                event_ring: parking_lot::Mutex::new(std::collections::VecDeque::with_capacity(200)),
                session_tags: DashMap::new(),
                voice_cache: RwLock::new(None),
            }),
        };

        let path = Self::tags_file_path(&state.inner.base_storage_path);
        if let Ok(bytes) = tokio::fs::read(&path).await {
            if let Ok(map) =
                serde_json::from_slice::<std::collections::HashMap<String, Vec<String>>>(&bytes)
            {
                for (sid, tags) in map {
                    state
                        .inner
                        .session_tags
                        .insert(sid, tags.into_iter().collect());
                }
                tracing::info!(
                    target: "waxum::tags",
                    entries = state.inner.session_tags.len(),
                    "loaded session tags from {}",
                    path.display()
                );
            }
        }

        state
    }

    fn tags_file_path(base: &str) -> std::path::PathBuf {
        std::path::Path::new(base).join("session_tags.json")
    }

    async fn persist_tags(&self) {
        let snapshot: std::collections::HashMap<String, Vec<String>> = self
            .inner
            .session_tags
            .iter()
            .map(|kv| (kv.key().clone(), kv.value().iter().cloned().collect()))
            .collect();
        let path = Self::tags_file_path(&self.inner.base_storage_path);
        match serde_json::to_vec_pretty(&snapshot) {
            Ok(bytes) => {
                if let Err(e) = tokio::fs::write(&path, bytes).await {
                    tracing::warn!(target: "waxum::tags", "persist tags failed: {e}");
                }
            }
            Err(e) => tracing::warn!(target: "waxum::tags", "serialise tags failed: {e}"),
        }
    }

    pub fn list_tags(&self, session_id: &str) -> Vec<String> {
        self.inner
            .session_tags
            .get(session_id)
            .map(|kv| {
                let mut v: Vec<String> = kv.value().iter().cloned().collect();
                v.sort();
                v
            })
            .unwrap_or_default()
    }

    pub async fn set_tags(&self, session_id: &str, tags: Vec<String>) {
        let cleaned: std::collections::HashSet<String> = tags
            .into_iter()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect();
        if cleaned.is_empty() {
            self.inner.session_tags.remove(session_id);
        } else {
            self.inner
                .session_tags
                .insert(session_id.to_string(), cleaned);
        }
        self.persist_tags().await;
    }

    pub async fn add_tag(&self, session_id: &str, tag: &str) -> bool {
        let tag = tag.trim().to_string();
        if tag.is_empty() {
            return false;
        }
        let inserted = self
            .inner
            .session_tags
            .entry(session_id.to_string())
            .or_default()
            .insert(tag);
        if inserted {
            self.persist_tags().await;
        }
        inserted
    }

    pub async fn remove_tag(&self, session_id: &str, tag: &str) -> bool {
        let mut changed = false;
        let mut drop_key = false;
        if let Some(mut entry) = self.inner.session_tags.get_mut(session_id) {
            changed = entry.remove(tag);
            if entry.is_empty() {
                drop_key = true;
            }
        }
        if drop_key {
            self.inner.session_tags.remove(session_id);
        }
        if changed {
            self.persist_tags().await;
        }
        changed
    }

    pub fn sessions_with_tag(&self, tag: &str) -> Vec<String> {
        self.inner
            .session_tags
            .iter()
            .filter_map(|kv| {
                if kv.value().contains(tag) {
                    Some(kv.key().clone())
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn all_session_ids_with_tags(&self) -> Vec<String> {
        self.inner
            .session_tags
            .iter()
            .map(|kv| kv.key().clone())
            .collect()
    }

    pub async fn drop_tags_for(&self, session_id: &str) {
        if self.inner.session_tags.remove(session_id).is_some() {
            self.persist_tags().await;
        }
    }

    pub fn push_event(&self, session_id: &str, event: &str, payload: &str) {
        const PREVIEW_MAX: usize = 160;
        let preview: String = payload.chars().take(PREVIEW_MAX).collect();
        let now = chrono::Utc::now().timestamp_millis();
        let mut ring = self.inner.event_ring.lock();
        if ring.len() >= 200 {
            ring.pop_front();
        }
        ring.push_back(ConsoleEvent {
            session_id: session_id.to_string(),
            event_type: event.to_string(),
            payload_preview: preview,
            at_epoch_ms: now,
        });
        tracing::info!(
            target: "waxum::event",
            session_id = %session_id,
            event = %event,
            "{}",
            payload.chars().take(PREVIEW_MAX).collect::<String>()
        );
    }

    pub fn recent_events(&self, limit: usize) -> Vec<ConsoleEvent> {
        let ring = self.inner.event_ring.lock();
        ring.iter().rev().take(limit).cloned().collect()
    }

    pub fn cached_voices(&self) -> Option<Vec<crate::models::calls::VoiceEntry>> {
        self.inner.voice_cache.read().clone()
    }

    pub fn set_cached_voices(&self, voices: Vec<crate::models::calls::VoiceEntry>) {
        *self.inner.voice_cache.write() = Some(voices);
    }

    pub fn incoming_calls(&self) -> &DashMap<String, wacore::types::call::IncomingCall> {
        &self.inner.incoming_calls
    }

    pub fn active_calls(&self) -> &DashMap<String, Arc<whatsapp_rust::voip::CallHandle>> {
        &self.inner.active_calls
    }

    pub fn call_audio_channels(&self) -> &DashMap<String, ActiveCallAudio> {
        &self.inner.call_audio_channels
    }

    /// Should we still attempt this webhook URL right now?
    pub fn webhook_circuit_allows(&self, url: &str) -> bool {
        let now = std::time::Instant::now();
        let map = &self.inner.webhook_circuits;
        let entry = map.get(url);
        match entry.as_deref() {
            Some(c) => match c.opened_until {
                Some(until) => now >= until,
                None => true,
            },
            None => true,
        }
    }

    pub fn webhook_circuits_open_count(&self) -> usize {
        let now = std::time::Instant::now();
        self.inner
            .webhook_circuits
            .iter()
            .filter(|c| c.value().opened_until.map(|u| now < u).unwrap_or(false))
            .count()
    }

    pub fn webhook_record_success(&self, url: &str) {
        let map = &self.inner.webhook_circuits;
        if let Some(mut c) = map.get_mut(url) {
            c.failures = 0;
            c.opened_until = None;
            c.last_event = std::time::Instant::now();
        }
    }

    /// Returns the delta after this failure: `Open` when the circuit
    /// first tripped and should skip dispatch for 5 min, `HardDisable`
    /// when the target has been failing so long we're going to persist
    /// `enabled=false` and stop even queuing events for it.
    pub fn webhook_record_failure(&self, url: &str) -> WebhookFailureAction {
        const OPEN_THRESHOLD: u32 = 25;
        const HARD_DISABLE_THRESHOLD: u32 = 100;
        const COOLDOWN: std::time::Duration = std::time::Duration::from_secs(300);
        let map = &self.inner.webhook_circuits;
        let mut entry = map.entry(url.to_string()).or_default();
        entry.last_event = std::time::Instant::now();
        entry.failures = entry.failures.saturating_add(1);
        if entry.failures >= HARD_DISABLE_THRESHOLD {
            return WebhookFailureAction::HardDisable;
        }
        if entry.failures >= OPEN_THRESHOLD && entry.opened_until.is_none() {
            entry.opened_until = Some(std::time::Instant::now() + COOLDOWN);
            return WebhookFailureAction::Open;
        }
        WebhookFailureAction::Noop
    }

    /// Wipe every in-memory registration for `url` so once the DB row is
    /// marked disabled the dispatcher stops considering it too.
    pub fn purge_webhook_by_url(&self, url: &str) {
        let sessions_with_url: Vec<(String, Vec<String>)> = self
            .inner
            .webhooks
            .iter()
            .filter_map(|entry| {
                let ids: Vec<String> = entry
                    .value()
                    .iter()
                    .filter(|w| w.value().url == url)
                    .map(|w| w.key().clone())
                    .collect();
                if ids.is_empty() {
                    None
                } else {
                    Some((entry.key().clone(), ids))
                }
            })
            .collect();
        for (session_id, ids) in sessions_with_url {
            if let Some(session_map) = self.inner.webhooks.get(&session_id) {
                for id in ids {
                    session_map.remove(&id);
                }
            }
        }
        self.inner.webhook_circuits.remove(url);
    }

    pub fn purge_webhooks_for_session(&self, session_id: &str) {
        self.inner.webhooks.remove(session_id);
    }

    /// Bulk close-and-reset every open circuit. Used by
    /// `POST /api/v1/webhooks/reenable-all` so an operator does not have
    /// to walk every session's URL list by hand after fixing a mass
    /// downstream outage. Returns the URLs whose state was actually
    /// cleared (open circuits only; healthy circuits are left alone).
    pub fn reenable_all_open_circuits(&self) -> Vec<String> {
        let now = std::time::Instant::now();
        let mut reset: Vec<String> = Vec::new();
        for mut entry in self.inner.webhook_circuits.iter_mut() {
            let opened = entry.value().opened_until.map(|u| now < u).unwrap_or(false);
            if opened {
                let e = entry.value_mut();
                e.failures = 0;
                e.opened_until = None;
                e.last_event = now;
                reset.push(entry.key().clone());
            }
        }
        reset
    }

    pub fn nats(&self) -> Option<&NatsManager> {
        self.inner.nats.as_ref()
    }

    pub fn recordings(&self) -> &crate::storage::RecordingStore {
        &self.inner.recordings
    }

    /// Publish an event to NATS JetStream (no-op if NATS not configured).
    pub async fn publish_to_nats(&self, session_id: &str, event_type: &str, payload: &str) {
        if let Some(nats) = &self.inner.nats {
            crate::nats::publisher::publish_event(
                nats.jetstream(),
                session_id,
                event_type,
                payload,
            )
            .await;
        }
    }

    pub fn session_manager(&self) -> &SessionManager {
        &self.inner.session_manager
    }

    pub fn base_storage_path(&self) -> &str {
        &self.inner.base_storage_path
    }

    pub fn get_or_create_session(&self, session_id: &str, storage_path: &str) -> Arc<SessionState> {
        self.inner
            .sessions
            .entry(session_id.to_string())
            .or_insert_with(|| Arc::new(SessionState::new(storage_path.to_string())))
            .clone()
    }

    pub fn get_session(&self, session_id: &str) -> Option<Arc<SessionState>> {
        self.inner.sessions.get(session_id).map(|r| r.clone())
    }

    pub fn remove_session(&self, session_id: &str) -> Option<Arc<SessionState>> {
        self.inner.sessions.remove(session_id).map(|(_, v)| v)
    }

    pub fn session_iter(&self) -> Vec<Arc<SessionState>> {
        self.inner
            .sessions
            .iter()
            .map(|r| r.value().clone())
            .collect()
    }

    #[allow(dead_code)]
    pub fn has_session(&self, session_id: &str) -> bool {
        self.inner.sessions.contains_key(session_id)
    }

    pub fn register_webhook(&self, session_id: &str, webhook_id: &str, config: WebhookConfig) {
        self.inner
            .webhooks
            .entry(session_id.to_string())
            .or_default()
            .insert(webhook_id.to_string(), config);
    }

    pub fn get_webhooks(&self, session_id: &str) -> Vec<(String, WebhookConfig)> {
        self.inner
            .webhooks
            .get(session_id)
            .map(|m| {
                m.iter()
                    .map(|r| (r.key().clone(), r.value().clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn remove_webhook(&self, session_id: &str, webhook_id: &str) -> Option<WebhookConfig> {
        self.inner
            .webhooks
            .get(session_id)
            .and_then(|m| m.remove(webhook_id).map(|(_, v)| v))
    }

    pub async fn broadcast_to_webhooks(&self, session_id: &str, event: &str, payload: &str) {
        self.push_event(session_id, event, payload);

        let webhooks = self.get_webhooks(session_id);
        let client = webhook_client();
        let pool = self.session_manager().pool().clone();
        let session_id_owned = session_id.to_string();
        let event_owned = event.to_string();

        for (_, config) in webhooks {
            if !config.enabled {
                continue;
            }

            if !config.events.iter().any(|e| e.matches(event)) {
                continue;
            }

            if !self.webhook_circuit_allows(&config.url) {
                continue;
            }

            let url = config.url.clone();
            let payload = payload.to_string();
            let secret = config.secret.clone();
            let client = client.clone();
            let pool = pool.clone();
            let session_id_owned = session_id_owned.clone();
            let event_owned = event_owned.clone();
            let state_for_task = self.clone();

            tokio::spawn(async move {
                let backoff_ms = [0u64, 1000, 3000, 7000];
                let mut last_err: Option<String> = None;
                for (i, delay) in backoff_ms.iter().enumerate() {
                    if *delay > 0 {
                        tokio::time::sleep(Duration::from_millis(*delay)).await;
                    }

                    let mut req = client
                        .post(&url)
                        .header("Content-Type", "application/json")
                        .body(payload.clone());

                    if let Some(secret) = &secret {
                        use hmac::{Hmac, Mac};
                        use sha2::Sha256;

                        type HmacSha256 = Hmac<Sha256>;
                        if let Ok(mut mac) = HmacSha256::new_from_slice(secret.as_bytes()) {
                            mac.update(payload.as_bytes());
                            let signature = hex::encode(mac.finalize().into_bytes());
                            req =
                                req.header("X-Webhook-Signature", format!("sha256={}", signature));
                        }
                    }

                    match req.send().await {
                        Ok(resp) => {
                            let status = resp.status();
                            if status.is_success() {
                                state_for_task.webhook_record_success(&url);
                                return;
                            }
                            if status.is_client_error()
                                && status.as_u16() != 408
                                && status.as_u16() != 429
                            {
                                tracing::warn!(
                                    "Webhook {} rejected with {} — not retrying",
                                    url,
                                    status
                                );
                                return;
                            }
                            last_err = Some(format!("HTTP {}", status));
                        }
                        Err(e) => {
                            last_err = Some(e.to_string());
                        }
                    }
                    let _ = i;
                }
                if let Some(err) = last_err {
                    let action = state_for_task.webhook_record_failure(&url);
                    match action {
                        WebhookFailureAction::HardDisable => {
                            tracing::warn!(
                                "Webhook {} auto-DISABLED after 100 consecutive failures — DB row switched to enabled=false",
                                url
                            );
                            let reason = format!("100 consecutive failures ({err})");
                            match state_for_task
                                .session_manager()
                                .disable_webhook_by_url(&url, &reason)
                                .await
                            {
                                Ok(n) => tracing::info!(
                                    "webhook auto-disable: {} row(s) marked enabled=false for {}",
                                    n,
                                    url
                                ),
                                Err(err) => tracing::warn!(
                                    "webhook auto-disable persist failed for {}: {}",
                                    url,
                                    err
                                ),
                            }
                            state_for_task.purge_webhook_by_url(&url);
                        }
                        WebhookFailureAction::Open => {
                            tracing::warn!(
                                "Webhook {} circuit OPEN after 25 consecutive failures — skipping dispatch for 5 min",
                                url
                            );
                        }
                        WebhookFailureAction::Noop => {
                            tracing::warn!(
                                "Failed to send webhook to {} after {} attempts: {}",
                                url,
                                backoff_ms.len(),
                                err
                            );
                        }
                    }
                    crate::db::webhook_dlq::record_failure(
                        &pool,
                        &session_id_owned,
                        &url,
                        &event_owned,
                        &payload,
                        &err,
                        backoff_ms.len() as i32,
                    )
                    .await;
                }
            });
        }
    }
}
