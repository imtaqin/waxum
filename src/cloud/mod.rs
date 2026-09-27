//! WhatsApp Cloud API integration (Phase 1).
//!
//! Meta's official Business API is a fundamentally different transport
//! from the unofficial multi-device protocol `whatsapp-rust` implements:
//! plain REST calls against `graph.facebook.com` instead of a persistent
//! noise-protocol socket, and a webhook contract with its own GET
//! verification handshake and `X-Hub-Signature-256` signing instead of
//! waxum's own `X-Webhook-Signature`. This module is kept deliberately
//! separate from the whatsapp-rust integration rather than intermixed
//! with it.
//!
//! - [`client`] — outbound Graph API calls (`CloudClient`).
//! - [`webhook`] — inbound webhook verification handshake, signature
//!   check, and payload normalization into the same event shape
//!   `handlers::sessions::message_event_data` produces for Web sessions.
//! - [`embedded_signup`] — the Embedded Signup OAuth code exchange plus
//!   the WABA `subscribed_apps`/`phone_numbers` onboarding calls.
//! - [`flows_crypto`] — the Flows Data Exchange RSA/AES-GCM handshake.

pub mod client;
pub mod embedded_signup;
pub mod flows_crypto;
pub mod graph;
pub mod webhook;
