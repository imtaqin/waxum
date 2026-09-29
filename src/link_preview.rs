//! Link previews for outbound text messages (`link_preview: true` on
//! `POST /messages/text` and the NATS text command).
//!
//! WhatsApp shows a preview card only when the sender attaches the page
//! metadata to the message itself (`ExtendedTextMessage.matched_text`,
//! `title`, `description`, `jpeg_thumbnail`); the recipient's phone never
//! fetches the page. So waxum fetches it, for the first `http(s)` URL in
//! the text, once per send.
//!
//! The fetch is caller-driven: whoever calls the API chooses the URL. It
//! therefore goes through [`crate::net_guard`] exactly like media-by-URL
//! (public addresses only, re-checked on every redirect hop) and is
//! bounded in time and size. The page gets [`PAGE_BUDGET`] and is read
//! only up to `</head>` (at most [`MAX_HTML_BYTES`]); the thumbnail gets
//! its own [`THUMBNAIL_BUDGET`] and [`MAX_IMAGE_BYTES`], so a slow image
//! still leaves a title-and-description preview. A page that fails yields
//! `None` and the text is sent without a preview; a preview is never
//! worth failing a message for.
//!
//! Results are cached for [`CACHE_TTL`] so a blast of the same link to
//! many recipients fetches the page once.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;

const PAGE_BUDGET: Duration = Duration::from_secs(5);
const THUMBNAIL_BUDGET: Duration = Duration::from_secs(3);
const MAX_HTML_BYTES: usize = 512 * 1024;
const MAX_IMAGE_BYTES: usize = 3 * 1024 * 1024;
const THUMBNAIL_MAX_SIDE: u32 = 300;
const CACHE_TTL: Duration = Duration::from_secs(10 * 60);
const CACHE_CAPACITY: usize = 256;

/// Page metadata to attach to an `ExtendedTextMessage`.
#[derive(Debug, Clone, PartialEq)]
pub struct LinkPreview {
    /// The URL exactly as it appears in the message text.
    pub matched_text: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub jpeg_thumbnail: Option<Vec<u8>>,
    pub thumbnail_width: Option<u32>,
    pub thumbnail_height: Option<u32>,
}

/// Copies `preview` onto an outgoing text message. `text` is left as is;
/// `matched_text` must be the URL as it appears in it, which is what
/// [`first_url`] returns.
pub fn apply(message: &mut waproto::whatsapp::message::ExtendedTextMessage, preview: LinkPreview) {
    message.matched_text = Some(preview.matched_text);
    message.title = preview.title;
    message.description = preview.description;
    message.jpeg_thumbnail = preview.jpeg_thumbnail;
    message.thumbnail_width = preview.thumbnail_width;
    message.thumbnail_height = preview.thumbnail_height;
}

type Cache = Mutex<HashMap<String, (Instant, Option<LinkPreview>)>>;
static CACHE: Lazy<Cache> = Lazy::new(|| Mutex::new(HashMap::new()));

/// The first `http://` or `https://` URL in `text`, with trailing
/// sentence punctuation and unbalanced closing brackets trimmed, as it
/// appears in the text.
pub fn first_url(text: &str) -> Option<&str> {
    let start = text
        .find("https://")
        .into_iter()
        .chain(text.find("http://"))
        .min()?;
    let rest = &text[start..];
    let end = rest
        .find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"'))
        .unwrap_or(rest.len());
    let mut url = &rest[..end];
    loop {
        let trimmed = url.trim_end_matches(['.', ',', ';', ':', '!', '?', '\'']);
        let trimmed = match trimmed.chars().last() {
            Some(')') if trimmed.matches('(').count() < trimmed.matches(')').count() => {
                &trimmed[..trimmed.len() - 1]
            }
            Some(']') if trimmed.matches('[').count() < trimmed.matches(']').count() => {
                &trimmed[..trimmed.len() - 1]
            }
            _ => trimmed,
        };
        if trimmed == url {
            break;
        }
        url = trimmed;
    }
    let host_part = url.split("://").nth(1).unwrap_or("");
    (!host_part.is_empty()).then_some(url)
}

