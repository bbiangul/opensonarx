use sha2::{Digest, Sha256};

/// Compute a deterministic shard ID from a URL.
///
/// Uses SHA-256 of the full URL, then takes the first 8 bytes as a u64
/// and computes `hash % num_shards`. Returns `"shard-{N}"`.
pub fn compute_shard_id(url: &str, num_shards: u32) -> String {
    assert!(num_shards > 0, "num_shards must be > 0");
    let hash = Sha256::digest(url.as_bytes());
    let val = u64::from_be_bytes(hash[..8].try_into().unwrap());
    let shard_num = (val % num_shards as u64) as u32;
    format!("shard-{shard_num}")
}

// ── Sub-sharding ────────────────────────────────────────────────────────

/// Default number of sub-shards per topic shard.
pub const DEFAULT_SUB_SHARDS: usize = 10;

/// Compute a sub-shard ID from a topic shard and content hash.
///
/// Takes the first 8 bytes of the content hash as a u64, mods by `num_sub_shards`,
/// and returns `"{topic_shard}/{bucket}"` (e.g. `"topic-technology/3"`).
pub fn compute_sub_shard_id(topic_shard: &str, content_hash: &str, num_sub_shards: usize) -> String {
    assert!(num_sub_shards > 0, "num_sub_shards must be > 0");
    let hash = Sha256::digest(content_hash.as_bytes());
    let val = u64::from_be_bytes(hash[..8].try_into().unwrap());
    let bucket = (val % num_sub_shards as u64) as usize;
    format!("{topic_shard}/{bucket}")
}

/// Extract the parent topic shard from a sub-shard ID.
///
/// `"topic-technology/3"` → `"topic-technology"`.
/// If no `/` suffix exists, returns the input unchanged.
pub fn parent_shard(sub_shard_id: &str) -> &str {
    sub_shard_id.rsplit_once('/').map_or(sub_shard_id, |(parent, _)| parent)
}

/// Check if a sub-shard belongs to a given parent shard.
///
/// `is_sub_shard_of("topic-technology/3", "topic-technology")` → `true`.
/// Also returns true for exact matches (backward compat).
pub fn is_sub_shard_of(sub_shard: &str, parent: &str) -> bool {
    sub_shard == parent || sub_shard.starts_with(&format!("{parent}/"))
}

// ── Topic-based shard assignment ────────────────────────────────────────

/// A built-in topic definition for embedding-based classification.
pub struct TopicDefinition {
    /// Shard ID assigned to this topic (e.g., "topic-technology")
    pub shard_id: &'static str,
    /// Human-readable topic name
    pub name: &'static str,
    /// Short description used to compute the centroid embedding
    pub description: &'static str,
}

/// The 10 built-in topic categories.
pub static BUILTIN_TOPICS: &[TopicDefinition] = &[
    TopicDefinition {
        shard_id: "topic-technology",
        name: "Technology",
        description: "software engineering, programming, computers, AI, tech industry",
    },
    TopicDefinition {
        shard_id: "topic-science",
        name: "Science",
        description: "physics, chemistry, biology, research, scientific discovery",
    },
    TopicDefinition {
        shard_id: "topic-health",
        name: "Health",
        description: "medicine, healthcare, fitness, nutrition, mental health",
    },
    TopicDefinition {
        shard_id: "topic-business",
        name: "Business",
        description: "finance, economics, startups, investing, corporate strategy",
    },
    TopicDefinition {
        shard_id: "topic-news",
        name: "News",
        description: "current events, politics, world affairs, breaking news, journalism",
    },
    TopicDefinition {
        shard_id: "topic-education",
        name: "Education",
        description: "learning, teaching, academic courses, university, tutorials",
    },
    TopicDefinition {
        shard_id: "topic-entertainment",
        name: "Entertainment",
        description: "movies, music, games, television, pop culture, celebrity",
    },
    TopicDefinition {
        shard_id: "topic-sports",
        name: "Sports",
        description: "football, basketball, soccer, athletics, tournaments, teams",
    },
    TopicDefinition {
        shard_id: "topic-lifestyle",
        name: "Lifestyle",
        description: "travel, food, fashion, home, relationships, personal development",
    },
    TopicDefinition {
        shard_id: "topic-general",
        name: "General",
        description: "general knowledge, reference, miscellaneous information",
    },
];

/// Cosine similarity between two vectors.
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    let denom = norm_a * norm_b;
    if denom < 1e-10 {
        0.0
    } else {
        dot / denom
    }
}

/// Embedding-based topic classifier.
///
/// Holds pre-computed centroid embeddings for each topic. Classification is
/// a simple argmax of cosine similarity between the chunk embedding and each
/// centroid.
pub struct TopicRouter {
    /// (shard_id, centroid_embedding) pairs
    centroids: Vec<(String, Vec<f32>)>,
    /// Minimum similarity to assign a topic; below this, falls back to "topic-general"
    threshold: f32,
}

