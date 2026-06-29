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
}

impl AppState {
    pub async fn new(pool: DbPool, nats: Option<NatsManager>) -> Self {
        let base_storage_path = std::env::var("WHATSAPP_STORAGE_PATH")
            .unwrap_or_else(|_| "./whatsapp_sessions".to_string());

        let _ = tokio::fs::create_dir_all(&base_storage_path).await;

        let session_manager = SessionManager::new(pool);

        Self {
            inner: Arc::new(AppStateInner {
                session_manager,
                sessions: DashMap::new(),
                webhooks: DashMap::new(),
                base_storage_path,
                nats,
            }),
        }
    }

    pub fn nats(&self) -> Option<&NatsManager> {
        self.inner.nats.as_ref()
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

            let url = config.url.clone();
            let payload = payload.to_string();
            let secret = config.secret.clone();
            let client = client.clone();
            let pool = pool.clone();
            let session_id_owned = session_id_owned.clone();
            let event_owned = event_owned.clone();

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
                    tracing::warn!(
                        "Failed to send webhook to {} after {} attempts: {}",
                        url,
                        backoff_ms.len(),
                        err
                    );
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
