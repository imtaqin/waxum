use dashmap::DashMap;
use deadpool_postgres::Pool;
use parking_lot::RwLock;
use std::sync::Arc;
use tokio::sync::broadcast;
use whatsapp_rust::Client;

use crate::db::SessionManager;
use crate::models::sessions::SessionStatus;
use crate::models::webhooks::WebhookConfig;

/// Per-session runtime state (not persisted)
pub struct SessionState {
    /// WhatsApp client instance
    pub client: RwLock<Option<Arc<Client>>>,
    /// Current QR codes
    pub qr_codes: RwLock<Vec<String>>,
    /// Current pair code
    pub pair_code: RwLock<Option<String>>,
    /// Current status
    pub status: RwLock<SessionStatus>,
    /// Event broadcast channel
    pub event_tx: broadcast::Sender<String>,
    /// Storage path for this session
    #[allow(dead_code)]
    pub storage_path: String,
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
        }
    }

    pub fn get_client(&self) -> Option<Arc<Client>> {
        self.client.read().clone()
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

/// Shared application state
#[derive(Clone)]
pub struct AppState {
    inner: Arc<AppStateInner>,
}

struct AppStateInner {
    /// Session database manager
    pub session_manager: SessionManager,
    /// Active sessions (runtime state)
    pub sessions: DashMap<String, Arc<SessionState>>,
    /// In-memory webhooks cache (session_id -> webhook_id -> config)
    pub webhooks: DashMap<String, DashMap<String, WebhookConfig>>,
    /// Base storage path
    pub base_storage_path: String,
}

impl AppState {
    pub async fn new(pool: Pool) -> Self {
        let base_storage_path = std::env::var("WHATSAPP_STORAGE_PATH")
            .unwrap_or_else(|_| "./whatsapp_sessions".to_string());

        // Create base storage directory
        let _ = tokio::fs::create_dir_all(&base_storage_path).await;

        let session_manager = SessionManager::new(pool);

        Self {
            inner: Arc::new(AppStateInner {
                session_manager,
                sessions: DashMap::new(),
                webhooks: DashMap::new(),
                base_storage_path,
            }),
        }
    }

    /// Get the session manager
    pub fn session_manager(&self) -> &SessionManager {
        &self.inner.session_manager
    }

    /// Get base storage path
    pub fn base_storage_path(&self) -> &str {
        &self.inner.base_storage_path
    }

    /// Get or create session runtime state
    pub fn get_or_create_session(&self, session_id: &str, storage_path: &str) -> Arc<SessionState> {
        self.inner
            .sessions
            .entry(session_id.to_string())
            .or_insert_with(|| Arc::new(SessionState::new(storage_path.to_string())))
            .clone()
    }

    /// Get session runtime state
    pub fn get_session(&self, session_id: &str) -> Option<Arc<SessionState>> {
        self.inner.sessions.get(session_id).map(|r| r.clone())
    }

    /// Remove session runtime state
    pub fn remove_session(&self, session_id: &str) -> Option<Arc<SessionState>> {
        self.inner.sessions.remove(session_id).map(|(_, v)| v)
    }

    /// Check if session exists in runtime
    #[allow(dead_code)]
    pub fn has_session(&self, session_id: &str) -> bool {
        self.inner.sessions.contains_key(session_id)
    }

    /// Register webhook for session
    pub fn register_webhook(&self, session_id: &str, webhook_id: &str, config: WebhookConfig) {
        self.inner
            .webhooks
            .entry(session_id.to_string())
            .or_default()
            .insert(webhook_id.to_string(), config);
    }

    /// Get webhooks for session
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

    /// Remove webhook
    pub fn remove_webhook(&self, session_id: &str, webhook_id: &str) -> Option<WebhookConfig> {
        self.inner
            .webhooks
            .get(session_id)
            .and_then(|m| m.remove(webhook_id).map(|(_, v)| v))
    }

    /// Broadcast event to all webhooks for a session
    pub async fn broadcast_to_webhooks(&self, session_id: &str, event: &str, payload: &str) {
        let webhooks = self.get_webhooks(session_id);
        let client = reqwest::Client::new();

        for (_, config) in webhooks {
            if !config.enabled {
                continue;
            }

            // Check if this webhook is interested in this event
            if !config.events.iter().any(|e| e.matches(event)) {
                continue;
            }

            let url = config.url.clone();
            let payload = payload.to_string();
            let secret = config.secret.clone();
            let client = client.clone();

            // Fire and forget
            tokio::spawn(async move {
                let mut req = client
                    .post(&url)
                    .header("Content-Type", "application/json")
                    .body(payload.clone());

                // Add HMAC signature if secret is set
                if let Some(secret) = secret {
                    use hmac::{Hmac, Mac};
                    use sha2::Sha256;

                    type HmacSha256 = Hmac<Sha256>;
                    if let Ok(mut mac) = HmacSha256::new_from_slice(secret.as_bytes()) {
                        mac.update(payload.as_bytes());
                        let signature = hex::encode(mac.finalize().into_bytes());
                        req = req.header("X-Webhook-Signature", format!("sha256={}", signature));
                    }
                }

                if let Err(e) = req.send().await {
                    tracing::warn!("Failed to send webhook to {}: {}", url, e);
                }
            });
        }
    }
}
