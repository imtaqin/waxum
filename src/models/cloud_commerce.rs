//! Request/response types for WhatsApp Commerce on `whatsapp_cloud`
//! sessions: commerce settings and the three catalog-backed interactive
//! message types (single product, multi-product, catalog).

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// `POST /sessions/{id}/cloud/commerce-settings`. Either flag may be
/// omitted to leave it unchanged, but at least one must be present.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct UpdateCommerceSettingsRequest {
    /// Lets customers add items to a cart and send it as an order.
    #[schema(example = true)]
    pub is_cart_enabled: Option<bool>,
    /// Shows the storefront icon and catalog on the business profile.
    #[schema(example = true)]
    pub is_catalog_visible: Option<bool>,
}

/// `POST /sessions/{id}/messages/product` -- one product from a catalog,
/// per "Send Single Product Message".
#[derive(Debug, Deserialize, ToSchema)]
pub struct SendProductRequest {
    #[schema(example = "6281234567890")]
    pub to: String,
    #[schema(example = "367025965434465")]
    pub catalog_id: String,
    /// The product's SKU (retailer ID) in the catalog.
    #[schema(example = "SKU-001")]
    pub product_retailer_id: String,
    pub body: Option<String>,
    pub footer: Option<String>,
}

/// One titled group of products in a multi-product message.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ProductSection {
    #[schema(example = "Best sellers")]
    pub title: String,
    #[schema(example = json!(["SKU-001", "SKU-002"]))]
    pub product_retailer_ids: Vec<String>,
}

/// `POST /sessions/{id}/messages/product-list` -- up to 30 products in up
/// to 10 sections, per "Send Multi-Product Message". Meta requires the
/// text header and body on this message type.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SendProductListRequest {
    #[schema(example = "6281234567890")]
    pub to: String,
    #[schema(example = "146265584024623")]
    pub catalog_id: String,
    #[schema(example = "Our picks for you")]
    pub header: String,
    #[schema(example = "Tap a product to see details")]
    pub body: String,
    pub footer: Option<String>,
    pub sections: Vec<ProductSection>,
}

/// `POST /sessions/{id}/messages/catalog` -- opens the whole catalog, per
/// "Send Catalog Message".
#[derive(Debug, Deserialize, ToSchema)]
pub struct SendCatalogRequest {
    #[schema(example = "6281234567890")]
    pub to: String,
    #[schema(example = "Browse our catalog and order right here in the chat.")]
    pub body: String,
    pub footer: Option<String>,
    /// SKU whose image is used as the message thumbnail; Meta picks the
    /// first catalog item when omitted.
    pub thumbnail_product_retailer_id: Option<String>,
}