/// Builds a preview for the first URL in `text`, or `None` when there is
/// no URL or nothing usable could be fetched in time.
pub async fn for_text(text: &str) -> Option<LinkPreview> {
    let url = first_url(text)?.to_string();

    if let Ok(cache) = CACHE.lock() {
        if let Some((at, cached)) = cache.get(&url) {
            if at.elapsed() < CACHE_TTL {
                return cached.clone();
            }
        }
    }

    let result = fetch(&url).await;

    if let Ok(mut cache) = CACHE.lock() {
        if cache.len() >= CACHE_CAPACITY {
            cache.retain(|_, (at, _)| at.elapsed() < CACHE_TTL);
            if cache.len() >= CACHE_CAPACITY {
                cache.clear();
            }
        }
        cache.insert(url, (Instant::now(), result.clone()));
    }
    result
}

async fn fetch(url: &str) -> Option<LinkPreview> {
    let (meta, final_url) = tokio::time::timeout(PAGE_BUDGET, fetch_page(url))
        .await
        .ok()
        .flatten()?;
    if meta.title.is_none() && meta.description.is_none() {
        return None;
    }

    let mut preview = LinkPreview {
        matched_text: url.to_string(),
        title: meta.title,
        description: meta.description,
        jpeg_thumbnail: None,
        thumbnail_width: None,
        thumbnail_height: None,
    };
    if let Some(image_url) = meta.image.and_then(|i| final_url.join(&i).ok()) {
        let thumbnail = tokio::time::timeout(THUMBNAIL_BUDGET, fetch_thumbnail(image_url.as_str()))
            .await
            .ok()
            .flatten();
        if let Some((jpeg, w, h)) = thumbnail {
            preview.jpeg_thumbnail = Some(jpeg);
            preview.thumbnail_width = Some(w);
            preview.thumbnail_height = Some(h);
        }
    }
    Some(preview)
}

async fn fetch_page(url: &str) -> Option<(PageMeta, url::Url)> {
    let page_url = crate::net_guard::validate_public_url(url).await.ok()?;
    let resp = crate::net_guard::safe_http_client()
        .get(page_url.clone())
        .header("accept", "text/html,application/xhtml+xml")
        .header("user-agent", "Mozilla/5.0 (compatible; waxum-link-preview)")
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let is_html = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|ct| ct.contains("html"))
        .unwrap_or(false);
    if !is_html {
        return None;
    }
    let final_url = resp.url().clone();
    let html_bytes = read_capped(resp, MAX_HTML_BYTES, Some(b"</head>")).await?;
    let html = String::from_utf8_lossy(&html_bytes);
    Some((parse_meta(&html), final_url))
}

/// Reads the body up to `cap` bytes, stopping early once `stop_at` has
/// been seen (case-insensitively).
async fn read_capped(
    mut resp: reqwest::Response,
    cap: usize,
    stop_at: Option<&[u8]>,
) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    while let Some(chunk) = resp.chunk().await.ok()? {
        let room = cap.saturating_sub(out.len());
        if room == 0 {
            break;
        }
        let scan_from = out.len().saturating_sub(stop_at.map_or(0, <[u8]>::len));
        out.extend_from_slice(&chunk[..chunk.len().min(room)]);
        if let Some(marker) = stop_at {
            if out[scan_from..]
                .windows(marker.len())
                .any(|w| w.eq_ignore_ascii_case(marker))
            {
                break;
            }
        }
    }
    Some(out)
}

async fn fetch_thumbnail(url: &str) -> Option<(Vec<u8>, u32, u32)> {
    let url = crate::net_guard::validate_public_url(url).await.ok()?;
    let resp = crate::net_guard::safe_http_client()
        .get(url)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    if resp
        .content_length()
        .is_some_and(|n| n as usize > MAX_IMAGE_BYTES)
    {
        return None;
    }
    let bytes = read_capped(resp, MAX_IMAGE_BYTES + 1, None).await?;
    if bytes.len() > MAX_IMAGE_BYTES {
        return None;
    }
    tokio::task::spawn_blocking(move || encode_thumbnail(&bytes))
        .await
        .ok()
        .flatten()
}