impl TopicRouter {
    /// Create a new TopicRouter from pre-computed centroid embeddings.
    ///
    /// Each entry is `(shard_id, embedding)`. The caller is responsible for
    /// embedding the topic descriptions (keeps oi-types free of embedder deps).
    pub fn new(centroids: Vec<(String, Vec<f32>)>) -> Self {
        Self {
            centroids,
            threshold: 0.3,
        }
    }

    /// Set the minimum similarity threshold. Below this, content is classified
    /// as "topic-general".
    pub fn with_threshold(mut self, threshold: f32) -> Self {
        self.threshold = threshold;
        self
    }

    /// Classify an embedding into the best-matching topic shard.
    ///
    /// Returns "topic-general" if no centroid exceeds the threshold.
    pub fn classify(&self, embedding: &[f32]) -> &str {
        let (shard_id, _) = self.classify_with_confidence(embedding);
        shard_id
    }

    /// Rank all topic shards by cosine similarity to the given embedding.
    /// Returns `(shard_id, similarity)` sorted descending (best match first).
    pub fn rank_shards(&self, embedding: &[f32]) -> Vec<(&str, f32)> {
        let mut ranked: Vec<(&str, f32)> = self
            .centroids
            .iter()
            .map(|(shard_id, centroid)| (shard_id.as_str(), cosine_similarity(embedding, centroid)))
            .collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        ranked
    }

