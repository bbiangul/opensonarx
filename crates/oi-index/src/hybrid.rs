use oi_types::{ChunkMeta, ContentChunk, QueryFilters, Result, SearchResultItem};
use oi_types::traits::{Reranker, ReputationProvider, SearchIndex, StakeRegistry};
use crate::bm25::Bm25Index;
use crate::hnsw::HnswIndex;
use crate::ranking::{self, RankingParams};
use crate::simhash;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Instant;

/// Per-component timing breakdown for a single query execution.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct QueryTimings {
    /// HNSW vector search time in milliseconds
    pub hnsw_ms: f64,
    /// BM25 keyword search time in milliseconds
    pub bm25_ms: f64,
    /// Score fusion (merge + filter) time in milliseconds
    pub fusion_ms: f64,
    /// Cross-encoder rerank time in milliseconds (0 if no reranker)
    pub rerank_ms: f64,
    /// Stake/reputation enrichment + final scoring time in milliseconds
    pub enrichment_ms: f64,
    /// Total wall-clock time in milliseconds
    pub total_ms: f64,
}

/// Helper to acquire a read lock, converting poison errors into oi_types::Error.
fn read_lock<T>(lock: &RwLock<T>) -> Result<std::sync::RwLockReadGuard<'_, T>> {
    lock.read().map_err(|e| oi_types::Error::Internal(format!("lock poisoned: {e}")))
}

/// Helper to acquire a write lock, converting poison errors into oi_types::Error.
fn write_lock<T>(lock: &RwLock<T>) -> Result<std::sync::RwLockWriteGuard<'_, T>> {
    lock.write().map_err(|e| oi_types::Error::Internal(format!("lock poisoned: {e}")))
}

/// Boost factor for chunks whose domain matches a technology keyword in the query.
/// Addresses cross-corpus confusion where e.g. Rust "enum" docs outrank TypeScript "enum"
/// docs for a TypeScript-specific query.
fn query_domain_boost(query_lower: &str, domain: &str) -> f64 {
    const AFFINITIES: &[(&str, &[&str])] = &[
        ("typescript", &["typescriptlang.org"]),
        ("tsconfig", &["typescriptlang.org"]),
        ("rust", &["rust-lang.org", "docs.rs"]),
        ("react", &["react.dev", "reactjs.org"]),
        ("python", &["python.org"]),
        ("javascript", &["javascript.info", "developer.mozilla.org"]),
        ("nodejs", &["nodejs.org"]),
        ("node.js", &["nodejs.org"]),
        ("go ", &["go.dev", "golang.org"]),
        ("golang", &["go.dev", "golang.org"]),
    ];

    let domain_lower = domain.to_lowercase();

    for &(keyword, patterns) in AFFINITIES {
        if query_lower.contains(keyword)
            && patterns.iter().any(|p| domain_lower.contains(p))
        {
            return 1.5;
        }
    }

    1.0
}

/// NaN-safe total ordering for f64. NaN sorts last (smallest).
fn total_cmp_desc(a: f64, b: f64) -> std::cmp::Ordering {
    b.partial_cmp(&a).unwrap_or_else(|| {
        // NaN handling: push NaN to the end
        match (a.is_nan(), b.is_nan()) {
            (true, true) => std::cmp::Ordering::Equal,
            (true, false) => std::cmp::Ordering::Greater,  // a is NaN → a goes after b
            (false, true) => std::cmp::Ordering::Less,     // b is NaN → b goes after a
            _ => std::cmp::Ordering::Equal,
        }
    })
}

/// Fusion strategy for combining vector and BM25 scores.
#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum FusionMethod {
    /// Weighted sum of max-normalized scores: w1*vec + w2*bm25
    #[default]
    WeightedSum,
    /// Reciprocal Rank Fusion: 1/(k+rank_vec) + 1/(k+rank_bm25), k=60
    Rrf,
}

/// Configuration for the hybrid search index.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HybridConfig {
    /// Weight for semantic (vector) scores in preliminary fusion (default 0.7)
    pub semantic_weight: f64,
    /// Weight for BM25 (keyword) scores in preliminary fusion (default 0.3)
    pub bm25_weight: f64,
    /// ef_search parameter for HNSW (default 50)
    pub ef_search: usize,
    /// Candidate pool multiplier: top_k * this (default 2)
    pub pool_multiplier: usize,
    /// Score fusion strategy (default WeightedSum)
    pub fusion: FusionMethod,
    /// Skip deduplication at ingest (useful for benchmarks with unique passages)
    pub skip_dedup: bool,
    /// Maximum chunks to ingest per URL. Prevents large pages from dominating
    /// the vector index with disproportionate "surface area". 0 = no limit.
    pub max_chunks_per_url: usize,
    /// SimHash Hamming-distance threshold for cross-source near-duplicate
    /// rejection at ingest. See `Default` for calibration notes.
    pub simhash_threshold: u32,
    /// Name of the embedding model whose vectors this index holds. Empty for
    /// legacy indexes (adopted on first save). Querying with a DIFFERENT
    /// model silently returns garbage similarity — callers must check this
    /// and re-embed on mismatch (`oi index reembed`).
    #[serde(default)]
    pub embedder_name: String,
}

impl Default for HybridConfig {
    fn default() -> Self {
        Self {
            // sw=0.80 is the consistent optimum on real IR: it beat 0.70 on both
            // BEIR scifact (0.6980 vs 0.6905 nDCG@10) and nfcorpus (0.3322 vs 0.3306),
            // and hybrid beats pure-dense on both. See hybrid-final-score fix.
            semantic_weight: 0.8,
            bm25_weight: 0.2,
            ef_search: 50,
            pool_multiplier: 15,
            fusion: FusionMethod::Rrf,
            skip_dedup: false,
            // Benchmark-proven (agent-cost eval): uncapped pages flood the
            // index — Rust API docs alone reached 31% of a dogfood corpus and
            // removing the cap dropped retrieval accuracy 82% → 68%.
            max_chunks_per_url: 8,
            // Scale-calibrated: at threshold 8 (the old simhash::DEFAULT),
            // ingesting 100k distinct MS MARCO passages rejected 96% of them
            // as false-positive "near-duplicates" — with 100k 64-bit
            // fingerprints, unrelated short texts routinely land within 8
            // bits. 3 is the standard 64-bit SimHash threshold (Charikar);
            // real copies/mirror spam still land at 0-3.
            simhash_threshold: 3,
            embedder_name: String::new(),
        }
    }
}

/// Maximum results from a single domain in query output.
/// Document-level scoring (best-chunk-per-URL) ensures per-page diversity,
/// so this cap only needs to limit domain monopolisation of result slots.
const MAX_PER_DOMAIN: usize = 5;

/// Hybrid search index combining HNSW (vector) and BM25 (keyword) search.
/// Result of `HybridIndex::audit` — cross-store consistency and content
/// quality aggregates for operational health checks.
#[derive(Debug, Default)]
pub struct IndexAudit {
    pub hnsw_count: usize,
    pub bm25_count: usize,
    pub meta_count: usize,
    pub content_hash_count: usize,
    /// Metadata rows whose chunk text is missing from BM25 (should be 0).
    pub meta_without_bm25: usize,
    pub per_domain: HashMap<String, usize>,
    pub chunk_lengths: Vec<usize>,
    pub missing_title: usize,
    pub missing_url: usize,
    pub avg_quality: f64,
    /// Metadata rows with an empty content_hash (excluded from exact dedup).
    pub empty_content_hash: usize,
    /// Distinct content hashes among live metadata rows.
    pub distinct_live_hashes: usize,
    /// Hashes in the dedup set that belong to no live chunk (history of
    /// rejected/re-crawled content; monotonic growth is expected but large
    /// counts indicate wasted rejections).
    pub orphan_hashes: usize,
}

pub struct HybridIndex {
    hnsw: RwLock<HnswIndex>,
    bm25: RwLock<Bm25Index>,
    /// Per-chunk metadata for result population and filtering
    metadata: RwLock<HashMap<String, ChunkMeta>>,
    /// Exact content hashes for dedup at ingest (SHA256 hex strings)
    content_hashes: RwLock<HashSet<String>>,
    /// SimHash fingerprints for near-duplicate detection at ingest (band-based LSH)
    simhashes: RwLock<simhash::SimHashIndex>,
    dim: usize,
    config: HybridConfig,
    stake_registry: Option<Arc<dyn StakeRegistry>>,
    reputation_provider: Option<Arc<dyn ReputationProvider>>,
    reranker: Option<Arc<dyn Reranker>>,
    dirty: AtomicBool,
    unsaved_count: AtomicU64,
    /// Monotonic sequence counter for incremental replication.
    /// Each ingested chunk gets an incrementing seq number.
    next_seq: AtomicU64,
    /// Maps chunk_id -> assigned seq for incremental replication.
    chunk_seqs: RwLock<HashMap<String, u64>>,
}

impl HybridIndex {
    pub fn new(dim: usize) -> Self {
        Self::with_config(dim, HybridConfig::default())
    }

    pub fn with_config(dim: usize, config: HybridConfig) -> Self {
        let hnsw = HnswIndex::with_params(dim, 16, 200, config.ef_search);
        Self {
            hnsw: RwLock::new(hnsw),
            bm25: RwLock::new(Bm25Index::new()),
            metadata: RwLock::new(HashMap::new()),
            content_hashes: RwLock::new(HashSet::new()),
            simhashes: RwLock::new(simhash::SimHashIndex::new()),
            dim,
            config,
            stake_registry: None,
            reputation_provider: None,
            reranker: None,
            dirty: AtomicBool::new(false),
            unsaved_count: AtomicU64::new(0),
            next_seq: AtomicU64::new(1),
            chunk_seqs: RwLock::new(HashMap::new()),
        }
    }

    /// Reconstruct content chunks for replication to other nodes.
    ///
    /// Iterates metadata entries, optionally filtering by `shard_id`, and
    /// reconstructs `ContentChunk` from the BM25 content store and HNSW vectors.
    /// Returns up to `max_chunks` chunks.
    pub fn get_chunks_for_replication(
        &self,
        shard_id: Option<&str>,
        max_chunks: usize,
    ) -> Result<Vec<ContentChunk>> {
        self.chunks_since(shard_id, 0, max_chunks)
    }

    /// Return the current sequence number (highest assigned seq).
    pub fn current_seq(&self) -> u64 {
        self.next_seq.load(Ordering::Acquire).saturating_sub(1)
    }

    /// Return chunks added after sequence `from_seq`, optionally filtered by shard.
    /// Used for incremental replication — only transfer new content.
    pub fn chunks_since(
        &self,
        shard_id: Option<&str>,
        from_seq: u64,
        max_chunks: usize,
    ) -> Result<Vec<ContentChunk>> {
        let metadata = read_lock(&self.metadata)?;
        let bm25 = read_lock(&self.bm25)?;
        let hnsw = read_lock(&self.hnsw)?;
        let chunk_seqs = read_lock(&self.chunk_seqs)?;

        // Ascending-seq order is REQUIRED for watermark-based incremental
        // replication: iterating the HashMap unordered handed out near-max
        // seqs in the first batch, the receiver's watermark jumped to the
        // end, and catch-up silently skipped almost the whole index.
        let mut ordered: Vec<(&String, u64)> =
            chunk_seqs.iter().map(|(id, &seq)| (id, seq)).collect();
        ordered.sort_unstable_by_key(|&(_, seq)| seq);

        let mut chunks = Vec::new();
        for (chunk_id, seq) in ordered {
            if seq <= from_seq {
                continue;
            }
            if chunks.len() >= max_chunks {
                break;
            }

            if let Some(meta) = metadata.get(chunk_id) {
                // Filter by shard_id if specified (prefix match for sub-shards)
                if let Some(sid) = shard_id {
                    match &meta.shard_id {
                        Some(chunk_shard) if oi_types::shard::is_sub_shard_of(chunk_shard, sid) => {}
                        Some(_) => continue,
                        None => continue,
                    }
                }

                let content = bm25.get_doc_content(chunk_id).unwrap_or("").to_string();
                let embedding = hnsw.get_vector(chunk_id).cloned();

                chunks.push(ContentChunk {
                    chunk_id: chunk_id.clone(),
                    content,
                    embedding,
                    seq,
                    meta: Some(meta.clone()),
                });
            }
        }

        // Sort by seq ascending for consistent ordering
        chunks.sort_by_key(|c| c.seq);
        Ok(chunks)
    }

    /// Merge chunks from another index into this one.
    /// Returns count of new (non-duplicate) chunks added.
    pub fn merge_from(&self, other: &HybridIndex) -> Result<usize> {
        let other_meta = read_lock(&other.metadata)?;
        let other_bm25 = read_lock(&other.bm25)?;
        let other_hnsw = read_lock(&other.hnsw)?;

        let mut chunks = Vec::new();
        for (chunk_id, meta) in other_meta.iter() {
            let content = other_bm25.get_doc_content(chunk_id).unwrap_or("").to_string();
            let embedding = other_hnsw.get_vector(chunk_id).cloned();

            chunks.push(ContentChunk {
                chunk_id: chunk_id.clone(),
                content,
                embedding,
                seq: 0,
                meta: Some(meta.clone()),
            });
        }

        let before = self.doc_count();
        self.ingest(&chunks)?;
        Ok(self.doc_count() - before)
    }