/// Decodes any supported image and re-encodes it as a JPEG no larger than
/// [`THUMBNAIL_MAX_SIDE`] on its longer side.
fn encode_thumbnail(bytes: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    let img = image::load_from_memory(bytes).ok()?;
    let thumb = img
        .thumbnail(THUMBNAIL_MAX_SIDE, THUMBNAIL_MAX_SIDE)
        .to_rgb8();
    let (w, h) = thumb.dimensions();
    let mut out = Vec::new();
    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 70);
    thumb.write_with_encoder(encoder).ok()?;
    Some((out, w, h))
}

#[derive(Debug, Default, PartialEq)]
struct PageMeta {
    title: Option<String>,
    description: Option<String>,
    image: Option<String>,
}

/// Pulls title, description and image out of an HTML page, preferring
/// Open Graph, then Twitter card, then the plain `<title>` and
/// `<meta name="description">`. Only `<head>`-style tags are read; this is
/// a tag scanner, not a DOM parser, which is all preview metadata needs.
fn parse_meta(html: &str) -> PageMeta {
    let mut og = HashMap::new();
    let lower = html.to_ascii_lowercase();
    let mut pos = 0;
    while let Some(off) = lower[pos..].find("<meta") {
        let start = pos + off;
        let Some(end_off) = lower[start..].find('>') else {
            break;
        };
        let tag = &html[start..start + end_off];
        let attrs = parse_attrs(tag);
        let key = attrs
            .get("property")
            .or_else(|| attrs.get("name"))
            .map(|k| k.to_ascii_lowercase());
        if let (Some(key), Some(content)) = (key, attrs.get("content")) {
            let content = decode_entities(content).trim().to_string();
            if !content.is_empty() {
                og.entry(key).or_insert(content);
            }
        }
        pos = start + end_off;
    }

    let title_tag = lower.find("<title").and_then(|s| {
        let open_end = s + lower[s..].find('>')? + 1;
        let close = open_end + lower[open_end..].find("</title")?;
        let t = decode_entities(&html[open_end..close]).trim().to_string();
        (!t.is_empty()).then_some(t)
    });

    let pick = |keys: &[&str]| keys.iter().find_map(|k| og.get(*k).cloned());
    PageMeta {
        title: pick(&["og:title", "twitter:title"])
            .or(title_tag)
            .map(|t| clip(&t, 200)),
        description: pick(&["og:description", "twitter:description", "description"])
            .map(|d| clip(&d, 400)),
        image: pick(&[
            "og:image:secure_url",
            "og:image",
            "og:image:url",
            "twitter:image",
            "twitter:image:src",
        ]),
    }
}

fn clip(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars - 1).collect();
    out.push('…');
    out
}

