use oi_types::entity::EntityRef;
use oi_types::{ChunkMeta, ContentChunk, CrawlMetadata, CrawlOutput};
use sha2::{Digest, Sha256};

use crate::adapter::{DiscoveredUrl, PlatformAdapter};
use crate::chunker;
use crate::quality;

/// Reddit adapter — discovers content via free RSS feeds, extracts post
/// content via Reddit's JSON API (append `.json` to any URL).
pub struct RedditAdapter;

impl RedditAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for RedditAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl PlatformAdapter for RedditAdapter {
    fn handles_url(&self, url: &str) -> bool {
        matches!(
            crate::adapter::extract_host(url).as_deref(),
            Some("reddit.com" | "www.reddit.com" | "old.reddit.com" | "new.reddit.com")
        )
    }

    fn extract_entity(&self, url: &str) -> Option<EntityRef> {
        EntityRef::parse(url)
            .ok()
            .filter(|e| e.entity_path().is_some())
    }

    async fn discover_content(&self, entity: &EntityRef) -> Result<Vec<DiscoveredUrl>, String> {
        let path = entity.entity_path().unwrap_or("");

        // Fetch RSS feed for subreddit or user
        let feed_url = format!("https://www.reddit.com/{path}/.rss");
        let feed_xml = fetch_text(&feed_url).await?;
        let urls = parse_reddit_feed(&feed_xml);

        Ok(urls)
    }

    async fn crawl_content(&self, url: &str) -> Result<CrawlOutput, String> {
        // Insert .json before query string (if any) to get Reddit's JSON API response
        let json_url = match url.find('?') {
            Some(pos) => format!("{}.json{}", &url[..pos], &url[pos..]),
            None => format!("{url}.json"),
        };
        let json_text = fetch_text(&json_url).await?;

        let (title, body, subreddit) = parse_reddit_post_json(&json_text)?;

        // Build markdown from title + body + top comments
        let markdown = format!("# {title}\n\n{body}");

        // Quality check with social profile
        let qc =
            quality::check_quality_with_profile(&markdown, quality::QualityProfile::SocialPost);
        if !qc.passed {
            return Err(format!(
                "Quality check failed: {}",
                qc.reason.unwrap_or_default()
            ));
        }

        let mut hasher = Sha256::new();
        hasher.update(markdown.as_bytes());
        let content_hash = format!("{:x}", hasher.finalize());

        let entity = subreddit
            .as_ref()
            .map(|sub| format!("reddit.com/r/{}", sub.to_lowercase()));
        let domain = "reddit.com".to_string();
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
                    content_kind: Some("post".to_string()),
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
                description: None,
                language: None,
                published_at: None,
                content_type: "application/json".to_string(),
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

/// Parse Reddit RSS/Atom feed to discover post URLs.
fn parse_reddit_feed(xml: &str) -> Vec<DiscoveredUrl> {
    let mut urls = Vec::new();

    // Reddit RSS uses <entry> with <link href="..."/>
    for entry in xml.split("<entry>").skip(1) {
        let title = extract_xml_content(entry, "title");
        let link = extract_link_href(entry);

        if let Some(url) = link {
            // Skip external links, keep reddit.com post links
            if url.contains("reddit.com") {
                urls.push(DiscoveredUrl {
                    url,
                    title,
                    content_kind: Some("post".to_string()),
                });
            }
        }
    }

    urls
}

/// Parse Reddit's JSON API response for a post.
/// Returns (title, body_text, subreddit).
fn parse_reddit_post_json(json: &str) -> Result<(String, String, Option<String>), String> {
    let parsed: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("JSON parse error: {e}"))?;

    // Reddit JSON for a post is an array: [listing, comments]
    let listing = parsed
        .as_array()
        .and_then(|arr| arr.first())
        .ok_or("Expected array response from Reddit JSON API")?;

    let post = listing
        .get("data")
        .and_then(|d| d.get("children"))
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|child| child.get("data"))
        .ok_or("Could not extract post data from Reddit JSON")?;

    let title = post
        .get("title")
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();

    let body = post
        .get("selftext")
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();

    let subreddit = post
        .get("subreddit")
        .and_then(|s| s.as_str())
        .map(|s| s.to_string());

    Ok((title, body, subreddit))
}

