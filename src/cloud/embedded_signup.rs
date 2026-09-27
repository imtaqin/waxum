//! Embedded Signup onboarding: exchanges the OAuth `code` the client-side
//! Embedded Signup JS SDK hands back for a long-lived access token, then
//! runs the two "Step 2: Integrate with Required Endpoints" calls from
//! the Embedded Signup collection that finish attaching waxum to the
//! client's WABA -- subscribing the app to receive its webhooks, and
//! listing its phone numbers so the caller can pick which one to attach
//! to a waxum session via [`crate::handlers::cloud::connect_cloud`].
//!
//! The code-for-token exchange itself
//! (`GET /{version}/oauth/access_token`) is documented in Meta's OAuth
//! reference rather than in either Postman collection, since it is the
//! same endpoint every Graph API OAuth integration uses, not a
//! WhatsApp-specific one.

use serde_json::Value;

use super::client::CloudError;

fn api_version() -> String {
    std::env::var("WHATSAPP_CLOUD_API_VERSION").unwrap_or_else(|_| "v21.0".to_string())
}

async fn get_json(http: &reqwest::Client, url: &str) -> Result<Value, CloudError> {
    let resp = http.get(url).send().await?;
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

/// `GET /{version}/oauth/access_token?client_id=...&client_secret=...&code=...`
/// -- exchanges the Embedded Signup `code` for an access token. Returns the
/// raw Graph API response (`access_token`, `token_type`, and on some app
/// configurations `expires_in`).
pub async fn exchange_code(
    app_id: &str,
    app_secret: &str,
    code: &str,
) -> Result<Value, CloudError> {
    let http = reqwest::Client::new();
    let url = format!(
        "https://graph.facebook.com/{}/oauth/access_token?client_id={}&client_secret={}&code={}",
        api_version(),
        app_id,
        app_secret,
        code
    );
    get_json(&http, &url).await
}

/// `POST /{version}/{waba_id}/subscribed_apps` -- subscribes waxum's app
/// to the client's WABA so its webhook deliveries start arriving, per
/// "Subscribe App to WhatsApp Business Account".
pub async fn subscribe_app(waba_id: &str, access_token: &str) -> Result<Value, CloudError> {
    let http = reqwest::Client::new();
    let url = format!(
        "https://graph.facebook.com/{}/{}/subscribed_apps",
        api_version(),
        waba_id
    );
    let resp = http.post(url).bearer_auth(access_token).send().await?;
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

/// `GET /{version}/{waba_id}/phone_numbers` -- lists the phone numbers on
/// a WABA so the caller can pick which one to attach to a waxum session.
pub async fn list_phone_numbers(waba_id: &str, access_token: &str) -> Result<Value, CloudError> {
    let http = reqwest::Client::new();
    let url = format!(
        "https://graph.facebook.com/{}/{}/phone_numbers",
        api_version(),
        waba_id
    );
    let resp = http.get(&url).bearer_auth(access_token).send().await?;
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