    /// Attach a stake registry for enriching query results with on-chain data.
    pub fn with_stake_registry(mut self, registry: Arc<dyn StakeRegistry>) -> Self {
        self.stake_registry = Some(registry);
        self
    }

    /// Attach a reputation provider for applying domain reputation in core ranking.
    pub fn with_reputation_provider(mut self, provider: Arc<dyn ReputationProvider>) -> Self {
        self.reputation_provider = Some(provider);
        self
    }

    /// Attach a cross-encoder reranker for second-stage scoring.
    /// When set, the top 100 candidates from Pass 1 fusion are rescored
    /// using the cross-encoder before stake/reputation enrichment.
    pub fn with_reranker(mut self, reranker: Arc<dyn Reranker>) -> Self {
        self.reranker = Some(reranker);
        self
    }

    pub fn doc_count(&self) -> usize {
        self.bm25.read().map(|b| b.len()).unwrap_or(0)
    }

    /// Embedding dimension this index was built for.
    pub fn dim(&self) -> usize {
        self.dim
    }

    /// Name of the embedding model this index's vectors came from
    /// (empty for legacy indexes built before the field existed).
    pub fn embedder_name(&self) -> &str {
        &self.config.embedder_name
    }

    /// Suggest a spelling correction for a query from the indexed vocabulary
    /// (see `Bm25Index::correct_query`). Returns `Some(corrected)` only if a
    /// token changed.
    pub fn correct_query(&self, query: &str) -> Option<String> {
        self.bm25.read().ok().and_then(|b| b.correct_query(query))
    }

    /// Index health audit: cross-store consistency (every chunk must exist in
    /// HNSW, BM25, and metadata), content-quality aggregates, and per-domain
    /// distribution. Cheap enough to run on demand (`oi index info`).
    pub fn audit(&self) -> IndexAudit {
        let hnsw_count = self.hnsw.read().map(|h| h.len()).unwrap_or(0);
        let bm25 = self.bm25.read().ok();
        let metadata = self.metadata.read().ok();
        let content_hash_count = self.content_hashes.read().map(|h| h.len()).unwrap_or(0);

        let mut audit = IndexAudit {
            hnsw_count,
            bm25_count: bm25.as_ref().map(|b| b.len()).unwrap_or(0),
            meta_count: metadata.as_ref().map(|m| m.len()).unwrap_or(0),
            content_hash_count,
            ..IndexAudit::default()
        };

        if let (Some(bm25), Some(metadata)) = (bm25, metadata) {
            let mut q_sum = 0.0;
            let mut q_n = 0usize;
            let mut live_hashes: HashSet<&str> = HashSet::new();
            for (chunk_id, meta) in metadata.iter() {
                *audit.per_domain.entry(meta.domain.clone()).or_default() += 1;
                if meta.title.trim().is_empty() {
                    audit.missing_title += 1;
                }
                if meta.url.trim().is_empty() {
                    audit.missing_url += 1;
                }
                if meta.content_hash.is_empty() {
                    audit.empty_content_hash += 1;
                } else {
                    live_hashes.insert(meta.content_hash.as_str());
                }
                if let Some(q) = meta.quality_score {
                    q_sum += q;
                    q_n += 1;
                }
                match bm25.get_doc_content(chunk_id) {
                    Some(text) => audit.chunk_lengths.push(text.len()),
                    None => audit.meta_without_bm25 += 1,
                }
            }
            audit.distinct_live_hashes = live_hashes.len();
            if let Ok(all_hashes) = self.content_hashes.read() {
                audit.orphan_hashes = all_hashes
                    .iter()
                    .filter(|h| !live_hashes.contains(h.as_str()))
                    .count();
            }
            audit.avg_quality = if q_n > 0 { q_sum / q_n as f64 } else { 0.0 };
        }
        audit
    }

    /// Execute a query and return results with per-component timing breakdown.
    pub fn query_timed(
        &self,
        embedding: &[f32],
        text: &str,
        filters: &QueryFilters,
        top_k: usize,
    ) -> Result<(Vec<SearchResultItem>, QueryTimings)> {
        let total_start = Instant::now();
        let pool_size = top_k * self.config.pool_multiplier;

        // ── HNSW search ──
        let t = Instant::now();
        let hnsw = read_lock(&self.hnsw)?;
        let vector_results = hnsw.search(embedding, pool_size);
        let hnsw_ms = t.elapsed().as_secs_f64() * 1000.0;

        // ── BM25 search (skip entirely when weight is zero) ──
        let t = Instant::now();
        let bm25 = read_lock(&self.bm25)?;
        let bm25_results = if self.config.bm25_weight > 0.0 {
            bm25.search(text, pool_size)
        } else {
            vec![]
        };
        let bm25_ms = t.elapsed().as_secs_f64() * 1000.0;

        let meta_store = read_lock(&self.metadata)?;

        // Pre-compute freshness cutoff
        let freshness_cutoff = filters.freshness.as_ref().and_then(|f| {
            let secs = parse_freshness_secs(f)?;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            Some(now.saturating_sub(secs))
        });

        // ── Fusion ──
        let t = Instant::now();
        let candidates = self.fuse_candidates(
            &vector_results, &bm25_results, &meta_store,
            filters, freshness_cutoff, pool_size,
        );
        let fusion_ms = t.elapsed().as_secs_f64() * 1000.0;

        // ── Rerank ──
        let t = Instant::now();
        let candidates = self.maybe_rerank(candidates, text, &bm25);
        let rerank_ms = t.elapsed().as_secs_f64() * 1000.0;

        // ── Enrichment + final scoring ──
        let t = Instant::now();
        let mut results = self.enrich_and_score(candidates, top_k, &meta_store, &bm25, text);

        // ── Trust gate ──
        // Stake/reputation never move the ranking score; they surface as a trust
        // verdict that callers can require a floor on. Disputed/unknown sort below all.
        if let Some(ref min) = filters.min_trust {
            let floor = ranking::trust_rank(min);
            results.retain(|r| ranking::trust_rank(&r.trust_verdict) >= floor);
        }
        let enrichment_ms = t.elapsed().as_secs_f64() * 1000.0;

        let total_ms = total_start.elapsed().as_secs_f64() * 1000.0;

        let timings = QueryTimings {
            hnsw_ms,
            bm25_ms,
            fusion_ms,
            rerank_ms,
            enrichment_ms,
            total_ms,
        };

        Ok((results, timings))
    }

    /// Save the index to disk via the given Storage backend.
    /// Writes segments: hnsw.bin, bm25.bin, metadata.bin, config.bin,
    /// content_hashes.bin, simhashes.bin.
    ///
    /// Serializes under read locks for consistency, then drops locks before
    /// writing to disk. Uses write-to-temp-then-rename to minimize corruption
    /// risk if the process is interrupted mid-save.
    pub fn save(&self, storage: &crate::storage::Storage) -> std::io::Result<()> {
        // Serialize everything under locks for a consistent snapshot
        let hnsw_bytes;
        let bm25_bytes;
        let meta_bytes;
        let content_hashes_bytes;
        let simhashes_bytes;
        {
            let hnsw = self.hnsw.read()
                .map_err(|e| std::io::Error::other(format!("hnsw lock poisoned: {e}")))?;
            let bm25 = self.bm25.read()
                .map_err(|e| std::io::Error::other(format!("bm25 lock poisoned: {e}")))?;
            let meta = self.metadata.read()
                .map_err(|e| std::io::Error::other(format!("metadata lock poisoned: {e}")))?;
            let ch = self.content_hashes.read()
                .map_err(|e| std::io::Error::other(format!("content_hashes lock poisoned: {e}")))?;
            let sh = self.simhashes.read()
                .map_err(|e| std::io::Error::other(format!("simhashes lock poisoned: {e}")))?;

            hnsw_bytes = hnsw.to_bytes();
            bm25_bytes = bm25.to_bytes();
            meta_bytes = bincode::serialize(&*meta)
                .map_err(std::io::Error::other)?;
            content_hashes_bytes = bincode::serialize(&*ch)
                .map_err(std::io::Error::other)?;
            // Serialize as flat Vec<u64> for backward compatibility
            simhashes_bytes = bincode::serialize(sh.to_hashes())
                .map_err(std::io::Error::other)?;
        }
        // Locks are dropped — writers can proceed while we do I/O

        let config_bytes = bincode::serialize(&self.config)
            .map_err(std::io::Error::other)?;

        // Write to temp names first, then rename to final names.
        storage.write_segment("hnsw.bin.tmp", &hnsw_bytes)?;
        storage.write_segment("bm25.bin.tmp", &bm25_bytes)?;
        storage.write_segment("metadata.bin.tmp", &meta_bytes)?;
        storage.write_segment("config.bin.tmp", &config_bytes)?;
        storage.write_segment("content_hashes.bin.tmp", &content_hashes_bytes)?;
        storage.write_segment("simhashes.bin.tmp", &simhashes_bytes)?;

        storage.rename_segment("hnsw.bin.tmp", "hnsw.bin")?;
        storage.rename_segment("bm25.bin.tmp", "bm25.bin")?;
        storage.rename_segment("metadata.bin.tmp", "metadata.bin")?;
        storage.rename_segment("config.bin.tmp", "config.bin")?;
        storage.rename_segment("content_hashes.bin.tmp", "content_hashes.bin")?;
        storage.rename_segment("simhashes.bin.tmp", "simhashes.bin")?;

        self.dirty.store(false, Ordering::Release);
        self.unsaved_count.store(0, Ordering::Release);

        Ok(())
    }

    /// Load a previously saved index from the given Storage backend.
    ///
    /// Backward-compatible: if `content_hashes.bin` or `simhashes.bin` don't
    /// exist (older index), content_hashes are rebuilt from metadata and
    /// simhashes start empty.
    pub fn load(storage: &crate::storage::Storage) -> std::io::Result<Self> {
        let hnsw_bytes = storage.read_segment("hnsw.bin")?;
        let hnsw = crate::hnsw::HnswIndex::from_bytes(&hnsw_bytes)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        let bm25_bytes = storage.read_segment("bm25.bin")?;
        let bm25 = crate::bm25::Bm25Index::from_bytes(&bm25_bytes)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        let meta_bytes = storage.read_segment("metadata.bin")?;
        let metadata: HashMap<String, ChunkMeta> = match bincode::deserialize(&meta_bytes) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("metadata.bin format changed, starting with empty metadata: {e}");
                HashMap::new()
            }
        };

        // Config is tunables only (no data): when the struct gained fields and
        // an older config.bin can't deserialize (bincode is not
        // self-describing), fall back to defaults instead of bricking the
        // index. The next save rewrites the current format.
        let config_bytes = storage.read_segment("config.bin")?;
        let config: HybridConfig = match bincode::deserialize(&config_bytes) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    "config.bin format changed, using default config (tunables reset): {e}"
                );
                HybridConfig::default()
            }
        };

        // Load content_hashes — fall back to rebuilding from metadata
        let content_hashes: HashSet<String> = if storage.segment_exists("content_hashes.bin") {
            let bytes = storage.read_segment("content_hashes.bin")?;
            bincode::deserialize(&bytes)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?
        } else {
            // Backward compat: rebuild from metadata content_hash fields
            metadata
                .values()
                .filter(|m| !m.content_hash.is_empty())
                .map(|m| m.content_hash.clone())
                .collect()
        };

        // Load simhashes — deserialize flat Vec<u64> and rebuild band-based index
        let simhashes = if storage.segment_exists("simhashes.bin") {
            let bytes = storage.read_segment("simhashes.bin")?;
            let flat: Vec<u64> = bincode::deserialize(&bytes)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            simhash::SimHashIndex::from_hashes(flat)
        } else {
            simhash::SimHashIndex::new()
        };

        let dim = hnsw.dim();

        // Initialize chunk_seqs from metadata (assign seq=index for loaded chunks)
        let mut chunk_seqs = HashMap::new();
        for (i, chunk_id) in metadata.keys().enumerate() {
            chunk_seqs.insert(chunk_id.clone(), (i + 1) as u64);
        }
        let next_seq = (chunk_seqs.len() as u64) + 1;

        Ok(Self {
            hnsw: RwLock::new(hnsw),
            bm25: RwLock::new(bm25),
            metadata: RwLock::new(metadata),
            content_hashes: RwLock::new(content_hashes),
            simhashes: RwLock::new(simhashes),
            dim,
            config,
            stake_registry: None,
            reputation_provider: None,
            reranker: None,
            dirty: AtomicBool::new(false),
            unsaved_count: AtomicU64::new(0),
            next_seq: AtomicU64::new(next_seq),
            chunk_seqs: RwLock::new(chunk_seqs),
        })
    }

    /// Returns true if the index has been modified since the last save.
    pub fn is_dirty(&self) -> bool {
        self.dirty.load(Ordering::Acquire)
    }

    /// Returns the number of chunks ingested since the last save.
    pub fn unsaved_count(&self) -> u64 {
        self.unsaved_count.load(Ordering::Acquire)
    }
}

