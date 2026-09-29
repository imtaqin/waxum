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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::broadcast;
use whatsapp_rust::Client;

use crate::db::session::DbPool;
use crate::db::SessionManager;
use crate::models::sessions::SessionStatus;
use crate::models::webhooks::{WebhookConfig, WebhookDlqEntry};
use crate::nats::NatsManager;

/// Value of the `X-Webhook-Signature-Version` header sent with every
/// webhook delivery. `v2` = HMAC-SHA256 over `"{timestamp}.{body}"`
/// (timestamp-prefixed, hex digest with a `sha256=` prefix); the legacy
/// pre-0.9.8 scheme signed the raw body alone and carried no version
/// header at all, so consumers can treat a missing header as `v1`.
pub const WEBHOOK_SIGNATURE_VERSION: &str = "v2";

/// Runtime retry/dead-letter policy for webhook delivery, read once at
/// startup from the environment:
///
/// - `WEBHOOK_RETRY_MAX_ATTEMPTS` (default 3) — total delivery attempts
///   per event before the payload lands in the DLQ. Backoff between
///   attempts: immediate, +5 s, +30 s, then doubling (60 s, 120 s, ...)
///   capped at 5 min.
/// - `WEBHOOK_RETRY_ON_4XX` (default true) — also retry 4xx responses.
///   A 401/403 window is usually a consumer-side auth misconfig that
///   gets fixed; dropping those events on the first attempt loses
///   messages for good. Set to `false` to restore the old "4xx is
///   permanent" behavior (408/429 are retried either way).
/// - `WEBHOOK_DLQ_CAPACITY` (default 100) — per-session in-memory DLQ
///   ring size; oldest entries are evicted past the cap.
#[derive(Clone, Debug)]
pub struct WebhookRetryConfig {
    pub max_attempts: usize,
    pub retry_on_4xx: bool,
    pub dlq_capacity: usize,
}

impl WebhookRetryConfig {
    fn from_env() -> Self {
        let max_attempts = std::env::var("WEBHOOK_RETRY_MAX_ATTEMPTS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|n| *n > 0)
            .unwrap_or(3);
        let retry_on_4xx = std::env::var("WEBHOOK_RETRY_ON_4XX")
            .map(|v| {
                !matches!(
                    v.to_ascii_lowercase().as_str(),
                    "0" | "false" | "no" | "off"
                )
            })
            .unwrap_or(true);
        let dlq_capacity = std::env::var("WEBHOOK_DLQ_CAPACITY")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|n| *n > 0)
            .unwrap_or(100);
        Self {
            max_attempts,
            retry_on_4xx,
            dlq_capacity,
        }
    }
}

/// Escalating cooldown applied when WhatsApp locks an account
/// (`ConnectFailureReason::AccountLocked`, WA Web 403 / REASON_LOCKED).
/// Read once at startup from `ACCOUNT_LOCK_BACKOFF_SECS` — a
/// comma-separated list of cooldown seconds, default `300,900,3600`
/// (5 min → 15 min → 60 min). The first lock applies step 0, the next
/// lock inside the same process lifetime applies step 1, and so on; the
/// last step repeats once the list is exhausted. A locked account only
/// recovers when reconnect attempts stop, so a manual
/// `POST /sessions/:id/connect` is the intended way out and clears the
/// cooldown.
#[derive(Clone, Debug)]
pub struct AccountLockBackoffConfig {
    pub schedule: Vec<i64>,
}

impl AccountLockBackoffConfig {
    fn from_env() -> Self {
        let schedule = std::env::var("ACCOUNT_LOCK_BACKOFF_SECS")
            .ok()
            .map(|raw| {
                raw.split(',')
                    .filter_map(|s| s.trim().parse::<i64>().ok())
                    .filter(|n| *n > 0)
                    .collect::<Vec<_>>()
            })
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| vec![300, 900, 3600]);
        Self { schedule }
    }
}

/// Backoff before attempt `n` (0-based): the first try is immediate,
/// then 5 s, 30 s, and doubling from 60 s afterwards, capped at 5 min.
fn webhook_retry_delay(attempt: usize) -> Duration {
    match attempt {
        0 => Duration::ZERO,
        1 => Duration::from_secs(5),
        2 => Duration::from_secs(30),
        n => Duration::from_secs(60u64.saturating_mul(2u64.saturating_pow((n - 3) as u32)))
            .min(Duration::from_secs(300)),
    }
}

