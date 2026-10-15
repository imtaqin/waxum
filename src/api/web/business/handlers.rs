use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::Deserialize;
use wacore_binary::Jid;

use crate::api::web::business::models::*;
use crate::error::ApiError;
use crate::state::AppState;

fn get_client(
    state: &AppState,
    session_id: &str,
) -> Result<std::sync::Arc<whatsapp_rust::Client>, ApiError> {
    let runtime = state
        .get_session(session_id)
        .ok_or(ApiError::NotConnected)?;
    runtime.get_live_client().ok_or(ApiError::NotConnected)
}

fn parse_jid(s: &str) -> Result<Jid, ApiError> {
    s.parse()
        .map_err(|e| ApiError::InvalidJid(format!("{s}: {e}")))
}

#[derive(Deserialize)]
pub struct CatalogParams {
    pub jid: String,
    pub limit: Option<u32>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub after: Option<String>,
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/business/catalog",
    tag = "business",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("jid" = String, Query, description = "Business JID"),
        ("limit" = Option<u32>, Query, description = "Products per page"),
        ("after" = Option<String>, Query, description = "Pagination cursor"),
    ),
    responses(
        (status = 200, description = "Catalog", body = BusinessCatalogResponse),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn get_catalog(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(params): Query<CatalogParams>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let jid = parse_jid(&params.jid)?;
    let mut opts = whatsapp_rust::CatalogOptions::default();
    if let Some(l) = params.limit {
        opts.limit = l;
    }
    if let Some(a) = params.after {
        opts.after = Some(a);
    }
    if let Some(w) = params.width {
        opts.image_width = w;
    }
    if let Some(h) = params.height {
        opts.image_height = h;
    }
    let catalog = client
        .business()
        .get_catalog(&jid, &opts)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let products = serde_json::json!(format!("{:?}", catalog.products));
    Ok(Json(serde_json::json!({
        "products": products,
        "after_cursor": catalog.after_cursor,
        "before_cursor": catalog.before_cursor,
    })))
}

#[derive(Deserialize)]
pub struct CollectionsParams {
    pub jid: String,
    pub limit: Option<u32>,
    pub item_limit: Option<u32>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub after: Option<String>,
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/business/collections",
    tag = "business",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("jid" = String, Query, description = "Business JID"),
    ),
    responses(
        (status = 200, description = "Collections", body = BusinessCollectionsResponse),
        (status = 404, description = "Session not found"),
    )
)]
pub async fn get_collections(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(params): Query<CollectionsParams>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let jid = parse_jid(&params.jid)?;
    let mut opts = whatsapp_rust::CollectionOptions::default();
    if let Some(l) = params.limit {
        opts.collection_limit = l;
    }
    if let Some(il) = params.item_limit {
        opts.item_limit = il;
    }
    if let Some(w) = params.width {
        opts.image_width = w;
    }
    if let Some(h) = params.height {
        opts.image_height = h;
    }
    if let Some(a) = params.after {
        opts.after = Some(a);
    }
    let cols = client
        .business()
        .get_collections(&jid, &opts)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let collections = serde_json::json!(format!("{:?}", cols.collections));
    Ok(Json(serde_json::json!({
        "collections": collections,
        "after_cursor": cols.after_cursor,
    })))
}

#[derive(Deserialize)]
pub struct OrderParams {
    pub jid: String,
    pub order_id: String,
    pub token: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/business/order",
    tag = "business",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("jid" = String, Query, description = "Business JID"),
        ("order_id" = String, Query, description = "Order ID"),
        ("token" = String, Query, description = "Order token"),
    ),
    responses(
        (status = 200, description = "Order", body = BusinessOrderResponse),
    )
)]
pub async fn get_order(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(params): Query<OrderParams>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let jid = parse_jid(&params.jid)?;
    let order = client
        .business()
        .get_order(&jid, &params.order_id, &params.token)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let value = serde_json::json!(format!("{:?}", order));
    Ok(Json(serde_json::json!({ "order": value })))
}

#[utoipa::path(
    patch,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/business/profile",
    tag = "business",
    params(("session_id" = String, Path, description = "Session ID")),
    request_body = BusinessProfileUpdateRequest,
    responses((status = 200, description = "Profile updated"))
)]
pub async fn update_business_profile(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(req): Json<BusinessProfileUpdateRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let mut update = wacore::iq::business::BusinessProfileUpdate::default();
    if let Some(desc) = req.description {
        update.description = Some(desc);
    }
    if let Some(email) = req.email {
        update.email = Some(email);
    }
    if let Some(webs) = req.websites {
        update.websites = Some(webs);
    }
    if let Some(addr) = req.address {
        update.address = Some(addr);
    }
    if let Some(cat) = req.category {
        update.categories = Some(vec![cat]);
    }
    client
        .business()
        .update_profile(&update)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(serde_json::json!({ "success": true })))
}

#[utoipa::path(
    delete,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/business/cover-photo/{photo_id}",
    tag = "business",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("photo_id" = String, Path, description = "Cover photo fbid"),
    ),
    responses((status = 200, description = "Cover photo removed"))
)]
pub async fn remove_cover_photo(
    State(state): State<AppState>,
    Path((session_id, photo_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let client = get_client(&state, &session_id)?;
    client
        .business()
        .remove_cover_photo(&photo_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(serde_json::json!({ "success": true })))
}
