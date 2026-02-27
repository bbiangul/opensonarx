use oi_types::{ChunkMeta, ContentChunk, CrawlMetadata, CrawlOutput, EmbeddingPipeline, Result, TopicRouter};
use oi_types::entity::entity_from_url;
use oi_types::traits::CrawlPipeline;
#[cfg(feature = "facts")]
use oi_types::traits::FactExtractionPipeline;
use sha2::{Digest, Sha256};
use std::sync::Arc;

use crate::chunker;
use crate::extractor;
use crate::fetcher;
use crate::quality;

fn domain_from_url(url: &str) -> String {
    url.split("//")
        .nth(1)
        .and_then(|s| s.split('/').next())
        .unwrap_or("")
        .to_string()
}

/// The Distiller: fetches, extracts, chunks, and quality-gates web content.
/// Optionally embeds chunks through an EmbeddingPipeline and extracts facts
/// through a FactExtractionPipeline.
pub struct Distiller {
    max_tokens: usize,
    overlap: usize,
    embedder: Option<Arc<dyn EmbeddingPipeline>>,
    topic_router: Option<Arc<TopicRouter>>,
    #[cfg(feature = "facts")]
    factifier: Option<Arc<dyn FactExtractionPipeline>>,
}

impl Distiller {
    pub fn new() -> Self {
        Self {
            max_tokens: 512,
            overlap: 128,
            embedder: None,
            topic_router: None,
            #[cfg(feature = "facts")]
            factifier: None,
        }
    }

    pub fn with_chunk_params(max_tokens: usize, overlap: usize) -> Self {
        Self {
            max_tokens,
            overlap,
            embedder: None,
            topic_router: None,
            #[cfg(feature = "facts")]
            factifier: None,
        }
    }

    /// Set an embedding pipeline to auto-embed chunks after chunking.
    pub fn with_embedder(mut self, embedder: Arc<dyn EmbeddingPipeline>) -> Self {
        self.embedder = Some(embedder);
        self
    }

    /// Set a topic router for classify-at-crawl. Each chunk with an embedding
    /// will be classified into a topic shard and have `shard_id` set in its metadata.
    pub fn with_topic_router(mut self, router: Arc<TopicRouter>) -> Self {
        self.topic_router = Some(router);
        self
    }

    /// Set a fact extraction pipeline to extract entities and facts at crawl time.
    #[cfg(feature = "facts")]
    pub fn with_factifier(mut self, factifier: Arc<dyn FactExtractionPipeline>) -> Self {
        self.factifier = Some(factifier);
        self
    }
}

impl Default for Distiller {
    fn default() -> Self {
        Self::new()
    }
}

/// Embed chunks in-place using an embedding pipeline.
/// Chunks without content are skipped.
pub fn embed_chunks(
    chunks: &mut [ContentChunk],
    embedder: &dyn EmbeddingPipeline,
) -> Result<()> {
    let texts: Vec<&str> = chunks.iter().map(|c| c.content.as_str()).collect();
    let embeddings = embedder.embed_text_batch(&texts)?;
    for (chunk, emb) in chunks.iter_mut().zip(embeddings) {
        chunk.embedding = Some(emb);
    }
    Ok(())
}

