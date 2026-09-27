//! Generic Graph API calls for the account-administration surface
//! (phone numbers, templates, QR codes, subscriptions, analytics, BSP
//! credit sharing, ...), where each endpoint is a thin pass-through and a
//! dedicated [`CloudClient`] method per call would only repeat the same
//! request/response plumbing.
//!
//! Every path segment that comes from a caller (a template ID, QR code,
//! credit-line ID, ...) goes through [`graph_id`] first. axum percent-
//! decodes path parameters, so an unchecked `%2F` would let a caller
//! steer the session's access token at an arbitrary Graph API path.

use reqwest::Method;
use serde_json::Value;

use super::client::{CloudClient, CloudError};

/// Accepts a Graph API object ID or similar opaque token (IDs, QR codes,
/// template names): ASCII letters, digits, `_`, `-` and `.`, 1-128 chars.
pub fn graph_id(raw: &str) -> Option<&str> {
    let ok = !raw.is_empty()
        && raw.len() <= 128
        && raw
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'));
    ok.then_some(raw)
}

impl CloudClient {
    pub fn phone_number_id(&self) -> &str {
        &self.phone_number_id
    }

    /// Calls `{base_url}/{path}` with the session's bearer token. `path`
    /// is relative (`"{waba_id}/message_templates"`) and must only be
    /// built from validated IDs; `query` is sent URL-encoded; `body`, when
    /// present, is sent as JSON.
    pub async fn graph(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<Value>,
    ) -> Result<Value, CloudError> {
        let mut req = self
            .http
            .request(method, format!("{}/{}", self.base_url, path))
            .bearer_auth(&self.access_token)
            .query(query);
        if let Some(body) = body {
            req = req.json(&body);
        }
        parse(req.send().await?).await
    }

    /// Multipart form variant of [`Self::graph`], for the few endpoints
    /// (`migrate_flows`) Meta only accepts as `multipart/form-data`.
    pub async fn graph_form(
        &self,
        path: &str,
        form: reqwest::multipart::Form,
    ) -> Result<Value, CloudError> {
        let resp = self
            .http
            .post(format!("{}/{}", self.base_url, path))
            .bearer_auth(&self.access_token)
            .multipart(form)
            .send()
            .await?;
        parse(resp).await
    }

    /// Resumable Upload API, used for the business profile photo: opens an
    /// upload session on `app_id`, sends the bytes in one chunk, and
    /// returns the file handle (`h`) Meta expects as
    /// `profile_picture_handle`. This API authenticates with
    /// `Authorization: OAuth <token>`, not `Bearer`.
    pub async fn resumable_upload(
        &self,
        app_id: &str,
        bytes: Vec<u8>,
        mime_type: &str,
        file_name: &str,
    ) -> Result<String, CloudError> {
        let auth = format!("OAuth {}", self.access_token);
        let session = parse(
            self.http
                .post(format!("{}/{}/uploads", self.base_url, app_id))
                .header("Authorization", &auth)
                .query(&[
                    ("file_length", bytes.len().to_string()),
                    ("file_type", mime_type.to_string()),
                    ("file_name", file_name.to_string()),
                ])
                .send()
                .await?,
        )
        .await?;
        let upload_id = session
            .get("id")
            .and_then(Value::as_str)
            .and_then(graph_id_with_colon)
            .ok_or_else(|| CloudError::Api {
                status: 0,
                body: format!("upload session response had no usable id: {session}"),
            })?
            .to_string();

        let uploaded = parse(
            self.http
                .post(format!("{}/{}", self.base_url, upload_id))
                .header("Authorization", &auth)
                .header("file_offset", "0")
                .body(bytes)
                .send()
                .await?,
        )
        .await?;
        uploaded
            .get("h")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| CloudError::Api {
                status: 0,
                body: format!("upload response had no file handle: {uploaded}"),
            })
    }
}

/// Upload session IDs look like `upload:MTphdHRh...?sig=...`; they come
/// from Meta, not the caller, but are still checked before being used as
/// a URL path.
fn graph_id_with_colon(raw: &str) -> Option<&str> {
    let ok = !raw.is_empty()
        && raw.len() <= 1024
        && raw.bytes().all(|b| {
            b.is_ascii_alphanumeric()
                || matches!(b, b'_' | b'-' | b'.' | b':' | b'?' | b'=' | b'&' | b'%')
        });
    ok.then_some(raw)
}

async fn parse(resp: reqwest::Response) -> Result<Value, CloudError> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_id_accepts_meta_ids_and_rejects_path_tricks() {
        assert_eq!(graph_id("102290129340398"), Some("102290129340398"));
        assert_eq!(graph_id("seasonal_promo-2.v1"), Some("seasonal_promo-2.v1"));
        for bad in ["", "a/b", "../me", "123?fields=x", "a b", "a%2Fb", "é"] {
            assert_eq!(graph_id(bad), None, "{bad:?} should be rejected");
        }
        assert_eq!(graph_id(&"9".repeat(129)), None);
    }

    #[test]
    fn upload_ids_allow_metas_query_suffix_but_not_slashes() {
        assert!(graph_id_with_colon("upload:MTphdHRhY2g=?sig=ARZ").is_some());
        assert!(graph_id_with_colon("upload:abc/../me").is_none());
    }
}
