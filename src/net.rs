use whatsapp_rust_ureq_http_client::UreqHttpClient;

pub fn build_http_client() -> UreqHttpClient {
    match proxy_url() {
        Some(url) => match build_proxied_agent(&url) {
            Ok(agent) => {
                tracing::info!("HTTP client using proxy: {}", url);
                UreqHttpClient::with_agent(agent)
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to configure proxy '{}', falling back to direct: {}",
                    url,
                    e
                );
                UreqHttpClient::new()
            }
        },
        None => UreqHttpClient::new(),
    }
}

fn proxy_url() -> Option<String> {
    for key in ["WA_PROXY", "HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy"] {
        if let Ok(v) = std::env::var(key) {
            let v = v.trim();
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

fn build_proxied_agent(url: &str) -> anyhow::Result<ureq::Agent> {
    let proxy = ureq::Proxy::new(url)?;
    let config = ureq::config::Config::builder()
        .proxy(Some(proxy))
        .input_buffer_size(16 * 1024)
        .output_buffer_size(16 * 1024)
        .max_idle_connections(3)
        .max_idle_connections_per_host(2)
        .build();
    Ok(config.into())
}