/// Shared reqwest client for webhook delivery. Per-call `Client::new()` skips
/// the connection pool and uses the OS-level TCP timeout (~75 s), so a
/// downtime on a webhook target piled tokio tasks faster than they could
/// drain — we observed ~600 threads on a 0 % CPU idle process. A shared
/// client with explicit timeouts keeps each task bounded to ~10 s.
///
/// `dns_resolver` is [`crate::net_guard::SsrfSafeResolver`]: registration
/// already rejects a webhook URL that resolves to a private/internal
/// address (`net_guard::validate_public_url`), but that check happens once,
/// up front. This resolver repeats it on every actual delivery — including
/// redirect hops, which re-resolve their target host — so a URL that only
/// *later* gets DNS-rebound to an internal target still can't be reached.
fn webhook_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_secs(5))
            .pool_idle_timeout(Duration::from_secs(90))
            .pool_max_idle_per_host(16)
            .dns_resolver(std::sync::Arc::new(crate::net_guard::SsrfSafeResolver))
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

    /// Unix timestamp of the start of the *current* unplanned reconnect
    /// window, set when `Event::Disconnected` fires and cleared on the next
    /// `Event::Connected` (or when the session goes away entirely via
    /// `Event::LoggedOut`). `None` means "not currently retrying" -- either
    /// genuinely connected, or never connected in the first place. The
    /// reconnect watchdog ([`crate::handlers::sessions::run_reconnect_watchdog`])
    /// reads this to decide when a session has been stuck retrying for too
    /// long and needs a forced full rebuild rather than waiting on
    /// whatsapp-rust's own backoff indefinitely.
    pub reconnecting_since: RwLock<Option<i64>>,

    /// Unix timestamp until which auto-reconnect is paused because the
    /// server locked the account (`AccountLocked`). `None` = not locked.
    /// In-memory only: after a restart the DB status is already
    /// `Disconnected`, so the startup reconnect skips the session anyway
    /// and the next lock event simply re-arms the cooldown. Cleared by a
    /// manual `POST /sessions/:id/connect`.
    pub lock_cooldown_until: RwLock<Option<i64>>,

    /// Consecutive `AccountLocked` events seen in this process lifetime.
    /// Indexes into [`AccountLockBackoffConfig::schedule`] so repeat
    /// locks cool down for progressively longer.
    pub lock_strikes: RwLock<u32>,

    /// Operator's auto-reconnect preference, stored gateway-side so
    /// `GET/PUT /sessions/:id/reconnect` work while the socket is down.
    /// Applied to each freshly built client in `connect_client`; a live
    /// client is also updated in place by the PUT handler. Defaults to
    /// `true`, matching the historical hardcoded behavior.
    pub auto_reconnect_pref: AtomicBool,

    /// Same idea for `GET/PUT /sessions/:id/history-sync`: the
    /// skip-history-sync flag, stored gateway-side and applied to every
    /// newly built client.
    pub skip_history_sync_pref: AtomicBool,

    /// Held while a client is installed so the client keeps emitting
    /// `Event::EncDecryptFailed` (upstream gates the event on at least one
    /// lease being held; dropping the last lease silences it). Acquired in
    /// `connect_client` alongside `set_client` and dropped with the client.
    pub enc_decrypt_failed_lease: RwLock<Option<whatsapp_rust::EncDecryptFailedLease>>,

    /// Session-wide message history materialized into the session's
    /// whatsapp.db by the upstream chat store. Backs
    /// `GET /sessions/{id}/messages`. Installed alongside the client;
    /// `None` when there is no live client (or the store failed to open,
    /// which is logged and non-fatal).
    pub chat_store: RwLock<Option<Arc<whatsapp_rust_chat_store::ChatStore>>>,

    /// Keeps the chat store's event handler subscribed; dropping it
    /// unsubscribes. Always mirrors the `chat_store` slot.
    pub chat_store_subscription: RwLock<Option<wacore::types::events::Subscription>>,

    /// Recently forwarded `message` event keys, so a message the server
    /// delivers twice -- once during the offline drain and again live,
    /// the case in issue #135 -- reaches webhooks/NATS/SSE only once. See
    /// [`RecentMessageIds`].
    pub recent_message_ids: parking_lot::Mutex<RecentMessageIds>,

    /// `message` events forwarded to webhooks/NATS/SSE, and when the last
    /// one was (unix seconds, 0 = never). Surfaced in the status
    /// diagnostics.
    pub messages_forwarded: std::sync::atomic::AtomicU64,
    pub last_message_forwarded_at: std::sync::atomic::AtomicI64,
}

