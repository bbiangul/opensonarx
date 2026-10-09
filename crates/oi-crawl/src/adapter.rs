use oi_types::CrawlOutput;
use oi_types::entity::EntityRef;

/// A URL discovered by a platform adapter for crawling.
#[derive(Debug, Clone)]
pub struct DiscoveredUrl {
    pub url: String,
    pub title: Option<String>,
    pub content_kind: Option<String>,
}

/// Trait for platform-specific content adapters.
///
/// Each platform (YouTube, Reddit, etc.) implements this trait to handle
/// content discovery via RSS/JSON feeds and platform-specific extraction.
#[async_trait::async_trait]
pub trait PlatformAdapter: Send + Sync {
    /// Check if this adapter handles a given URL.
    fn handles_url(&self, url: &str) -> bool;

    /// Extract an entity reference from a URL (e.g., channel, subreddit).
    fn extract_entity(&self, url: &str) -> Option<EntityRef>;

    /// Discover content URLs for an entity (via RSS feed, API, etc.).
    async fn discover_content(&self, entity: &EntityRef) -> Result<Vec<DiscoveredUrl>, String>;

    /// Crawl a specific content URL and produce structured output.
    async fn crawl_content(&self, url: &str) -> Result<CrawlOutput, String>;
}

/// Extract the lowercase host from a URL, stripping protocol and port.
pub fn extract_host(url: &str) -> Option<String> {
    let without_proto = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    let host = without_proto.split('/').next()?;
    let host = host.split(':').next()?; // strip port
    if host.is_empty() {
        return None;
    }
    Some(host.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_host_https() {
        assert_eq!(
            extract_host("https://example.com/path"),
            Some("example.com".to_string())
        );
    }

    #[test]
    fn test_extract_host_http() {
        assert_eq!(
            extract_host("http://example.com/path"),
            Some("example.com".to_string())
        );
    }

    #[test]
    fn test_extract_host_no_protocol() {
        assert_eq!(
            extract_host("example.com/path"),
            Some("example.com".to_string())
        );
    }

    #[test]
    fn test_extract_host_with_port() {
        assert_eq!(
            extract_host("https://example.com:8080/path"),
            Some("example.com".to_string())
        );
    }

    #[test]
    fn test_extract_host_uppercase() {
        assert_eq!(
            extract_host("https://EXAMPLE.COM/path"),
            Some("example.com".to_string())
        );
    }

    #[test]
    fn test_extract_host_empty() {
        assert_eq!(extract_host(""), None);
    }

    #[test]
    fn test_extract_host_protocol_only() {
        assert_eq!(extract_host("https://"), None);
    }

    #[test]
    fn test_extract_host_subdomain() {
        assert_eq!(
            extract_host("https://www.example.com/path"),
            Some("www.example.com".to_string())
        );
    }
}
