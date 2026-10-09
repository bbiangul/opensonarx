use oi_types::entity::EntityRef;
use oi_types::{ChunkMeta, ContentChunk, CrawlMetadata, CrawlOutput};
use sha2::{Digest, Sha256};

use crate::adapter::{DiscoveredUrl, PlatformAdapter};
use crate::chunker;
use crate::quality;

/// YouTube adapter — discovers content via free RSS feeds, extracts metadata
/// from video page HTML (LD+JSON structured data, no JS rendering needed).
pub struct YouTubeAdapter;

impl YouTubeAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for YouTubeAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl PlatformAdapter for YouTubeAdapter {
    fn handles_url(&self, url: &str) -> bool {
        matches!(
            crate::adapter::extract_host(url).as_deref(),
            Some("youtube.com" | "www.youtube.com" | "m.youtube.com" | "youtu.be")
        )
    }

    fn extract_entity(&self, url: &str) -> Option<EntityRef> {
        EntityRef::parse(url)
            .ok()
            .filter(|e| e.entity_path().is_some())
    }

    async fn discover_content(&self, entity: &EntityRef) -> Result<Vec<DiscoveredUrl>, String> {
        let path = entity.entity_path().unwrap_or("");

        // Resolve channel ID from path
        let channel_id = if path.starts_with("channel/") {
            path.strip_prefix("channel/").unwrap_or("").to_string()
        } else {
            // For /c/Name or /@handle, we need to resolve the channel page
            // to find the channel ID. Fetch the channel page HTML.
            let page_url = format!("https://youtube.com/{path}");
            let html = fetch_text(&page_url).await?;
            extract_channel_id(&html)
                .ok_or_else(|| format!("Could not resolve channel ID for {path}"))?
        };

        // Validate channel ID format to prevent URL injection
        if !channel_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(format!("Invalid channel ID: {channel_id}"));
        }

        // Fetch RSS feed
        let feed_url = format!("https://www.youtube.com/feeds/videos.xml?channel_id={channel_id}");
        let feed_xml = fetch_text(&feed_url).await?;
        let urls = parse_youtube_feed(&feed_xml);

        Ok(urls)
    }

    async fn crawl_content(&self, url: &str) -> Result<CrawlOutput, String> {
        let html = fetch_text(url).await?;

        // Extract structured data from LD+JSON or meta tags
        let title = extract_meta_content(&html, "og:title")
            .or_else(|| extract_title_tag(&html))
            .unwrap_or_default();
        let description = extract_meta_content(&html, "og:description").unwrap_or_default();

        // Build markdown from title + description
        let markdown = format!("# {title}\n\n{description}");

        // Quality check with social profile
        let qc = quality::check_quality_with_profile(
            &markdown,
            quality::QualityProfile::VideoDescription,
        );
        if !qc.passed {
            return Err(format!(
                "Quality check failed: {}",
                qc.reason.unwrap_or_default()
            ));
        }

        let mut hasher = Sha256::new();
        hasher.update(markdown.as_bytes());
        let content_hash = format!("{:x}", hasher.finalize());

        let entity = oi_types::entity::entity_from_url(url);
        let domain = "youtube.com".to_string();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let text_chunks = chunker::chunk_text(&markdown, 512, 128);
        let chunks: Vec<ContentChunk> = text_chunks
            .into_iter()
            .enumerate()
            .map(|(i, text)| ContentChunk {
                chunk_id: format!("{content_hash}_{i}"),
                content: text,
                embedding: None,
                seq: i as u64,
                meta: Some(ChunkMeta {
                    url: url.to_string(),
                    title: title.clone(),
                    domain: domain.clone(),
                    content_hash: content_hash.clone(),
                    crawled_at: Some(now),
                    content_type: Some("text/html".to_string()),
                    entity: entity.clone(),
                    content_kind: Some("video".to_string()),
                    quality_score: Some(qc.quality_score),
                    spam_score: Some(qc.spam_score),
                    slop_score: Some(qc.slop_score),
                    published_at: None,
                    ..Default::default()
                }),
            })
            .collect();

        Ok(CrawlOutput {
            url: url.to_string(),
            markdown,
            chunks,
            metadata: CrawlMetadata {
                title: Some(title),
                description: Some(description),
                language: None,
                published_at: None,
                content_type: "text/html".to_string(),
                fetch_time_ms: 0,
            },
            content_hash,
            extraction_quality: qc.quality_score,
        })
    }
}