/// Bounded set of the most recent message keys seen on one session,
/// oldest evicted first.
///
/// In memory only: after a process restart the set starts empty, so a
/// redelivery that straddles a restart can still produce a second event.
/// Consumers that need exactly-once processing should still key on
/// `data.message_id`.
pub struct RecentMessageIds {
    order: std::collections::VecDeque<String>,
    seen: std::collections::HashSet<String>,
    capacity: usize,
}

impl RecentMessageIds {
    pub const DEFAULT_CAPACITY: usize = 4096;

    pub fn new(capacity: usize) -> Self {
        Self {
            order: std::collections::VecDeque::with_capacity(capacity.min(1024)),
            seen: std::collections::HashSet::with_capacity(capacity.min(1024)),
            capacity: capacity.max(1),
        }
    }

    /// Records `key` and returns `true` the first time it is seen,
    /// `false` for a repeat still inside the window.
    pub fn first_sighting(&mut self, key: &str) -> bool {
        if self.seen.contains(key) {
            return false;
        }
        if self.order.len() >= self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.seen.remove(&oldest);
            }
        }
        self.order.push_back(key.to_string());
        self.seen.insert(key.to_string());
        true
    }
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
            reconnecting_since: RwLock::new(None),
            lock_cooldown_until: RwLock::new(None),
            lock_strikes: RwLock::new(0),
            auto_reconnect_pref: AtomicBool::new(true),
            skip_history_sync_pref: AtomicBool::new(false),
            enc_decrypt_failed_lease: RwLock::new(None),
            chat_store: RwLock::new(None),
            chat_store_subscription: RwLock::new(None),
            recent_message_ids: parking_lot::Mutex::new(RecentMessageIds::new(
                RecentMessageIds::DEFAULT_CAPACITY,
            )),
            messages_forwarded: std::sync::atomic::AtomicU64::new(0),
            last_message_forwarded_at: std::sync::atomic::AtomicI64::new(0),
        }
    }

    pub fn auto_reconnect_enabled(&self) -> bool {
        self.auto_reconnect_pref.load(Ordering::Relaxed)
    }

    pub fn set_auto_reconnect_pref(&self, enabled: bool) {
        self.auto_reconnect_pref.store(enabled, Ordering::Relaxed);
    }

    pub fn skip_history_sync(&self) -> bool {
        self.skip_history_sync_pref.load(Ordering::Relaxed)
    }

    pub fn set_skip_history_sync_pref(&self, skip: bool) {
        self.skip_history_sync_pref.store(skip, Ordering::Relaxed);
    }

    /// Record an `AccountLocked` event and return the cooldown just
    /// applied as `(cooldown_secs, cooldown_until)`. Each strike walks
    /// one step further down `schedule`, clamped at the last entry.
    pub fn record_lock_cooldown(&self, schedule: &[i64]) -> (i64, i64) {
        let mut strikes = self.lock_strikes.write();
        *strikes = strikes.saturating_add(1);
        let idx = (*strikes as usize)
            .saturating_sub(1)
            .min(schedule.len() - 1);
        let secs = schedule[idx];
        let until = chrono::Utc::now().timestamp() + secs;
        *self.lock_cooldown_until.write() = Some(until);
        (secs, until)
    }

    /// Manual intervention (`POST /sessions/:id/connect`): lift the
    /// lock cooldown and reset the escalation ladder.
    pub fn clear_lock_cooldown(&self) {
        *self.lock_cooldown_until.write() = None;
        *self.lock_strikes.write() = 0;
    }

    /// Seconds of lock cooldown remaining, or `None` when the session
    /// is not currently paused. The watchdog and other self-heal paths
    /// must skip the session while this is `Some`.
    pub fn lock_cooldown_remaining(&self) -> Option<i64> {
        let until = (*self.lock_cooldown_until.read())?;
        let remaining = until - chrono::Utc::now().timestamp();
        if remaining > 0 {
            Some(remaining)
        } else {
            None
        }
    }

    pub fn mark_reconnecting_now(&self) {
        let mut slot = self.reconnecting_since.write();
        if slot.is_none() {
            *slot = Some(chrono::Utc::now().timestamp());
        }
    }

    pub fn clear_reconnecting(&self) {
        *self.reconnecting_since.write() = None;
    }

    /// Seconds since the current unplanned reconnect window started, or
    /// `None` if not currently in one.
    pub fn reconnecting_for_secs(&self) -> Option<i64> {
        let since = (*self.reconnecting_since.read())?;
        Some((chrono::Utc::now().timestamp() - since).max(0))
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

    /// Raw socket liveness, deliberately separate from [`Self::is_alive`]:
    /// `true` as soon as the transport is connected, even before login
    /// completes (QR/pair flows) — and `false` during the "limbo" where a
    /// cached `LoggedIn` status outlives a dead socket. Surfaced as
    /// `socket_alive` on /status, /readyz, and /metrics.
    pub fn socket_alive(&self) -> bool {
        match self.client.read().as_ref() {
            Some(c) => c.is_connected(),
            None => false,
        }
    }

    pub fn set_client(&self, client: Option<Arc<Client>>) {
        if client.is_none() {
            *self.enc_decrypt_failed_lease.write() = None;
            *self.chat_store.write() = None;
            *self.chat_store_subscription.write() = None;
        }
        *self.client.write() = client;
    }

    /// Install the [`whatsapp_rust::EncDecryptFailedLease`] for the current
    /// client; call right after building a client, before [`Self::set_client`].
    pub fn set_enc_decrypt_failed_lease(&self, lease: whatsapp_rust::EncDecryptFailedLease) {
        *self.enc_decrypt_failed_lease.write() = Some(lease);
    }

    /// Install the chat store and the subscription keeping its event
    /// handler registered; call right after building a client, before
    /// [`Self::set_client`].
    pub fn set_chat_store(
        &self,
        store: Arc<whatsapp_rust_chat_store::ChatStore>,
        subscription: wacore::types::events::Subscription,
    ) {
        *self.chat_store.write() = Some(store);
        *self.chat_store_subscription.write() = Some(subscription);
    }

    pub fn get_chat_store(&self) -> Option<Arc<whatsapp_rust_chat_store::ChatStore>> {
        self.chat_store.read().clone()
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

    /// Retry/dead-letter policy for webhook delivery (env-configurable,
    /// see [`WebhookRetryConfig`]).
    pub webhook_retry: WebhookRetryConfig,

    /// Escalating cooldown schedule for server-side account locks
    /// (env-configurable, see [`AccountLockBackoffConfig`]).
    pub account_lock_backoff: AccountLockBackoffConfig,

    /// Per-session dead-letter queue for webhook deliveries that
    /// exhausted every retry attempt. Bounded ring per session
    /// (`webhook_retry.dlq_capacity`), in-memory only — lost on restart
    /// (the DB `webhook_dlq` table keeps a durable copy of each failure).
    pub webhook_dlq: DashMap<String, std::collections::VecDeque<WebhookDlqEntry>>,

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
                webhook_retry: WebhookRetryConfig::from_env(),
                account_lock_backoff: AccountLockBackoffConfig::from_env(),
                webhook_dlq: DashMap::new(),
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

    /// `(circuit_open, consecutive_failures)` for one webhook URL.
    pub fn webhook_circuit_snapshot(&self, url: &str) -> (bool, u32) {
        let now = std::time::Instant::now();
        match self.inner.webhook_circuits.get(url) {
            Some(c) => (c.opened_until.map(|u| now < u).unwrap_or(false), c.failures),
            None => (false, 0),
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
        self.inner.webhook_dlq.remove(session_id);
    }

    /// Push a permanently-failed delivery onto the session's in-memory
    /// DLQ ring, evicting the oldest entries past the configured
    /// capacity.
    pub fn webhook_dlq_push(&self, entry: WebhookDlqEntry) {
        let capacity = self.inner.webhook_retry.dlq_capacity;
        let mut ring = self
            .inner
            .webhook_dlq
            .entry(entry.session_id.clone())
            .or_default();
        while ring.len() >= capacity {
            ring.pop_front();
        }
        ring.push_back(entry);
    }

    /// List the session's DLQ entries, newest first.
    pub fn webhook_dlq_list(&self, session_id: &str) -> Vec<WebhookDlqEntry> {
        self.inner
            .webhook_dlq
            .get(session_id)
            .map(|r| r.value().iter().rev().cloned().collect())
            .unwrap_or_default()
    }

    /// Remove and return one DLQ entry. Used by the replay endpoint —
    /// the re-delivery goes through the same delivery path and re-enters
    /// the queue on its own if it fails again.
    pub fn webhook_dlq_take(&self, session_id: &str, entry_id: &str) -> Option<WebhookDlqEntry> {
        let mut ring = self.inner.webhook_dlq.get_mut(session_id)?;
        let pos = ring.iter().position(|e| e.id == entry_id)?;
        ring.remove(pos)
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

    pub fn account_lock_backoff(&self) -> &AccountLockBackoffConfig {
        &self.inner.account_lock_backoff
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

    /// Same as [`Self::session_iter`], but keeping each runtime's session
    /// id alongside it -- needed by anything that has to act back on a
    /// specific session (e.g. the reconnect watchdog calling
    /// [`crate::handlers::sessions::connect_client`]), since
    /// [`SessionState`] doesn't carry its own id.
    pub fn session_iter_with_ids(&self) -> Vec<(String, Arc<SessionState>)> {
        self.inner
            .sessions
            .iter()
            .map(|r| (r.key().clone(), r.value().clone()))
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

    /// Delivers `payload` to every enabled, matching webhook for
    /// `session_id`, HMAC-signing it when the webhook has a `secret`.
    ///
    /// The signature is computed over `{timestamp}.{payload}`, with the
    /// timestamp also sent as `X-Webhook-Timestamp`, not over the payload
    /// alone: a captured `(url, signature, body)` tuple would otherwise
    /// replay forever with a still-valid signature. Binding the signature
    /// to a timestamp that ships alongside it lets a receiver reject
    /// anything outside a short window -- see webhooks.md for the
    /// verification recipe receivers are expected to implement. Every
    /// delivery also carries `X-Webhook-Signature-Version: v2`
    /// ([`WEBHOOK_SIGNATURE_VERSION`]) so consumers can detect the
    /// timestamp-prefixed scheme introduced in v0.9.8.
    ///
    /// Each webhook's delivery runs in its own spawned task
    /// ([`Self::deliver_webhook`]) with configurable retries and a
    /// dead-letter queue, so a slow or dead target never blocks the
    /// event pipeline.
    pub async fn broadcast_to_webhooks(&self, session_id: &str, event: &str, payload: &str) {
        self.push_event(session_id, event, payload);

        let webhooks = self.get_webhooks(session_id);

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

            let state_for_task = self.clone();
            let session_id_owned = session_id.to_string();
            let event_owned = event.to_string();
            let payload_owned = payload.to_string();

            tokio::spawn(async move {
                state_for_task
                    .deliver_webhook(
                        &session_id_owned,
                        &event_owned,
                        &config.url,
                        config.secret,
                        &payload_owned,
                    )
                    .await;
            });
        }
    }

    /// One full delivery run of `payload` against `url`: up to
    /// `WEBHOOK_RETRY_MAX_ATTEMPTS` tries with exponential backoff
    /// (immediate, +5 s, +30 s, ...), HMAC-signing each attempt when
    /// `secret` is set. Retries cover 5xx and transport errors, plus
    /// 4xx when `WEBHOOK_RETRY_ON_4XX` is on (the default — a 401/403
    /// window is usually a fixable consumer misconfig, and dropping
    /// those events loses messages).
    ///
    /// When every attempt fails the failure is recorded against the
    /// per-URL circuit breaker and the payload is dead-lettered — onto
    /// the session's bounded in-memory DLQ (replayable via
    /// `POST .../webhooks/dlq/{entry_id}/replay`) and into the DB
    /// `webhook_dlq` table. Returns `true` when a delivery succeeded.
    ///
    /// Used both by [`Self::broadcast_to_webhooks`] (fresh events) and
    /// by the DLQ replay endpoint (stored payloads), so both paths
    /// share the exact same signing and retry behavior.
    pub async fn deliver_webhook(
        &self,
        session_id: &str,
        event: &str,
        url: &str,
        secret: Option<String>,
        payload: &str,
    ) -> bool {
        let retry_cfg = self.inner.webhook_retry.clone();
        let max_attempts = retry_cfg.max_attempts.max(1);
        let client = webhook_client();
        let pool = self.session_manager().pool().clone();
        let mut last_err: Option<String> = None;

        for attempt in 0..max_attempts {
            let delay = webhook_retry_delay(attempt);
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }

            let timestamp = chrono::Utc::now().timestamp();
            let mut req = client
                .post(url)
                .header("Content-Type", "application/json")
                .header("X-Webhook-Timestamp", timestamp.to_string())
                .header("X-Webhook-Signature-Version", WEBHOOK_SIGNATURE_VERSION)
                .body(payload.to_string());

            if let Some(secret) = &secret {
                use hmac::{Hmac, Mac};
                use sha2::Sha256;

                type HmacSha256 = Hmac<Sha256>;
                if let Ok(mut mac) = HmacSha256::new_from_slice(secret.as_bytes()) {
                    mac.update(format!("{timestamp}.{payload}").as_bytes());
                    let signature = hex::encode(mac.finalize().into_bytes());
                    req = req.header("X-Webhook-Signature", format!("sha256={}", signature));
                }
            }

            match req.send().await {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        self.webhook_record_success(url);
                        return true;
                    }
                    if !retry_cfg.retry_on_4xx
                        && status.is_client_error()
                        && status.as_u16() != 408
                        && status.as_u16() != 429
                    {
                        tracing::warn!(
                            "Webhook {} rejected with {} — not retrying (WEBHOOK_RETRY_ON_4XX=false), dead-lettering",
                            url,
                            status
                        );
                        last_err = Some(format!("HTTP {}", status));
                        break;
                    }
                    last_err = Some(format!("HTTP {}", status));
                }
                Err(e) => {
                    last_err = Some(e.to_string());
                }
            }
        }

        let err = match last_err {
            Some(err) => err,
            None => return false,
        };

        let action = self.webhook_record_failure(url);
        match action {
            WebhookFailureAction::HardDisable => {
                tracing::warn!(
                    "Webhook {} auto-DISABLED after 100 consecutive failures — DB row switched to enabled=false",
                    url
                );
                let reason = format!("100 consecutive failures ({err})");
                match self
                    .session_manager()
                    .disable_webhook_by_url(url, &reason)
                    .await
                {
                    Ok(n) => tracing::info!(
                        "webhook auto-disable: {} row(s) marked enabled=false for {}",
                        n,
                        url
                    ),
                    Err(err) => {
                        tracing::warn!("webhook auto-disable persist failed for {}: {}", url, err)
                    }
                }
                self.purge_webhook_by_url(url);
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
                    max_attempts,
                    err
                );
            }
        }

        self.webhook_dlq_push(WebhookDlqEntry {
            id: uuid::Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            webhook_url: url.to_string(),
            event: event.to_string(),
            payload: payload.to_string(),
            secret,
            last_error: err.clone(),
            attempts: max_attempts,
            failed_at: chrono::Utc::now().timestamp(),
        });

        crate::db::webhook_dlq::record_failure(
            &pool,
            session_id,
            url,
            event,
            payload,
            &err,
            max_attempts as i32,
        )
        .await;

        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconnecting_since_starts_unset() {
        let s = SessionState::new("/tmp/x".to_string());
        assert_eq!(s.reconnecting_for_secs(), None);
    }

    #[test]
    fn mark_reconnecting_is_idempotent() {
        let s = SessionState::new("/tmp/x".to_string());
        s.mark_reconnecting_now();
        let first = *s.reconnecting_since.read();
        assert!(first.is_some());

        s.mark_reconnecting_now();
        let second = *s.reconnecting_since.read();
        assert_eq!(
            first, second,
            "a second mark while already reconnecting must not reset the window start"
        );
        assert!(s.reconnecting_for_secs().is_some());
    }

    #[test]
    fn clear_reconnecting_resets_to_none() {
        let s = SessionState::new("/tmp/x".to_string());
        s.mark_reconnecting_now();
        assert!(s.reconnecting_for_secs().is_some());

        s.clear_reconnecting();
        assert_eq!(s.reconnecting_for_secs(), None);
    }

    #[test]
    fn a_redelivered_message_is_only_forwarded_once() {
        let s = SessionState::new("/tmp/x".to_string());
        let key = "6281@s.whatsapp.net|3EB0ABC";
        assert!(s.recent_message_ids.lock().first_sighting(key));
        assert!(!s.recent_message_ids.lock().first_sighting(key));
        assert!(s
            .recent_message_ids
            .lock()
            .first_sighting("6282@s.whatsapp.net|3EB0ABC"));
    }

    #[test]
    fn recent_message_ids_evict_oldest_first_at_capacity() {
        let mut ids = RecentMessageIds::new(2);
        assert!(ids.first_sighting("a"));
        assert!(ids.first_sighting("b"));
        assert!(!ids.first_sighting("a"));
        assert!(ids.first_sighting("c"));
        assert!(ids.first_sighting("a"), "a was evicted when c arrived");
        assert!(!ids.first_sighting("c"));
        assert_eq!(ids.order.len(), 2);
        assert_eq!(ids.seen.len(), 2);
    }
}