/// Check content-type, freshness, entity, and content_kind filters for a candidate.
fn pass_filters(
    meta_store: &HashMap<String, ChunkMeta>,
    id: &str,
    filters: &QueryFilters,
    freshness_cutoff: Option<u64>,
) -> bool {
    if let Some(meta) = meta_store.get(id) {
        if !filters.types.is_empty()
            && let Some(ct) = &meta.content_type
            && !filters.types.iter().any(|t| ct.contains(t.as_str()))
        {
            return false;
        }
        if let Some(cutoff) = freshness_cutoff
            && let Some(crawled) = meta.crawled_at
            && crawled < cutoff
        {
            return false;
        }
        // Entity filter: content must match one of the requested entities
        if !filters.entities.is_empty() {
            if let Some(entity) = &meta.entity {
                if !filters.entities.iter().any(|e| entity.contains(e)) {
                    return false;
                }
            } else {
                return false; // no entity = doesn't match entity filter
            }
        }
        // Content kind filter
        if !filters.content_kinds.is_empty() {
            if let Some(kind) = &meta.content_kind {
                if !filters.content_kinds.iter().any(|k| k == kind) {
                    return false;
                }
            } else {
                return false;
            }
        }
        // Site/domain scope filter
        if let Some(ref site) = filters.site
            && meta.domain != *site && !meta.domain.ends_with(&format!(".{site}"))
        {
            return false;
        }
        // Shard filter: content must match one of the requested shards.
        // Uses prefix matching so "topic-technology" matches "topic-technology/3" (sub-shards).
        if !filters.shards.is_empty()
            && let Some(sid) = &meta.shard_id
            && !filters.shards.iter().any(|s| oi_types::shard::is_sub_shard_of(sid, s))
        {
            return false;
        }
        // shard_id: None (unclassified) passes through — backward compatible
    } else {
        // No metadata available — reject if any metadata-dependent filter is active
        if !filters.types.is_empty()
            || freshness_cutoff.is_some()
            || !filters.entities.is_empty()
            || !filters.content_kinds.is_empty()
            || filters.site.is_some()
            || !filters.shards.is_empty()
        {
            return false;
        }
    }
    true
}

/// Extract the domain from a URL (e.g. "https://example.com/page" → "example.com").
#[cfg(test)]
fn extract_domain_from_url(url: &str) -> String {
    url.split("//")
        .nth(1)
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or(url)
        .to_string()
}

/// Parse freshness string ("1h", "1d", "1w", "1m", "1y") into seconds
fn parse_freshness_secs(freshness: &str) -> Option<u64> {
    if freshness.len() < 2 {
        return None;
    }
    let (num, unit) = freshness.split_at(freshness.len() - 1);
    let n: u64 = num.parse().ok()?;
    let secs = match unit {
        "h" => n * 3600,
        "d" => n * 86400,
        "w" => n * 604800,
        "m" => n * 2_592_000,  // 30 days
        "y" => n * 31_536_000, // 365 days
        _ => return None,
    };
    Some(secs)
}

/// Keep only the best-scoring candidate per URL.
/// Candidates without a URL (no metadata) are always kept.
fn dedup_best_per_url(candidates: Vec<Candidate>, meta_store: &HashMap<String, ChunkMeta>) -> Vec<Candidate> {
    let mut best_per_url: HashMap<String, usize> = HashMap::new();
    for (i, c) in candidates.iter().enumerate() {
        let url = meta_store.get(&c.id).map(|m| m.url.as_str()).unwrap_or("");
        if url.is_empty() {
            continue;
        }
        match best_per_url.entry(url.to_string()) {
            std::collections::hash_map::Entry::Vacant(e) => { e.insert(i); }
            std::collections::hash_map::Entry::Occupied(mut e) => {
                if candidates[i].preliminary_score > candidates[*e.get()].preliminary_score {
                    e.insert(i);
                }
            }
        }
    }
    let keep_set: HashSet<usize> = best_per_url.values().copied().collect();
    candidates
        .into_iter()
        .enumerate()
        .filter(|(i, c)| {
            let url = meta_store.get(&c.id).map(|m| m.url.as_str()).unwrap_or("");
            url.is_empty() || keep_set.contains(i)
        })
        .map(|(_, c)| c)
        .collect()
}

/// Internal candidate from fusion/rerank stage.
struct Candidate {
    id: String,
    vec_score: f64,
    bm25_score: f64,
    preliminary_score: f64,
    reranked: bool,
}

impl HybridIndex {
    /// Fuse HNSW and BM25 results into a merged candidate list, applying filters.
    fn fuse_candidates(
        &self,
        vector_results: &[(String, f32)],
        bm25_results: &[(String, f64)],
        meta_store: &HashMap<String, ChunkMeta>,
        filters: &QueryFilters,
        freshness_cutoff: Option<u64>,
        pool_size: usize,
    ) -> Vec<Candidate> {
        match self.config.fusion {
            FusionMethod::WeightedSum => {
                let max_vec = vector_results.first().map(|(_, s)| *s as f64).unwrap_or(1.0).max(1e-10);
                let max_bm25 = bm25_results.first().map(|(_, s)| *s).unwrap_or(1.0).max(1e-10);

                let mut merged: HashMap<String, (f64, f64)> = HashMap::new();
                for (id, score) in vector_results {
                    merged.entry(id.clone()).or_default().0 = *score as f64 / max_vec;
                }
                for (id, score) in bm25_results {
                    merged.entry(id.clone()).or_default().1 = *score / max_bm25;
                }

                let sw = self.config.semantic_weight;
                let bw = self.config.bm25_weight;

                merged
                    .into_iter()
                    .filter_map(|(id, (vec_score, bm25_score))| {
                        if !pass_filters(meta_store, &id, filters, freshness_cutoff) {
                            return None;
                        }
                        let preliminary_score = sw * vec_score + bw * bm25_score;
                        Some(Candidate { id, vec_score, bm25_score, preliminary_score, reranked: false })
                    })
                    .collect()
            }
            FusionMethod::Rrf => {
                const RRF_K: f64 = 60.0;

                let mut vec_rank: HashMap<String, usize> = HashMap::new();
                for (rank, (id, _)) in vector_results.iter().enumerate() {
                    vec_rank.insert(id.clone(), rank + 1);
                }
                let mut bm25_rank: HashMap<String, usize> = HashMap::new();
                for (rank, (id, _)) in bm25_results.iter().enumerate() {
                    bm25_rank.insert(id.clone(), rank + 1);
                }

                let max_vec = vector_results.first().map(|(_, s)| *s as f64).unwrap_or(1.0).max(1e-10);
                let max_bm25 = bm25_results.first().map(|(_, s)| *s).unwrap_or(1.0).max(1e-10);
                let mut raw_scores: HashMap<String, (f64, f64)> = HashMap::new();
                for (id, score) in vector_results {
                    raw_scores.entry(id.clone()).or_default().0 = *score as f64 / max_vec;
                }
                for (id, score) in bm25_results {
                    raw_scores.entry(id.clone()).or_default().1 = *score / max_bm25;
                }

                let all_ids: std::collections::HashSet<String> = vec_rank
                    .keys()
                    .chain(bm25_rank.keys())
                    .cloned()
                    .collect();

                let absent_rank = pool_size + 1;

                all_ids
                    .into_iter()
                    .filter_map(|id| {
                        if !pass_filters(meta_store, &id, filters, freshness_cutoff) {
                            return None;
                        }
                        let vr = *vec_rank.get(&id).unwrap_or(&absent_rank);
                        let br = *bm25_rank.get(&id).unwrap_or(&absent_rank);
                        let rrf_score = 1.0 / (RRF_K + vr as f64) + 1.0 / (RRF_K + br as f64);
                        let (vs, bs) = raw_scores.get(&id).copied().unwrap_or((0.0, 0.0));
                        Some(Candidate { id, vec_score: vs, bm25_score: bs, preliminary_score: rrf_score, reranked: false })
                    })
                    .collect()
            }
        }
    }

    /// Optionally rerank candidates using a cross-encoder.
    fn maybe_rerank(
        &self,
        mut candidates: Vec<Candidate>,
        text: &str,
        bm25: &Bm25Index,
    ) -> Vec<Candidate> {
        if let Some(ref reranker) = self.reranker {
            candidates.sort_by(|a, b| total_cmp_desc(a.preliminary_score, b.preliminary_score));
            let rerank_count = candidates.len().min(100);
            let passages: Vec<String> = candidates[..rerank_count]
                .iter()
                .map(|c| bm25.get_doc_content(&c.id).unwrap_or("").to_string())
                .collect();
            let passage_refs: Vec<&str> = passages.iter().map(|s| s.as_str()).collect();

            if let Ok(scores) = reranker.rerank(text, &passage_refs) {
                let mut reranked: Vec<Candidate> = candidates.drain(..rerank_count).collect();
                let tail = candidates;

                let mut scored: Vec<(Candidate, f32)> = reranked
                    .drain(..)
                    .zip(scores)
                    .collect();
                scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

                let n = scored.len().max(1) as f64;
                candidates = scored
                    .into_iter()
                    .enumerate()
                    .map(|(i, (mut c, _raw))| {
                        c.preliminary_score = 2.0 - (i as f64 / n);
                        c.reranked = true;
                        c
                    })
                    .collect();

                candidates.extend(tail);
            }
        }
        candidates
    }

