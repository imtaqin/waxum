//! Server metadata endpoint — returns version + self-detected geo location.
//!
//! The server detects its own public IP and geolocation at first request,
//! then caches for the lifetime of the process. Admin can override any
//! field via env vars: WA_LOCATION_CODE, WA_LOCATION_COUNTRY,
//! WA_LOCATION_CITY, WA_LOCATION_REGION, WA_LOCATION_LAT, WA_LOCATION_LON,
//! WA_LOCATION_TZ.
//!
//! This is needed because some deployments run behind Tailscale or other
//! overlay networks — the adonis gateway can't resolve their location from
//! a private 100.64.0.0/10 address. waxum itself has outbound internet
//! access via its real public IP (even if serving via Tailscale), so it
//! can self-detect reliably.

use axum::Json;
use serde::{Deserialize, Serialize};
use tokio::sync::OnceCell;
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LocationInfo {
    pub ip: Option<String>,
    pub country_code: Option<String>,
    pub country_name: Option<String>,
    pub city: Option<String>,
    pub region: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub timezone: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ServerInfo {
    pub version: String,
    pub location: LocationInfo,
}

static CACHED_LOCATION: OnceCell<LocationInfo> = OnceCell::const_new();

fn env_opt(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|s| !s.trim().is_empty())
}

fn location_from_env() -> Option<LocationInfo> {
    let code = env_opt("WA_LOCATION_CODE");
    let country = env_opt("WA_LOCATION_COUNTRY");
    let city = env_opt("WA_LOCATION_CITY");
    let region = env_opt("WA_LOCATION_REGION");

    if code.is_none() && country.is_none() && city.is_none() {
        return None;
    }

    Some(LocationInfo {
        ip: env_opt("WA_LOCATION_IP"),
        country_code: code,
        country_name: country,
        city: city.clone(),
        region: region.or(city),
        latitude: env_opt("WA_LOCATION_LAT").and_then(|v| v.parse().ok()),
        longitude: env_opt("WA_LOCATION_LON").and_then(|v| v.parse().ok()),
        timezone: env_opt("WA_LOCATION_TZ"),
    })
}

#[derive(Deserialize)]
struct IpApiResponse {
    status: String,
    query: Option<String>,
    country: Option<String>,
    #[serde(rename = "countryCode")]
    country_code: Option<String>,
    city: Option<String>,
    #[serde(rename = "regionName")]
    region: Option<String>,
    lat: Option<f64>,
    lon: Option<f64>,
    timezone: Option<String>,
}

async fn fetch_ipapi() -> Option<LocationInfo> {
    let url = "http://ip-api.com/json/?fields=status,query,country,countryCode,city,regionName,lat,lon,timezone";
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .ok()?;

    let body = client.get(url).send().await.ok()?.text().await.ok()?;
    let resp: IpApiResponse = serde_json::from_str(&body).ok()?;
    if resp.status != "success" {
        return None;
    }

    let city = resp.city.clone();
    Some(LocationInfo {
        ip: resp.query,
        country_code: resp.country_code,
        country_name: resp.country,
        city,
        region: resp.region,
        latitude: resp.lat,
        longitude: resp.lon,
        timezone: resp.timezone,
    })
}

async fn detect_location() -> LocationInfo {
    // Env override takes precedence
    if let Some(loc) = location_from_env() {
        return loc;
    }
    // Auto-detect via ip-api.com (server's outbound public IP)
    fetch_ipapi().await.unwrap_or(LocationInfo {
        ip: None,
        country_code: None,
        country_name: None,
        city: None,
        region: None,
        latitude: None,
        longitude: None,
        timezone: None,
    })
}

pub async fn get_info() -> Json<ServerInfo> {
    let location = CACHED_LOCATION.get_or_init(detect_location).await.clone();
    Json(ServerInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        location,
    })
}
