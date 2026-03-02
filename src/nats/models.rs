use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::models::messages::{
    ButtonItem, ContactCard, ListSection, MediaData, NativeFlowButtonItem,
};

/// Outbound message command received from NATS queue.
/// Published to `wa.send.{session_id}`.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(dead_code)]
pub enum OutboundCommand {
    Text {
        to: String,
        text: String,
        reply_to: Option<String>,
        request_id: Option<String>,
    },
    Image {
        to: String,
        image: MediaData,
        caption: Option<String>,
        reply_to: Option<String>,
        request_id: Option<String>,
    },
    Video {
        to: String,
        video: MediaData,
        caption: Option<String>,
        reply_to: Option<String>,
        request_id: Option<String>,
    },
    Audio {
        to: String,
        audio: MediaData,
        #[serde(default)]
        ptt: bool,
        reply_to: Option<String>,
        request_id: Option<String>,
    },
    Document {
        to: String,
        document: MediaData,
        filename: String,
        caption: Option<String>,
        reply_to: Option<String>,
        request_id: Option<String>,
    },
    Sticker {
        to: String,
        sticker: MediaData,
        reply_to: Option<String>,
        request_id: Option<String>,
    },
    Location {
        to: String,
        latitude: f64,
        longitude: f64,
        name: Option<String>,
        address: Option<String>,
        reply_to: Option<String>,
        request_id: Option<String>,
    },
    Contact {
        to: String,
        contact: ContactCard,
        reply_to: Option<String>,
        request_id: Option<String>,
    },
    Reaction {
        to: String,
        message_id: String,
        emoji: String,
        request_id: Option<String>,
    },
    Poll {
        to: String,
        name: String,
        options: Vec<String>,
        #[serde(default)]
        selectable_count: u32,
        reply_to: Option<String>,
        request_id: Option<String>,
    },
    Buttons {
        to: String,
        content_text: String,
        footer: Option<String>,
        buttons: Vec<ButtonItem>,
        header_text: Option<String>,
        reply_to: Option<String>,
        request_id: Option<String>,
    },
    List {
        to: String,
        title: String,
        description: String,
        button_text: String,
        sections: Vec<ListSection>,
        footer: Option<String>,
        reply_to: Option<String>,
        request_id: Option<String>,
    },
    Interactive {
        to: String,
        body_text: String,
        footer_text: Option<String>,
        buttons: Vec<NativeFlowButtonItem>,
        reply_to: Option<String>,
        request_id: Option<String>,
    },
    Revoke {
        to: String,
        message_id: String,
        original_sender: Option<String>,
        request_id: Option<String>,
    },
    Edit {
        to: String,
        message_id: String,
        text: String,
        request_id: Option<String>,
    },
    Read {
        chat_jid: String,
        sender: Option<String>,
        message_ids: Vec<String>,
        request_id: Option<String>,
    },
}

impl OutboundCommand {
    /// Extract the request_id from any variant
    pub fn request_id(&self) -> Option<String> {
        match self {
            Self::Text { request_id, .. }
            | Self::Image { request_id, .. }
            | Self::Video { request_id, .. }
            | Self::Audio { request_id, .. }
            | Self::Document { request_id, .. }
            | Self::Sticker { request_id, .. }
            | Self::Location { request_id, .. }
            | Self::Contact { request_id, .. }
            | Self::Reaction { request_id, .. }
            | Self::Poll { request_id, .. }
            | Self::Buttons { request_id, .. }
            | Self::List { request_id, .. }
            | Self::Interactive { request_id, .. }
            | Self::Revoke { request_id, .. }
            | Self::Edit { request_id, .. }
            | Self::Read { request_id, .. } => request_id.clone(),
        }
    }
}

/// Result of processing an outbound command.
/// Published to `wa.events.{session_id}.send_result`.
#[derive(Debug, Serialize, ToSchema)]
pub struct SendResult {
    pub request_id: Option<String>,
    pub success: bool,
    pub message_id: Option<String>,
    pub error: Option<String>,
    pub timestamp: i64,
}

/// NATS status response for the REST endpoint.
#[derive(Debug, Serialize, ToSchema)]
pub struct NatsStatusResponse {
    pub enabled: bool,
    pub connected: bool,
    pub url: Option<String>,
    pub events_stream: Option<NatsStreamInfo>,
    pub send_stream: Option<NatsStreamInfo>,
}

/// Stream information for NATS status endpoint.
#[derive(Debug, Serialize, ToSchema)]
pub struct NatsStreamInfo {
    pub name: String,
    pub messages: u64,
    pub bytes: u64,
    pub consumer_count: usize,
    pub first_seq: u64,
    pub last_seq: u64,
}