/// Extract text content from an XML tag.
fn extract_xml_content(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    let content = xml[start..end].trim();
    // Handle CDATA
    let content = content
        .strip_prefix("<![CDATA[")
        .and_then(|s| s.strip_suffix("]]>"))
        .unwrap_or(content);
    Some(content.to_string())
}

/// Extract href attribute from a <link> tag.
fn extract_link_href(xml: &str) -> Option<String> {
    let link_start = xml.find("<link")?;
    let rest = &xml[link_start..];
    let href_start = rest.find("href=\"")? + "href=\"".len();
    let href_rest = &rest[href_start..];
    let end = href_rest.find('"')?;
    Some(href_rest[..end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_handles_url() {
        let adapter = RedditAdapter::new();
        assert!(adapter.handles_url("https://reddit.com/r/rust"));
        assert!(adapter.handles_url("https://www.reddit.com/r/rust"));
        assert!(adapter.handles_url("https://old.reddit.com/r/rust"));
        assert!(!adapter.handles_url("https://youtube.com/c/test"));
        // Must NOT match substring false positives
        assert!(!adapter.handles_url("https://notreddit.com/page"));
        assert!(!adapter.handles_url("https://example.com/reddit.com"));
    }

    #[test]
    fn test_extract_entity() {
        let adapter = RedditAdapter::new();
        let entity = adapter.extract_entity("https://reddit.com/r/rust").unwrap();
        assert_eq!(entity.domain(), "reddit.com");
        assert_eq!(entity.entity_path(), Some("r/rust"));
    }

    #[test]
    fn test_parse_reddit_feed() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed>
  <title>r/rust</title>
  <entry>
    <title>How to use async/await in Rust</title>
    <link href="https://www.reddit.com/r/rust/comments/abc123/how_to_use_async/"/>
  </entry>
  <entry>
    <title>Announcing tokio 2.0</title>
    <link href="https://www.reddit.com/r/rust/comments/def456/announcing_tokio/"/>
  </entry>
</feed>"#;

        let urls = parse_reddit_feed(xml);
        assert_eq!(urls.len(), 2);
        assert_eq!(
            urls[0].url,
            "https://www.reddit.com/r/rust/comments/abc123/how_to_use_async/"
        );
        assert_eq!(
            urls[0].title.as_deref(),
            Some("How to use async/await in Rust")
        );
        assert_eq!(urls[0].content_kind.as_deref(), Some("post"));
    }

    #[test]
    fn test_parse_reddit_post_json() {
        let json = r#"[
            {
                "data": {
                    "children": [
                        {
                            "data": {
                                "title": "Announcing Rust 2.0",
                                "selftext": "We are excited to announce the release of Rust 2.0 with many improvements.",
                                "subreddit": "rust"
                            }
                        }
                    ]
                }
            },
            {
                "data": {
                    "children": []
                }
            }
        ]"#;

        let (title, body, subreddit) = parse_reddit_post_json(json).unwrap();
        assert_eq!(title, "Announcing Rust 2.0");
        assert!(body.contains("excited to announce"));
        assert_eq!(subreddit.as_deref(), Some("rust"));
    }

    #[test]
    fn test_json_url_with_query_params() {
        // Verify .json is inserted before query string, not appended after
        let url = "https://www.reddit.com/r/rust/comments/abc?sort=top";
        let json_url = match url.find('?') {
            Some(pos) => format!("{}.json{}", &url[..pos], &url[pos..]),
            None => format!("{url}.json"),
        };
        assert_eq!(
            json_url,
            "https://www.reddit.com/r/rust/comments/abc.json?sort=top"
        );

        // Without query string, .json appended normally
        let url2 = "https://www.reddit.com/r/rust/comments/abc";
        let json_url2 = match url2.find('?') {
            Some(pos) => format!("{}.json{}", &url2[..pos], &url2[pos..]),
            None => format!("{url2}.json"),
        };
        assert_eq!(json_url2, "https://www.reddit.com/r/rust/comments/abc.json");
    }

    #[test]
    fn test_extract_link_href() {
        let xml = r#"<link href="https://www.reddit.com/r/rust/comments/abc123/"/>"#;
        assert_eq!(
            extract_link_href(xml),
            Some("https://www.reddit.com/r/rust/comments/abc123/".to_string())
        );
    }
}