impl CrawlPipeline for Distiller {
    async fn crawl(&self, url: &str) -> Result<CrawlOutput> {
        // 1. Fetch
        let fetch_result = fetcher::fetch_url(url).await?;

        if fetch_result.status >= 400 {
            return Err(oi_types::Error::Internal(format!(
                "HTTP {} for {}",
                fetch_result.status, url
            )));
        }

        // 2. Extract
        let extracted = extractor::html_to_markdown(&fetch_result.html);
        let publish_date = extractor::extract_publish_date(&fetch_result.html);
        let published_at = publish_date
            .as_deref()
            .and_then(extractor::parse_date_to_timestamp);
        let markdown = extracted.markdown;

        // 3. Quality gate (before hash to avoid hashing rejected content)
        let quality = quality::check_quality(&markdown);
        if !quality.passed {
            return Err(oi_types::Error::Internal(format!(
                "Quality check failed: {}",
                quality.reason.unwrap_or_default()
            )));
        }

        // 4. Content hash (only for content that passed quality gate)
        let mut hasher = Sha256::new();
        hasher.update(markdown.as_bytes());
        let content_hash = format!("{:x}", hasher.finalize());

        // 5. Chunk
        let text_chunks = chunker::chunk_text(&markdown, self.max_tokens, self.overlap);
        let domain = domain_from_url(url);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // Batch-embed chunks if an embedder is configured
        let embeddings = if let Some(ref embedder) = self.embedder {
            let texts: Vec<&str> = text_chunks.iter().map(|s| s.as_str()).collect();
            match embedder.embed_text_batch(&texts) {
                Ok(embs) => embs.into_iter().map(Some).collect(),
                Err(e) => {
                    tracing::warn!("Embedding failed, continuing without: {e}");
                    vec![None; text_chunks.len()]
                }
            }
        } else {
            vec![None; text_chunks.len()]
        };

        let entity = entity_from_url(url);

        let chunks: Vec<ContentChunk> = text_chunks
            .into_iter()
            .zip(embeddings)
            .enumerate()
            .map(|(i, (text, emb))| {
                // Per-chunk content hash for accurate dedup (page hash stays in CrawlOutput)
                let mut chunk_hasher = Sha256::new();
                chunk_hasher.update(text.as_bytes());
                let chunk_hash = format!("{:x}", chunk_hasher.finalize());

                // Classify chunk into a topic shard if router + embedding are available,
                // then sub-shard using content hash for granular distribution
                let shard_id = match (&self.topic_router, &emb) {
                    (Some(router), Some(embedding)) => {
                        let topic = router.classify(embedding).to_string();
                        Some(oi_types::shard::compute_sub_shard_id(
                            &topic,
                            &chunk_hash,
                            oi_types::shard::DEFAULT_SUB_SHARDS,
                        ))
                    }
                    _ => None,
                };

                // Extract facts per-chunk if factifier is configured
                #[cfg(feature = "facts")]
                let chunk_facts = if let Some(ref factifier) = self.factifier {
                    match factifier.extract(&text) {
                        Ok(facts) => Some(facts),
                        Err(e) => {
                            tracing::warn!("Fact extraction failed for chunk {i}, continuing without: {e}");
                            None
                        }
                    }
                } else {
                    None
                };

                ContentChunk {
                    chunk_id: format!("{content_hash}_{i}"),
                    content: text,
                    embedding: emb,
                    seq: i as u64,
                    meta: Some(ChunkMeta {
                        url: url.to_string(),
                        title: extracted.title.clone().unwrap_or_default(),
                        domain: domain.clone(),
                        content_hash: chunk_hash,
                        crawled_at: Some(now),
                        content_type: Some(fetch_result.content_type.clone()),
                        entity: entity.clone(),
                        content_kind: None,
                        quality_score: Some(quality.quality_score),
                        spam_score: Some(quality.spam_score),
                        slop_score: Some(quality.slop_score),
                        published_at,
                        shard_id,
                        #[cfg(feature = "facts")]
                        facts: chunk_facts,
                        #[cfg(not(feature = "facts"))]
                        facts: None,
                    }),
                }
            })
            .collect();

        Ok(CrawlOutput {
            url: url.to_string(),
            markdown,
            chunks,
            metadata: CrawlMetadata {
                title: extracted.title,
                description: None,
                language: None,
                published_at,
                content_type: fetch_result.content_type,
                fetch_time_ms: fetch_result.fetch_time_ms,
            },
            content_hash,
            extraction_quality: quality.quality_score,
        })
    }
}

