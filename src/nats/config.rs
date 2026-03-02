/// NATS configuration loaded from environment variables.
/// Returns `None` if `NATS_URL` is not set (NATS disabled).
pub struct NatsConfig {
    pub url: String,
    pub events_stream: String,
    pub send_stream: String,
    pub events_max_age_days: u64,
    pub send_max_age_days: u64,
    pub creds_file: Option<String>,
    pub token: Option<String>,
}

impl NatsConfig {
    pub fn from_env() -> Option<Self> {
        let url = std::env::var("NATS_URL").ok()?;
        Some(Self {
            url,
            events_stream: std::env::var("NATS_EVENTS_STREAM")
                .unwrap_or_else(|_| "WA_EVENTS".into()),
            send_stream: std::env::var("NATS_SEND_STREAM").unwrap_or_else(|_| "WA_SEND".into()),
            events_max_age_days: std::env::var("NATS_EVENTS_MAX_AGE_DAYS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(7),
            send_max_age_days: std::env::var("NATS_SEND_MAX_AGE_DAYS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(1),
            creds_file: std::env::var("NATS_CREDS_FILE").ok(),
            token: std::env::var("NATS_TOKEN").ok(),
        })
    }
}