    /// Enrich candidates with stake/reputation data, score, sort, cap per-domain/URL, assign ranks.
    fn enrich_and_score(
        &self,
        mut candidates: Vec<Candidate>,
        top_k: usize,
        meta_store: &HashMap<String, ChunkMeta>,
        bm25: &Bm25Index,
        query_text: &str,
    ) -> Vec<SearchResultItem> {
        // Apply domain affinity boost BEFORE sorting so that domain-matching
        // candidates can overtake cross-language competitors.
        let query_lower = query_text.to_lowercase();
        for c in &mut candidates {
            let domain = meta_store.get(&c.id).map(|m| m.domain.as_str()).unwrap_or("");
            let boost = query_domain_boost(&query_lower, domain);
            c.preliminary_score *= boost;
        }

        candidates.sort_by(|a, b| total_cmp_desc(a.preliminary_score, b.preliminary_score));

        // Document-level dedup: keep only best chunk per URL.
        candidates = dedup_best_per_url(candidates, meta_store);
        {
            // Apply domain-level cap
            let mut domain_counts: HashMap<String, usize> = HashMap::new();
            candidates.retain(|c| {
                let domain = meta_store.get(&c.id).map(|m| m.domain.as_str()).unwrap_or("");
                if !domain.is_empty() {
                    let dc = domain_counts.entry(domain.to_string()).or_insert(0);
                    *dc += 1;
                    if *dc > MAX_PER_DOMAIN {
                        return false;
                    }
                }
                true
            });
        }

        candidates.truncate(top_k);

        // Pre-compute lowercased query words for title boosting
        let query_words: Vec<&str> = query_text
            .split_whitespace()
            .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()))
            .filter(|w| w.len() > 2) // skip short words like "a", "in", "do"
            .collect();

        // Collect unique domains + entities for batch lookup
        let mut lookup_keys: std::collections::HashSet<String> = std::collections::HashSet::new();
        for c in &candidates {
            if let Some(meta) = meta_store.get(&c.id) {
                lookup_keys.insert(meta.domain.clone());
                if let Some(entity) = &meta.entity {
                    lookup_keys.insert(entity.clone());
                }
            }
        }

        let stake_map = if let Some(ref reg) = self.stake_registry {
            let key_refs: Vec<&str> = lookup_keys.iter().map(|s| s.as_str()).collect();
            reg.get_stakes_batch(&key_refs).unwrap_or_default()
        } else {
            HashMap::new()
        };

        let unique_domains: Vec<String> = candidates
            .iter()
            .filter_map(|c| meta_store.get(&c.id).map(|m| m.domain.clone()))
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();

        let rep_map = if let Some(ref provider) = self.reputation_provider {
            let domain_refs: Vec<&str> = unique_domains.iter().map(|s| s.as_str()).collect();
            provider.get_reputations_batch(&domain_refs)
        } else {
            HashMap::new()
        };

        let mut results: Vec<SearchResultItem> = candidates
            .into_iter()
            .map(|c| {
                let meta = meta_store.get(&c.id);
                let domain = meta.map(|m| m.domain.as_str()).unwrap_or("");
                let entity = meta.and_then(|m| m.entity.as_deref());

                let entity_info = entity
                    .filter(|e| *e != domain)
                    .and_then(|e| stake_map.get(e));
                let domain_info = stake_map.get(domain);

                let entity_stake_usd = entity_info.map(|i| i.amount_usd).unwrap_or(0.0);
                let domain_stake_usd = domain_info.map(|i| i.amount_usd).unwrap_or(0.0);

                let is_ugc = oi_types::entity::EntityRef::parse(domain)
                    .map(|e| e.is_ugc_platform())
                    .unwrap_or(false);

                let effective_info = if is_ugc {
                    entity_info
                } else {
                    entity_info.or(domain_info)
                };
                let (stake_truth, stake_usd_val) = if let Some(info) = effective_info {
                    (info.amount_lamports, info.amount_usd.round() as u64)
                } else {
                    (0, 0)
                };

                let rep = rep_map
                    .get(domain)
                    .map(|&(score, _)| score)
                    .or_else(|| effective_info.map(|i| i.reputation))
                    .unwrap_or_else(|| domain_info.map(|i| i.reputation).unwrap_or(1.0));

                let dns_verified = effective_info.map(|i| i.dns_verified);
                let quality_score = meta.and_then(|m| m.quality_score);
                let spam_score = meta.and_then(|m| m.spam_score);
                let slop_score = meta.and_then(|m| m.slop_score);
                let published_at = meta.and_then(|m| m.published_at);

                let disputed = effective_info.map(|i| i.disputed).unwrap_or(false);
                let staked_at = effective_info
                    .map(|i| i.staked_at)
                    .filter(|&t| t > 0);

                let rep_votes = rep_map
                    .get(domain)
                    .map(|&(_, votes)| votes)
                    .unwrap_or(0);

                let params = RankingParams {
                    // Pass the configured hybrid weights so compute_score blends the
                    // (max-normalized) vector + BM25 scores. See base_score note below.
                    semantic_weight: self.config.semantic_weight,
                    bm25_weight: self.config.bm25_weight,
                    entity_stake_usd,
                    domain_stake_usd,
                    is_ugc_platform: is_ugc,
                    reputation: rep,
                    reputation_votes: rep_votes,
                    quality_score,
                    spam_score,
                    slop_score,
                    dns_verified,
                    published_at,
                    staked_at,
                    ..Default::default()
                };
                let base_score = if c.reranked {
                    c.preliminary_score
                } else {
                    // Rank by the weighted blend of the (max-normalized) vector + BM25
                    // scores, then modulate by quality/freshness/dns. Empirically this
                    // weighted-sum base beats a normalized rank-fusion (RRF) base on real
                    // IR — BEIR scifact nDCG@10 0.6980 vs 0.6875 — because it preserves
                    // score *magnitude* (e.g. a stronger BM25 term match) that pure rank
                    // fusion discards. The config weights must be passed in: with the
                    // RankingParams defaults (1.0/0.0) the final ranking collapses to
                    // vector-only and silently drops the BM25 signal entirely.
                    ranking::compute_score(c.vec_score, c.bm25_score, &params)
                };

                // Title relevance boost: pages whose title matches query terms
                // are more likely to be the canonical documentation for that topic.
                let title = meta.map(|m| m.title.as_str()).unwrap_or("");
                let title_lower = title.to_lowercase();
                let title_boost = if !query_words.is_empty() && !title_lower.is_empty() {
                    let hits = query_words.iter()
                        .filter(|w| title_lower.contains(&w.to_lowercase()))
                        .count();
                    let ratio = hits as f64 / query_words.len() as f64;
                    1.0 + 0.25 * ratio
                } else {
                    1.0
                };

                let score = base_score * title_boost;

                let content = bm25.get_doc_content(&c.id).unwrap_or("");
                let snippet = if content.len() > 200 {
                    // Find the last char boundary at or before byte 200
                    let mut end = 200;
                    while end > 0 && !content.is_char_boundary(end) {
                        end -= 1;
                    }
                    format!("{}...", &content[..end])
                } else {
                    content.to_string()
                };

                let content_relevant = None; // Populated at SDK level with semantic scoring

                let trust_verdict = ranking::compute_verdict(
                    stake_usd_val as f64,
                    rep,
                    rep_votes,
                    disputed,
                    dns_verified.unwrap_or(false),
                );

                SearchResultItem {
                    result_id: c.id.clone(),
                    rank: 0,
                    url: meta.map(|m| m.url.clone()).unwrap_or_default(),
                    title: meta.map(|m| m.title.clone()).unwrap_or_default(),
                    snippet,
                    content_md: content.to_string(),
                    semantic_score: c.vec_score,
                    bm25_score: c.bm25_score,
                    final_score: score,
                    stake_truth,
                    stake_usd: stake_usd_val,
                    stake_type: if entity_info.is_some() { "entity" } else { "domain" }.to_string(),
                    reputation: rep,
                    content_hash: meta.map(|m| m.content_hash.clone()).unwrap_or_default(),
                    extraction_quality: "full".to_string(),
                    source: "local".to_string(),
                    entity: meta.and_then(|m| m.entity.clone()),
                    content_kind: meta.and_then(|m| m.content_kind.clone()),
                    crawled_at: meta.and_then(|m| m.crawled_at),
                    published_at: meta.and_then(|m| m.published_at),
                    trust_verdict: trust_verdict.to_string(),
                    reputation_votes: rep_votes,
                    disputed,
                    facts: meta.and_then(|m| m.facts.clone()),
                    content_relevant,
                    domain_first_seen_at: None,
                    domain_url_count: None,
                    domain_reputation_score: None,
                    domain_total_votes: None,
                }
            })
            .collect();

        results.sort_by(|a, b| total_cmp_desc(a.final_score, b.final_score));

        // Domain/URL caps already applied before truncation in enrich_and_score.
        // Re-apply only domain cap here as a safety net (scores may have reordered).

        for (i, r) in results.iter_mut().enumerate() {
            r.rank = (i + 1) as u32;
        }

        results
    }
}

impl SearchIndex for HybridIndex {
    fn query(
        &self,
        embedding: &[f32],
        text: &str,
        filters: &QueryFilters,
        top_k: usize,
    ) -> Result<Vec<SearchResultItem>> {
        let (results, _timings) = self.query_timed(embedding, text, filters, top_k)?;
        Ok(results)
    }

    fn remove_by_url(&self, url: &str) -> Result<usize> {
        let meta_store = read_lock(&self.metadata)?;

        // Collect chunk_ids matching the URL
        let chunk_ids: Vec<String> = meta_store
            .iter()
            .filter(|(_, meta)| meta.url == url)
            .map(|(id, _)| id.clone())
            .collect();

        if chunk_ids.is_empty() {
            return Ok(0);
        }
        drop(meta_store);

        // Acquire write locks
        let mut hnsw = write_lock(&self.hnsw)?;
        let mut bm25 = write_lock(&self.bm25)?;
        let mut meta_store = write_lock(&self.metadata)?;
        let mut content_hashes = write_lock(&self.content_hashes)?;

        let mut removed = 0;
        for chunk_id in &chunk_ids {
            // Remove content_hash from dedup set before removing metadata
            if let Some(meta) = meta_store.get(chunk_id)
                && !meta.content_hash.is_empty()
            {
                content_hashes.remove(&meta.content_hash);
            }

            hnsw.mark_deleted(chunk_id);
            bm25.mark_deleted(chunk_id);
            meta_store.remove(chunk_id);
            removed += 1;
        }

        if removed > 0 {
            self.dirty.store(true, Ordering::Release);
        }

        Ok(removed)
    }

