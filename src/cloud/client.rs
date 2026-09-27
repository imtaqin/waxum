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

/// Arguments for [`CloudClient::send_interactive_list`], grouped into a
/// struct to stay under clippy's argument-count lint (the same pattern
/// `ConverseRequest` uses elsewhere in this workspace for the same
/// reason).
pub struct SendInteractiveListRequest<'a> {
    pub to: &'a str,
    pub header_text: Option<&'a str>,
    pub body_text: &'a str,
    pub footer_text: Option<&'a str>,
    pub button_text: &'a str,
    pub sections: Value,
    pub reply_to: Option<&'a str>,
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
        self.get_json(&format!(
            "{}/{}?phone_number_id={}",
            self.base_url, media_id, self.phone_number_id
        ))
        .await
    }

    /// `GET {phone_number_id}/whatsapp_business_profile`
    pub async fn get_business_profile(&self) -> Result<Value, CloudError> {
        self.get_json(&format!(
            "{}/{}/whatsapp_business_profile",
            self.base_url, self.phone_number_id
        ))
        .await
    }

    async fn get_json(&self, url: &str) -> Result<Value, CloudError> {
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

    fn attach_reply(mut body: Value, reply_to: Option<&str>) -> Value {
        if let Some(stanza_id) = reply_to {
            body["context"] = json!({ "message_id": stanza_id });
        }
        body
    }

    /// Sends a media message (`image`/`video`/`audio`/`document`/`sticker`)
    /// by either `{"link": url}` or `{"id": media_id}`, per the "Send Image
    /// Message by ID/URL" family of requests. `caption`/`filename` are
    /// merged into the media object where the type supports them --
    /// stickers and audio accept neither, matching the collection.
    pub async fn send_media(
        &self,
        to: &str,
        kind: &str,
        mut media: Value,
        caption: Option<&str>,
        filename: Option<&str>,
        reply_to: Option<&str>,
    ) -> Result<Value, CloudError> {
        if let Some(c) = caption {
            media["caption"] = json!(c);
        }
        if let Some(f) = filename {
            media["filename"] = json!(f);
        }
        let body = Self::attach_reply(
            json!({
                "messaging_product": "whatsapp",
                "recipient_type": "individual",
                "to": to,
                "type": kind,
                kind: media,
            }),
            reply_to,
        );
        self.post_messages(body).await
    }

    /// `POST {phone_number_id}/messages` with `type: location`.
    pub async fn send_location(
        &self,
        to: &str,
        latitude: f64,
        longitude: f64,
        name: Option<&str>,
        address: Option<&str>,
        reply_to: Option<&str>,
    ) -> Result<Value, CloudError> {
        let body = Self::attach_reply(
            json!({
                "messaging_product": "whatsapp",
                "recipient_type": "individual",
                "to": to,
                "type": "location",
                "location": {
                    "latitude": latitude,
                    "longitude": longitude,
                    "name": name,
                    "address": address,
                },
            }),
            reply_to,
        );
        self.post_messages(body).await
    }

    /// `POST {phone_number_id}/messages` with `type: contacts`, per the
    /// "Send Contact Message" request. Takes an already-shaped `contacts`
    /// array so callers control the full contact-card schema (addresses,
    /// emails, org, phones, urls) without this client duplicating it.
    pub async fn send_contacts(&self, to: &str, contacts: Value) -> Result<Value, CloudError> {
        self.post_messages(json!({
            "messaging_product": "whatsapp",
            "to": to,
            "type": "contacts",
            "contacts": contacts,
        }))
        .await
    }

    /// `POST {phone_number_id}/messages` with `type: reaction`.
    pub async fn send_reaction(
        &self,
        to: &str,
        message_id: &str,
        emoji: &str,
    ) -> Result<Value, CloudError> {
        self.post_messages(json!({
            "messaging_product": "whatsapp",
            "recipient_type": "individual",
            "to": to,
            "type": "reaction",
            "reaction": {
                "message_id": message_id,
                "emoji": emoji,
            },
        }))
        .await
    }

    /// `POST {phone_number_id}/messages` with `type: interactive`,
    /// `interactive.type: button`, per "Send Reply Button".
    pub async fn send_interactive_buttons(
        &self,
        to: &str,
        body_text: &str,
        buttons: Value,
        reply_to: Option<&str>,
    ) -> Result<Value, CloudError> {
        let body = Self::attach_reply(
            json!({
                "messaging_product": "whatsapp",
                "recipient_type": "individual",
                "to": to,
                "type": "interactive",
                "interactive": {
                    "type": "button",
                    "body": { "text": body_text },
                    "action": { "buttons": buttons },
                },
            }),
            reply_to,
        );
        self.post_messages(body).await
    }

    /// `POST {phone_number_id}/messages` with `type: interactive`,
    /// `interactive.type: list`, per "Send List Message".
    pub async fn send_interactive_list(
        &self,
        request: SendInteractiveListRequest<'_>,
    ) -> Result<Value, CloudError> {
        let mut interactive = json!({
            "type": "list",
            "body": { "text": request.body_text },
            "action": { "button": request.button_text, "sections": request.sections },
        });
        if let Some(h) = request.header_text {
            interactive["header"] = json!({ "type": "text", "text": h });
        }
        if let Some(f) = request.footer_text {
            interactive["footer"] = json!({ "text": f });
        }
        let body = Self::attach_reply(
            json!({
                "messaging_product": "whatsapp",
                "recipient_type": "individual",
                "to": request.to,
                "type": "interactive",
                "interactive": interactive,
            }),
            request.reply_to,
        );
        self.post_messages(body).await
    }

    /// `POST {phone_number_id}/messages` with `type: template`, per the
    /// "Send Message Template Text/Media/Interactive" family. `components`
    /// is the already-shaped Cloud API template-component array (body
    /// parameters, header media, button quick-reply payloads, ...); this
    /// client does not attempt to model every parameter/component variant
    /// as Rust types given how open-ended the template schema is.
    pub async fn send_template(
        &self,
        to: &str,
        name: &str,
        language_code: &str,
        components: Value,
    ) -> Result<Value, CloudError> {
        self.post_messages(json!({
            "messaging_product": "whatsapp",
            "recipient_type": "individual",
            "to": to,
            "type": "template",
            "template": {
                "name": name,
                "language": { "code": language_code },
                "components": components,
            },
        }))
        .await
    }

    /// `POST {phone_number_id}/media` (multipart) -- uploads a media file
    /// and returns its Cloud API media ID, per "Upload Image"/"Upload
    /// Sticker"/"Upload Audio".
    pub async fn upload_media(
        &self,
        bytes: Vec<u8>,
        mime_type: &str,
        filename: &str,
    ) -> Result<Value, CloudError> {
        let url = format!("{}/{}/media", self.base_url, self.phone_number_id);
        let part = reqwest::multipart::Part::bytes(bytes)
            .file_name(filename.to_string())
            .mime_str(mime_type)
            .map_err(|e| CloudError::Api {
                status: 0,
                body: format!("invalid mime type {mime_type}: {e}"),
            })?;
        let form = reqwest::multipart::Form::new()
            .text("messaging_product", "whatsapp")
            .part("file", part);
        let resp = self
            .http
            .post(url)
            .bearer_auth(&self.access_token)
            .multipart(form)
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

    /// `GET` the short-lived signed URL [`Self::get_media_url`] returns,
    /// with the same bearer token, per "Download Media" -- Meta's media
    /// URLs require the same app access token as every other Graph call,
    /// unlike a plain public link.
    pub async fn download_media_bytes(&self, media_url: &str) -> Result<Vec<u8>, CloudError> {
        let resp = self
            .http
            .get(media_url)
            .bearer_auth(&self.access_token)
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(CloudError::Api {
                status: status.as_u16(),
                body,
            });
        }
        Ok(resp.bytes().await?.to_vec())
    }

    /// `POST {waba_id}/flows` (multipart) -- creates a Flow in draft
    /// status, per "Create Flow". `categories` is the JSON-encoded array
    /// string Meta expects in the form field (e.g. `["OTHER"]`).
    pub async fn create_flow(
        &self,
        waba_id: &str,
        name: &str,
        categories: &[String],
        clone_flow_id: Option<&str>,
    ) -> Result<Value, CloudError> {
        let categories_json =
            serde_json::to_string(categories).unwrap_or_else(|_| "[]".to_string());
        let mut form = reqwest::multipart::Form::new()
            .text("name", name.to_string())
            .text("categories", categories_json);
        if let Some(id) = clone_flow_id {
            form = form.text("clone_flow_id", id.to_string());
        }
        self.post_multipart(&format!("{}/{}/flows", self.base_url, waba_id), form)
            .await
    }

    /// `GET {waba_id}/flows`, per "List Flows".
    pub async fn list_flows(&self, waba_id: &str) -> Result<Value, CloudError> {
        self.get_json(&format!("{}/{}/flows", self.base_url, waba_id))
            .await
    }

    /// `GET {flow_id}?fields=...`, per "Get Flow".
    pub async fn get_flow(&self, flow_id: &str) -> Result<Value, CloudError> {
        self.get_json(&format!(
            "{}/{}?fields=id,name,categories,preview,status,validation_errors,json_version,data_api_version,data_channel_uri,health_status,whatsapp_business_account,application",
            self.base_url, flow_id
        ))
        .await
    }

    /// `POST {flow_id}` (multipart) -- renames/re-categorizes a Flow or
    /// sets its `endpoint_uri`, per "Update Flow Metadata".
    pub async fn update_flow_metadata(
        &self,
        flow_id: &str,
        name: Option<&str>,
        categories: Option<&[String]>,
        endpoint_uri: Option<&str>,
    ) -> Result<Value, CloudError> {
        let mut form = reqwest::multipart::Form::new();
        if let Some(name) = name {
            form = form.text("name", name.to_string());
        }
        if let Some(categories) = categories {
            let categories_json =
                serde_json::to_string(categories).unwrap_or_else(|_| "[]".to_string());
            form = form.text("categories", categories_json);
        }
        if let Some(uri) = endpoint_uri {
            form = form.text("endpoint_uri", uri.to_string());
        }
        self.post_multipart(&format!("{}/{}", self.base_url, flow_id), form)
            .await
    }

    /// `POST {flow_id}/assets` (multipart file upload, `asset_type:
    /// FLOW_JSON`), per "Update Flow JSON".
    pub async fn update_flow_json(
        &self,
        flow_id: &str,
        flow_json_bytes: Vec<u8>,
    ) -> Result<Value, CloudError> {
        let part = reqwest::multipart::Part::bytes(flow_json_bytes)
            .file_name("flow.json".to_string())
            .mime_str("application/json")
            .map_err(|e| CloudError::Api {
                status: 0,
                body: format!("invalid flow json part: {e}"),
            })?;
        let form = reqwest::multipart::Form::new()
            .text("name", "flow.json".to_string())
            .text("asset_type", "FLOW_JSON".to_string())
            .part("file", part);
        self.post_multipart(&format!("{}/{}/assets", self.base_url, flow_id), form)
            .await
    }

    /// `GET {flow_id}/assets`, per "List Assets (Get Flow JSON URL)".
    pub async fn get_flow_assets(&self, flow_id: &str) -> Result<Value, CloudError> {
        self.get_json(&format!("{}/{}/assets", self.base_url, flow_id))
            .await
    }

    /// `POST {flow_id}/publish`, per "Publish Flow".
    pub async fn publish_flow(&self, flow_id: &str) -> Result<Value, CloudError> {
        self.post_json(&format!("{}/{}/publish", self.base_url, flow_id), json!({}))
            .await
    }

    /// `POST {flow_id}/deprecate`, per "Deprecate Flow".
    pub async fn deprecate_flow(&self, flow_id: &str) -> Result<Value, CloudError> {
        self.post_json(
            &format!("{}/{}/deprecate", self.base_url, flow_id),
            json!({}),
        )
        .await
    }

    /// `DELETE {flow_id}`, per "Delete Flow". Only draft (never
    /// published) Flows can actually be deleted -- Meta itself enforces
    /// that, this client just forwards the call.
    pub async fn delete_flow(&self, flow_id: &str) -> Result<Value, CloudError> {
        self.delete_json(&format!("{}/{}", self.base_url, flow_id))
            .await
    }

    /// `POST {phone_number_id}/whatsapp_business_encryption` (multipart)
    /// -- registers the business's RSA public key with Meta, per "Set
    /// Encryption Public Key". The matching private key stays local,
    /// stored on the session, and is only ever used to unwrap the
    /// per-request AES key in [`crate::cloud::flows_crypto`].
    pub async fn set_flow_encryption_public_key(
        &self,
        public_key_pem: &str,
    ) -> Result<Value, CloudError> {
        let form =
            reqwest::multipart::Form::new().text("business_public_key", public_key_pem.to_string());
        self.post_multipart(
            &format!(
                "{}/{}/whatsapp_business_encryption",
                self.base_url, self.phone_number_id
            ),
            form,
        )
        .await
    }

    async fn post_multipart(
        &self,
        url: &str,
        form: reqwest::multipart::Form,
    ) -> Result<Value, CloudError> {
        let resp = self
            .http
            .post(url)
            .bearer_auth(&self.access_token)
            .multipart(form)
            .send()
            .await?;
        Self::parse_response(resp).await
    }

    async fn post_json(&self, url: &str, body: Value) -> Result<Value, CloudError> {
        let resp = self
            .http
            .post(url)
            .bearer_auth(&self.access_token)
            .json(&body)
            .send()
            .await?;
        Self::parse_response(resp).await
    }

    async fn delete_json(&self, url: &str) -> Result<Value, CloudError> {
        let resp = self
            .http
            .delete(url)
            .bearer_auth(&self.access_token)
            .send()
            .await?;
        Self::parse_response(resp).await
    }

    async fn parse_response(resp: reqwest::Response) -> Result<Value, CloudError> {
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

    /// `DELETE {media_id}?phone_number_id=...`, per "Delete Media".
    pub async fn delete_media(&self, media_id: &str) -> Result<Value, CloudError> {
        let url = format!(
            "{}/{}?phone_number_id={}",
            self.base_url, media_id, self.phone_number_id
        );
        let resp = self
            .http
            .delete(url)
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

    /// `GET {phone_number_id}/whatsapp_commerce_settings`, per "Get
    /// commerce settings".
    pub async fn get_commerce_settings(&self) -> Result<Value, CloudError> {
        self.get_json(&format!(
            "{}/{}/whatsapp_commerce_settings",
            self.base_url, self.phone_number_id
        ))
        .await
    }

    /// `POST {phone_number_id}/whatsapp_commerce_settings`, per "Set or
    /// update commerce settings". Meta takes both flags as query
    /// parameters with no body; a flag left `None` is not sent and keeps
    /// its current value.
    pub async fn set_commerce_settings(
        &self,
        is_cart_enabled: Option<bool>,
        is_catalog_visible: Option<bool>,
    ) -> Result<Value, CloudError> {
        let mut query: Vec<(&str, String)> = Vec::new();
        if let Some(v) = is_cart_enabled {
            query.push(("is_cart_enabled", v.to_string()));
        }
        if let Some(v) = is_catalog_visible {
            query.push(("is_catalog_visible", v.to_string()));
        }
        let resp = self
            .http
            .post(format!(
                "{}/{}/whatsapp_commerce_settings",
                self.base_url, self.phone_number_id
            ))
            .bearer_auth(&self.access_token)
            .query(&query)
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
