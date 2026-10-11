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
pub fn compute_sub_shard_id(
    topic_shard: &str,
    content_hash: &str,
    num_sub_shards: usize,
) -> String {
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
    sub_shard_id
        .rsplit_once('/')
        .map_or(sub_shard_id, |(parent, _)| parent)
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
    if denom < 1e-10 { 0.0 } else { dot / denom }
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

// ── Routing learned from stored content ─────────────────────────────────

fn unit(vector: &[f32]) -> Vec<f32> {
    let norm = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        vector.iter().map(|x| x / norm).collect()
    } else {
        vector.to_vec()
    }
}

/// Spherical k-means with a fixed start: the first centroids are evenly
/// spaced vectors of the input, so the same input always gives the same
/// centroids. An empty cluster keeps its previous centroid.
pub fn learn_centroids(vectors: &[Vec<f32>], k: usize, rounds: usize) -> Vec<Vec<f32>> {
    if vectors.is_empty() {
        return Vec::new();
    }
    let k = k.min(vectors.len()).max(1);
    let mut centroids: Vec<Vec<f32>> = (0..k)
        .map(|cluster| unit(&vectors[cluster * vectors.len() / k]))
        .collect();
    let dimension = centroids[0].len();
    for _ in 0..rounds {
        let mut sums = vec![vec![0.0f32; dimension]; k];
        let mut counts = vec![0usize; k];
        for vector in vectors {
            let normal = unit(vector);
            let nearest = (0..k)
                .max_by(|a, b| {
                    cosine_similarity(&normal, &centroids[*a])
                        .total_cmp(&cosine_similarity(&normal, &centroids[*b]))
                })
                .unwrap_or(0);
            counts[nearest] += 1;
            for (sum, value) in sums[nearest].iter_mut().zip(&normal) {
                *sum += value;
            }
        }
        for cluster in 0..k {
            if counts[cluster] > 0 {
                centroids[cluster] = unit(&sums[cluster]);
            }
        }
    }
    centroids
}

/// Shard centroids learned from the content an index holds, in place of the
/// built-in topic descriptions.
///
/// Every node of a network has to route with the same centroids, or one
/// shard name means different content on different nodes. The version is a
/// hash of the centroids and is part of every shard name, so nodes that
/// loaded different files announce different shards and never mix them.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LearnedRouting {
    /// First 12 hex digits of the SHA-256 of the embedder name and centroids.
    pub version: String,
    /// The embedder the centroids belong to; another embedder's vectors
    /// cannot be routed with them.
    pub embedder: String,
    pub centroids: Vec<Vec<f32>>,
}

impl LearnedRouting {
    /// Rounds of k-means used by `learn`.
    pub const ROUNDS: usize = 20;

    pub fn from_centroids(embedder: &str, centroids: Vec<Vec<f32>>) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(embedder.as_bytes());
        for centroid in &centroids {
            hasher.update([0u8]);
            for value in centroid {
                hasher.update(value.to_le_bytes());
            }
        }
        let version = format!("{:x}", hasher.finalize())[..12].to_string();
        Self {
            version,
            embedder: embedder.to_string(),
            centroids,
        }
    }

    /// Learn `shards` centroids from stored embeddings. `None` when there is
    /// nothing to learn from.
    pub fn learn(vectors: &[Vec<f32>], shards: usize, embedder: &str) -> Option<Self> {
        let centroids = learn_centroids(vectors, shards, Self::ROUNDS);
        (!centroids.is_empty()).then(|| Self::from_centroids(embedder, centroids))
    }

    pub fn shard_id(&self, cluster: usize) -> String {
        format!("learned-{}-{cluster}", self.version)
    }

    pub fn shard_ids(&self) -> Vec<String> {
        (0..self.centroids.len())
            .map(|cluster| self.shard_id(cluster))
            .collect()
    }

    /// A router over these centroids. Every embedding goes to its nearest
    /// centroid; nothing falls back to a general shard.
    pub fn router(&self) -> TopicRouter {
        TopicRouter::new(
            self.centroids
                .iter()
                .enumerate()
                .map(|(cluster, centroid)| (self.shard_id(cluster), centroid.clone()))
                .collect(),
        )
        .with_threshold(-1.0)
    }

    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        let body = serde_json::to_string(self).map_err(std::io::Error::other)?;
        std::fs::write(path, body)
    }

    /// Read a routing file and check that its version is the hash of its
    /// centroids, so an edited or truncated file is not used under the name
    /// of another.
    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        let stored: Self =
            serde_json::from_str(&raw).map_err(|error| format!("{}: {error}", path.display()))?;
        let expected = Self::from_centroids(&stored.embedder, stored.centroids.clone());
        if stored.centroids.is_empty() {
            return Err(format!("{} has no centroids", path.display()));
        }
        if expected.version != stored.version {
            return Err(format!(
                "{}: version {} does not match its centroids ({})",
                path.display(),
                stored.version,
                expected.version
            ));
        }
        Ok(stored)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learned_routing_separates_groups_and_names_shards_by_version() {
        let vectors = vec![
            vec![1.0, 0.0],
            vec![0.9, 0.1],
            vec![0.0, 1.0],
            vec![0.1, 0.9],
        ];
        let routing = LearnedRouting::learn(&vectors, 2, "test-embedder").unwrap();
        assert_eq!(routing.centroids.len(), 2);
        assert_eq!(routing.version.len(), 12);
        let router = routing.router();
        assert_eq!(router.classify(&[1.0, 0.05]), router.classify(&[0.95, 0.0]));
        assert_ne!(router.classify(&[1.0, 0.05]), router.classify(&[0.05, 1.0]));
        assert!(
            router
                .classify(&[1.0, 0.0])
                .starts_with(&format!("learned-{}-", routing.version))
        );
        // Nothing falls back to a general shard, however far from a centroid.
        assert!(router.classify(&[-1.0, -1.0]).starts_with("learned-"));

        // The same input gives the same routing; other centroids or another
        // embedder give another version.
        assert_eq!(
            LearnedRouting::learn(&vectors, 2, "test-embedder").unwrap(),
            routing
        );
        assert_ne!(
            LearnedRouting::learn(&vectors, 2, "other-embedder")
                .unwrap()
                .version,
            routing.version
        );
        assert!(LearnedRouting::learn(&[], 2, "test-embedder").is_none());
    }

    #[test]
    fn a_routing_file_is_rejected_when_its_version_is_not_its_content() {
        let dir = std::env::temp_dir().join(format!("oi-routing-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("routing.json");
        let routing = LearnedRouting::from_centroids("e", vec![vec![0.6, 0.8], vec![1.0, 0.0]]);
        routing.save(&path).unwrap();
        assert_eq!(LearnedRouting::load(&path).unwrap(), routing);

        let mut edited = routing.clone();
        edited.centroids[0][0] = 0.5;
        std::fs::write(&path, serde_json::to_string(&edited).unwrap()).unwrap();
        assert!(
            LearnedRouting::load(&path)
                .unwrap_err()
                .contains("does not match")
        );
        std::fs::remove_dir_all(&dir).ok();
    }

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
            assert!(
                t.shard_id.starts_with("topic-"),
                "bad shard_id: {}",
                t.shard_id
            );
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
        let router = TopicRouter::new(vec![("topic-technology".into(), vec![1.0, 0.0, 0.0, 0.0])])
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
        let bucket: usize = id
            .strip_prefix("topic-technology/")
            .unwrap()
            .parse()
            .unwrap();
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