/// Fetch text content from a URL.
async fn fetch_text(url: &str) -> Result<String, String> {
    let resp = crate::fetcher::fetch_url(url)
        .await
        .map_err(|e| e.to_string())?;
    if resp.status >= 400 {
        return Err(format!("HTTP {} for {}", resp.status, url));
    }
    Ok(resp.html)
}

/// Extract channel ID from a YouTube page HTML (looks for channelId in various places).
fn extract_channel_id(html: &str) -> Option<String> {
    // Look for "channelId":"UC..." pattern
    if let Some(pos) = html.find("\"channelId\":\"") {
        let start = pos + "\"channelId\":\"".len();
        let rest = &html[start..];
        if let Some(end) = rest.find('"') {
            let id = &rest[..end];
            if id.starts_with("UC") {
                return Some(id.to_string());
            }
        }
    }
    // Look for /channel/UC... in links
    if let Some(pos) = html.find("/channel/UC") {
        let start = pos + "/channel/".len();
        let rest = &html[start..];
        let end = rest
            .find(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
            .unwrap_or(rest.len());
        let id = &rest[..end];
        if !id.is_empty() {
            return Some(id.to_string());
        }
    }
    None
}

/// Parse YouTube RSS feed XML and extract video URLs.
fn parse_youtube_feed(xml: &str) -> Vec<DiscoveredUrl> {
    let mut urls = Vec::new();

    // Simple XML parsing — look for <entry> blocks with <link> and <title>
    for entry in xml.split("<entry>").skip(1) {
        let video_id = extract_xml_tag(entry, "yt:videoId");
        let title = extract_xml_tag(entry, "title");

        if let Some(vid) = video_id {
            urls.push(DiscoveredUrl {
                url: format!("https://www.youtube.com/watch?v={vid}"),
                title,
                content_kind: Some("video".to_string()),
            });
        }
    }

    urls
}

/// Extract content from a simple XML tag.
fn extract_xml_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(xml[start..end].trim().to_string())
}

/// Extract content from HTML meta tags (og:title, og:description, etc.).
/// Handles both `<meta property="..." content="...">` and
/// `<meta content="..." property="...">` attribute orders.
fn extract_meta_content(html: &str, property: &str) -> Option<String> {
    let prop_pattern = format!("property=\"{property}\"");
    let mut search_from = 0;
    while let Some(rel_pos) = html[search_from..].find(&prop_pattern) {
        let abs_pos = search_from + rel_pos;
        // Find the enclosing <meta tag
        let tag_start = html[..abs_pos].rfind('<')?;
        let tag_end_rel = html[abs_pos..].find('>')?;
        let tag = &html[tag_start..abs_pos + tag_end_rel + 1];

        // Extract content="..." from within this tag
        if let Some(content_start) = tag.find("content=\"") {
            let value_start = content_start + "content=\"".len();
            let value_rest = &tag[value_start..];
            if let Some(end) = value_rest.find('"') {
                return Some(html_decode(&value_rest[..end]));
            }
        }
        search_from = abs_pos + prop_pattern.len();
    }
    None
}

/// Extract <title> tag content.
fn extract_title_tag(html: &str) -> Option<String> {
    let start = html.find("<title>")? + "<title>".len();
    let end = html[start..].find("</title>")? + start;
    Some(html_decode(html[start..end].trim()))
}

/// Basic HTML entity decoding.
fn html_decode(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#34;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_handles_url() {
        let adapter = YouTubeAdapter::new();
        assert!(adapter.handles_url("https://youtube.com/watch"));
        assert!(adapter.handles_url("https://www.youtube.com/c/test"));
        assert!(adapter.handles_url("https://youtu.be/abc123"));
        assert!(adapter.handles_url("https://m.youtube.com/watch"));
        assert!(!adapter.handles_url("https://reddit.com/r/rust"));
        // Must NOT match substring false positives
        assert!(!adapter.handles_url("https://notyoutube.com/page"));
        assert!(!adapter.handles_url("https://example.com/youtube.com"));
    }

    #[test]
    fn test_extract_entity() {
        let adapter = YouTubeAdapter::new();
        let entity = adapter
            .extract_entity("https://youtube.com/c/3Blue1Brown")
            .unwrap();
        assert_eq!(entity.domain(), "youtube.com");
        assert_eq!(entity.entity_path(), Some("c/3blue1brown"));
    }

    #[test]
    fn test_extract_entity_domain_only() {
        let adapter = YouTubeAdapter::new();
        // Domain-only URL should return None (no entity path)
        assert!(adapter.extract_entity("https://youtube.com").is_none());
    }

    #[test]
    fn test_parse_youtube_feed() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed>
  <title>3Blue1Brown</title>
  <entry>
    <yt:videoId>abc123</yt:videoId>
    <title>Linear Algebra Explained</title>
  </entry>
  <entry>
    <yt:videoId>def456</yt:videoId>
    <title>Calculus Deep Dive</title>
  </entry>
