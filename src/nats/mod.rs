//! NATS JetStream integration.
//!
//! Two subject trees are used:
//!
//! - `wa.events.{session_id}.{event_type}` — every event fired by the
//!   upstream client is published here for external consumers (dashboards,
//!   analytics, downstream apps).
//! - `wa.send.{session_id}` — inbound queue for send requests posted by
//!   external producers; the [`consumer`] drains it and forwards the
//!   payload through the normal handler path.
//!
//! NATS is optional: the gateway boots with `NATS: Disabled` when the
//! connection URL is unset or unreachable; publishing then becomes a no-op.

pub mod config;
pub mod consumer;
pub mod models;
pub mod publisher;

use std::time::Duration;

use async_nats::jetstream::{self, stream};

use self::config::NatsConfig;

/// Manages the NATS connection and JetStream context.
pub struct NatsManager {
    #[allow(dead_code)]
    client: async_nats::Client,
    jetstream: jetstream::Context,
    config: NatsConfig,
}

impl NatsManager {
    /// Connect to NATS server. Returns error if connection fails.
    pub async fn connect(config: NatsConfig) -> anyhow::Result<Self> {
        let mut connect_options = async_nats::ConnectOptions::new();

        if let Some(token) = &config.token {
            connect_options = connect_options.token(token.clone());
        }
        if let Some(creds) = &config.creds_file {
            connect_options = connect_options.credentials_file(creds).await?;
        }

        let client = connect_options.connect(&config.url).await?;
        let jetstream = jetstream::new(client.clone());

        Ok(Self {
            client,
            jetstream,
            config,
        })
    }

    /// Create or update JetStream streams for events and outbound messages.
    pub async fn init_streams(&self) -> anyhow::Result<()> {
        // WA_EVENTS stream — incoming WhatsApp events
        self.jetstream
            .get_or_create_stream(stream::Config {
                name: self.config.events_stream.clone(),
                subjects: vec!["wa.events.>".into()],
                retention: stream::RetentionPolicy::Limits,
                max_age: Duration::from_secs(86400 * self.config.events_max_age_days),
                max_bytes: 1_073_741_824,     // 1 GB
                max_message_size: 10_485_760, // 10 MB
                storage: stream::StorageType::File,
                num_replicas: 1,
                discard: stream::DiscardPolicy::Old,
                duplicate_window: Duration::from_secs(120),
                ..Default::default()
            })
            .await?;

        tracing::info!(
            "JetStream stream '{}' ready (subjects: wa.events.>)",
            self.config.events_stream
        );

        // WA_SEND stream — outbound message commands
        self.jetstream
            .get_or_create_stream(stream::Config {
                name: self.config.send_stream.clone(),
                subjects: vec!["wa.send.>".into()],
                retention: stream::RetentionPolicy::WorkQueue,
                max_age: Duration::from_secs(86400 * self.config.send_max_age_days),
                max_bytes: 536_870_912,       // 512 MB
                max_message_size: 10_485_760, // 10 MB
                storage: stream::StorageType::File,
                num_replicas: 1,
                discard: stream::DiscardPolicy::Old,
                ..Default::default()
            })
            .await?;

        tracing::info!(
            "JetStream stream '{}' ready (subjects: wa.send.>)",
            self.config.send_stream
        );

        Ok(())
    }

    pub fn jetstream(&self) -> &jetstream::Context {
        &self.jetstream
    }

    pub fn config(&self) -> &NatsConfig {
        &self.config
    }
}