    /// Classify with confidence score.
    ///
    /// Returns `(shard_id, cosine_similarity)`.
    pub fn classify_with_confidence(&self, embedding: &[f32]) -> (&str, f32) {
        let mut best_shard = "topic-general";
        let mut best_score: f32 = -1.0;

        for (shard_id, centroid) in &self.centroids {
            let sim = cosine_similarity(embedding, centroid);
            if sim > best_score {
                best_score = sim;
                best_shard = shard_id;
            }
        }

        if best_score < self.threshold {
            ("topic-general", best_score)
        } else {
            (best_shard, best_score)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deterministic() {
        let a = compute_shard_id("https://example.com/page", 16);
        let b = compute_shard_id("https://example.com/page", 16);
        assert_eq!(a, b);
    }

    #[test]
    fn test_different_urls_may_differ() {
        let a = compute_shard_id("https://example.com/page-a", 1024);
        let b = compute_shard_id("https://example.com/page-b", 1024);
        // With 1024 shards and different URLs, they should differ (very high probability)
        // This is a probabilistic test, but SHA-256 should make collisions rare
        assert_ne!(a, b);
    }

    #[test]
    fn test_format() {
        let id = compute_shard_id("https://example.com", 8);
        assert!(id.starts_with("shard-"), "got: {id}");
        let num: u32 = id.strip_prefix("shard-").unwrap().parse().unwrap();
        assert!(num < 8);
    }

    #[test]
    fn test_single_shard() {
        let id = compute_shard_id("https://anything.com", 1);
        assert_eq!(id, "shard-0");
    }

    // ── Topic router tests ──────────────────────────────────────────────

    #[test]
    fn test_builtin_topics_count() {
        assert_eq!(BUILTIN_TOPICS.len(), 10);
        // Verify all shard_ids start with "topic-"
        for t in BUILTIN_TOPICS {
            assert!(t.shard_id.starts_with("topic-"), "bad shard_id: {}", t.shard_id);
        }
    }

    #[test]
    fn test_cosine_similarity() {
        // Identical vectors → 1.0
        let a = vec![1.0, 0.0, 0.0];
        assert!((cosine_similarity(&a, &a) - 1.0).abs() < 1e-6);

        // Orthogonal vectors → 0.0
        let b = vec![0.0, 1.0, 0.0];
        assert!(cosine_similarity(&a, &b).abs() < 1e-6);

        // Opposite vectors → -1.0
        let c = vec![-1.0, 0.0, 0.0];
        assert!((cosine_similarity(&a, &c) + 1.0).abs() < 1e-6);

        // Zero vector → 0.0
        let z = vec![0.0, 0.0, 0.0];
        assert!(cosine_similarity(&a, &z).abs() < 1e-6);
    }

    #[test]
    fn test_topic_router_classify() {
        let router = TopicRouter::new(vec![
            ("topic-technology".into(), vec![1.0, 0.0, 0.0, 0.0]),
            ("topic-science".into(), vec![0.0, 1.0, 0.0, 0.0]),
            ("topic-health".into(), vec![0.0, 0.0, 1.0, 0.0]),
        ]);

        // Close to technology
        assert_eq!(router.classify(&[0.9, 0.1, 0.0, 0.0]), "topic-technology");
        // Close to science
        assert_eq!(router.classify(&[0.1, 0.9, 0.0, 0.0]), "topic-science");
        // Close to health
        assert_eq!(router.classify(&[0.0, 0.1, 0.9, 0.0]), "topic-health");
    }

    #[test]
    fn test_classify_with_confidence() {
        let router = TopicRouter::new(vec![
            ("topic-technology".into(), vec![1.0, 0.0, 0.0]),
            ("topic-science".into(), vec![0.0, 1.0, 0.0]),
        ]);

        let (shard, confidence) = router.classify_with_confidence(&[1.0, 0.0, 0.0]);
        assert_eq!(shard, "topic-technology");
        assert!((confidence - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_classify_below_threshold_falls_back() {
        let router = TopicRouter::new(vec![
            ("topic-technology".into(), vec![1.0, 0.0, 0.0, 0.0]),
        ])
        .with_threshold(0.9);

        // This embedding is far from all centroids
        let result = router.classify(&[0.0, 0.0, 0.0, 1.0]);
        assert_eq!(result, "topic-general");
    }

    #[test]
    fn test_classify_empty_centroids() {
        let router = TopicRouter::new(vec![]);
        assert_eq!(router.classify(&[1.0, 0.0, 0.0]), "topic-general");
    }

    #[test]
    fn test_rank_shards() {
        let router = TopicRouter::new(vec![
            ("topic-technology".into(), vec![1.0, 0.0, 0.0, 0.0]),
            ("topic-science".into(), vec![0.0, 1.0, 0.0, 0.0]),
            ("topic-health".into(), vec![0.0, 0.0, 1.0, 0.0]),
        ]);

        // Embedding closest to science, then slightly tech
        let ranked = router.rank_shards(&[0.1, 0.9, 0.0, 0.0]);
        assert_eq!(ranked.len(), 3);
        assert_eq!(ranked[0].0, "topic-science");
        assert_eq!(ranked[1].0, "topic-technology");
        assert_eq!(ranked[2].0, "topic-health");
        // Best score should be high
        assert!(ranked[0].1 > ranked[1].1);
        assert!(ranked[1].1 > ranked[2].1);
    }

    #[test]
    fn test_rank_shards_empty() {
        let router = TopicRouter::new(vec![]);
        let ranked = router.rank_shards(&[1.0, 0.0, 0.0]);
        assert!(ranked.is_empty());
    }

    // ── Sub-sharding tests ────────────────────────────────────────────

    #[test]
    fn test_compute_sub_shard_id_format() {
        let id = compute_sub_shard_id("topic-technology", "abc123hash", 10);
        assert!(id.starts_with("topic-technology/"));
        let bucket: usize = id.strip_prefix("topic-technology/").unwrap().parse().unwrap();
        assert!(bucket < 10);
    }

    #[test]
    fn test_compute_sub_shard_id_deterministic() {
        let a = compute_sub_shard_id("topic-science", "hash1", 10);
        let b = compute_sub_shard_id("topic-science", "hash1", 10);
        assert_eq!(a, b);
    }

    #[test]
    fn test_compute_sub_shard_distributes_evenly() {
        let mut counts = [0usize; 10];
        for i in 0..1000 {
            let hash = format!("content-hash-{i}");
            let id = compute_sub_shard_id("topic-general", &hash, 10);
            let bucket: usize = id.strip_prefix("topic-general/").unwrap().parse().unwrap();
            counts[bucket] += 1;
        }
        // Each bucket should have roughly 100 ± 50 (at worst)
        for (i, &count) in counts.iter().enumerate() {
            assert!(
                count > 50 && count < 200,
                "bucket {i} has {count} entries (expected ~100)"
            );
        }
    }

    #[test]
    fn test_parent_shard() {
        assert_eq!(parent_shard("topic-technology/3"), "topic-technology");
        assert_eq!(parent_shard("topic-science/0"), "topic-science");
        assert_eq!(parent_shard("topic-general"), "topic-general"); // no sub-shard
    }

    #[test]
    fn test_is_sub_shard_of() {
        assert!(is_sub_shard_of("topic-technology/3", "topic-technology"));
        assert!(is_sub_shard_of("topic-technology/0", "topic-technology"));
        assert!(is_sub_shard_of("topic-technology", "topic-technology")); // exact match
        assert!(!is_sub_shard_of("topic-science/3", "topic-technology"));
        assert!(!is_sub_shard_of("topic-technology/3", "topic-tech")); // partial prefix
    }

    #[test]
    fn test_old_shard_ids_still_work() {
        // Old "topic-technology" shard IDs (without /N suffix) should work with is_sub_shard_of
        assert!(is_sub_shard_of("topic-technology", "topic-technology"));
    }
}
