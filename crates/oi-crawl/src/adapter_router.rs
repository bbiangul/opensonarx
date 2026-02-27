use oi_types::entity::EntityRef;
use oi_types::CrawlOutput;

use crate::adapter::{DiscoveredUrl, PlatformAdapter};
use crate::pipeline::Distiller;
use oi_types::traits::CrawlPipeline;

/// Routes URLs to the correct platform adapter, falling back to the
/// generic `Distiller` pipeline for regular web pages.
pub struct AdapterRouter {
    adapters: Vec<Box<dyn PlatformAdapter>>,
    fallback: Distiller,
}

impl AdapterRouter {
    pub fn new() -> Self {
        Self {
            adapters: Vec::new(),
            fallback: Distiller::new(),
        }
    }

    /// Register a platform adapter.
    pub fn add_adapter(&mut self, adapter: Box<dyn PlatformAdapter>) {
        self.adapters.push(adapter);
    }

    /// Check if any adapter handles this URL.
    pub fn has_adapter(&self, url: &str) -> bool {
        self.adapters.iter().any(|a| a.handles_url(url))
    }

    /// Extract entity from a URL using the appropriate adapter.
    pub fn extract_entity(&self, url: &str) -> Option<EntityRef> {
        self.adapters
            .iter()
            .find(|a| a.handles_url(url))
            .and_then(|a| a.extract_entity(url))
    }

    /// Discover content URLs for an entity using the appropriate adapter.
    pub async fn discover_content(&self, entity: &EntityRef) -> Result<Vec<DiscoveredUrl>, String> {
        let entity_url = format!("https://{}", entity.canonical());
        for adapter in &self.adapters {
            if adapter.handles_url(&entity_url) {
                return adapter.discover_content(entity).await;
            }
        }
        Err(format!("No adapter for entity: {}", entity.canonical()))
    }

    /// Crawl a URL using the appropriate adapter or the fallback distiller.
    pub async fn crawl(&self, url: &str) -> Result<CrawlOutput, String> {
        for adapter in &self.adapters {
            if adapter.handles_url(url) {
                return adapter.crawl_content(url).await;
            }
        }
        // Fallback to generic distiller
        self.fallback.crawl(url).await.map_err(|e| e.to_string())
    }
}

impl Default for AdapterRouter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::youtube::YouTubeAdapter;
    use crate::adapters::reddit::RedditAdapter;

    #[test]
    fn test_router_no_adapters() {
        let router = AdapterRouter::new();
        assert!(!router.has_adapter("https://youtube.com/watch?v=abc"));
        assert!(router.extract_entity("https://youtube.com/c/test").is_none());
    }

    #[test]
    fn test_router_with_youtube_adapter() {
        let mut router = AdapterRouter::new();
        router.add_adapter(Box::new(YouTubeAdapter::new()));

        assert!(router.has_adapter("https://youtube.com/watch?v=abc"));
        assert!(!router.has_adapter("https://reddit.com/r/rust"));
        assert!(!router.has_adapter("https://example.com"));
    }

    #[test]
    fn test_router_with_multiple_adapters() {
        let mut router = AdapterRouter::new();
        router.add_adapter(Box::new(YouTubeAdapter::new()));
        router.add_adapter(Box::new(RedditAdapter::new()));

        assert!(router.has_adapter("https://youtube.com/watch?v=abc"));
        assert!(router.has_adapter("https://reddit.com/r/rust"));
        assert!(!router.has_adapter("https://example.com"));
    }

    #[test]
    fn test_router_extract_entity_youtube() {
        let mut router = AdapterRouter::new();
        router.add_adapter(Box::new(YouTubeAdapter::new()));

        let entity = router.extract_entity("https://youtube.com/c/3Blue1Brown");
        assert!(entity.is_some());
        let e = entity.unwrap();
        assert_eq!(e.domain(), "youtube.com");
        assert_eq!(e.entity_path(), Some("c/3blue1brown"));
    }

    #[test]
    fn test_router_extract_entity_reddit() {
        let mut router = AdapterRouter::new();
        router.add_adapter(Box::new(RedditAdapter::new()));

        let entity = router.extract_entity("https://reddit.com/r/rust");
        assert!(entity.is_some());
        let e = entity.unwrap();
        assert_eq!(e.domain(), "reddit.com");
        assert_eq!(e.entity_path(), Some("r/rust"));
    }

    #[test]
    fn test_router_extract_entity_no_match() {
        let mut router = AdapterRouter::new();
        router.add_adapter(Box::new(YouTubeAdapter::new()));

        // No adapter for example.com
        assert!(router.extract_entity("https://example.com/page").is_none());
    }

    #[test]
    fn test_router_domain_only_no_entity() {
        let mut router = AdapterRouter::new();
        router.add_adapter(Box::new(YouTubeAdapter::new()));

        // youtube.com with no sub-path → no entity
        assert!(router.extract_entity("https://youtube.com").is_none());
    }

    #[test]
    fn test_router_default_impl() {
        let router = AdapterRouter::default();
        assert!(!router.has_adapter("https://example.com"));
    }
}