    fn ingest(&self, chunks: &[ContentChunk]) -> Result<()> {
        let mut hnsw = write_lock(&self.hnsw)?;
        let mut bm25 = write_lock(&self.bm25)?;
        let mut meta_store = write_lock(&self.metadata)?;
        let mut content_hashes = write_lock(&self.content_hashes)?;
        let mut simhashes = write_lock(&self.simhashes)?;

        // URL-level replacement: a re-crawled page whose content changed must
        // REPLACE its old chunks, not stack next to them. Without this, any
        // drift (page edited, extractor updated) duplicates the page on every
        // re-ingest — observed doubling the corpus (545 → 1012 docs) on one
        // re-seed. A URL is "changed" when at least one incoming chunk's
        // content hash is unseen; its stale chunks (and their dedup hashes)
        // are removed before normal ingestion. Unchanged URLs fall through to
        // exact-hash dedup as before.
        let mut replaced_urls: HashSet<String> = HashSet::new();
        if !self.config.skip_dedup {
            let mut changed_urls: HashSet<&str> = HashSet::new();
            for chunk in chunks {
                if let Some(meta) = &chunk.meta
                    && !meta.url.is_empty()
                    && !meta.content_hash.is_empty()
                    && !content_hashes.contains(&meta.content_hash)
                {
                    changed_urls.insert(meta.url.as_str());
                }
            }
            if !changed_urls.is_empty() {
                let stale: Vec<String> = meta_store
                    .iter()
                    .filter(|(_, m)| changed_urls.contains(m.url.as_str()))
                    .map(|(id, _)| id.clone())
                    .collect();
                for chunk_id in &stale {
                    if let Some(meta) = meta_store.get(chunk_id) {
                        if !meta.content_hash.is_empty() {
                            content_hashes.remove(&meta.content_hash);
                        }
                        // Replaced URLs count as same-page below: their OLD
                        // SimHash fingerprints are still in the LSH index, and
                        // the replacement chunks must not be rejected as
                        // near-duplicates of their own predecessors.
                        replaced_urls.insert(meta.url.clone());
                    }
                    hnsw.mark_deleted(chunk_id);
                    bm25.mark_deleted(chunk_id);
                    meta_store.remove(chunk_id);
                }
                if !stale.is_empty() {
                    tracing::debug!(
                        replaced = stale.len(),
                        urls = changed_urls.len(),
                        "ingest: replacing stale chunks for re-crawled URLs"
                    );
                }
            }
        }

        // Collect URLs already in the index so we can skip SimHash for same-page chunks.
        // SimHash dedup targets cross-source spam — overlapping chunks from the same
        // page are expected to be similar due to the sliding window.
        let existing_urls: HashSet<String> = if self.config.skip_dedup {
            HashSet::new()
        } else {
            meta_store.values().map(|m| m.url.clone()).collect()
        };
        let mut batch_urls: HashSet<String> = replaced_urls;
        let mut url_ingest_counts: HashMap<String, usize> = HashMap::new();
        let mut inserted: u64 = 0;

        for chunk in chunks {
            // Per-URL chunk cap: prevent large pages from dominating the index
            if self.config.max_chunks_per_url > 0 {
                let url = chunk.meta.as_ref().map(|m| m.url.as_str()).unwrap_or("");
                if !url.is_empty() {
                    let count = url_ingest_counts.entry(url.to_string()).or_insert(0);
                    if *count >= self.config.max_chunks_per_url {
                        continue;
                    }
                    *count += 1;
                }
            }
            if !self.config.skip_dedup {
                // Exact-duplicate rejection by content_hash (always applies)
                if let Some(meta) = &chunk.meta
                    && !meta.content_hash.is_empty()
                    && !content_hashes.insert(meta.content_hash.clone())
                {
                    tracing::debug!(
                        chunk_id = %chunk.chunk_id,
                        content_hash = %meta.content_hash,
                        "skipping exact duplicate"
                    );
                    continue;
                }

                // Near-duplicate rejection via SimHash (band-based LSH — O(1) avg lookup)
                // Skip for same-URL chunks: overlapping windows from the same page are
                // inherently similar. SimHash dedup targets cross-source Sybil spam.
                //
                // Each URL faces at most ONE cross-source check (its first
                // chunk); siblings are exempt regardless of that check's
                // outcome. The exemption must not depend on acceptance: when
                // the first chunk was falsely flagged against another page's
                // template text, gating on acceptance made every subsequent
                // chunk re-face the check and mass-reject the page (observed:
                // 19 of 29 chunks dropped across six unrelated pages).
                let chunk_url = chunk.meta.as_ref().map(|m| m.url.as_str()).unwrap_or("");
                let is_same_page = !chunk_url.is_empty()
                    && (batch_urls.contains(chunk_url) || existing_urls.contains(chunk_url));
                if !chunk_url.is_empty() {
                    batch_urls.insert(chunk_url.to_string());
                }

                let sh = simhash::simhash(&chunk.content);
                if !is_same_page && simhashes.is_near_duplicate(sh, self.config.simhash_threshold) {
                    tracing::debug!(
                        chunk_id = %chunk.chunk_id,
                        "skipping near-duplicate (SimHash)"
                    );
                    continue;
                }
                simhashes.insert(sh);
            }

            if let Some(embedding) = &chunk.embedding {
                hnsw.insert(chunk.chunk_id.clone(), embedding.clone())
                    .map_err(|e| oi_types::Error::Internal(
                        format!("HNSW insert failed for chunk {}: {e}", chunk.chunk_id)
                    ))?;
            }
            // Include title in BM25 text so keyword matches on page titles
            // boost relevant chunks (e.g., searching "tuple" matches a page titled
            // "Tuple Types" even if the body text doesn't repeat it).
            let bm25_text = if let Some(meta) = &chunk.meta
                && !meta.title.is_empty()
            {
                format!("{} {}", meta.title, chunk.content)
            } else {
                chunk.content.clone()
            };
            bm25.insert(chunk.chunk_id.clone(), bm25_text);

            // Store metadata if provided
            if let Some(meta) = &chunk.meta {
                meta_store.insert(chunk.chunk_id.clone(), meta.clone());
            }

            // Assign monotonic seq for incremental replication
            let seq = self.next_seq.fetch_add(1, Ordering::Release);
            if let Ok(mut seqs) = self.chunk_seqs.write() {
                seqs.insert(chunk.chunk_id.clone(), seq);
            }

            inserted += 1;
        }

        if inserted > 0 {
            self.dirty.store(true, Ordering::Release);
            self.unsaved_count.fetch_add(inserted, Ordering::Release);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Storage;
    use oi_types::{ContentChunk, StakeInfo};

    fn make_stake_info(domain: &str, usd: f64) -> StakeInfo {
        StakeInfo {
            domain: domain.to_string(),
            amount_lamports: (usd * 1_000_000.0) as u64,
            amount_usd: usd,
            staked_at: 0,
            cooldown_until: None,
            wallet: "11111111111111111111111111111111".to_string(),
            dns_verified: false,
            reputation: 1.0,
            entity: None,
            disputed: false,
        }
    }

    struct MockStakeRegistry {
        stakes: HashMap<String, StakeInfo>,
    }

    impl StakeRegistry for MockStakeRegistry {
        fn get_stake(&self, domain: &str) -> oi_types::Result<Option<StakeInfo>> {
            Ok(self.stakes.get(domain).cloned())
        }
        fn list_active_stakes(&self) -> oi_types::Result<Vec<StakeInfo>> {
            Ok(self.stakes.values().cloned().collect())
        }
        fn is_staked(&self, domain: &str) -> oi_types::Result<bool> {
            Ok(self.stakes.contains_key(domain))
        }
    }

    fn make_entity_chunk(
        id: &str, text: &str, embedding: Vec<f32>,
        url: &str, title: &str,
        entity: Option<&str>, content_kind: Option<&str>,
    ) -> ContentChunk {
        ContentChunk {
            chunk_id: id.to_string(),
            content: text.to_string(),
            embedding: Some(embedding),
            seq: 0,
            meta: Some(ChunkMeta {
                url: url.to_string(),
                title: title.to_string(),
                domain: url.split("//").nth(1).and_then(|s| s.split('/').next()).unwrap_or("").to_string(),
                content_hash: format!("hash_{id}"),
                crawled_at: Some(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs(),
                ),
                content_type: Some("text/html".to_string()),
                entity: entity.map(|s| s.to_string()),
                content_kind: content_kind.map(|s| s.to_string()),
                ..Default::default()
            }),
        }
    }

    fn make_chunk(id: &str, text: &str, embedding: Vec<f32>) -> ContentChunk {
        ContentChunk {
            chunk_id: id.to_string(),
            content: text.to_string(),
            embedding: Some(embedding),
            seq: 0,
            meta: None,
        }
    }

    fn make_chunk_with_meta(id: &str, text: &str, embedding: Vec<f32>, url: &str, title: &str) -> ContentChunk {
        ContentChunk {
            chunk_id: id.to_string(),
            content: text.to_string(),
            embedding: Some(embedding),
            seq: 0,
            meta: Some(ChunkMeta {
                url: url.to_string(),
                title: title.to_string(),
                domain: url.split("//").nth(1).and_then(|s| s.split('/').next()).unwrap_or("").to_string(),
                content_hash: format!("hash_{id}"),
                crawled_at: Some(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs(),
                ),
                content_type: Some("text/html".to_string()),
                entity: None,
                content_kind: None,
                ..Default::default()
            }),
        }
    }

    #[test]
    fn test_hybrid_ingest_and_query() {
        let dim = 4;
        let index = HybridIndex::new(dim);

        let chunks = vec![
            make_chunk("c1", "rust programming language systems", vec![1.0, 0.0, 0.0, 0.0]),
            make_chunk("c2", "python scripting language data science", vec![0.0, 1.0, 0.0, 0.0]),
            make_chunk("c3", "rust web framework actix", vec![0.9, 0.1, 0.0, 0.0]),
        ];
        index.ingest(&chunks).unwrap();
        assert_eq!(index.doc_count(), 3);

        let results = index
            .query(
                &[1.0, 0.0, 0.0, 0.0],
                "rust programming",
                &QueryFilters::default(),
                2,
            )
            .unwrap();

        assert!(!results.is_empty());
        assert!(results.len() <= 2);
    }

    #[test]
    fn test_recrawl_replaces_changed_url() {
        // A re-crawled page whose content changed must REPLACE its old
        // chunks, not stack next to them (regression: one re-seed doubled
        // the corpus 545 → 1012 docs).
        let index = HybridIndex::new(4);
        let url = "https://example.com/page";
        index
            .ingest(&[
                make_chunk_with_meta("v1a", "old content alpha version", vec![1.0, 0.0, 0.0, 0.0], url, "Page"),
                make_chunk_with_meta("v1b", "old content beta version", vec![0.9, 0.1, 0.0, 0.0], url, "Page"),
            ])
            .unwrap();
        assert_eq!(index.doc_count(), 2);

        // Re-crawl: content drifted → new hashes, same URL.
        index
            .ingest(&[
                make_chunk_with_meta("v2a", "new content alpha revised", vec![1.0, 0.0, 0.0, 0.0], url, "Page"),
                make_chunk_with_meta("v2b", "new content beta revised", vec![0.9, 0.1, 0.0, 0.0], url, "Page"),
            ])
            .unwrap();
        assert_eq!(index.doc_count(), 2, "changed URL must replace, not stack");

        let results = index
            .query(&[1.0, 0.0, 0.0, 0.0], "content alpha", &QueryFilters::default(), 4)
            .unwrap();
        assert!(!results.is_empty());
        for r in &results {
            assert!(
                !r.content_md.contains("old content"),
                "stale chunks must be gone, got: {}",
                r.content_md
            );
        }
    }

    #[test]
    fn test_recrawl_unchanged_url_is_noop() {
        let index = HybridIndex::new(4);
        let url = "https://example.com/stable";
        let batch = vec![
            make_chunk_with_meta("s1", "stable content one", vec![1.0, 0.0, 0.0, 0.0], url, "Stable"),
            make_chunk_with_meta("s2", "stable content two", vec![0.0, 1.0, 0.0, 0.0], url, "Stable"),
        ];
        index.ingest(&batch).unwrap();
        index.ingest(&batch).unwrap();
        assert_eq!(index.doc_count(), 2, "unchanged re-crawl must dedup to a no-op");
    }

    #[test]
    fn test_default_chunk_cap_applies() {
        // Default config caps chunks per URL at 8 (benchmark-proven) so huge
        // pages can't flood the index.
        let index = HybridIndex::new(4);
        let chunks: Vec<ContentChunk> = (0..12)
            .map(|i| {
                make_chunk_with_meta(
                    &format!("cap{i}"),
                    &format!("distinct chunk text number {i} with unique words w{i}x w{i}y"),
                    vec![1.0 - i as f32 * 0.05, i as f32 * 0.05, 0.0, 0.0],
                    "https://example.com/huge-page",
                    "Huge Page",
                )
            })
            .collect();
        index.ingest(&chunks).unwrap();
        assert_eq!(index.doc_count(), 8);
    }

    #[test]
    fn test_hybrid_empty() {
        let index = HybridIndex::new(4);
        let results = index
            .query(&[1.0, 0.0, 0.0, 0.0], "test", &QueryFilters::default(), 5)
            .unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_metadata_populates_results() {
        let dim = 4;
        let index = HybridIndex::new(dim);

        let chunks = vec![
            make_chunk_with_meta(
                "c1", "rust programming guide", vec![1.0, 0.0, 0.0, 0.0],
                "https://example.com/rust", "Rust Guide",
            ),
        ];
        index.ingest(&chunks).unwrap();

        let results = index
            .query(&[1.0, 0.0, 0.0, 0.0], "rust", &QueryFilters::default(), 5)
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url, "https://example.com/rust");
        assert_eq!(results[0].title, "Rust Guide");
        assert_eq!(results[0].content_hash, "hash_c1");
        assert_eq!(results[0].rank, 1);
    }

    #[test]
    fn test_content_type_filter() {
        let dim = 4;
        let index = HybridIndex::new(dim);

        let mut pdf_chunk = make_chunk_with_meta(
            "c1", "pdf document content", vec![1.0, 0.0, 0.0, 0.0],
            "https://example.com/doc.pdf", "PDF Doc",
        );
        pdf_chunk.meta.as_mut().unwrap().content_type = Some("application/pdf".to_string());

        let html_chunk = make_chunk_with_meta(
            "c2", "html page content about programming", vec![0.9, 0.1, 0.0, 0.0],
            "https://example.com/page", "HTML Page",
        );

        index.ingest(&[pdf_chunk, html_chunk]).unwrap();

        // Filter for PDF only
        let filters = QueryFilters {
            types: vec!["pdf".to_string()],
            ..Default::default()
        };
        let results = index
            .query(&[1.0, 0.0, 0.0, 0.0], "content", &filters, 5)
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "PDF Doc");
    }

    #[test]
    fn test_freshness_filter() {
        let dim = 4;
        let index = HybridIndex::new(dim);

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let mut recent = make_chunk_with_meta(
            "c1", "recent news article", vec![1.0, 0.0, 0.0, 0.0],
            "https://example.com/new", "Recent",
        );
        recent.meta.as_mut().unwrap().crawled_at = Some(now - 3600); // 1 hour ago

        let mut old = make_chunk_with_meta(
            "c2", "old news article stale", vec![0.9, 0.1, 0.0, 0.0],
            "https://example.com/old", "Old",
        );
        old.meta.as_mut().unwrap().crawled_at = Some(now - 86400 * 30); // 30 days ago

        index.ingest(&[recent, old]).unwrap();

        // Filter for last 1 day
        let filters = QueryFilters {
            freshness: Some("1d".to_string()),
            ..Default::default()
        };
        let results = index
            .query(&[1.0, 0.0, 0.0, 0.0], "news article", &filters, 5)
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Recent");
    }

    #[test]
    fn test_parse_freshness() {
        assert_eq!(parse_freshness_secs("1h"), Some(3600));
        assert_eq!(parse_freshness_secs("1d"), Some(86400));
        assert_eq!(parse_freshness_secs("1w"), Some(604800));
        assert_eq!(parse_freshness_secs("1m"), Some(2_592_000));
        assert_eq!(parse_freshness_secs("1y"), Some(31_536_000));
        assert_eq!(parse_freshness_secs("2d"), Some(172800));
        assert_eq!(parse_freshness_secs("bad"), None);
        assert_eq!(parse_freshness_secs(""), None);
    }

    #[test]
    fn test_custom_config_weights() {
        // Verify with_config stores and uses custom weights.
        // Use content with NO BM25 overlap with query so ranking is purely vector-based.
        let config = HybridConfig {
            semantic_weight: 0.9,
            bm25_weight: 0.1,
            ..Default::default()
        };
        let index = HybridIndex::with_config(4, config);

        let chunks = vec![
            make_chunk("c1", "alpha beta gamma delta", vec![1.0, 0.0, 0.0, 0.0]),
            make_chunk("c2", "epsilon zeta eta theta", vec![0.0, 1.0, 0.0, 0.0]),
        ];
        index.ingest(&chunks).unwrap();

        // Query vector is closest to c1; query text matches neither doc in BM25
        let results = index
            .query(&[1.0, 0.0, 0.0, 0.0], "unrelated search terms", &QueryFilters::default(), 2)
            .unwrap();

        assert_eq!(results.len(), 2);
        // c1 has perfect vector similarity, c2 has zero — c1 must rank first
        assert_eq!(results[0].result_id, "c1");
        assert_eq!(results[1].result_id, "c2");
    }

    #[test]
    fn test_rrf_fusion() {
        let config = HybridConfig {
            fusion: FusionMethod::Rrf,
            ..Default::default()
        };
        let index = HybridIndex::with_config(4, config);

        let chunks = vec![
            make_chunk("c1", "rust programming language systems", vec![1.0, 0.0, 0.0, 0.0]),
            make_chunk("c2", "python scripting language data science", vec![0.0, 1.0, 0.0, 0.0]),
            make_chunk("c3", "rust web framework actix", vec![0.9, 0.1, 0.0, 0.0]),
        ];
        index.ingest(&chunks).unwrap();

        let results = index
            .query(&[1.0, 0.0, 0.0, 0.0], "rust programming", &QueryFilters::default(), 3)
            .unwrap();

        assert!(!results.is_empty());
        assert!(results.len() <= 3);
    }

    #[test]
    fn test_pool_multiplier() {
        let config = HybridConfig {
            pool_multiplier: 4,
            ..Default::default()
        };
        let index = HybridIndex::with_config(4, config);

        let chunks = vec![
            make_chunk("c1", "rust programming language", vec![1.0, 0.0, 0.0, 0.0]),
            make_chunk("c2", "python data science", vec![0.0, 1.0, 0.0, 0.0]),
        ];
        index.ingest(&chunks).unwrap();

        let results = index
            .query(&[1.0, 0.0, 0.0, 0.0], "rust", &QueryFilters::default(), 2)
            .unwrap();

        assert!(!results.is_empty());
    }

    // ── Stake enrichment tests ──────────────────────────────────────────

    #[test]
    fn test_stake_enrichment_boosts_staked_content() {
        let dim = 4;
        let mut stakes = HashMap::new();
        stakes.insert("example.com".to_string(), make_stake_info("example.com", 1000.0));
        let registry = Arc::new(MockStakeRegistry { stakes });

        let index = HybridIndex::with_config(dim, HybridConfig::default())
            .with_stake_registry(registry);

        let chunks = vec![
            make_chunk_with_meta(
                "c1", "rust programming guide covering ownership and borrowing in detail", vec![1.0, 0.0, 0.0, 0.0],
                "https://example.com/rust", "Staked Rust Guide",
            ),
            make_chunk_with_meta(
                "c2", "introduction to the rust compiler architecture and build system internals", vec![0.99, 0.01, 0.0, 0.0],
                "https://other.com/rust", "Unstaked Rust Guide",
            ),
        ];
        index.ingest(&chunks).unwrap();

        let results = index
            .query(&[1.0, 0.0, 0.0, 0.0], "rust programming", &QueryFilters::default(), 2)
            .unwrap();

        assert_eq!(results.len(), 2);
        // c1 ranks first on RELEVANCE (exact vector match), not stake — stake is
        // enriched onto the result (stake_truth > 0) but never moves the score.
        assert_eq!(results[0].url, "https://example.com/rust");
        assert!(results[0].stake_truth > 0);
        assert!(results[0].final_score > results[1].final_score);
    }

    #[test]
    fn test_publisher_domain_passthrough() {
        let dim = 4;
        let mut stakes = HashMap::new();
        stakes.insert("nytimes.com".to_string(), make_stake_info("nytimes.com", 1000.0));
        let registry = Arc::new(MockStakeRegistry { stakes });

        let index = HybridIndex::with_config(dim, HybridConfig::default())
            .with_stake_registry(registry);

        let chunks = vec![
            make_chunk_with_meta(
                "c1", "breaking news article economics", vec![1.0, 0.0, 0.0, 0.0],
                "https://nytimes.com/article", "Breaking News",
            ),
        ];
        index.ingest(&chunks).unwrap();

        let results = index
            .query(&[1.0, 0.0, 0.0, 0.0], "breaking news", &QueryFilters::default(), 1)
            .unwrap();

        assert_eq!(results.len(), 1);
        // Publisher domain: stake passes through
        assert!(results[0].stake_truth > 0);
        assert_eq!(results[0].stake_type, "domain");
    }

    #[test]
    fn test_ugc_domain_stake_zero_passthrough() {
        let dim = 4;
        let mut stakes = HashMap::new();
        // Stake on youtube.com (UGC) — should give ZERO boost to channel content
        stakes.insert("youtube.com".to_string(), make_stake_info("youtube.com", 5000.0));
        let registry = Arc::new(MockStakeRegistry { stakes });

        let index = HybridIndex::with_config(dim, HybridConfig::default())
            .with_stake_registry(registry);

        let chunks = vec![
            make_entity_chunk(
                "c1", "linear algebra video math", vec![1.0, 0.0, 0.0, 0.0],
                "https://youtube.com/watch?v=abc", "Linear Algebra",
                Some("youtube.com/c/3blue1brown"), Some("video"),
            ),
        ];
        index.ingest(&chunks).unwrap();

        let results = index
            .query(&[1.0, 0.0, 0.0, 0.0], "linear algebra", &QueryFilters::default(), 1)
            .unwrap();

        assert_eq!(results.len(), 1);
        // UGC: domain stake gives zero passthrough — no entity stake either
        assert_eq!(results[0].stake_truth, 0);
    }

    #[test]
    fn test_entity_stake_on_ugc_platform() {
        let dim = 4;
        let mut stakes = HashMap::new();
        // Stake on specific YouTube channel — should boost that channel's content
        stakes.insert(
            "youtube.com/c/3blue1brown".to_string(),
            make_stake_info("youtube.com/c/3blue1brown", 1000.0),
        );
        let registry = Arc::new(MockStakeRegistry { stakes });

        let index = HybridIndex::with_config(dim, HybridConfig::default())
            .with_stake_registry(registry);

        let chunks = vec![
            make_entity_chunk(
                "c1", "linear algebra video math", vec![1.0, 0.0, 0.0, 0.0],
                "https://youtube.com/watch?v=abc", "Linear Algebra",
                Some("youtube.com/c/3blue1brown"), Some("video"),
            ),
        ];
        index.ingest(&chunks).unwrap();

        let results = index
            .query(&[1.0, 0.0, 0.0, 0.0], "linear algebra", &QueryFilters::default(), 1)
            .unwrap();

        assert_eq!(results.len(), 1);
        assert!(results[0].stake_truth > 0);
        assert_eq!(results[0].stake_type, "entity");
    }

    #[test]
    fn test_entity_equals_domain_no_double_count() {
        let dim = 4;
        let mut stakes = HashMap::new();
        // Only domain-level stake (entity == domain case)
        stakes.insert("example.com".to_string(), make_stake_info("example.com", 500.0));
        let registry = Arc::new(MockStakeRegistry { stakes });

        let index = HybridIndex::with_config(dim, HybridConfig::default())
            .with_stake_registry(registry);

        // Chunk where entity == domain — should NOT double-count
        let chunks = vec![
            make_entity_chunk(
                "c1", "example site content page", vec![1.0, 0.0, 0.0, 0.0],
                "https://example.com/page", "Example",
                Some("example.com"), None,
            ),
        ];
        index.ingest(&chunks).unwrap();

        let results = index
            .query(&[1.0, 0.0, 0.0, 0.0], "example site", &QueryFilters::default(), 1)
            .unwrap();

        assert_eq!(results.len(), 1);
        // Should be "domain" type since entity lookup was skipped (entity == domain)
        assert_eq!(results[0].stake_type, "domain");
    }

    // ── Filter tests ────────────────────────────────────────────────────

    #[test]
    fn test_entity_filter() {
        let dim = 4;
        let index = HybridIndex::new(dim);

        let chunks = vec![
            make_entity_chunk(
                "c1", "rust programming video tutorial", vec![1.0, 0.0, 0.0, 0.0],
                "https://youtube.com/watch?v=abc", "Rust Video",
                Some("youtube.com/c/rustchannel"), Some("video"),
            ),
            make_entity_chunk(
                "c2", "rust programming article guide", vec![0.9, 0.1, 0.0, 0.0],
                "https://blog.com/rust", "Rust Article",
                None, None,
            ),
        ];
        index.ingest(&chunks).unwrap();

        let filters = QueryFilters {
            entities: vec!["youtube.com/c/rustchannel".to_string()],
            ..Default::default()
        };
        let results = index
            .query(&[1.0, 0.0, 0.0, 0.0], "rust programming", &filters, 2)
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entity.as_deref(), Some("youtube.com/c/rustchannel"));
    }