/// Attributes of one tag, keys lowercased. Handles double-, single- and
/// unquoted values.
fn parse_attrs(tag: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let chars: Vec<char> = tag.chars().collect();
    let mut i = 0;
    while i < chars.len() && !chars[i].is_whitespace() {
        i += 1;
    }
    while i < chars.len() {
        while i < chars.len() && (chars[i].is_whitespace() || chars[i] == '/') {
            i += 1;
        }
        let key_start = i;
        while i < chars.len() && !chars[i].is_whitespace() && chars[i] != '=' && chars[i] != '/' {
            i += 1;
        }
        let key: String = chars[key_start..i]
            .iter()
            .collect::<String>()
            .to_ascii_lowercase();
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        if i < chars.len() && chars[i] == '=' {
            i += 1;
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            let value = if i < chars.len() && (chars[i] == '"' || chars[i] == '\'') {
                let quote = chars[i];
                i += 1;
                let v_start = i;
                while i < chars.len() && chars[i] != quote {
                    i += 1;
                }
                let v: String = chars[v_start..i].iter().collect();
                i += 1;
                v
            } else {
                let v_start = i;
                while i < chars.len() && !chars[i].is_whitespace() {
                    i += 1;
                }
                chars[v_start..i].iter().collect()
            };
            if !key.is_empty() {
                out.entry(key).or_insert(value);
            }
        } else if key.is_empty() {
            i += 1;
        }
    }
    out
}

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp..];
        let Some(semi) = after[..after.len().min(12)].find(';') else {
            out.push('&');
            rest = &after[1..];
            continue;
        };
        let entity = &after[1..semi];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" | "#39" => Some('\''),
            "nbsp" => Some(' '),
            _ if entity.starts_with("#x") || entity.starts_with("#X") => {
                u32::from_str_radix(&entity[2..], 16)
                    .ok()
                    .and_then(char::from_u32)
            }
            _ if entity.starts_with('#') => entity[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &after[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &after[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_url_finds_the_link_and_trims_sentence_punctuation() {
        assert_eq!(
            first_url("Read this: https://example.com/article."),
            Some("https://example.com/article")
        );
        assert_eq!(
            first_url("(see https://en.wikipedia.org/wiki/Rust_(programming_language))"),
            Some("https://en.wikipedia.org/wiki/Rust_(programming_language)")
        );
        assert_eq!(
            first_url("a http://x.io/a?b=1, then more"),
            Some("http://x.io/a?b=1")
        );
        assert_eq!(
            first_url("first http://a.io then https://b.io"),
            Some("http://a.io")
        );
        assert_eq!(first_url("no links here"), None);
        assert_eq!(first_url("broken https:// only"), None);
    }

    #[test]
    fn parse_meta_prefers_open_graph_and_decodes_entities() {
        let html = r#"<html><head>
            <title>Plain title</title>
            <meta name="description" content="Plain description">
            <meta property="og:title" content="Rust &amp; WhatsApp">
            <meta content='OG description &#8212; long' property='og:description' />
            <META PROPERTY="og:image" CONTENT="/img/card.png">
        </head></html>"#;
        let m = parse_meta(html);
        assert_eq!(m.title.as_deref(), Some("Rust & WhatsApp"));
        assert_eq!(
            m.description.as_deref(),
            Some("OG description \u{2014} long")
        );
        assert_eq!(m.image.as_deref(), Some("/img/card.png"));
    }

    #[test]
    fn parse_meta_falls_back_to_title_tag_and_meta_description() {
        let html = "<head><title> Hello &lt;world&gt; </title><meta name=description content=Short></head>";
        let m = parse_meta(html);
        assert_eq!(m.title.as_deref(), Some("Hello <world>"));
        assert_eq!(m.description.as_deref(), Some("Short"));
        assert_eq!(m.image, None);
    }

    #[test]
    fn apply_fills_the_preview_fields_and_keeps_the_text() {
        let mut m = waproto::whatsapp::message::ExtendedTextMessage {
            text: Some("Read this: https://example.com/a".to_string()),
            ..Default::default()
        };
        apply(
            &mut m,
            LinkPreview {
                matched_text: "https://example.com/a".to_string(),
                title: Some("Title".to_string()),
                description: Some("Desc".to_string()),
                jpeg_thumbnail: Some(vec![0xFF, 0xD8, 0xFF]),
                thumbnail_width: Some(300),
                thumbnail_height: Some(150),
            },
        );
        assert_eq!(m.text.as_deref(), Some("Read this: https://example.com/a"));
        assert_eq!(m.matched_text.as_deref(), Some("https://example.com/a"));
        assert_eq!(m.title.as_deref(), Some("Title"));
        assert_eq!(m.description.as_deref(), Some("Desc"));
        assert_eq!(m.thumbnail_width, Some(300));
        assert!(m.jpeg_thumbnail.is_some());
    }

    #[test]
    fn long_titles_are_clipped() {
        let long = "x".repeat(500);
        let html = format!(r#"<meta property="og:title" content="{long}">"#);
        assert_eq!(parse_meta(&html).title.unwrap().chars().count(), 200);
    }

    #[test]
    fn thumbnails_are_resized_to_a_small_jpeg() {
        let img = image::RgbImage::from_pixel(1200, 600, image::Rgb([10, 120, 200]));
        let mut png = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let (jpeg, w, h) = encode_thumbnail(&png).expect("thumbnail");
        assert_eq!((w, h), (300, 150));
        assert_eq!(&jpeg[..3], &[0xFF, 0xD8, 0xFF], "JPEG magic");
        assert!(encode_thumbnail(b"not an image").is_none());
    }

    #[tokio::test]
    async fn private_and_local_urls_are_never_fetched() {
        for text in [
            "http://127.0.0.1:1/admin",
            "http://169.254.169.254/latest/meta-data",
            "http://localhost:3451/api",
            "http://10.0.0.1/",
        ] {
            assert_eq!(for_text(text).await, None, "{text}");
        }
    }
}