/// Process raw HTML content without fetching (for testing)
pub fn process_html(url: &str, html: &str, max_tokens: usize, overlap: usize) -> Result<CrawlOutput> {
    let extracted = extractor::html_to_markdown(html);
    let publish_date = extractor::extract_publish_date(html);
    let published_at = publish_date
        .as_deref()
        .and_then(extractor::parse_date_to_timestamp);
    let markdown = extracted.markdown;

    let quality = quality::check_quality(&markdown);
    if !quality.passed {
        return Err(oi_types::Error::Internal(format!(
            "Quality check failed: {}",
            quality.reason.unwrap_or_default()
        )));
    }

    let mut hasher = Sha256::new();
    hasher.update(markdown.as_bytes());
    let content_hash = format!("{:x}", hasher.finalize());

    let domain = domain_from_url(url);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let entity = entity_from_url(url);

    let text_chunks = chunker::chunk_text(&markdown, max_tokens, overlap);
    let chunks: Vec<ContentChunk> = text_chunks
        .into_iter()
        .enumerate()
        .map(|(i, text)| {
            // Per-chunk content hash for accurate dedup
            let mut chunk_hasher = Sha256::new();
            chunk_hasher.update(text.as_bytes());
            let chunk_hash = format!("{:x}", chunk_hasher.finalize());

            ContentChunk {
                chunk_id: format!("{content_hash}_{i}"),
                content: text,
                embedding: None,
                seq: i as u64,
                meta: Some(ChunkMeta {
                    url: url.to_string(),
                    title: extracted.title.clone().unwrap_or_default(),
                    domain: domain.clone(),
                    content_hash: chunk_hash,
                    crawled_at: Some(now),
                    content_type: Some("text/html".to_string()),
                    entity: entity.clone(),
                    content_kind: None,
                    quality_score: Some(quality.quality_score),
                    spam_score: Some(quality.spam_score),
                    slop_score: Some(quality.slop_score),
                    published_at,
                    ..Default::default()
                }),
            }
        })
        .collect();

    Ok(CrawlOutput {
        url: url.to_string(),
        markdown,
        chunks,
        metadata: CrawlMetadata {
            title: extracted.title,
            description: None,
            language: None,
            published_at,
            content_type: "text/html".to_string(),
            fetch_time_ms: 0,
        },
        content_hash,
        extraction_quality: quality.quality_score,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_html() -> String {
        "<html><head><title>Test Page</title></head><body>\
            <h1>Welcome</h1>\
            <p>Rust is a systems programming language focused on safety and concurrency. \
            Its ownership model prevents data races at compile time. The borrow checker ensures \
            references are always valid. Pattern matching provides exhaustive checks on enum variants. \
            Traits enable polymorphism without inheritance. Error handling uses Result and Option types \
            for explicit control flow. The standard library provides efficient collections like Vec, \
            HashMap, and BTreeMap. Cargo manages dependencies and builds reproducibly. Clippy catches \
            common mistakes and suggests improvements. Rustfmt enforces consistent formatting across \
            all projects in the ecosystem.</p>\
            </body></html>".to_string()
    }

    #[test]
    fn test_process_html() {
        let html = sample_html();
        let result = process_html("https://example.com", &html, 512, 128).unwrap();
        assert_eq!(result.url, "https://example.com");
        assert!(!result.chunks.is_empty());
        assert!(!result.content_hash.is_empty());
        assert!(result.extraction_quality > 0.0);
    }

    #[test]
    fn test_content_hash_deterministic() {
        let html = sample_html();
        let r1 = process_html("https://example.com", &html, 512, 128).unwrap();
        let r2 = process_html("https://example.com", &html, 512, 128).unwrap();
        assert_eq!(r1.content_hash, r2.content_hash);
    }

    #[test]
    fn test_quality_gate_rejects_short() {
        let html = "<html><body><p>Hi</p></body></html>";
        let result = process_html("https://example.com", html, 512, 128);
        assert!(result.is_err());
    }

    #[test]
    fn test_title_extraction() {
        let html = sample_html();
        let result = process_html("https://example.com", &html, 512, 128).unwrap();
        assert_eq!(result.metadata.title, Some("Test Page".to_string()));
    }

    #[test]
    fn test_embed_chunks() {
        use oi_embed::MockEmbedder;
        let embedder = MockEmbedder::default();
        let mut chunks = vec![
            ContentChunk {
                chunk_id: "c1".to_string(),
                content: "hello world".to_string(),
                embedding: None,
                seq: 0,
                meta: None,
            },
            ContentChunk {
                chunk_id: "c2".to_string(),
                content: "foo bar".to_string(),
                embedding: None,
                seq: 1,
                meta: None,
            },
        ];
        embed_chunks(&mut chunks, &embedder).unwrap();
        assert!(chunks[0].embedding.is_some());
        assert!(chunks[1].embedding.is_some());
        assert_eq!(chunks[0].embedding.as_ref().unwrap().len(), 384);
    }

    #[test]
    fn test_domain_from_url_standard() {
        assert_eq!(domain_from_url("https://example.com/path"), "example.com");
    }

    #[test]
    fn test_domain_from_url_with_port() {
        assert_eq!(domain_from_url("https://example.com:8080/path"), "example.com:8080");
    }

    #[test]
    fn test_domain_from_url_no_path() {
        assert_eq!(domain_from_url("https://example.com"), "example.com");
    }

    #[test]
    fn test_domain_from_url_no_protocol() {
        // Without //, returns empty string
        assert_eq!(domain_from_url("example.com/path"), "");
    }

    #[test]
    fn test_entity_propagation_youtube() {
        let html = "<html><head><title>Test Video</title></head><body>\
            <h1>Welcome</h1>\
            <p>Linear algebra forms the mathematical foundation for machine learning algorithms. \
            Vectors represent data points in high-dimensional spaces while matrices transform those \
            spaces through rotations, scaling, and projections. Eigenvalues reveal the principal \
            directions of variance in datasets. Singular value decomposition enables dimensionality \
            reduction and noise filtering in practical applications.</p>\
            </body></html>".to_string();
        let result = process_html(
            "https://youtube.com/c/3blue1brown/video123",
            &html,
            512,
            128,
        )
        .unwrap();

        // Entity should be populated from the URL
        for chunk in &result.chunks {
            let meta = chunk.meta.as_ref().unwrap();
            assert!(meta.entity.is_some(), "entity should be populated for YouTube URL");
            let entity = meta.entity.as_ref().unwrap();
            assert!(entity.contains("youtube.com"), "entity should contain youtube.com");
        }
    }

    #[test]
    fn test_entity_propagation_domain_only() {
        let html = sample_html();
        // Domain-only URL (no path) → entity_from_url returns None
        let result = process_html("https://example.com", &html, 512, 128).unwrap();

        for chunk in &result.chunks {
            let meta = chunk.meta.as_ref().unwrap();
            assert!(
                meta.entity.is_none(),
                "entity should be None for domain-only URL"
            );
        }
    }

    #[test]
    fn test_chunk_meta_domain_populated() {
        let html = sample_html();
        let result = process_html("https://example.com/page", &html, 512, 128).unwrap();
        for chunk in &result.chunks {
            let meta = chunk.meta.as_ref().unwrap();
            assert_eq!(meta.domain, "example.com");
            assert_eq!(meta.url, "https://example.com/page");
            assert!(meta.crawled_at.is_some());
        }
    }
}