    #[test]
    fn test_content_kind_filter() {
        let dim = 4;
        let index = HybridIndex::new(dim);

        let chunks = vec![
            make_entity_chunk(
                "c1", "linear algebra explained visual", vec![1.0, 0.0, 0.0, 0.0],
                "https://youtube.com/watch?v=abc", "Math Video",
                Some("youtube.com/c/3blue1brown"), Some("video"),
            ),
            make_entity_chunk(
                "c2", "linear algebra explained text post", vec![0.9, 0.1, 0.0, 0.0],
                "https://reddit.com/r/math/post1", "Math Post",
                Some("reddit.com/r/math"), Some("post"),
            ),
        ];
        index.ingest(&chunks).unwrap();

        let filters = QueryFilters {
            content_kinds: vec!["video".to_string()],
            ..Default::default()
        };
        let results = index
            .query(&[1.0, 0.0, 0.0, 0.0], "linear algebra", &filters, 2)
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].content_kind.as_deref(), Some("video"));
    }

    #[test]
    fn test_no_metadata_rejected_with_entity_filter() {
        let dim = 4;
        let index = HybridIndex::new(dim);

        // Chunk with no metadata
        let chunks = vec![
            make_chunk("c1", "rust programming language", vec![1.0, 0.0, 0.0, 0.0]),
        ];
        index.ingest(&chunks).unwrap();

        let filters = QueryFilters {
            entities: vec!["youtube.com/c/test".to_string()],
            ..Default::default()
        };
        let results = index
            .query(&[1.0, 0.0, 0.0, 0.0], "rust", &filters, 5)
            .unwrap();

        // No metadata → can't match entity filter → rejected
        assert!(results.is_empty());
    }

    #[test]
    fn test_no_metadata_rejected_with_content_kind_filter() {
        let dim = 4;
        let index = HybridIndex::new(dim);

        let chunks = vec![
            make_chunk("c1", "rust programming language", vec![1.0, 0.0, 0.0, 0.0]),
        ];
        index.ingest(&chunks).unwrap();

        let filters = QueryFilters {
            content_kinds: vec!["video".to_string()],
            ..Default::default()
        };
        let results = index
            .query(&[1.0, 0.0, 0.0, 0.0], "rust", &filters, 5)
            .unwrap();

        assert!(results.is_empty());
    }

    #[test]
    fn test_save_load_roundtrip() {
        let dim = 4;
        let index = HybridIndex::new(dim);

        let chunks = vec![
            make_chunk_with_meta(
                "c1", "rust programming guide", vec![1.0, 0.0, 0.0, 0.0],
                "https://example.com/rust", "Rust Guide",
            ),
            make_chunk_with_meta(
                "c2", "python data science", vec![0.0, 1.0, 0.0, 0.0],
                "https://example.com/python", "Python Guide",
            ),
        ];
        index.ingest(&chunks).unwrap();

        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::new(dir.path()).unwrap();
        index.save(&storage).unwrap();

        let loaded = HybridIndex::load(&storage).unwrap();
        assert_eq!(loaded.doc_count(), 2);

        let results = loaded
            .query(&[1.0, 0.0, 0.0, 0.0], "rust", &QueryFilters::default(), 5)
            .unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].url, "https://example.com/rust");
    }

    #[test]
    fn test_load_nonexistent_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::new(dir.path()).unwrap();
        let result = HybridIndex::load(&storage);
        assert!(result.is_err());
    }

    #[test]
    fn test_no_metadata_passes_without_filters() {
        let dim = 4;
        let index = HybridIndex::new(dim);

        let chunks = vec![
            make_chunk("c1", "rust programming language", vec![1.0, 0.0, 0.0, 0.0]),
        ];
        index.ingest(&chunks).unwrap();

        // No filters → metadata-less chunks should still appear
        let results = index
            .query(&[1.0, 0.0, 0.0, 0.0], "rust", &QueryFilters::default(), 5)
            .unwrap();

        assert_eq!(results.len(), 1);
    }

    // ── Sybil resistance / dedup tests ─────────────────────────────────

    #[test]
    fn test_exact_duplicate_rejected_at_ingest() {
        let dim = 4;
        let index = HybridIndex::new(dim);

        let chunk1 = ContentChunk {
            chunk_id: "c1".to_string(),
            content: "rust programming language safety".to_string(),
            embedding: Some(vec![1.0, 0.0, 0.0, 0.0]),
            seq: 0,
            meta: Some(ChunkMeta {
                url: "https://example.com/rust".to_string(),
                title: "Rust".to_string(),
                domain: "example.com".to_string(),
                content_hash: "deadbeef123".to_string(),
                ..Default::default()
            }),
        };

        // Same content_hash, different chunk_id and URL (sybil duplicate)
        let chunk2 = ContentChunk {
            chunk_id: "c2".to_string(),
            content: "rust programming language safety".to_string(),
            embedding: Some(vec![1.0, 0.0, 0.0, 0.0]),
            seq: 1,
            meta: Some(ChunkMeta {
                url: "https://spam.com/rust".to_string(),
                title: "Rust Copy".to_string(),
                domain: "spam.com".to_string(),
                content_hash: "deadbeef123".to_string(), // same hash
                ..Default::default()
            }),
        };

        index.ingest(&[chunk1, chunk2]).unwrap();
        assert_eq!(index.doc_count(), 1, "exact duplicate should be rejected");
    }

    #[test]
    fn test_near_duplicate_rejected_at_ingest() {
        let dim = 4;
        let index = HybridIndex::new(dim);

        let chunk1 = ContentChunk {
            chunk_id: "c1".to_string(),
            content: "Rust is a systems programming language focused on safety and performance for concurrent applications".to_string(),
            embedding: Some(vec![1.0, 0.0, 0.0, 0.0]),
            seq: 0,
            meta: Some(ChunkMeta {
                url: "https://example.com/rust".to_string(),
                title: "Rust".to_string(),
                domain: "example.com".to_string(),
                content_hash: "hash_a".to_string(),
                ..Default::default()
            }),
        };

        // Near-verbatim mirror (one appended word) — different content_hash
        // but within the default Hamming threshold (3). Paraphrase-level
        // copies (a swapped word, distance ~4-8) are consciously allowed by
        // the scale-calibrated default; see test_paraphrase_rejected_at_loose_threshold.
        let chunk2 = ContentChunk {
            chunk_id: "c2".to_string(),
            content: "Rust is a systems programming language focused on safety and performance for concurrent applications.".to_string(),
            embedding: Some(vec![0.99, 0.01, 0.0, 0.0]),
            seq: 1,
            meta: Some(ChunkMeta {
                url: "https://spam.com/rust".to_string(),
                title: "Rust Copy".to_string(),
                domain: "spam.com".to_string(),
                content_hash: "hash_b".to_string(), // different hash
                ..Default::default()
            }),
        };

        index.ingest(&[chunk1, chunk2]).unwrap();
        assert_eq!(index.doc_count(), 1, "near-duplicate should be rejected");
    }

    #[test]
    fn test_paraphrase_rejected_at_loose_threshold() {
        // Word-swap paraphrases (Hamming ~4-8) are caught only when the
        // operator opts into a looser threshold; the scale-calibrated default
        // (3) lets them through — at 100k docs a threshold of 8 rejected 96%
        // of distinct passages as false positives.
        let config = HybridConfig { simhash_threshold: 8, ..HybridConfig::default() };
        let index = HybridIndex::with_config(4, config);
        let mk = |id: &str, content: &str, hash: &str, url: &str| ContentChunk {
            chunk_id: id.to_string(),
            content: content.to_string(),
            embedding: Some(vec![1.0, 0.0, 0.0, 0.0]),
            seq: 0,
            meta: Some(ChunkMeta {
                url: url.to_string(),
                title: "t".to_string(),
                domain: "d".to_string(),
                content_hash: hash.to_string(),
                ..Default::default()
            }),
        };
        index
            .ingest(&[
                mk("p1", "Rust is a systems programming language focused on safety and performance for concurrent applications", "h1", "https://a.com/x"),
                mk("p2", "Rust is a systems programming language focused on safety and speed for concurrent applications", "h2", "https://b.com/x"),
            ])
            .unwrap();
        assert_eq!(index.doc_count(), 1, "paraphrase should be rejected at threshold 8");
    }

    #[test]
    fn test_different_content_not_rejected() {
        let dim = 4;
        let index = HybridIndex::new(dim);

        let chunk1 = ContentChunk {
            chunk_id: "c1".to_string(),
            content: "Rust is a systems programming language focused on safety and performance".to_string(),
            embedding: Some(vec![1.0, 0.0, 0.0, 0.0]),
            seq: 0,
            meta: Some(ChunkMeta {
                url: "https://example.com/rust".to_string(),
                title: "Rust".to_string(),
                domain: "example.com".to_string(),
                content_hash: "hash_a".to_string(),
                ..Default::default()
            }),
        };

        let chunk2 = ContentChunk {
            chunk_id: "c2".to_string(),
            content: "The global economy shows signs of recovery as inflation rates decline across major markets and trade volumes increase".to_string(),
            embedding: Some(vec![0.0, 1.0, 0.0, 0.0]),
            seq: 1,
            meta: Some(ChunkMeta {
                url: "https://other.com/economy".to_string(),
                title: "Economy".to_string(),
                domain: "other.com".to_string(),
                content_hash: "hash_b".to_string(),
                ..Default::default()
            }),
        };

        index.ingest(&[chunk1, chunk2]).unwrap();
        assert_eq!(index.doc_count(), 2, "different content should both be ingested");
    }

    #[test]
    fn test_per_domain_cap_in_query() {
        let dim = 4;
        let index = HybridIndex::new(dim);

        // Ingest 5 chunks from the same domain
        let mut chunks = Vec::new();
        for i in 0..5 {
            chunks.push(ContentChunk {
                chunk_id: format!("c{i}"),
                // Make content sufficiently different to avoid near-dup rejection
                content: format!("unique topic number {i} about subject {}", i * 100 + 42),
                embedding: Some(vec![1.0 - i as f32 * 0.1, i as f32 * 0.1, 0.0, 0.0]),
                seq: i as u64,
                meta: Some(ChunkMeta {
                    url: format!("https://example.com/page{i}"),
                    title: format!("Page {i}"),
                    domain: "example.com".to_string(),
                    content_hash: format!("hash_{i}"),
                    ..Default::default()
                }),
            });
        }
        // Add one chunk from a different domain
        chunks.push(ContentChunk {
            chunk_id: "other".to_string(),
            content: "completely different website about gardening flowers and plants".to_string(),
            embedding: Some(vec![0.5, 0.5, 0.0, 0.0]),
            seq: 5,
            meta: Some(ChunkMeta {
                url: "https://other.com/page".to_string(),
                title: "Other".to_string(),
                domain: "other.com".to_string(),
                content_hash: "hash_other".to_string(),
                ..Default::default()
            }),
        });

        index.ingest(&chunks).unwrap();

        let results = index
            .query(&[1.0, 0.0, 0.0, 0.0], "unique topic", &QueryFilters::default(), 10)
            .unwrap();

        // Count per domain
        let mut domain_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for r in &results {
            *domain_counts.entry(extract_domain_from_url(&r.url)).or_insert(0) += 1;
        }

        for (domain, count) in &domain_counts {
            assert!(
                *count <= super::MAX_PER_DOMAIN,
                "domain {domain} has {count} results, max is {}",
                super::MAX_PER_DOMAIN
            );
        }

        // Verify ranks are sequential
        for (i, r) in results.iter().enumerate() {
            assert_eq!(r.rank, (i + 1) as u32, "ranks should be sequential");
        }
    }

    #[test]
    fn test_save_load_roundtrip_with_dedup_data() {
        let dim = 4;
        let index = HybridIndex::new(dim);

        let chunks = vec![
            make_chunk_with_meta(
                "c1", "rust programming guide", vec![1.0, 0.0, 0.0, 0.0],
                "https://example.com/rust", "Rust Guide",
            ),
            make_chunk_with_meta(
                "c2", "python data science introduction", vec![0.0, 1.0, 0.0, 0.0],
                "https://example.com/python", "Python Guide",
            ),
        ];
        index.ingest(&chunks).unwrap();

        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::new(dir.path()).unwrap();
        index.save(&storage).unwrap();

        let loaded = HybridIndex::load(&storage).unwrap();
        assert_eq!(loaded.doc_count(), 2);

        // Content hashes should have been persisted — exact duplicate should be rejected
        let dup = ContentChunk {
            chunk_id: "c3".to_string(),
            content: "rust programming guide".to_string(),
            embedding: Some(vec![1.0, 0.0, 0.0, 0.0]),
            seq: 2,
            meta: Some(ChunkMeta {
                url: "https://spam.com/rust".to_string(),
                title: "Spam".to_string(),
                domain: "spam.com".to_string(),
                content_hash: "hash_c1".to_string(), // same as c1
                ..Default::default()
            }),
        };
        loaded.ingest(&[dup]).unwrap();
        assert_eq!(loaded.doc_count(), 2, "exact dup should be rejected after load");

        // Verify search still works
        let results = loaded
            .query(&[1.0, 0.0, 0.0, 0.0], "rust", &QueryFilters::default(), 5)
            .unwrap();
        assert!(!results.is_empty());
    }

    #[test]
    fn test_get_chunks_for_replication() {
        let dim = 4;
        let index = HybridIndex::new(dim);

        let mut tech_chunk = make_chunk_with_meta(
            "c1", "rust programming guide", vec![1.0, 0.0, 0.0, 0.0],
            "https://example.com/rust", "Rust Guide",
        );
        tech_chunk.meta.as_mut().unwrap().shard_id = Some("topic-technology".to_string());

        let mut sci_chunk = make_chunk_with_meta(
            "c2", "quantum physics research paper discoveries", vec![0.0, 1.0, 0.0, 0.0],
            "https://example.com/quantum", "Quantum Physics",
        );
        sci_chunk.meta.as_mut().unwrap().shard_id = Some("topic-science".to_string());

        index.ingest(&[tech_chunk, sci_chunk]).unwrap();

        // Get all chunks (no shard filter)
        let all = index.get_chunks_for_replication(None, 100).unwrap();
        assert_eq!(all.len(), 2);

        // Filter by technology shard
        let tech = index.get_chunks_for_replication(Some("topic-technology"), 100).unwrap();
        assert_eq!(tech.len(), 1);
        assert_eq!(tech[0].chunk_id, "c1");
        assert!(tech[0].embedding.is_some());
        assert!(!tech[0].content.is_empty());

        // Filter by science shard
        let sci = index.get_chunks_for_replication(Some("topic-science"), 100).unwrap();
        assert_eq!(sci.len(), 1);
        assert_eq!(sci[0].chunk_id, "c2");

        // Non-existent shard
        let none = index.get_chunks_for_replication(Some("topic-sports"), 100).unwrap();
        assert!(none.is_empty());

        // max_chunks cap
        let capped = index.get_chunks_for_replication(None, 1).unwrap();
        assert_eq!(capped.len(), 1);
    }

    #[test]
    fn test_shard_filter() {
        let dim = 4;
        let index = HybridIndex::new(dim);

        let mut tech_chunk = make_chunk_with_meta(
            "c1", "rust programming guide covering systems design", vec![1.0, 0.0, 0.0, 0.0],
            "https://example.com/rust", "Rust Guide",
        );
        tech_chunk.meta.as_mut().unwrap().shard_id = Some("topic-technology".to_string());

        let mut sci_chunk = make_chunk_with_meta(
            "c2", "quantum physics research paper discoveries", vec![0.9, 0.1, 0.0, 0.0],
            "https://example.com/quantum", "Quantum Physics",
        );
        sci_chunk.meta.as_mut().unwrap().shard_id = Some("topic-science".to_string());

        index.ingest(&[tech_chunk, sci_chunk]).unwrap();

        // Filter for technology only
        let filters = QueryFilters {
            shards: vec!["topic-technology".to_string()],
            ..Default::default()
        };
        let results = index
            .query(&[1.0, 0.0, 0.0, 0.0], "programming", &filters, 5)
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Rust Guide");
    }

    #[test]
    fn test_shard_filter_unclassified_passes() {
        let dim = 4;
        let index = HybridIndex::new(dim);

        // Chunk with no shard_id (unclassified / old data)
        let chunk = make_chunk_with_meta(
            "c1", "some content without shard classification", vec![1.0, 0.0, 0.0, 0.0],
            "https://example.com/page", "Unclassified",
        );
        index.ingest(&[chunk]).unwrap();

        // Shard filter is active, but unclassified content should pass through
        let filters = QueryFilters {
            shards: vec!["topic-technology".to_string()],
            ..Default::default()
        };
        let results = index
            .query(&[1.0, 0.0, 0.0, 0.0], "content", &filters, 5)
            .unwrap();

        assert_eq!(results.len(), 1, "unclassified content should pass shard filter");
    }

    #[test]
    fn test_backward_compat_load_without_dedup_segments() {
        // Save with the current code (includes dedup segments), then
        // verify load works. The real backward compat test would need
        // old-format data, but we at least verify the happy path.
        let dim = 4;
        let index = HybridIndex::new(dim);

        let chunks = vec![
            make_chunk_with_meta(
                "c1", "test content for backward compat", vec![1.0, 0.0, 0.0, 0.0],
                "https://example.com/page", "Test",
            ),
        ];
        index.ingest(&chunks).unwrap();

        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::new(dir.path()).unwrap();
        index.save(&storage).unwrap();

        // Delete the new segment files to simulate old format
        let _ = std::fs::remove_file(dir.path().join("content_hashes.bin"));
        let _ = std::fs::remove_file(dir.path().join("simhashes.bin"));

        let loaded = HybridIndex::load(&storage).unwrap();
        assert_eq!(loaded.doc_count(), 1);

        // Content hashes should have been rebuilt from metadata
        // So a duplicate with the same content_hash should be rejected
        let dup = ContentChunk {
            chunk_id: "c2".to_string(),
            content: "test content for backward compat".to_string(),
            embedding: Some(vec![1.0, 0.0, 0.0, 0.0]),
            seq: 1,
            meta: Some(ChunkMeta {
                url: "https://spam.com/page".to_string(),
                title: "Spam".to_string(),
                domain: "spam.com".to_string(),
                content_hash: "hash_c1".to_string(), // same as original
                ..Default::default()
            }),
        };
        loaded.ingest(&[dup]).unwrap();
        assert_eq!(loaded.doc_count(), 1, "dup should be rejected even after backward-compat load");
    }

    #[test]
    fn test_remove_by_url() {
        let dim = 4;
        let index = HybridIndex::new(dim);

        let chunks = vec![
            make_chunk_with_meta(
                "c1", "rust programming guide", vec![1.0, 0.0, 0.0, 0.0],
                "https://example.com/rust", "Rust Guide",
            ),
            make_chunk_with_meta(
                "c2", "python data science", vec![0.0, 1.0, 0.0, 0.0],
                "https://other.com/python", "Python Guide",
            ),
        ];
        index.ingest(&chunks).unwrap();
        assert_eq!(index.doc_count(), 2);

        // Remove by URL
        let removed = index.remove_by_url("https://example.com/rust").unwrap();
        assert_eq!(removed, 1);
        assert_eq!(index.doc_count(), 1);

        // Search should no longer find the removed URL
        let results = index
            .query(&[1.0, 0.0, 0.0, 0.0], "rust", &QueryFilters::default(), 5)
            .unwrap();
        assert!(results.iter().all(|r| r.url != "https://example.com/rust"));

        // The other URL should still be findable
        let results = index
            .query(&[0.0, 1.0, 0.0, 0.0], "python", &QueryFilters::default(), 5)
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url, "https://other.com/python");
    }

    #[test]
    fn test_remove_by_url_nonexistent() {
        let dim = 4;
        let index = HybridIndex::new(dim);

        let chunks = vec![
            make_chunk_with_meta(
                "c1", "rust programming guide", vec![1.0, 0.0, 0.0, 0.0],
                "https://example.com/rust", "Rust Guide",
            ),
        ];
        index.ingest(&chunks).unwrap();

        let removed = index.remove_by_url("https://nonexistent.com/page").unwrap();
        assert_eq!(removed, 0);
        assert_eq!(index.doc_count(), 1);
    }

    #[test]
    fn test_bm25_short_circuit_skips_search() {
        let config = HybridConfig {
            semantic_weight: 1.0,
            bm25_weight: 0.0,
            skip_dedup: true,
            ..Default::default()
        };
        let index = HybridIndex::with_config(4, config);

        let chunks = vec![
            make_chunk("c1", "rust programming language", vec![1.0, 0.0, 0.0, 0.0]),
            make_chunk("c2", "python data science", vec![0.0, 1.0, 0.0, 0.0]),
        ];
        index.ingest(&chunks).unwrap();

        let (results, timings) = index
            .query_timed(&[1.0, 0.0, 0.0, 0.0], "rust programming", &QueryFilters::default(), 2)
            .unwrap();

        // BM25 should be effectively zero (just lock acquisition)
        assert!(timings.bm25_ms < 0.1, "bm25_ms={}, expected near zero", timings.bm25_ms);
        // Results should still come back from HNSW
        assert!(!results.is_empty());
        assert_eq!(results[0].result_id, "c1");
    }

    /// Compare hybrid vs vector-only quality across query categories.
    /// Uses controlled embeddings to isolate BM25's contribution.
    #[test]
    fn test_quality_hybrid_vs_vector_only() {
        let dim = 8;

        // Build a small corpus with controlled embeddings.
        // Embeddings use first 3 dims as "topic signal", rest noise.
        // "rust" cluster: [0.9, 0.1, 0.0, ...]
        // "python" cluster: [0.1, 0.9, 0.0, ...]
        // "error" cluster: [0.0, 0.0, 0.9, ...]
        fn emb(a: f32, b: f32, c: f32) -> Vec<f32> {
            let mut v = vec![a, b, c, 0.01, 0.01, 0.01, 0.01, 0.01];
            let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            for x in &mut v { *x /= norm; }
            v
        }

        let chunks = vec![
            // Rust docs — strong vector match to "rust" queries
            make_chunk("rust1", "rust ownership borrowing lifetimes", emb(0.95, 0.05, 0.0)),
            make_chunk("rust2", "rust async await tokio runtime", emb(0.90, 0.10, 0.0)),
            make_chunk("rust3", "cargo build system dependencies", emb(0.85, 0.05, 0.1)),
            // Python docs
            make_chunk("py1", "python pandas dataframe analysis", emb(0.05, 0.95, 0.0)),
            make_chunk("py2", "python numpy array operations", emb(0.10, 0.90, 0.0)),
            // Error code doc — weak vector signal, strong keyword signal
            make_chunk("err1", "error E2025 borrow checker conflict resolution", emb(0.4, 0.1, 0.5)),
            // API doc — exact function name matters
            make_chunk("api1", "fn get_program_accounts returns all accounts matching filter", emb(0.5, 0.1, 0.4)),
            // Mixed: keyword overlap with rust, but embedding in python cluster
            make_chunk("mixed1", "rust vs python performance comparison benchmark", emb(0.1, 0.85, 0.05)),
        ];

        // --- Hybrid index (80/20) ---
        let hybrid = HybridIndex::with_config(dim, HybridConfig {
            semantic_weight: 0.8, bm25_weight: 0.2, skip_dedup: true, ..Default::default()
        });
        hybrid.ingest(&chunks).unwrap();

        // --- Vector-only index (100/0) ---
        let vector_only = HybridIndex::with_config(dim, HybridConfig {
            semantic_weight: 1.0, bm25_weight: 0.0, skip_dedup: true, ..Default::default()
        });
        vector_only.ingest(&chunks).unwrap();

        let filters = QueryFilters::default();

        // Category 1: Semantic query — embedding close to rust cluster
        // Both modes should find rust docs. BM25 adds little here.
        let q_emb = emb(0.92, 0.08, 0.0);
        let h = hybrid.query(&q_emb, "systems programming memory safety", &filters, 3).unwrap();
        let v = vector_only.query(&q_emb, "systems programming memory safety", &filters, 3).unwrap();
        let h_ids: Vec<&str> = h.iter().map(|r| r.result_id.as_str()).collect();
        let v_ids: Vec<&str> = v.iter().map(|r| r.result_id.as_str()).collect();
        println!("Semantic query:    hybrid={h_ids:?}  vector={v_ids:?}");
        // Both should return rust docs in top-3
        assert!(h_ids.contains(&"rust1") || h_ids.contains(&"rust2"));
        assert!(v_ids.contains(&"rust1") || v_ids.contains(&"rust2"));

        // Category 2: Exact keyword — "error E2025"
        // Embedding is ambiguous (middle of space), but BM25 nails it.
        let q_emb = emb(0.4, 0.2, 0.4);
        let h = hybrid.query(&q_emb, "error E2025", &filters, 3).unwrap();
        let v = vector_only.query(&q_emb, "error E2025", &filters, 3).unwrap();
        let h_ids: Vec<&str> = h.iter().map(|r| r.result_id.as_str()).collect();
        let v_ids: Vec<&str> = v.iter().map(|r| r.result_id.as_str()).collect();
        println!("Exact keyword:     hybrid={h_ids:?}  vector={v_ids:?}");
        let h_has_err = h_ids.iter().position(|&id| id == "err1");
        let v_has_err = v_ids.iter().position(|&id| id == "err1");
        // Hybrid should rank err1 at least as high as vector-only
        // (BM25 keyword match boosts err1)
        println!("  err1 rank: hybrid={h_has_err:?}  vector={v_has_err:?}");

        // Category 3: Function name — "get_program_accounts"
        let q_emb = emb(0.5, 0.1, 0.4);
        let h = hybrid.query(&q_emb, "get_program_accounts", &filters, 3).unwrap();
        let v = vector_only.query(&q_emb, "get_program_accounts", &filters, 3).unwrap();
        let h_ids: Vec<&str> = h.iter().map(|r| r.result_id.as_str()).collect();
        let v_ids: Vec<&str> = v.iter().map(|r| r.result_id.as_str()).collect();
        println!("API name:          hybrid={h_ids:?}  vector={v_ids:?}");
        let h_has_api = h_ids.iter().position(|&id| id == "api1");
        let v_has_api = v_ids.iter().position(|&id| id == "api1");
        println!("  api1 rank: hybrid={h_has_api:?}  vector={v_has_api:?}");

        // Category 4: Keyword overlap but wrong embedding — "rust" in text, python embedding
        // Vector-only correctly ignores mixed1. Hybrid might wrongly boost it.
        let q_emb = emb(0.92, 0.08, 0.0);
        let h = hybrid.query(&q_emb, "rust", &filters, 5).unwrap();
        let v = vector_only.query(&q_emb, "rust", &filters, 5).unwrap();
        let h_ids: Vec<&str> = h.iter().map(|r| r.result_id.as_str()).collect();
        let v_ids: Vec<&str> = v.iter().map(|r| r.result_id.as_str()).collect();
        println!("Keyword mislead:   hybrid={h_ids:?}  vector={v_ids:?}");
        let h_mixed_rank = h_ids.iter().position(|&id| id == "mixed1");
        let v_mixed_rank = v_ids.iter().position(|&id| id == "mixed1");
        println!("  mixed1 rank: hybrid={h_mixed_rank:?}  vector={v_mixed_rank:?}");
        // Vector-only should rank mixed1 LOWER (correct — it's in python cluster)
    }

    #[test]
    fn test_chunks_since_returns_only_newer() {
        let cfg = HybridConfig { skip_dedup: true, ..Default::default() };
        let idx = HybridIndex::with_config(4, cfg);
        let chunks: Vec<ContentChunk> = (0..5)
            .map(|i| ContentChunk {
                chunk_id: format!("chunk-{i}"),
                content: format!("unique content number {i} with distinctly different words and phrases"),
                embedding: Some(vec![1.0, 0.0, 0.0, 0.0]),
                seq: 0,
                meta: Some(ChunkMeta {
                    url: format!("https://example.com/page-{i}"),
                    title: format!("Page {i}"),
                    domain: "example.com".to_string(),
                    content_hash: format!("hash-{i}"),
                    shard_id: Some("topic-technology".to_string()),
                    ..Default::default()
                }),
            })
            .collect();
        idx.ingest(&chunks).unwrap();

        // All chunks should be returned with from_seq=0
        let all = idx.chunks_since(None, 0, 100).unwrap();
        assert_eq!(all.len(), 5);

        // Only chunks after seq 3 should be returned
        let newer = idx.chunks_since(None, 3, 100).unwrap();
        assert_eq!(newer.len(), 2); // seq 4 and 5

        // current_seq should be 5
        assert_eq!(idx.current_seq(), 5);
    }

    #[test]
    fn test_chunks_since_shard_filter() {
        let cfg = HybridConfig { skip_dedup: true, ..Default::default() };
        let idx = HybridIndex::with_config(4, cfg);
        let mut chunks: Vec<ContentChunk> = Vec::new();
        for i in 0..4 {
            let shard = if i < 2 { "topic-science" } else { "topic-tech" };
            chunks.push(ContentChunk {
                chunk_id: format!("chunk-{i}"),
                content: format!("unique content number {i} about {shard} topic area"),
                embedding: Some(vec![1.0, 0.0, 0.0, 0.0]),
                seq: 0,
                meta: Some(ChunkMeta {
                    url: format!("https://example.com/page-{i}"),
                    title: format!("Page {i}"),
                    domain: "example.com".to_string(),
                    content_hash: format!("hash-{i}"),
                    shard_id: Some(shard.to_string()),
                    ..Default::default()
                }),
            });
        }
        idx.ingest(&chunks).unwrap();

        let science = idx.chunks_since(Some("topic-science"), 0, 100).unwrap();
        assert_eq!(science.len(), 2);

        let tech = idx.chunks_since(Some("topic-tech"), 0, 100).unwrap();
        assert_eq!(tech.len(), 2);
    }

    #[test]
    fn test_merge_from_deduplicates() {
        // Use skip_dedup=false so exact hash dedup works (but SimHash won't block distinct content)
        let idx1 = HybridIndex::new(4);
        let idx2 = HybridIndex::new(4);

        // Use very distinct content to avoid SimHash false positives
        let contents = [
            "Rust is a systems programming language focused on safety concurrency and performance",
            "Python is a high-level interpreted language known for readability and rapid prototyping",
            "JavaScript powers the interactive web with event-driven asynchronous non-blocking I/O",
        ];

        let chunks: Vec<ContentChunk> = contents
            .iter()
            .enumerate()
            .map(|(i, content)| ContentChunk {
                chunk_id: format!("chunk-{i}"),
                content: content.to_string(),
                embedding: Some(vec![i as f32, 0.0, 0.0, 0.0]),
                seq: 0,
                meta: Some(ChunkMeta {
                    url: format!("https://example{i}.com/page"),
                    title: format!("Page {i}"),
                    domain: format!("example{i}.com"),
                    content_hash: format!("hash-{i}"),
                    ..Default::default()
                }),
            })
            .collect();

        idx1.ingest(&chunks).unwrap();
        assert_eq!(idx1.doc_count(), 3);

        idx2.ingest(&chunks[..2]).unwrap(); // overlapping

        // Merge idx2 into idx1 — 2 duplicates should be skipped (exact hash match)
        let new = idx1.merge_from(&idx2).unwrap();
        assert_eq!(new, 0); // all were duplicates
        assert_eq!(idx1.doc_count(), 3);
    }

    #[test]
    fn test_document_level_scoring() {
        // Verify that document-level scoring keeps only the best chunk per URL,
        // preventing a large page (many chunks) from drowning out other pages.
        let dim = 4;
        let index = HybridIndex::new(dim);

        // URL-A: 10 chunks (simulates a large API page like Vec.html)
        let mut chunks = Vec::new();
        for i in 0..10 {
            chunks.push(ContentChunk {
                chunk_id: format!("a{i}"),
                content: format!("rust vector method number {i} for data manipulation {}", i * 17 + 3),
                embedding: Some(vec![0.9 - i as f32 * 0.05, 0.1 + i as f32 * 0.05, 0.0, 0.0]),
                seq: i as u64,
                meta: Some(ChunkMeta {
                    url: "https://doc.rust-lang.org/std/vec/struct.Vec.html".to_string(),
                    title: "Vec in std::vec".to_string(),
                    domain: "doc.rust-lang.org".to_string(),
                    content_hash: format!("hash_a{i}"),
                    ..Default::default()
                }),
            });
        }

        // URL-B: 2 chunks (simulates a small handbook page)
        for i in 0..2 {
            chunks.push(ContentChunk {
                chunk_id: format!("b{i}"),
                content: format!("typescript array methods tutorial guide part {i} with examples {}", i * 31 + 7),
                embedding: Some(vec![0.85, 0.15, 0.0, 0.0]),
                seq: i as u64,
                meta: Some(ChunkMeta {
                    url: "https://www.typescriptlang.org/docs/handbook/arrays.html".to_string(),
                    title: "TypeScript Arrays".to_string(),
                    domain: "www.typescriptlang.org".to_string(),
                    content_hash: format!("hash_b{i}"),
                    ..Default::default()
                }),
            });
        }

        index.ingest(&chunks).unwrap();

        let results = index
            .query(
                &[0.9, 0.1, 0.0, 0.0],
                "vector array methods",
                &QueryFilters::default(),
                10,
            )
            .unwrap();

        // Collect URLs from results
        let result_urls: Vec<&str> = results.iter().map(|r| r.url.as_str()).collect();

        // Both URLs should be represented (document-level scoring ensures diversity)
        assert!(
            result_urls.contains(&"https://doc.rust-lang.org/std/vec/struct.Vec.html"),
            "Rust Vec page should appear in results",
        );
        assert!(
            result_urls.contains(&"https://www.typescriptlang.org/docs/handbook/arrays.html"),
            "TypeScript arrays page should appear in results",
        );

        // Each URL should appear at most once (best-chunk-per-page)
        let mut url_counts: HashMap<String, usize> = HashMap::new();
        for r in &results {
            *url_counts.entry(r.url.clone()).or_insert(0) += 1;
        }
        for (url, count) in &url_counts {
            assert_eq!(*count, 1, "URL {url} appeared {count} times, expected 1");
        }
    }
}