</feed>"#;

        let urls = parse_youtube_feed(xml);
        assert_eq!(urls.len(), 2);
        assert_eq!(urls[0].url, "https://www.youtube.com/watch?v=abc123");
        assert_eq!(urls[0].title.as_deref(), Some("Linear Algebra Explained"));
        assert_eq!(urls[0].content_kind.as_deref(), Some("video"));
        assert_eq!(urls[1].url, "https://www.youtube.com/watch?v=def456");
    }

    #[test]
    fn test_extract_channel_id() {
        let html =
            r#"<script>var ytInitialData = {"channelId":"UCxxxx123","title":"Test"};</script>"#;
        assert_eq!(extract_channel_id(html), Some("UCxxxx123".to_string()));
    }

    #[test]
    fn test_extract_channel_id_from_link() {
        let html = r#"<link rel="canonical" href="https://www.youtube.com/channel/UCsXVk37bltHxD1rDPwtNM8Q">"#;
        assert_eq!(
            extract_channel_id(html),
            Some("UCsXVk37bltHxD1rDPwtNM8Q".to_string())
        );
    }

    #[test]
    fn test_extract_meta_content() {
        let html = r#"<meta property="og:title" content="My Video Title">"#;
        assert_eq!(
            extract_meta_content(html, "og:title"),
            Some("My Video Title".to_string())
        );
    }

    #[test]
    fn test_extract_meta_content_reversed_attrs() {
        // content before property — must still work
        let html = r#"<meta content="Reversed Title" property="og:title">"#;
        assert_eq!(
            extract_meta_content(html, "og:title"),
            Some("Reversed Title".to_string())
        );
    }

    #[test]
    fn test_html_decode() {
        assert_eq!(html_decode("A &amp; B"), "A & B");
        assert_eq!(html_decode("&lt;tag&gt;"), "<tag>");
        assert_eq!(html_decode("&#34;quoted&#34;"), "\"quoted\"");
        assert_eq!(html_decode("&apos;apos&apos;"), "'apos'");
        assert_eq!(html_decode("hello&nbsp;world"), "hello world");
    }

    #[test]
    fn test_extract_title_tag() {
        let html = "<html><head><title>My Video Title</title></head><body></body></html>";
        assert_eq!(extract_title_tag(html), Some("My Video Title".to_string()));
    }

    #[test]
    fn test_extract_title_tag_with_whitespace() {
        let html = "<html><head><title>  Spaced Title  </title></head></html>";
        assert_eq!(extract_title_tag(html), Some("Spaced Title".to_string()));
    }

    #[test]
    fn test_extract_title_tag_missing() {
        let html = "<html><head></head><body></body></html>";
        assert!(extract_title_tag(html).is_none());
    }

    #[test]
    fn test_extract_title_tag_with_entities() {
        let html = "<title>A &amp; B</title>";
        assert_eq!(extract_title_tag(html), Some("A & B".to_string()));
    }

    #[test]
    fn test_extract_channel_id_none() {
        let html = "<html><body>no channel here</body></html>";
        assert!(extract_channel_id(html).is_none());
    }

    #[test]
    fn test_extract_channel_id_invalid_prefix() {
        // channelId that doesn't start with UC should not match
        let html = r#"{"channelId":"ABCnotvalid"}"#;
        assert!(extract_channel_id(html).is_none());
    }

    #[test]
    fn test_parse_youtube_feed_empty() {
        let xml = r#"<?xml version="1.0"?><feed></feed>"#;
        assert!(parse_youtube_feed(xml).is_empty());
    }

    #[test]
    fn test_extract_xml_tag() {
        let xml = "<entry><yt:videoId>abc123</yt:videoId><title>Test</title></entry>";
        assert_eq!(
            extract_xml_tag(xml, "yt:videoId"),
            Some("abc123".to_string())
        );
        assert_eq!(extract_xml_tag(xml, "title"), Some("Test".to_string()));
        assert_eq!(extract_xml_tag(xml, "missing"), None);
    }

    #[test]
    fn test_channel_id_validation() {
        // Channel ID with special chars should be rejected in discover_content logic
        // Here we test the validation indirectly via the extract_channel_id function
        // which only extracts IDs starting with "UC" and containing valid chars
        let html = r#"<a href="/channel/UCvalid_id-123">link</a>"#;
        assert_eq!(extract_channel_id(html), Some("UCvalid_id-123".to_string()));
    }
}
