//! WhatsApp Commerce for `whatsapp_cloud` sessions: commerce settings and
//! the catalog-backed interactive messages (single product,
//! multi-product, catalog).
//!
//! These have no whatsapp-rust counterpart, so a `whatsapp_web` session
//! gets `400` rather than a dispatch. Meta's own limits on multi-product
//! messages (at most 10 sections and 30 products, each section titled
//! and non-empty) are checked here first, so a bad request fails with a
//! clear `400` instead of an opaque Graph API error after a round trip.
//! Inbound carts arrive through the existing webhook receiver as
//! `message_type: "order"` (see [`crate::cloud::webhook`]).

use axum::{
    extract::{Path, State},
    Json,
};
use serde_json::{json, Value};

use crate::cloud::client::CloudClient;
use crate::error::ApiError;
use crate::models::cloud_commerce::{
    SendCatalogRequest, SendProductListRequest, SendProductRequest, UpdateCommerceSettingsRequest,
};
use crate::models::messages::MessageResponse;
use crate::state::AppState;

const MAX_SECTIONS: usize = 10;
const MAX_PRODUCTS: usize = 30;

async fn cloud_client(state: &AppState, session_id: &str) -> Result<CloudClient, ApiError> {
    let creds = state
        .session_manager()
        .get_cloud_credentials(session_id)
        .await?
        .ok_or_else(|| {
            ApiError::BadRequest(
                "commerce is only supported for whatsapp_cloud sessions".to_string(),
            )
        })?;
    Ok(CloudClient::new(
        &creds.phone_number_id,
        &creds.access_token,
    ))
}

fn upstream(e: crate::cloud::client::CloudError) -> ApiError {
    ApiError::Internal(format!("cloud commerce call failed: {e}"))
}

