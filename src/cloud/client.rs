//! Outbound WhatsApp Cloud API (Meta Graph API) calls.
//!
//! Every call here is a plain authenticated REST request against
//! `graph.facebook.com/{version}/{phone_number_id}/...` -- there is no
//! persistent connection to hold open, unlike the whatsapp-rust
//! `Client`. [`CloudClient::new`] takes the phone number ID + access
//! token pulled from [`crate::db::session::CloudCredentials`] by the
//! caller; this type carries no session-lookup logic of its own.

use serde_json::{json, Value};

const DEFAULT_API_VERSION: &str = "v21.0";

fn api_version() -> String {
    std::env::var("WHATSAPP_CLOUD_API_VERSION").unwrap_or_else(|_| DEFAULT_API_VERSION.to_string())
}

/// Error returned by a Cloud API call: either the HTTP transport failed,
/// or Meta answered with a non-2xx status and an error body.
#[derive(Debug, thiserror::Error)]
pub enum CloudError {
    #[error("cloud API request failed: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("cloud API returned {status}: {body}")]
    Api { status: u16, body: String },
}

pub struct CloudClient {
    http: reqwest::Client,
    base_url: String,
    phone_number_id: String,
    access_token: String,
}

impl CloudClient {
    pub fn new(phone_number_id: &str, access_token: &str) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: format!("https://graph.facebook.com/{}", api_version()),
            phone_number_id: phone_number_id.to_string(),
            access_token: access_token.to_string(),
        }
    }

    fn messages_url(&self) -> String {
        format!("{}/{}/messages", self.base_url, self.phone_number_id)
    }

    async fn post_messages(&self, body: Value) -> Result<Value, CloudError> {
        let resp = self
            .http
            .post(self.messages_url())
            .bearer_auth(&self.access_token)
            .json(&body)
            .send()
            .await?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(CloudError::Api {
                status: status.as_u16(),
                body,
            });
        }
        serde_json::from_str(&body).map_err(|e| CloudError::Api {
            status: status.as_u16(),
            body: format!("failed to parse response: {e}: {body}"),
        })
    }

    /// `POST {phone_number_id}/messages` with `type: text`. `reply_to` sets
    /// `context.message_id`, mirroring the "Send Reply to Text Message"
    /// request in the Cloud API collection.
    pub async fn send_text(
        &self,
        to: &str,
        text: &str,
        preview_url: bool,
        reply_to: Option<&str>,
    ) -> Result<Value, CloudError> {
        let mut body = json!({
            "messaging_product": "whatsapp",
            "recipient_type": "individual",
            "to": to,
            "type": "text",
            "text": {
                "preview_url": preview_url,
                "body": text,
            },
        });
        if let Some(stanza_id) = reply_to {
            body["context"] = json!({ "message_id": stanza_id });
        }
        self.post_messages(body).await
    }

    /// `PUT`-equivalent read receipt: `POST {phone_number_id}/messages`
    /// with `status: read`, per the "Mark Message As Read" request.
    pub async fn mark_read(&self, message_id: &str) -> Result<Value, CloudError> {
        self.post_messages(json!({
            "messaging_product": "whatsapp",
            "status": "read",
            "message_id": message_id,
        }))
        .await
    }

    /// Sends an arbitrary already-shaped Cloud API message body (any
    /// `type`: image/video/audio/document/sticker/location/contacts/
    /// reaction/interactive/template/...). Callers build the
    /// type-specific payload; this just posts it.
    pub async fn send_raw(&self, body: Value) -> Result<Value, CloudError> {
        self.post_messages(body).await
    }

    /// `GET {media_id}?phone_number_id=...` -- resolves a media ID to its
    /// short-lived download URL.
    pub async fn get_media_url(&self, media_id: &str) -> Result<Value, CloudError> {
        let url = format!(
            "{}/{}?phone_number_id={}",
            self.base_url, media_id, self.phone_number_id
        );
        let resp = self
            .http
            .get(url)
            .bearer_auth(&self.access_token)
            .send()
            .await?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(CloudError::Api {
                status: status.as_u16(),
                body,
            });
        }
        serde_json::from_str(&body).map_err(|e| CloudError::Api {
            status: status.as_u16(),
            body: format!("failed to parse response: {e}: {body}"),
        })
    }

    /// `GET {phone_number_id}/whatsapp_business_profile`
    pub async fn get_business_profile(&self) -> Result<Value, CloudError> {
        let url = format!(
            "{}/{}/whatsapp_business_profile",
            self.base_url, self.phone_number_id
        );
        let resp = self
            .http
            .get(url)
            .bearer_auth(&self.access_token)
            .send()
            .await?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(CloudError::Api {
                status: status.as_u16(),
                body,
            });
        }
        serde_json::from_str(&body).map_err(|e| CloudError::Api {
            status: status.as_u16(),
            body: format!("failed to parse response: {e}: {body}"),
        })
    }
}