fn sent(resp: &Value, to: String) -> Json<MessageResponse> {
    Json(MessageResponse {
        message_id: resp
            .pointer("/messages/0/id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        timestamp: chrono::Utc::now().timestamp(),
        to,
    })
}

fn envelope(to: &str, interactive: Value) -> Value {
    json!({
        "messaging_product": "whatsapp",
        "recipient_type": "individual",
        "to": to,
        "type": "interactive",
        "interactive": interactive,
    })
}

fn product_payload(r: &SendProductRequest) -> Value {
    let mut interactive = json!({
        "type": "product",
        "action": {
            "catalog_id": r.catalog_id,
            "product_retailer_id": r.product_retailer_id,
        },
    });
    if let Some(body) = &r.body {
        interactive["body"] = json!({ "text": body });
    }
    if let Some(footer) = &r.footer {
        interactive["footer"] = json!({ "text": footer });
    }
    envelope(&r.to, interactive)
}

fn product_list_payload(r: &SendProductListRequest) -> Result<Value, ApiError> {
    if r.sections.is_empty() || r.sections.len() > MAX_SECTIONS {
        return Err(ApiError::BadRequest(format!(
            "sections must contain between 1 and {MAX_SECTIONS} entries"
        )));
    }
    let total: usize = r
        .sections
        .iter()
        .map(|s| s.product_retailer_ids.len())
        .sum();
    if total > MAX_PRODUCTS {
        return Err(ApiError::BadRequest(format!(
            "at most {MAX_PRODUCTS} products per message, got {total}"
        )));
    }
    if r.sections
        .iter()
        .any(|s| s.title.trim().is_empty() || s.product_retailer_ids.is_empty())
    {
        return Err(ApiError::BadRequest(
            "every section needs a title and at least one product".to_string(),
        ));
    }

    let sections: Vec<Value> = r
        .sections
        .iter()
        .map(|s| {
            json!({
                "title": s.title,
                "product_items": s
                    .product_retailer_ids
                    .iter()
                    .map(|id| json!({ "product_retailer_id": id }))
                    .collect::<Vec<_>>(),
            })
        })
        .collect();

    let mut interactive = json!({
        "type": "product_list",
        "header": { "type": "text", "text": r.header },
        "body": { "text": r.body },
        "action": { "catalog_id": r.catalog_id, "sections": sections },
    });
    if let Some(footer) = &r.footer {
        interactive["footer"] = json!({ "text": footer });
    }
    Ok(envelope(&r.to, interactive))
}

fn catalog_payload(r: &SendCatalogRequest) -> Value {
    let mut action = json!({ "name": "catalog_message" });
    if let Some(sku) = &r.thumbnail_product_retailer_id {
        action["parameters"] = json!({ "thumbnail_product_retailer_id": sku });
    }
    let mut interactive = json!({
        "type": "catalog_message",
        "body": { "text": r.body },
        "action": action,
    });
    if let Some(footer) = &r.footer {
        interactive["footer"] = json!({ "text": footer });
    }
    envelope(&r.to, interactive)
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/commerce-settings",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID")),
    responses(
        (status = 200, description = "Cart and catalog visibility flags (Meta's raw response)"),
        (status = 400, description = "Not a whatsapp_cloud session")
    )
)]
pub async fn get_commerce_settings(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let resp = cloud_client(&state, &session_id)
        .await?
        .get_commerce_settings()
        .await
        .map_err(upstream)?;
    Ok(Json(resp))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/commerce-settings",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID")),
    request_body = UpdateCommerceSettingsRequest,
    responses(
        (status = 200, description = "Settings updated (Meta's raw response)"),
        (status = 400, description = "Not a whatsapp_cloud session, or no flag given")
    )
)]
pub async fn update_commerce_settings(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<UpdateCommerceSettingsRequest>,
) -> Result<Json<Value>, ApiError> {
    if request.is_cart_enabled.is_none() && request.is_catalog_visible.is_none() {
        return Err(ApiError::BadRequest(
            "at least one of is_cart_enabled, is_catalog_visible is required".to_string(),
        ));
    }
    let resp = cloud_client(&state, &session_id)
        .await?
        .set_commerce_settings(request.is_cart_enabled, request.is_catalog_visible)
        .await
        .map_err(upstream)?;
    Ok(Json(resp))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/messages/product",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID")),
    request_body = SendProductRequest,
    responses(
        (status = 200, description = "Product message sent", body = MessageResponse),
        (status = 400, description = "Not a whatsapp_cloud session")
    )
)]
pub async fn send_product(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendProductRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    let resp = cloud_client(&state, &session_id)
        .await?
        .send_raw(product_payload(&request))
        .await
        .map_err(upstream)?;
    Ok(sent(&resp, request.to))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/messages/product-list",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID")),
    request_body = SendProductListRequest,
    responses(
        (status = 200, description = "Multi-product message sent", body = MessageResponse),
        (status = 400, description = "Not a whatsapp_cloud session, or over Meta's section/product limits")
    )
)]
pub async fn send_product_list(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendProductListRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    let payload = product_list_payload(&request)?;
    let resp = cloud_client(&state, &session_id)
        .await?
        .send_raw(payload)
        .await
        .map_err(upstream)?;
    Ok(sent(&resp, request.to))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/messages/catalog",
    tag = "cloud",
    params(("session_id" = String, Path, description = "Session ID")),
    request_body = SendCatalogRequest,
    responses(
        (status = 200, description = "Catalog message sent", body = MessageResponse),
        (status = 400, description = "Not a whatsapp_cloud session")
    )
)]
pub async fn send_catalog(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendCatalogRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    let resp = cloud_client(&state, &session_id)
        .await?
        .send_raw(catalog_payload(&request))
        .await
        .map_err(upstream)?;
    Ok(sent(&resp, request.to))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::cloud_commerce::ProductSection;

    fn list(sections: Vec<ProductSection>) -> SendProductListRequest {
        SendProductListRequest {
            to: "6281".to_string(),
            catalog_id: "CAT".to_string(),
            header: "Picks".to_string(),
            body: "Tap one".to_string(),
            footer: None,
            sections,
        }
    }

    fn section(title: &str, n: usize) -> ProductSection {
        ProductSection {
            title: title.to_string(),
            product_retailer_ids: (0..n).map(|i| format!("SKU-{i}")).collect(),
        }
    }

    #[test]
    fn single_product_matches_metas_shape() {
        let p = product_payload(&SendProductRequest {
            to: "6281".to_string(),
            catalog_id: "CAT".to_string(),
            product_retailer_id: "SKU-1".to_string(),
            body: Some("Look".to_string()),
            footer: None,
        });
        assert_eq!(p["type"], "interactive");
        assert_eq!(p["interactive"]["type"], "product");
        assert_eq!(p["interactive"]["action"]["catalog_id"], "CAT");
        assert_eq!(p["interactive"]["action"]["product_retailer_id"], "SKU-1");
        assert_eq!(p["interactive"]["body"]["text"], "Look");
        assert!(p["interactive"].get("footer").is_none());
    }

    #[test]
    fn product_list_matches_metas_shape() {
        let p = product_list_payload(&list(vec![section("A", 2), section("B", 1)])).unwrap();
        let i = &p["interactive"];
        assert_eq!(i["type"], "product_list");
        assert_eq!(i["header"], json!({"type": "text", "text": "Picks"}));
        assert_eq!(i["action"]["catalog_id"], "CAT");
        assert_eq!(i["action"]["sections"][0]["title"], "A");
        assert_eq!(
            i["action"]["sections"][0]["product_items"][1]["product_retailer_id"],
            "SKU-1"
        );
        assert_eq!(
            i["action"]["sections"][1]["product_items"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn product_list_enforces_metas_limits() {
        assert!(product_list_payload(&list(vec![])).is_err());
        assert!(product_list_payload(&list((0..11).map(|_| section("S", 1)).collect())).is_err());
        assert!(product_list_payload(&list(vec![section("A", 20), section("B", 11)])).is_err());
        assert!(product_list_payload(&list(vec![section("A", 20), section("B", 10)])).is_ok());
        assert!(product_list_payload(&list(vec![section("A", 0)])).is_err());
        assert!(product_list_payload(&list(vec![section(" ", 1)])).is_err());
    }

    #[test]
    fn catalog_message_thumbnail_is_optional() {
        let with = catalog_payload(&SendCatalogRequest {
            to: "6281".to_string(),
            body: "Shop".to_string(),
            footer: Some("Deals".to_string()),
            thumbnail_product_retailer_id: Some("SKU-9".to_string()),
        });
        assert_eq!(with["interactive"]["type"], "catalog_message");
        assert_eq!(with["interactive"]["action"]["name"], "catalog_message");
        assert_eq!(
            with["interactive"]["action"]["parameters"]["thumbnail_product_retailer_id"],
            "SKU-9"
        );
        assert_eq!(with["interactive"]["footer"]["text"], "Deals");

        let without = catalog_payload(&SendCatalogRequest {
            to: "6281".to_string(),
            body: "Shop".to_string(),
            footer: None,
            thumbnail_product_retailer_id: None,
        });
        assert!(without["interactive"]["action"].get("parameters").is_none());
    }
}
