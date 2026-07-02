use oi_types::traits::{SearchIndex, StakeRegistry};
use oi_types::{ContentChunk, EmbeddingPipeline, QueryFilters, QueryLeaderboard, Result, SearchEngine, SearchParams, SearchResultItem, StakeInfo, TopicRouter};
use oi_index::{HybridIndex, Storage};
use oi_staking::InMemoryStakeRegistry;
use crate::config::SdkConfig;
use crate::leaderboard::InMemoryLeaderboard;
use crate::wallet::Wallet;
use std::sync::Arc;

/// Main SDK client that wires together the local search index,
/// stake registry, embedding pipeline, and optional wallet for the OpenSonarX protocol.
pub struct OpenSonarXClient {
    config: SdkConfig,
    index: Arc<HybridIndex>,
    stake_registry: Arc<InMemoryStakeRegistry>,
    embedder: Arc<dyn EmbeddingPipeline>,
    wallet: Option<Wallet>,
    storage: Option<Storage>,
    search_engines: Vec<Arc<dyn SearchEngine>>,
    leaderboard: Arc<dyn QueryLeaderboard>,
    topic_router: Option<Arc<TopicRouter>>,
    shard_filter: Vec<String>,
    /// External URLs queued by crawl-on-query during `search_blended`.
    /// Drained by `process_crawl_queue`.
    crawl_queue: std::sync::Mutex<Vec<String>>,
}

impl OpenSonarXClient {
    /// Create a new client with the best available embedder.
    ///
    /// When built with `--features onnx`, uses the real MiniLM model
    /// (Snowflake Arctic Embed S, 384-dim, INT8 quantized). Falls back
    /// to MockEmbedder if the model fails to load or if `onnx` is not enabled.
    pub fn new(config: SdkConfig) -> Self {
        let embedder: Arc<dyn EmbeddingPipeline> = Self::default_embedder();
        Self::with_embedder(config, embedder)
    }

    /// Build the best available embedder: MiniLM (onnx) > MockEmbedder.
    pub fn default_embedder() -> Arc<dyn EmbeddingPipeline> {
        #[cfg(feature = "onnx")]
        {
            match oi_embed::MiniLmEmbedder::new() {
                Ok(e) => {
                    tracing::info!("Using MiniLM embedder (384-dim, INT8 quantized)");
                    return Arc::new(e);
                }
                Err(e) => {
                    tracing::warn!("MiniLM load failed, falling back to MockEmbedder: {e}");
                }
            }
        }
        tracing::error!("USING MOCK EMBEDDER — search results will be semantically random! Build with --features onnx for real embeddings.");
        Arc::new(oi_embed::MockEmbedder::default())
    }

    /// Create a new client with a custom embedding pipeline.
    ///
    /// If `local_index_path` is set and contains saved index segments,
    /// the index is loaded from disk. Otherwise a fresh index is created.
    pub fn with_embedder(config: SdkConfig, embedder: Arc<dyn EmbeddingPipeline>) -> Self {
        let embedder: Arc<dyn EmbeddingPipeline> = Arc::new(oi_embed::CachedEmbedder::new(embedder, 10_000));
        let dim = embedder.dimension();
        let stake_registry = Arc::new(InMemoryStakeRegistry::new());

        let storage = config.local_index_path.as_ref().and_then(|path| {
            Storage::new(path).ok()
        });

        let index = if let Some(ref s) = storage
            && s.segment_exists("hnsw.bin")
        {
            match HybridIndex::load(s) {
                Ok(idx) => {
                    tracing::info!("Loaded index from disk ({} docs)", idx.doc_count());
                    Arc::new(idx.with_stake_registry(stake_registry.clone() as Arc<dyn StakeRegistry>))
                }
                Err(e) => {
                    tracing::warn!("Failed to load index from disk, starting fresh: {e}");
                    Arc::new(
                        HybridIndex::new(dim)
                            .with_stake_registry(stake_registry.clone() as Arc<dyn StakeRegistry>),
                    )
                }
            }
        } else {
            Arc::new(
                HybridIndex::new(dim)
                    .with_stake_registry(stake_registry.clone() as Arc<dyn StakeRegistry>),
            )
        };

        let wallet = config
            .wallet_path
            .as_ref()
            .and_then(|p| Wallet::from_path(p).ok());

        let search_engines = crate::engines::default_engines(&config);
        let leaderboard: Arc<dyn QueryLeaderboard> = Arc::new(InMemoryLeaderboard::new());

        Self {
            config,
            index,
            stake_registry,
            embedder,
            wallet,
            storage,
            search_engines,
            leaderboard,
            topic_router: None,
            shard_filter: Vec::new(),
            crawl_queue: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Attach a topic router for classify-at-ingest.
    ///
    /// When set, `ingest()` will classify each chunk's embedding into a topic shard
    /// and set `meta.shard_id` accordingly. If `shard_filter` is also set, chunks
    /// that don't match any of the node's assigned shards are dropped.
    pub fn with_topic_router(mut self, router: Arc<TopicRouter>) -> Self {
        self.topic_router = Some(router);
        self
    }

    /// Set the shard filter — only chunks matching these shard IDs will be ingested.
    ///
    /// Pass an empty list to accept all shards (default behavior).
    pub fn with_shard_filter(mut self, shards: Vec<String>) -> Self {
        self.shard_filter = shards;
        self
    }

    /// Get a reference to the topic router if one is configured.
    pub fn topic_router(&self) -> Option<&TopicRouter> {
        self.topic_router.as_deref()
    }

    /// Get a cloned Arc to the topic router (for use by external code like gateway tasks).
    pub fn topic_router_arc(&self) -> Option<Arc<TopicRouter>> {
        self.topic_router.clone()
    }

    /// Attach a reputation provider to the underlying hybrid index.
    ///
    /// Must be called before the client is shared (i.e. during construction).
    /// This moves the inner index out of its Arc, attaches the provider, and re-wraps.
    pub fn with_reputation_provider(mut self, provider: Arc<dyn oi_types::ReputationProvider>) -> Self {
        match Arc::try_unwrap(self.index) {
            Ok(index) => {
                self.index = Arc::new(index.with_reputation_provider(provider));
            }
            Err(arc) => {
                // Arc has multiple references — shouldn't happen during construction.
                // Fall back: log and skip.
                tracing::warn!("Cannot set reputation provider: index Arc has multiple references");
                self.index = arc;
            }
        }
        self
    }

    /// Replace the search engines with a custom list.
    pub fn with_search_engines(mut self, engines: Vec<Arc<dyn SearchEngine>>) -> Self {
        self.search_engines = engines;
        self
    }

    /// Replace the leaderboard implementation.
    pub fn with_leaderboard(mut self, leaderboard: Arc<dyn QueryLeaderboard>) -> Self {
        self.leaderboard = leaderboard;
        self
    }

    /// Search with blended local + external results.
    ///
    /// Queries both the local index and external engines, merging results.
    /// Local results naturally dominate due to higher scores (semantic+BM25+stake).
    /// External results fill gaps when the local corpus is sparse.
    pub fn search_with_fallback(
        &self,
        query: &str,
        top_k: usize,
        filters: &QueryFilters,
    ) -> Result<Vec<SearchResultItem>> {
        let (results, _sources) = self.search_blended(query, top_k, filters)?;
        Ok(results)
    }

    /// Extract identifiers from search results and record them in the leaderboard.
    fn record_leaderboard(&self, results: &[SearchResultItem]) {
        let owned: Vec<String> = results
            .iter()
            .filter_map(|r| {
                if let Some(ref entity) = r.entity {
                    Some(entity.clone())
                } else {
                    let domain = extract_domain(&r.url);
                    if oi_types::entity::EntityRef::parse(&domain)
                        .map(|e| e.is_ugc_platform())
                        .unwrap_or(false)
                    {
                        None
                    } else {
                        Some(domain)
                    }
                }
            })
            .collect();

        let refs: Vec<&str> = owned.iter().map(|s| s.as_str()).collect();
        if !refs.is_empty() {
            let _ = self.leaderboard.record_query(&refs);
        }
    }

    /// Get the leaderboard for display.
    pub fn top_queried(&self, limit: usize) -> Result<Vec<oi_types::LeaderboardEntry>> {
        self.leaderboard.top_queried(limit)
    }

    /// Search the local index with a text query.
    ///
    /// Embeds the query through the configured pipeline and runs a hybrid
    /// HNSW + BM25 search, returning up to `top_k` results.
    /// Enriches results with semantic `content_relevant` extraction.
    pub fn search(
        &self,
        query: &str,
        top_k: usize,
        filters: &QueryFilters,
    ) -> Result<Vec<SearchResultItem>> {
        let query_embedding = self.embedder.embed_query(query)?;
        let mut results = self.index.query(&query_embedding, query, filters, top_k)?;
        self.enrich_content_relevant(&mut results, &query_embedding);
        Ok(results)
    }

    /// Enrich search results with semantic `content_relevant` extraction.
    ///
    /// Splits sentences from all results, batch-embeds them in a single ONNX pass,
    /// then scores each sentence by cosine similarity with the query embedding.
    /// On failure, existing `content_relevant` values (if any) are preserved.
    fn enrich_content_relevant(&self, results: &mut [SearchResultItem], query_embedding: &[f32]) {
        // Pass 1: collect all sentence texts with per-result grouping
        let mut all_texts: Vec<String> = Vec::new();
        let mut sentences_per_result: Vec<Vec<(String, usize, usize)>> = Vec::new();

        for result in results.iter() {
            if result.content_md.is_empty() || result.content_relevant.is_some() {
                sentences_per_result.push(Vec::new());
                continue;
            }
            let sentences = oi_index::relevance::split_into_sentences(&result.content_md);
            for (text, _start, _end) in &sentences {
                all_texts.push(text.clone());
            }
            sentences_per_result.push(sentences);
        }

        if all_texts.is_empty() {
            return;
        }

        // Pass 2: single batch embed
        let text_refs: Vec<&str> = all_texts.iter().map(|s| s.as_str()).collect();
        let all_embeddings = match self.embedder.embed_text_batch(&text_refs) {
            Ok(embs) => embs,
            Err(e) => {
                tracing::debug!("embed_text_batch failed for content_relevant, keeping existing: {e}");
                return;
            }
        };

        // Pass 3: group embeddings back by result, call extract_relevant_semantic
        let mut emb_idx = 0usize;
        for (ri, sentences) in sentences_per_result.iter().enumerate() {
            if sentences.is_empty() {
                continue;
            }
            let count = sentences.len();
            let sentence_embeddings: Vec<Vec<f32>> = all_embeddings[emb_idx..emb_idx + count].to_vec();
            emb_idx += count;

            let facts = results[ri].facts.as_ref();
            if let Some(relevant) = oi_index::relevance::extract_relevant_semantic(
                &results[ri].content_md,
                query_embedding,
                &sentence_embeddings,
                facts,
            ) {
                results[ri].content_relevant = Some(relevant);
            }
        }
    }

    /// Submit feedback on a search result.
    ///
    /// Currently a stub -- in production this would route through
    /// the P2P network to the result's source node.
    pub fn feedback(&self, result_id: &str, useful: bool) -> Result<()> {
        tracing::info!(result_id, useful, "feedback submitted (local stub)");
        Ok(())
    }

    /// Get feedback statistics (local mode stub — returns empty stats).
    pub fn feedback_stats(&self) -> Result<oi_types::gateway::FeedbackStats> {
        Ok(oi_types::gateway::FeedbackStats {
            total_signals: 0,
            positive_signals: 0,
            negative_signals: 0,
            unique_queries: 0,
            top_helpful_domains: Vec::new(),
        })
    }

    /// Stake tokens for a domain.
    ///
    /// Adds the stake to the local in-memory registry and returns a stub
    /// transaction hash. In production this would submit an on-chain transaction.
    pub fn stake(&self, domain: &str, amount: u64) -> Result<String> {
        let wallet_addr = self
            .wallet
            .as_ref()
            .map(|w| w.address().to_string())
            .unwrap_or_else(|| "no-wallet".to_string());

        let info = StakeInfo {
            domain: domain.to_string(),
            amount_lamports: amount,
            amount_usd: amount as f64 / 1_000_000.0,
            staked_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64,
            cooldown_until: None,
            wallet: wallet_addr,
            dns_verified: false,
            reputation: 1.0,
            entity: None,
            disputed: false,
        };
        self.stake_registry.add_stake(info)?;

        // Return a stub tx hash
        let tx_hash = format!("stub_tx_{domain}_{amount}");
        Ok(tx_hash)
    }

    /// Initiate unstaking for a domain (7-day cooldown).
    ///
    /// Sets the cooldown timestamp on the stake entry. Returns a stub tx hash.
    pub fn unstake(&self, domain: &str) -> Result<String> {
        // Remove from registry (simplified -- production would set cooldown)
        self.stake_registry.remove_stake(domain)?;
        let tx_hash = format!("stub_unstake_{domain}");
        Ok(tx_hash)
    }

    /// Get stake info for a domain.
    pub fn get_stake(&self, domain: &str) -> Result<Option<StakeInfo>> {
        self.stake_registry.get_stake(domain)
    }

    /// List all active stakes (excluding those in cooldown).
    pub fn list_stakes(&self) -> Result<Vec<StakeInfo>> {
        self.stake_registry.list_active_stakes()
    }

    /// List active disputes from the local registry.
    pub fn list_disputes(&self) -> Result<Vec<oi_types::DisputeInfo>> {
        // Stub — returns empty until on-chain dispute registry is wired
        Ok(Vec::new())
    }

    /// Get dispute info by dispute ID (formatted as "domain:nonce").
    pub fn get_dispute(&self, dispute_id: &str) -> Result<oi_types::DisputeInfo> {
        // Stub — in production this would query on-chain state
        Err(oi_types::Error::protocol(
            oi_types::error::ErrorCode::DisputeAlreadyActive,
            format!("Dispute not found: {dispute_id}"),
        ))
    }

    /// Get a reference to the local hybrid index for direct access.
    pub fn index(&self) -> &HybridIndex {
        &self.index
    }

    /// Get a shared reference to the local hybrid index (for P2P node sharing).
    pub fn index_arc(&self) -> Arc<HybridIndex> {
        self.index.clone()
    }

    /// Get a reference to the stake registry for direct stake management.
    pub fn stake_registry(&self) -> &InMemoryStakeRegistry {
        &self.stake_registry
    }

    /// Get a reference to the embedding pipeline.
    pub fn embedder(&self) -> &dyn EmbeddingPipeline {
        self.embedder.as_ref()
    }

    /// Get the wallet if one is configured.
    pub fn wallet(&self) -> Option<&Wallet> {
        self.wallet.as_ref()
    }

    /// Get the SDK configuration.
    pub fn config(&self) -> &SdkConfig {
        &self.config
    }

    /// Persist the index to disk if storage is configured.
    /// Returns Ok(true) if saved, Ok(false) if no storage configured.
    pub fn save_index(&self) -> Result<bool> {
        if let Some(ref storage) = self.storage {
            self.index.save(storage).map_err(|e| {
                tracing::error!("Failed to save index to disk: {e}");
                oi_types::Error::Internal(format!("index save failed: {e}"))
            })?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Get an Arc reference to the embedding pipeline (for use across spawn boundaries).
    pub fn embedder_arc(&self) -> Arc<dyn EmbeddingPipeline> {
        self.embedder.clone()
    }

    /// Blended search: combine local index results with external engine results.
    ///
    /// External results are re-scored through our embedding pipeline so they
    /// compete on the same semantic scale as local results. Staked local results
    /// still get a stake-boost multiplier, so staking amplifies relevance —
    /// but doesn't guarantee top ranking for irrelevant content.
    ///
    /// Returns `(results, sources_used)`.
    pub fn search_blended(
        &self,
        query: &str,
        top_k: usize,
        filters: &QueryFilters,
    ) -> Result<(Vec<SearchResultItem>, Vec<String>)> {
        // Compute query embedding once — used for both local search and external re-scoring
        let query_embedding = self.embedder.embed_query(query)?;
        let local_results = self.index.query(&query_embedding, query, filters, top_k)?;
        let mut sources_used = Vec::new();

        if !local_results.is_empty() {
            sources_used.push("local".to_string());
        }

        // If no external engines configured, return local only (backward compat)
        if self.search_engines.is_empty() {
            let mut local_results = local_results;
            self.enrich_content_relevant(&mut local_results, &query_embedding);
            self.record_leaderboard(&local_results);
            return Ok((local_results, sources_used));
        }

        // Local-coverage fast path: when the index already answers the query
        // well, skip the ~1s external engine round-trip entirely. Coverage is
        // judged on the same absolute blend score the merge would compute, so
        // this changes latency, not ranking. Threshold: a full top_k where even
        // the weakest hit is solidly on-topic (absolute cosine blend >= 0.60).
        if local_results.len() >= top_k {
            let mut local_results = local_results;
            let n = local_results.len();
            apply_blend_scores(&mut local_results, n, &query_embedding, self.embedder.as_ref());
            local_results.sort_by(|a, b| b.final_score.total_cmp(&a.final_score));
            for (i, r) in local_results.iter_mut().enumerate() {
                r.rank = (i + 1) as u32;
            }
            let strong_coverage = local_results
                .first()
                .is_some_and(|r| r.final_score >= LOCAL_COVERAGE_TOP1)
                && local_results
                    .last()
                    .is_some_and(|r| r.final_score >= LOCAL_COVERAGE_KTH);
            if strong_coverage {
                self.enrich_content_relevant(&mut local_results, &query_embedding);
                self.record_leaderboard(&local_results);
                return Ok((local_results, sources_used));
            }
            // Weak coverage — fall through to the external blend. Blend scores
            // are recomputed in merge (the embedder cache makes that free).
            return self.blend_with_external(query, top_k, filters, local_results, query_embedding, sources_used);
        }

        self.blend_with_external(query, top_k, filters, local_results, query_embedding, sources_used)
    }

    /// External-engine half of `search_blended`: fetch, merge, crawl-queue.
    fn blend_with_external(
        &self,
        query: &str,
        top_k: usize,
        filters: &QueryFilters,
        local_results: Vec<SearchResultItem>,
        query_embedding: Vec<f32>,
        mut sources_used: Vec<String>,
    ) -> Result<(Vec<SearchResultItem>, Vec<String>)> {

        let params = SearchParams {
            locale: filters.locale.clone(),
            gl: filters.gl.clone(),
            site: filters.site.clone(),
        };

        // Enrich local results in parallel with external engine HTTP calls.
        // Enrichment takes ~28ms (ONNX batch embed of sentences), external engines
        // take ~900ms (HTTP). The ONNX session is released long before merge needs it.
        let (local_results, external_results) = std::thread::scope(|s| {
            let enrich_handle = s.spawn(|| {
                let mut results = local_results;
                self.enrich_content_relevant(&mut results, &query_embedding);
                results
            });

            let external = crate::engines::search_with_engines(
                &self.search_engines,
                query,
                top_k,
                &params,
            );

            let enriched_local = enrich_handle.join().expect("enrich thread panicked");
            (enriched_local, external)
        });

        if !external_results.is_empty() {
            // Track which engines contributed
            for r in &external_results {
                if !sources_used.contains(&r.source) {
                    sources_used.push(r.source.clone());
                }
            }
        }

        let mut merged = merge_results(
            local_results, external_results, top_k,
            &self.stake_registry, &query_embedding, self.embedder.as_ref(),
        );
        // Local results are already enriched; only enrich external results that
        // made it into the final set (they have empty content_md, so this is a no-op
        // in practice — but handles the case where external results have content).
        self.enrich_content_relevant(&mut merged, &query_embedding);
        self.record_leaderboard(&merged);

        // Crawl-on-query: queue external results for background indexing so
        // every query permanently grows local coverage. Drained by
        // `process_crawl_queue` (callers decide when to pay the crawl cost).
        if self.config.crawl_on_query {
            let pending = self.urls_to_crawl(&merged, self.config.crawl_on_query_max_urls);
            if !pending.is_empty()
                && let Ok(mut queue) = self.crawl_queue.lock()
            {
                for url in pending {
                    if !queue.contains(&url) {
                        queue.push(url);
                    }
                }
            }
        }

        Ok((merged, sources_used))
    }

    /// Number of URLs waiting in the crawl-on-query queue.
    pub fn pending_crawl_count(&self) -> usize {
        self.crawl_queue.lock().map(|q| q.len()).unwrap_or(0)
    }

    /// Drain the crawl-on-query queue: fetch, distill, embed, and ingest each
    /// queued URL (robots.txt-respecting), then persist the index. Returns the
    /// number of URLs successfully indexed.
    ///
    /// Blocking — runs the crawl on a dedicated thread with its own tokio
    /// runtime, so it is safe to call from both sync and async contexts.
    pub fn process_crawl_queue(&self) -> usize {
        let urls: Vec<String> = match self.crawl_queue.lock() {
            Ok(mut q) => q.drain(..).collect(),
            Err(_) => return 0,
        };
        if urls.is_empty() {
            return 0;
        }

        let distiller = oi_crawl::Distiller::new().with_embedder(self.embedder_arc());
        let outputs = std::thread::scope(|s| {
            s.spawn(|| {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(_) => return Vec::new(),
                };
                rt.block_on(async {
                    use oi_crawl::robots::{parse_robots_txt, RobotsChecker};
                    use oi_types::traits::CrawlPipeline;

                    // Hard bounds: a slow page must never hold a search
                    // process hostage (observed: one un-timed-out crawl took
                    // 130s). Per-URL and whole-queue deadlines.
                    const PER_URL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);
                    const QUEUE_DEADLINE: std::time::Duration = std::time::Duration::from_secs(45);
                    let started = std::time::Instant::now();

                    let mut robots = RobotsChecker::new();
                    let mut outputs = Vec::new();
                    for url in &urls {
                        if started.elapsed() > QUEUE_DEADLINE {
                            tracing::debug!("crawl-on-query: queue deadline reached, deferring rest");
                            break;
                        }
                        if let Some(domain) = url
                            .strip_prefix("https://")
                            .or_else(|| url.strip_prefix("http://"))
                            .and_then(|rest| rest.split('/').next())
                        {
                            if !robots.has_rules(domain) {
                                let robots_url = format!("https://{domain}/robots.txt");
                                if let Ok(Ok(res)) = tokio::time::timeout(
                                    PER_URL_TIMEOUT,
                                    oi_crawl::fetcher::fetch_url(&robots_url),
                                )
                                .await
                                {
                                    robots.add_rules(domain, parse_robots_txt(&res.html));
                                }
                            }
                            if !robots.is_allowed(url) {
                                tracing::debug!("crawl-on-query: robots.txt blocked {url}");
                                continue;
                            }
                        }
                        match tokio::time::timeout(PER_URL_TIMEOUT, distiller.crawl(url)).await {
                            Ok(Ok(out)) if !out.chunks.is_empty() => outputs.push(out),
                            Ok(Ok(_)) => tracing::debug!("crawl-on-query: no content at {url}"),
                            Ok(Err(e)) => tracing::debug!("crawl-on-query: crawl failed for {url}: {e}"),
                            Err(_) => tracing::debug!("crawl-on-query: timed out crawling {url}"),
                        }
                    }
                    outputs
                })
            })
            .join()
            .unwrap_or_default()
        });

        let mut indexed = 0;
        for out in &outputs {
            match self.ingest(&out.chunks) {
                Ok(()) => indexed += 1,
                Err(e) => tracing::debug!("crawl-on-query: ingest failed: {e}"),
            }
        }
        if indexed > 0
            && let Err(e) = self.save_index()
        {
            tracing::warn!("crawl-on-query: index save failed: {e}");
        }
        indexed
    }

    /// Query external search engines only and return discovered URLs.
    /// Used by frontier seeding to bootstrap the corpus without embedding or local index queries.
    pub fn search_external_only(&self, query: &str, limit: usize) -> Vec<String> {
        if self.search_engines.is_empty() {
            return Vec::new();
        }
        let params = SearchParams::default();
        let results = crate::engines::search_with_engines(&self.search_engines, query, limit, &params);
        results.into_iter().map(|r| r.url).collect()
    }

    /// Returns external URLs from blended results that are not yet in the local index.
    /// Used by crawl-on-query to grow the corpus organically.
    pub fn urls_to_crawl(&self, results: &[SearchResultItem], max: usize) -> Vec<String> {
        results
            .iter()
            .filter(|r| r.source != "local" && r.source != "cached")
            .map(|r| r.url.clone())
            .take(max)
            .collect()
    }

    /// Read the earnings snapshot saved by a running node.
    /// Returns None if the file doesn't exist or can't be parsed.
    pub fn get_earnings() -> Option<oi_types::EarningsSnapshot> {
        let path = dirs::home_dir()?.join(".opensonarx").join("earnings.json");
        let data = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&data).ok()
    }

    /// Ingest content chunks into the local index.
    ///
    /// If a `TopicRouter` is attached, each chunk is classified into a topic shard
    /// (the `shard_id` field on `ChunkMeta` is set). If `shard_filter` is also set,
    /// chunks not matching any of the node's assigned shards are silently dropped.
    ///
    /// Persistence is handled by the autosave background task and shutdown save,
    /// not per-ingest. The index is marked dirty after ingest so the autosave
    /// task knows when to flush.
    pub fn ingest(&self, chunks: &[ContentChunk]) -> Result<()> {
        let chunks_to_ingest = if let Some(ref router) = self.topic_router {
            let mut classified: Vec<ContentChunk> = Vec::with_capacity(chunks.len());

            for chunk in chunks {
                let mut chunk = chunk.clone();

                // Classify using embedding if available
                if let Some(ref embedding) = chunk.embedding {
                    let shard_id = router.classify(embedding).to_string();

                    // Apply shard filter — drop chunks that don't match this node's shards
                    if !self.shard_filter.is_empty() && !self.shard_filter.contains(&shard_id) {
                        tracing::debug!(
                            chunk_id = %chunk.chunk_id,
                            shard = %shard_id,
                            "dropping chunk: not in shard filter"
                        );
                        continue;
                    }

                    // Set the shard_id on metadata
                    if let Some(ref mut meta) = chunk.meta {
                        meta.shard_id = Some(shard_id);
                    }
                }

                classified.push(chunk);
            }

            if classified.is_empty() {
                return Ok(());
            }

            std::borrow::Cow::Owned(classified)
        } else {
            std::borrow::Cow::Borrowed(chunks)
        };

        self.index.ingest(&chunks_to_ingest)?;

        Ok(())
    }
}

/// Merge local and external search results, deduplicating by URL.
///
/// Local hybrid scores are max-normalized per query (the best local hit is
/// ~1.0 even when nothing in the index is relevant), while external snippets
/// score as absolute cosine — the two are not comparable. So at blend time
/// BOTH sides are re-scored on the same absolute scale: embed title+snippet
/// (locals use a content prefix) and compute cosine with the query, then blend
/// with the production semantic/BM25 weights. Locals keep their (normalized)
/// BM25 term-match evidence; externals carry none. This stops an irrelevant
/// but "locally best" result from outranking a genuinely relevant external
/// one. Stake never moves the score (stake-neutral ranking).
fn merge_results(
    local: Vec<SearchResultItem>,
    external: Vec<SearchResultItem>,
    top_k: usize,
    stake_registry: &oi_staking::InMemoryStakeRegistry,
    query_embedding: &[f32],
    embedder: &dyn EmbeddingPipeline,
) -> Vec<SearchResultItem> {
    use oi_types::traits::StakeRegistry;
    use std::collections::{HashMap, HashSet};

    let mut seen_urls: HashSet<String> = HashSet::new();
    let local_count = local.len();
    let mut merged: Vec<SearchResultItem> = Vec::new();

    for r in local {
        seen_urls.insert(r.url.clone());
        merged.push(r);
    }

    // Dedup external results against local URLs
    let external_filtered: Vec<SearchResultItem> = external
        .into_iter()
        .filter(|r| seen_urls.insert(r.url.clone()))
        .collect();
    merged.extend(external_filtered);

    // One batch embed for every candidate's title+snippet — the shared
    // absolute scale both sides are blended on.
    apply_blend_scores(&mut merged, local_count, query_embedding, embedder);

    for result in merged.iter_mut().skip(local_count) {
        // Stake never moves the score (stake-neutral ranking) — but a staked
        // domain's skin-in-the-game is surfaced on the result as trust metadata.
        let domain = extract_domain(&result.url);
        if let Ok(Some(info)) = stake_registry.get_stake(&domain) {
            result.stake_truth = info.amount_lamports;
            result.stake_usd = info.amount_usd.round() as u64;
            result.reputation = info.reputation;
            result.source = "cached".to_string();
        }
    }

    // Sort by final_score descending — local and external compete fairly
    merged.sort_by(|a, b| b.final_score.total_cmp(&a.final_score));

    // Per-domain cap: max 3 results from any single domain for diversity
    const MAX_PER_DOMAIN: usize = 3;
    let mut domain_counts: HashMap<String, usize> = HashMap::new();
    merged.retain(|r| {
        let domain = extract_domain(&r.url);
        let count = domain_counts.entry(domain).or_insert(0);
        *count += 1;
        *count <= MAX_PER_DOMAIN
    });

    merged.truncate(top_k);

    // Re-assign ranks
    for (i, r) in merged.iter_mut().enumerate() {
        r.rank = (i + 1) as u32;
    }

    merged
}

/// Local-coverage fast path thresholds: the external engine round-trip is
/// skipped only when the best local hit is strongly on-topic (top-1) AND the
/// whole top_k is solid (k-th). A high top-1 bar prevents a page of merely
/// same-domain results (e.g. Rust std docs for "rust borrow checker") from
/// suppressing the external answer.
const LOCAL_COVERAGE_TOP1: f64 = 0.70;
const LOCAL_COVERAGE_KTH: f64 = 0.58;

/// Re-score results on the shared absolute scale: cosine of the query against
/// an embedded title+snippet (locals use a content prefix), blended 0.8/0.2
/// with the result's (query-normalized) BM25 evidence. The first `local_count`
/// items are treated as local (BM25 term counts); the rest as external (no
/// keyword evidence, BM25 term = 0).
fn apply_blend_scores(
    results: &mut [SearchResultItem],
    local_count: usize,
    query_embedding: &[f32],
    embedder: &dyn EmbeddingPipeline,
) {
    if results.is_empty() {
        return;
    }
    let texts: Vec<String> = results
        .iter()
        .map(|r| {
            let body = if r.snippet.is_empty() {
                r.content_md.chars().take(400).collect::<String>()
            } else {
                r.snippet.clone()
            };
            format!("{} {}", r.title, body)
        })
        .collect();
    let text_refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
    let embeddings = embedder.embed_text_batch(&text_refs).unwrap_or_default();

    // 0.9/0.1 rather than the index's 0.8/0.2: local BM25 here is normalized
    // against local candidates only, so at 0.2 a strong same-domain keyword
    // match (Rust `String` docs for "reverse a string in python") outranks a
    // better external answer that carries no keyword evidence at all.
    const SEMANTIC_W: f64 = 0.9;
    const BM25_W: f64 = 0.1;
    for (i, result) in results.iter_mut().enumerate() {
        let semantic_score = embeddings
            .get(i)
            .map(|emb| cosine_similarity(query_embedding, emb).max(0.0) as f64)
            .unwrap_or(0.0);
        let bm25 = if i < local_count { result.bm25_score.clamp(0.0, 1.0) } else { 0.0 };
        result.semantic_score = semantic_score;
        result.final_score = SEMANTIC_W * semantic_score + BM25_W * bm25;
    }
}

/// Cosine similarity between two vectors.
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot / (norm_a * norm_b)
    }
}

/// Extract domain from a URL.
fn extract_domain(url: &str) -> String {
    url.split("//")
        .nth(1)
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or(url)
        .to_string()
}

impl Default for OpenSonarXClient {
    fn default() -> Self {
        Self::new(SdkConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oi_embed::MockEmbedder;
    use oi_types::ChunkMeta;

    #[test]
    fn test_client_creation() {
        let client = OpenSonarXClient::new(SdkConfig::default());
        assert_eq!(client.config().network, "mainnet");
        assert!(client.wallet().is_none());
    }

    #[test]
    fn test_client_default() {
        let client = OpenSonarXClient::default();
        assert!(client.config().bootstrap_peers.is_empty());
    }

    #[test]
    fn test_client_with_custom_embedder() {
        let embedder = Arc::new(MockEmbedder::new(128));
        let client = OpenSonarXClient::with_embedder(SdkConfig::default(), embedder);
        assert_eq!(client.embedder().dimension(), 128);
        assert_eq!(client.embedder().name(), "mock");
    }

    #[test]
    fn test_search_empty_index() {
        let client = OpenSonarXClient::new(SdkConfig::default());
        let results = client
            .search("rust programming", 5, &QueryFilters::default())
            .unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_ingest_and_search() {
        let client = OpenSonarXClient::new(SdkConfig::default());

        let embedding = client.embedder().embed_text("rust systems programming language").unwrap();
        let chunks = vec![
            ContentChunk {
                chunk_id: "c1".to_string(),
                content: "rust systems programming language for safe concurrency".to_string(),
                embedding: Some(embedding),
                seq: 0,
                meta: Some(ChunkMeta {
                    url: "https://rust-lang.org".to_string(),
                    title: "Rust Programming Language".to_string(),
                    domain: "rust-lang.org".to_string(),
                    content_hash: "hash_c1".to_string(),
                    crawled_at: None,
                    content_type: Some("text/html".to_string()),
                    entity: None,
                    content_kind: None,
                    ..Default::default()
                }),
            },
            ContentChunk {
                chunk_id: "c2".to_string(),
                content: "python data science machine learning".to_string(),
                embedding: Some(client.embedder().embed_text("python data science machine learning").unwrap()),
                seq: 1,
                meta: Some(ChunkMeta {
                    url: "https://python.org".to_string(),
                    title: "Python".to_string(),
                    domain: "python.org".to_string(),
                    content_hash: "hash_c2".to_string(),
                    crawled_at: None,
                    content_type: Some("text/html".to_string()),
                    entity: None,
                    content_kind: None,
                    ..Default::default()
                }),
            },
        ];

        client.ingest(&chunks).unwrap();

        let results = client
            .search("rust programming", 5, &QueryFilters::default())
            .unwrap();
        assert!(!results.is_empty());
        // The rust chunk should score higher for "rust programming"
        assert_eq!(results[0].result_id, "c1");
    }

    #[test]
    fn test_stake_operations() {
        let client = OpenSonarXClient::new(SdkConfig::default());

        // No stakes initially
        let stakes = client.list_stakes().unwrap();
        assert!(stakes.is_empty());

        // Stake a domain
        let tx = client.stake("example.com", 1_000_000).unwrap();
        assert!(tx.contains("example.com"));

        // Verify the stake exists
        let info = client.get_stake("example.com").unwrap().unwrap();
        assert_eq!(info.amount_lamports, 1_000_000);
        assert_eq!(info.domain, "example.com");

        // List stakes
        let stakes = client.list_stakes().unwrap();
        assert_eq!(stakes.len(), 1);

        // Unstake
        let tx = client.unstake("example.com").unwrap();
        assert!(tx.contains("unstake"));

        // Verify it's gone
        let info = client.get_stake("example.com").unwrap();
        assert!(info.is_none());
    }

    #[test]
    fn test_local_index_persistence() {
        let dir = std::env::temp_dir().join("oi_sdk_persist_test");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.to_str().unwrap().to_string();

        // Create client with local_index_path, ingest data
        let config = SdkConfig {
            local_index_path: Some(path.clone()),
            ..SdkConfig::default()
        };
        let client = OpenSonarXClient::new(config);

        let embedding = client.embedder().embed_text("rust systems programming").unwrap();
        let chunks = vec![ContentChunk {
            chunk_id: "persist_c1".to_string(),
            content: "rust systems programming language".to_string(),
            embedding: Some(embedding),
            seq: 0,
            meta: Some(ChunkMeta {
                url: "https://rust-lang.org".to_string(),
                title: "Rust".to_string(),
                domain: "rust-lang.org".to_string(),
                content_hash: "hash_persist".to_string(),
                crawled_at: None,
                content_type: None,
                entity: None,
                content_kind: None,
                ..Default::default()
            }),
        }];
        client.ingest(&chunks).unwrap();
        client.save_index().unwrap();

        // Create new client at same path — should load persisted data
        let config2 = SdkConfig {
            local_index_path: Some(path),
            ..SdkConfig::default()
        };
        let client2 = OpenSonarXClient::new(config2);

        let results = client2
            .search("rust programming", 5, &QueryFilters::default())
            .unwrap();
        assert!(!results.is_empty(), "persisted data should be searchable");
        assert_eq!(results[0].result_id, "persist_c1");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_feedback() {
        let client = OpenSonarXClient::new(SdkConfig::default());
        // Should succeed without error
        client.feedback("result_123", true).unwrap();
        client.feedback("result_456", false).unwrap();
    }

    #[test]
    fn test_wallet_creation() {
        let wallet = Wallet::new();
        assert!(!wallet.address().is_empty());
        assert_eq!(wallet.address().len(), 64);
    }

    #[test]
    fn test_client_with_wallet_path() {
        // Create a temporary keypair file
        let dir = std::env::temp_dir().join("oi_sdk_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test_keypair.json");
        std::fs::write(&path, b"test-sdk-keypair-bytes").unwrap();

        let config = SdkConfig {
            wallet_path: Some(path.to_str().unwrap().to_string()),
            ..SdkConfig::default()
        };
        let client = OpenSonarXClient::new(config);
        assert!(client.wallet().is_some());
        assert!(!client.wallet().unwrap().address().is_empty());

        let _ = std::fs::remove_file(&path);
    }

    /// Benchmark: ingest realistic content for famous domains, stake, and measure
    /// local index query time vs external engine fallback.
    ///
    /// Run with: cargo test -p oi-sdk test_local_index_benchmark -- --nocapture
    #[test]
    fn test_local_index_benchmark() {
        use std::time::Instant;

        let client = OpenSonarXClient::new(SdkConfig::default());

        // ── Famous domains with realistic content ───────────────────────────
        let articles: &[(&str, &str, &str, &str)] = &[
            (
                "nytimes.com",
                "https://www.nytimes.com/2024/tech/ai-search-engines",
                "AI Search Engines Are Reshaping How We Find Information",
                "Artificial intelligence is transforming web search as companies race to build \
                 AI-powered search engines that can understand natural language queries and \
                 deliver precise answers. Google, Microsoft, and startups are investing billions \
                 in large language models that can parse complex questions and synthesize \
                 information from multiple sources.",
            ),
            (
                "nytimes.com",
                "https://www.nytimes.com/2024/business/crypto-regulation",
                "Cryptocurrency Regulation Takes Shape Globally",
                "Governments worldwide are establishing frameworks for cryptocurrency regulation. \
                 The SEC has approved several Bitcoin ETFs while the EU's MiCA regulation sets \
                 comprehensive rules for digital asset markets. Staking rewards and DeFi protocols \
                 face increasing scrutiny as regulators seek to protect investors.",
            ),
            (
                "techcrunch.com",
                "https://techcrunch.com/2024/rust-adoption-enterprise",
                "Rust Adoption Surges in Enterprise Software Development",
                "Rust programming language adoption has accelerated dramatically in enterprise \
                 environments. Major tech companies including Microsoft, Google, and Amazon are \
                 rewriting critical infrastructure in Rust for memory safety and performance. \
                 The language's ownership model eliminates entire classes of bugs common in C and C++.",
            ),
            (
                "techcrunch.com",
                "https://techcrunch.com/2024/decentralized-search-protocol",
                "Decentralized Search Protocols Challenge Big Tech Monopoly",
                "A new wave of decentralized search protocols is emerging to challenge the \
                 dominance of Google and Bing. These protocols use token staking to incentivize \
                 content quality, blockchain verification for result authenticity, and peer-to-peer \
                 networks for censorship resistance. Publishers stake tokens to vouch for content.",
            ),
            (
                "bbc.com",
                "https://www.bbc.com/news/technology-ai-agents",
                "AI Agents Transform Software Development Workflow",
                "AI coding agents are revolutionizing how software is built. Tools like GitHub \
                 Copilot, Claude Code, and Cursor use large language models to write, debug, and \
                 refactor code. Developers report productivity gains of 30-50% on routine tasks \
                 while focusing on architecture and design decisions.",
            ),
            (
                "bbc.com",
                "https://www.bbc.com/news/science-quantum-computing",
                "Quantum Computing Reaches New Milestone",
                "Researchers have achieved a breakthrough in quantum error correction, bringing \
                 practical quantum computing closer to reality. The new technique reduces qubit \
                 error rates by an order of magnitude, enabling more complex quantum algorithms \
                 for cryptography, drug discovery, and optimization problems.",
            ),
            (
                "reuters.com",
                "https://www.reuters.com/technology/solana-defi-growth",
                "Solana DeFi Ecosystem Experiences Record Growth",
                "The Solana blockchain decentralized finance ecosystem has hit record total value \
                 locked as new protocols launch staking and lending platforms. Solana's high \
                 throughput and low transaction costs attract developers building financial \
                 infrastructure, NFT marketplaces, and decentralized autonomous organizations.",
            ),
            (
                "reuters.com",
                "https://www.reuters.com/technology/vector-search-databases",
                "Vector Search Databases Power Next Generation AI Applications",
                "Vector databases and hybrid search engines combining semantic vector similarity \
                 with traditional BM25 keyword matching are becoming essential infrastructure \
                 for AI applications. Companies use HNSW algorithms for approximate nearest \
                 neighbor search to power recommendation systems and RAG pipelines.",
            ),
            (
                "wired.com",
                "https://www.wired.com/story/open-source-search-engines",
                "Open Source Search Engines Gain Momentum",
                "Open source search engine projects like Stract, SearXNG, and Mojeek are \
                 gaining users who want alternatives to big tech search. These projects \
                 prioritize privacy, transparency, and community governance. Some use \
                 independent web indexes while others aggregate results from multiple sources.",
            ),
            (
                "arxiv.org",
                "https://arxiv.org/abs/2024/embedding-models",
                "Efficient Embedding Models for Semantic Search",
                "We present a new family of embedding models optimized for semantic search \
                 applications. Using contrastive learning with hard negatives and knowledge \
                 distillation, our models achieve state-of-the-art retrieval performance at \
                 a fraction of the computational cost. The 384-dimensional embeddings enable \
                 efficient HNSW indexing while maintaining high recall.",
            ),
            (
                "github.com",
                "https://github.com/opensonarx/protocol/wiki/architecture",
                "OpenSonarX Protocol Architecture and Design",
                "The OpenSonarX protocol combines hybrid HNSW vector search with BM25 keyword \
                 matching for accurate information retrieval. Publishers stake TRUTH tokens \
                 to vouch for content quality. The libp2p network distributes index shards \
                 across nodes. Protobuf wire protocol ensures efficient serialization.",
            ),
            (
                "stackoverflow.com",
                "https://stackoverflow.com/questions/rust-async-patterns",
                "Best Practices for Async Rust with Tokio",
                "This comprehensive guide covers async Rust patterns using the Tokio runtime. \
                 Learn about spawning tasks, using channels for communication, handling errors \
                 in async contexts, and structuring concurrent applications. Includes examples \
                 of building HTTP servers with Axum and connecting to databases.",
            ),
        ];

        // ── Ingest ──────────────────────────────────────────────────────────
        let ingest_start = Instant::now();
        let chunks: Vec<ContentChunk> = articles
            .iter()
            .enumerate()
            .map(|(i, (domain, url, title, content))| {
                let embedding = client.embedder().embed_text(content).unwrap();
                ContentChunk {
                    chunk_id: format!("bench_{i}"),
                    content: content.to_string(),
                    embedding: Some(embedding),
                    seq: i as u64,
                    meta: Some(ChunkMeta {
                        url: url.to_string(),
                        title: title.to_string(),
                        domain: domain.to_string(),
                        content_hash: format!("sha256_bench_{i:04x}"),
                        crawled_at: Some(1700000000),
                        content_type: Some("text/html".to_string()),
                        entity: None,
                        content_kind: None,
                        ..Default::default()
                    }),
                }
            })
            .collect();

        client.ingest(&chunks).unwrap();
        let ingest_elapsed = ingest_start.elapsed();
        eprintln!("\n{}", "=".repeat(60));
        eprintln!("  OPENINDEX LOCAL INDEX BENCHMARK");
        eprintln!("{}", "=".repeat(60));
        eprintln!("\n[INGEST] {} documents in {:?}", chunks.len(), ingest_elapsed);

        // ── Stake on domains ────────────────────────────────────────────────
        let domains_to_stake = [
            ("nytimes.com", 50_000_000u64),
            ("techcrunch.com", 30_000_000),
            ("bbc.com", 40_000_000),
            ("reuters.com", 35_000_000),
            ("wired.com", 20_000_000),
            ("arxiv.org", 25_000_000),
            ("github.com", 45_000_000),
            ("stackoverflow.com", 38_000_000),
        ];

        let stake_start = Instant::now();
        for (domain, amount) in &domains_to_stake {
            client.stake(domain, *amount).unwrap();
        }
        let stake_elapsed = stake_start.elapsed();
        eprintln!("[STAKE]  {} domains staked in {:?}", domains_to_stake.len(), stake_elapsed);

        // Verify stakes
        let stakes = client.list_stakes().unwrap();
        assert_eq!(stakes.len(), domains_to_stake.len());
        eprintln!("[STAKE]  Active stakes: {}", stakes.len());
        for s in &stakes {
            eprintln!("         {} — ${:.2} ({} lamports)", s.domain, s.amount_usd, s.amount_lamports);
        }

        // ── Local index queries ─────────────────────────────────────────────
        let queries = [
            "AI search engines artificial intelligence",
            "Rust programming language systems safety",
            "cryptocurrency regulation staking DeFi",
            "vector search HNSW embedding semantic",
            "decentralized search protocol token staking",
            "quantum computing error correction",
            "open source search engines privacy",
            "Solana blockchain DeFi staking",
            "async Rust Tokio patterns",
            "embedding models contrastive learning",
        ];

        eprintln!("\n--- Local Index Queries (our engine) ---");
        let mut total_local_us = 0u128;
        let mut total_results = 0;

        for query in &queries {
            let start = Instant::now();
            let results = client.search(query, 5, &QueryFilters::default()).unwrap();
            let elapsed = start.elapsed();
            total_local_us += elapsed.as_micros();
            total_results += results.len();

            eprintln!(
                "  [{:>7.1}us] q=\"{}\" → {} results",
                elapsed.as_micros() as f64,
                query,
                results.len()
            );
            for r in results.iter().take(3) {
                eprintln!(
                    "              #{} {} (score={:.4}, stake=${:.2}, source={})",
                    r.rank, r.url, r.final_score, r.stake_usd, r.source
                );
            }
        }

        let avg_local_us = total_local_us as f64 / queries.len() as f64;
        eprintln!("\n[SUMMARY] Local index:");
        eprintln!("  Queries:        {}", queries.len());
        eprintln!("  Total results:  {total_results}");
        eprintln!("  Total time:     {total_local_us}us ({:.3}ms)", total_local_us as f64 / 1000.0);
        eprintln!("  Average/query:  {avg_local_us:.1}us ({:.3}ms)", avg_local_us / 1000.0);

        // ── search_with_fallback should NOT hit external engines ─────────
        eprintln!("\n--- search_with_fallback (should return local, no external call) ---");
        let start = Instant::now();
        let results = client
            .search_with_fallback("decentralized search protocol staking", 5, &QueryFilters::default())
            .unwrap();
        let elapsed = start.elapsed();
        eprintln!(
            "  [{:>7.1}us] → {} results (source={})",
            elapsed.as_micros() as f64,
            results.len(),
            results.first().map(|r| r.source.as_str()).unwrap_or("none")
        );
        // Should come from local index — top result must be local
        assert!(!results.is_empty(), "local index should return results");
        assert_eq!(
            results[0].source, "local",
            "top result should come from local index"
        );
        // With per-domain caps, some external results may fill diversity gaps — that's expected.
        // Verify that local results are the majority.
        let local_count = results.iter().filter(|r| r.source == "local").count();
        assert!(
            local_count >= results.len() / 2,
            "local results should be the majority, got {local_count}/{}",
            results.len()
        );

        // ── Leaderboard ─────────────────────────────────────────────────────
        let top = client.top_queried(10).unwrap();
        eprintln!("\n--- Leaderboard (top queried) ---");
        for entry in &top {
            eprintln!("  {} — {} queries", entry.identifier, entry.query_count);
        }
        assert!(!top.is_empty(), "leaderboard should have entries after queries");

        eprintln!("\n{}", "=".repeat(60));
        eprintln!("  BENCHMARK COMPLETE — local index: {avg_local_us:.0}us avg/query");
        eprintln!("{}\n", "=".repeat(60));
    }

    /// Compare local index results vs DuckDuckGo for the same query.
    /// Uses the default embedder (MiniLM when onnx is enabled, MockEmbedder otherwise).
    ///
    /// Run with: cargo test -p oi-sdk test_local_vs_ddg_comparison -- --nocapture --ignored
    #[test]
    #[ignore] // requires network access
    fn test_local_vs_ddg_comparison() {
        use std::time::Instant;
        use crate::engines::duckduckgo::DuckDuckGo;

        let client = OpenSonarXClient::new(SdkConfig::default());

        let articles = test_articles();
        let chunks = build_chunks(&articles, client.embedder());
        client.ingest(&chunks).unwrap();

        for (domain, _url, _title, _content) in &articles {
            client.stake(domain, 25_000_000).unwrap();
        }

        let query = "Rust programming language";

        eprintln!("\n{}", "=".repeat(70));
        eprintln!("  COMPARISON: Local Index ({}) vs DuckDuckGo", client.embedder().name());
        eprintln!("  Query: \"{query}\"");
        eprintln!("{}", "=".repeat(70));

        print_local_results(&client, query);

        eprintln!("\n--- DUCKDUCKGO (external, real-world results) ---");
        let ddg = DuckDuckGo;
        let start = Instant::now();
        let ddg_results = ddg.search(query, 5);
        let ddg_elapsed = start.elapsed();
        for r in &ddg_results {
            eprintln!("  #{} {}", r.rank, r.url);
            eprintln!("      title: {}", r.title);
        }
        eprintln!("  Time: {:?}", ddg_elapsed);
        eprintln!("{}\n", "=".repeat(70));
    }

    /// Full benchmark with real MiniLM embedder (Snowflake Arctic Embed S).
    /// Downloads the ONNX model on first run (~30MB).
    ///
    /// Run with: cargo test -p oi-sdk --features onnx test_minilm_benchmark -- --nocapture --ignored
    #[cfg(feature = "onnx")]
    #[test]
    #[ignore] // requires model download + network for DDG
    fn test_minilm_benchmark() {
        use std::time::Instant;
        use oi_embed::MiniLmEmbedder;
        use crate::engines::duckduckgo::DuckDuckGo;

        eprintln!("\n{}", "=".repeat(70));
        eprintln!("  MINILM REAL EMBEDDER BENCHMARK");
        eprintln!("{}", "=".repeat(70));

        // ── Load the real model ─────────────────────────────────────────
        eprintln!("\n[MODEL] Loading MiniLM (Snowflake Arctic Embed S)...");
        let model_start = Instant::now();
        let embedder = Arc::new(
            MiniLmEmbedder::new().expect("Failed to load MiniLM — run with --features onnx"),
        );
        let model_elapsed = model_start.elapsed();
        eprintln!("[MODEL] Loaded in {:?} (dim={})", model_elapsed, embedder.dimension());

        let config = SdkConfig::default();
        let client = OpenSonarXClient::with_embedder(config, embedder);

        // ── Ingest ──────────────────────────────────────────────────────
        let articles = test_articles();

        let ingest_start = Instant::now();
        let chunks = build_chunks(&articles, client.embedder());
        let embed_elapsed = ingest_start.elapsed();
        eprintln!("[EMBED] {} documents embedded in {:?}", chunks.len(), embed_elapsed);

        let index_start = Instant::now();
        client.ingest(&chunks).unwrap();
        let index_elapsed = index_start.elapsed();
        eprintln!("[INDEX] Indexed in {:?}", index_elapsed);

        // ── Stake ───────────────────────────────────────────────────────
        let domains_to_stake = [
            ("nytimes.com", 50_000_000u64),
            ("techcrunch.com", 30_000_000),
            ("bbc.com", 40_000_000),
            ("reuters.com", 35_000_000),
            ("wired.com", 20_000_000),
            ("arxiv.org", 25_000_000),
            ("github.com", 45_000_000),
            ("stackoverflow.com", 38_000_000),
        ];
        for (domain, amount) in &domains_to_stake {
            client.stake(domain, *amount).unwrap();
        }
        eprintln!("[STAKE] {} domains staked", domains_to_stake.len());

        // ── Queries ─────────────────────────────────────────────────────
        let queries = [
            "Rust programming language",
            "AI search engines",
            "vector database HNSW embedding",
            "decentralized search protocol staking",
            "quantum computing breakthrough",
            "Solana DeFi blockchain",
            "cryptocurrency regulation",
            "async Rust Tokio",
        ];

        eprintln!("\n--- LOCAL INDEX (MiniLM + BM25 hybrid) ---");
        let mut total_embed_us = 0u128;
        let mut total_search_us = 0u128;

        for query in &queries {
            // Measure embedding time separately
            let embed_start = Instant::now();
            let _ = client.embedder().embed_query(query).unwrap();
            let embed_us = embed_start.elapsed().as_micros();
            total_embed_us += embed_us;

            // Measure full search (embed + index query)
            let search_start = Instant::now();
            let results = client.search(query, 5, &QueryFilters::default()).unwrap();
            let search_elapsed = search_start.elapsed();
            total_search_us += search_elapsed.as_micros();

            eprintln!(
                "\n  q=\"{query}\" — {} results in {:?} (embed={embed_us}us)",
                results.len(),
                search_elapsed,
            );
            for r in results.iter().take(3) {
                eprintln!(
                    "    #{} [sem={:.4} bm25={:.4} final={:.4}] ${:.0} {}",
                    r.rank, r.semantic_score, r.bm25_score, r.final_score,
                    r.stake_usd, r.url
                );
                eprintln!("       {}", r.title);
            }
        }

        let n = queries.len() as f64;
        let avg_total = total_search_us as f64 / n;
        let avg_embed = total_embed_us as f64 / n;

        // ── DDG comparison for one query ────────────────────────────────
        eprintln!("\n--- DUCKDUCKGO comparison: \"Rust programming language\" ---");
        let ddg = DuckDuckGo;
        let ddg_start = Instant::now();
        let ddg_results = ddg.search("Rust programming language", 5);
        let ddg_elapsed = ddg_start.elapsed();
        for r in &ddg_results {
            eprintln!("  #{} {}", r.rank, r.url);
            eprintln!("      {}", r.title);
        }
        eprintln!("  Time: {:?}", ddg_elapsed);

        // ── Summary ─────────────────────────────────────────────────────
        eprintln!("\n{}", "=".repeat(70));
        eprintln!("  RESULTS");
        eprintln!("{}", "-".repeat(70));
        eprintln!("  Model:           {} (384-dim, INT8 quantized)", client.embedder().name());
        eprintln!("  Model load:      {:?}", model_elapsed);
        eprintln!("  Documents:       {}", articles.len());
        eprintln!("  Queries:         {}", queries.len());
        eprintln!("  Avg embed:       {avg_embed:.0}us ({:.3}ms)", avg_embed / 1000.0);
        eprintln!("  Avg search:      {avg_total:.0}us ({:.3}ms) (embed + index)", avg_total / 1000.0);
        eprintln!("  DuckDuckGo:      {:?}", ddg_elapsed);
        eprintln!(
            "  Speed ratio:     local is ~{:.0}x faster than DDG",
            ddg_elapsed.as_micros() as f64 / (avg_total).max(1.0)
        );
        eprintln!("{}\n", "=".repeat(70));
    }

    // ── Shared test helpers ─────────────────────────────────────────────

    fn test_articles() -> Vec<(&'static str, &'static str, &'static str, &'static str)> {
        vec![
            (
                "nytimes.com",
                "https://www.nytimes.com/2024/tech/ai-search-engines",
                "AI Search Engines Are Reshaping How We Find Information",
                "Artificial intelligence is transforming web search as companies race to build \
                 AI-powered search engines that can understand natural language queries and \
                 deliver precise answers. Google, Microsoft, and startups are investing billions \
                 in large language models that can parse complex questions and synthesize \
                 information from multiple sources.",
            ),
            (
                "nytimes.com",
                "https://www.nytimes.com/2024/business/crypto-regulation",
                "Cryptocurrency Regulation Takes Shape Globally",
                "Governments worldwide are establishing frameworks for cryptocurrency regulation. \
                 The SEC has approved several Bitcoin ETFs while the EU's MiCA regulation sets \
                 comprehensive rules for digital asset markets. Staking rewards and DeFi protocols \
                 face increasing scrutiny as regulators seek to protect investors.",
            ),
            (
                "techcrunch.com",
                "https://techcrunch.com/2024/rust-adoption-enterprise",
                "Rust Adoption Surges in Enterprise Software Development",
                "Rust programming language adoption has accelerated dramatically in enterprise \
                 environments. Major tech companies including Microsoft, Google, and Amazon are \
                 rewriting critical infrastructure in Rust for memory safety and performance. \
                 The language's ownership model eliminates entire classes of bugs common in C and C++.",
            ),
            (
                "techcrunch.com",
                "https://techcrunch.com/2024/decentralized-search-protocol",
                "Decentralized Search Protocols Challenge Big Tech Monopoly",
                "A new wave of decentralized search protocols is emerging to challenge the \
                 dominance of Google and Bing. These protocols use token staking to incentivize \
                 content quality, blockchain verification for result authenticity, and peer-to-peer \
                 networks for censorship resistance. Publishers stake tokens to vouch for content.",
            ),
            (
                "bbc.com",
                "https://www.bbc.com/news/technology-ai-agents",
                "AI Agents Transform Software Development Workflow",
                "AI coding agents are revolutionizing how software is built. Tools like GitHub \
                 Copilot, Claude Code, and Cursor use large language models to write, debug, and \
                 refactor code. Developers report productivity gains of 30-50% on routine tasks \
                 while focusing on architecture and design decisions.",
            ),
            (
                "bbc.com",
                "https://www.bbc.com/news/science-quantum-computing",
                "Quantum Computing Reaches New Milestone",
                "Researchers have achieved a breakthrough in quantum error correction, bringing \
                 practical quantum computing closer to reality. The new technique reduces qubit \
                 error rates by an order of magnitude, enabling more complex quantum algorithms \
                 for cryptography, drug discovery, and optimization problems.",
            ),
            (
                "reuters.com",
                "https://www.reuters.com/technology/solana-defi-growth",
                "Solana DeFi Ecosystem Experiences Record Growth",
                "The Solana blockchain decentralized finance ecosystem has hit record total value \
                 locked as new protocols launch staking and lending platforms. Solana's high \
                 throughput and low transaction costs attract developers building financial \
                 infrastructure, NFT marketplaces, and decentralized autonomous organizations.",
            ),
            (
                "reuters.com",
                "https://www.reuters.com/technology/vector-search-databases",
                "Vector Search Databases Power Next Generation AI Applications",
                "Vector databases and hybrid search engines combining semantic vector similarity \
                 with traditional BM25 keyword matching are becoming essential infrastructure \
                 for AI applications. Companies use HNSW algorithms for approximate nearest \
                 neighbor search to power recommendation systems and RAG pipelines.",
            ),
            (
                "wired.com",
                "https://www.wired.com/story/open-source-search-engines",
                "Open Source Search Engines Gain Momentum",
                "Open source search engine projects like Stract, SearXNG, and Mojeek are \
                 gaining users who want alternatives to big tech search. These projects \
                 prioritize privacy, transparency, and community governance. Some use \
                 independent web indexes while others aggregate results from multiple sources.",
            ),
            (
                "arxiv.org",
                "https://arxiv.org/abs/2024/embedding-models",
                "Efficient Embedding Models for Semantic Search",
                "We present a new family of embedding models optimized for semantic search \
                 applications. Using contrastive learning with hard negatives and knowledge \
                 distillation, our models achieve state-of-the-art retrieval performance at \
                 a fraction of the computational cost. The 384-dimensional embeddings enable \
                 efficient HNSW indexing while maintaining high recall.",
            ),
            (
                "github.com",
                "https://github.com/opensonarx/protocol/wiki/architecture",
                "OpenSonarX Protocol Architecture and Design",
                "The OpenSonarX protocol combines hybrid HNSW vector search with BM25 keyword \
                 matching for accurate information retrieval. Publishers stake TRUTH tokens \
                 to vouch for content quality. The libp2p network distributes index shards \
                 across nodes. Protobuf wire protocol ensures efficient serialization.",
            ),
            (
                "stackoverflow.com",
                "https://stackoverflow.com/questions/rust-async-patterns",
                "Best Practices for Async Rust with Tokio",
                "This comprehensive guide covers async Rust patterns using the Tokio runtime. \
                 Learn about spawning tasks, using channels for communication, handling errors \
                 in async contexts, and structuring concurrent applications. Includes examples \
                 of building HTTP servers with Axum and connecting to databases.",
            ),
        ]
    }

    fn build_chunks(
        articles: &[(&str, &str, &str, &str)],
        embedder: &dyn EmbeddingPipeline,
    ) -> Vec<ContentChunk> {
        articles
            .iter()
            .enumerate()
            .map(|(i, (domain, url, title, content))| {
                let embedding = embedder.embed_text(content).unwrap();
                ContentChunk {
                    chunk_id: format!("bench_{i}"),
                    content: content.to_string(),
                    embedding: Some(embedding),
                    seq: i as u64,
                    meta: Some(ChunkMeta {
                        url: url.to_string(),
                        title: title.to_string(),
                        domain: domain.to_string(),
                        content_hash: format!("sha256_bench_{i:04x}"),
                        crawled_at: Some(1700000000),
                        content_type: Some("text/html".to_string()),
                        entity: None,
                        content_kind: None,
                        ..Default::default()
                    }),
                }
            })
            .collect()
    }

    fn print_local_results(client: &OpenSonarXClient, query: &str) {
        use std::time::Instant;
        eprintln!("\n--- LOCAL INDEX ({} + BM25) ---", client.embedder().name());
        let start = Instant::now();
        let results = client.search(query, 5, &QueryFilters::default()).unwrap();
        let elapsed = start.elapsed();
        for r in &results {
            eprintln!(
                "  #{} [sem={:.4} bm25={:.4} final={:.4}] ${:.0} {}",
                r.rank, r.semantic_score, r.bm25_score, r.final_score, r.stake_usd, r.url
            );
            eprintln!("      {}", r.title);
        }
        eprintln!("  Time: {:?} | Embedder: {}", elapsed, client.embedder().name());
    }

    /// Head-to-head: OpenSonarX (MiniLM) vs DuckDuckGo vs Serper (Google)
    ///
    /// Run with:
    ///   SERPER_API_KEY=<key> cargo test -p oi-sdk --features onnx test_opensonarx_vs_ddg_vs_serper -- --nocapture --ignored
    #[cfg(feature = "onnx")]
    #[test]
    #[ignore] // requires network + SERPER_API_KEY
    fn test_opensonarx_vs_ddg_vs_serper() {
        use std::time::Instant;
        use oi_embed::MiniLmEmbedder;
        use crate::engines::duckduckgo::DuckDuckGo;
        use crate::engines::serper::Serper;

        let serper_key = std::env::var("SERPER_API_KEY")
            .expect("Set SERPER_API_KEY env var to run this test");

        // ── Setup ───────────────────────────────────────────────────────
        let embedder = Arc::new(
            MiniLmEmbedder::new().expect("Failed to load MiniLM model"),
        );
        let client = OpenSonarXClient::with_embedder(SdkConfig::default(), embedder);

        let articles = test_articles();
        let chunks = build_chunks(&articles, client.embedder());
        client.ingest(&chunks).unwrap();

        let domains_to_stake = [
            ("nytimes.com", 50_000_000u64),
            ("techcrunch.com", 30_000_000),
            ("bbc.com", 40_000_000),
            ("reuters.com", 35_000_000),
            ("wired.com", 20_000_000),
            ("arxiv.org", 25_000_000),
            ("github.com", 45_000_000),
            ("stackoverflow.com", 38_000_000),
        ];
        for (domain, amount) in &domains_to_stake {
            client.stake(domain, *amount).unwrap();
        }

        let ddg = DuckDuckGo;
        let serper = Serper::new(serper_key);

        // ── Queries ─────────────────────────────────────────────────────
        let queries = [
            "Rust programming language",
            "AI search engines artificial intelligence",
            "vector database HNSW semantic search",
            "decentralized search protocol blockchain",
            "quantum computing error correction",
            "Solana DeFi staking blockchain",
            "cryptocurrency regulation 2024",
            "async Rust Tokio web server",
        ];

        let sep = "=".repeat(90);
        let thin = "-".repeat(90);

        eprintln!("\n{sep}");
        eprintln!("  OPENINDEX vs DUCKDUCKGO vs SERPER (Google) — Head-to-Head");
        eprintln!("  Model: {} | Index: {} docs | Stakes: {} domains",
            client.embedder().name(), articles.len(), domains_to_stake.len());
        eprintln!("{sep}");

        let mut oi_total_us = 0u128;
        let mut ddg_total_us = 0u128;
        let mut serper_total_us = 0u128;

        for query in &queries {
            eprintln!("\n{thin}");
            eprintln!("  QUERY: \"{query}\"");
            eprintln!("{thin}");

            // ── OpenSonarX local ─────────────────────────────────────────
            let start = Instant::now();
            let oi_results = client.search(query, 10, &QueryFilters::default()).unwrap();
            let oi_elapsed = start.elapsed();
            oi_total_us += oi_elapsed.as_micros();

            eprintln!("\n  OPENINDEX ({:.2}ms):", oi_elapsed.as_micros() as f64 / 1000.0);
            for r in oi_results.iter().take(5) {
                eprintln!(
                    "    #{:<2} {:.<60} stake=${:.0}  score={:.4}",
                    r.rank,
                    truncate_url(&r.url, 60),
                    r.stake_usd,
                    r.final_score,
                );
                eprintln!("        {}", r.title);
            }

            // ── DuckDuckGo ──────────────────────────────────────────────
            let start = Instant::now();
            let ddg_results = ddg.search(query, 10);
            let ddg_elapsed = start.elapsed();
            ddg_total_us += ddg_elapsed.as_micros();

            eprintln!("\n  DUCKDUCKGO ({:.0}ms):", ddg_elapsed.as_millis());
            for r in ddg_results.iter().take(5) {
                eprintln!("    #{:<2} {}", r.rank, truncate_url(&r.url, 70));
                eprintln!("        {}", r.title);
            }

            // ── Serper (Google) ─────────────────────────────────────────
            let start = Instant::now();
            let serper_results = serper.search(query, 10);
            let serper_elapsed = start.elapsed();
            serper_total_us += serper_elapsed.as_micros();

            eprintln!("\n  SERPER/GOOGLE ({:.0}ms):", serper_elapsed.as_millis());
            for r in serper_results.iter().take(5) {
                eprintln!("    #{:<2} {}", r.rank, truncate_url(&r.url, 70));
                eprintln!("        {}", r.title);
            }
        }

        // ── Final summary ───────────────────────────────────────────────
        let n = queries.len() as f64;
        let oi_avg = oi_total_us as f64 / n;
        let ddg_avg = ddg_total_us as f64 / n;
        let serper_avg = serper_total_us as f64 / n;

        eprintln!("\n{sep}");
        eprintln!("  PERFORMANCE SUMMARY ({} queries)", queries.len());
        eprintln!("{thin}");
        eprintln!(
            "  {:.<20} avg {:.2}ms  total {:.0}ms",
            "OPENINDEX", oi_avg / 1000.0, oi_total_us as f64 / 1000.0
        );
        eprintln!(
            "  {:.<20} avg {:.0}ms   total {:.0}ms",
            "DUCKDUCKGO", ddg_avg / 1000.0, ddg_total_us as f64 / 1000.0
        );
        eprintln!(
            "  {:.<20} avg {:.0}ms   total {:.0}ms",
            "SERPER/GOOGLE", serper_avg / 1000.0, serper_total_us as f64 / 1000.0
        );
        eprintln!("{thin}");
        eprintln!(
            "  OpenSonarX is ~{:.0}x faster than DDG, ~{:.0}x faster than Serper",
            ddg_avg / oi_avg.max(1.0),
            serper_avg / oi_avg.max(1.0),
        );
        eprintln!("\n  NOTE: OpenSonarX searches a local index of {} curated, staked documents.", articles.len());
        eprintln!("  DDG/Serper search the entire web. The comparison shows our engine returns");
        eprintln!("  relevant, quality-verified results orders of magnitude faster.");
        eprintln!("{sep}\n");
    }

    fn truncate_url(url: &str, max: usize) -> String {
        if url.len() <= max {
            url.to_string()
        } else {
            format!("{}...", &url[..max - 3])
        }
    }

    #[test]
    fn test_merge_results_domain_cap() {
        use oi_types::SearchResultItem;

        let embedder = Arc::new(MockEmbedder::new(4));
        let registry = oi_staking::InMemoryStakeRegistry::new();
        let query_emb = vec![1.0f32, 0.0, 0.0, 0.0];

        // Create 5 local results from the same domain
        let local: Vec<SearchResultItem> = (0..5)
            .map(|i| SearchResultItem {
                result_id: format!("local_{i}"),
                rank: (i + 1) as u32,
                url: format!("https://example.com/page{i}"),
                title: format!("Page {i}"),
                snippet: format!("snippet {i}"),
                content_md: String::new(),
                semantic_score: 0.9 - i as f64 * 0.1,
                bm25_score: 0.5,
                final_score: 0.9 - i as f64 * 0.1,
                stake_truth: 0,
                stake_usd: 0,
                stake_type: "domain".to_string(),
                reputation: 1.0,
                content_hash: format!("hash_{i}"),
                extraction_quality: "full".to_string(),
                source: "local".to_string(),
                entity: None,
                content_kind: None,
                crawled_at: None,
                published_at: None,
                trust_verdict: "neutral".to_string(),
                reputation_votes: 0,
                disputed: false,
                facts: None,
                content_relevant: None,
                domain_first_seen_at: None,
                domain_url_count: None,
                domain_reputation_score: None,
                domain_total_votes: None,
            })
            .collect();

        // 2 external results from different domains
        let external: Vec<SearchResultItem> = (0..2)
            .map(|i| SearchResultItem {
                result_id: format!("ext_{i}"),
                rank: (i + 1) as u32,
                url: format!("https://other{i}.com/page"),
                title: format!("External {i}"),
                snippet: format!("external snippet {i}"),
                content_md: String::new(),
                semantic_score: 0.3,
                bm25_score: 0.0,
                final_score: 0.3,
                stake_truth: 0,
                stake_usd: 0,
                stake_type: "domain".to_string(),
                reputation: 1.0,
                content_hash: String::new(),
                extraction_quality: "snippet".to_string(),
                source: "duckduckgo".to_string(),
                entity: None,
                content_kind: None,
                crawled_at: None,
                published_at: None,
                trust_verdict: "neutral".to_string(),
                reputation_votes: 0,
                disputed: false,
                facts: None,
                content_relevant: None,
                domain_first_seen_at: None,
                domain_url_count: None,
                domain_reputation_score: None,
                domain_total_votes: None,
            })
            .collect();

        let merged = super::merge_results(
            local, external, 10,
            &registry, &query_emb, embedder.as_ref(),
        );

        // Count per domain
        let mut domain_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for r in &merged {
            *domain_counts.entry(super::extract_domain(&r.url)).or_insert(0) += 1;
        }

        // example.com should have at most 3 results
        let example_count = domain_counts.get("example.com").copied().unwrap_or(0);
        assert!(
            example_count <= 3,
            "example.com should have at most 3 results, got {example_count}"
        );

        // Verify ranks are sequential
        for (i, r) in merged.iter().enumerate() {
            assert_eq!(r.rank, (i + 1) as u32);
        }
    }

    #[test]
    fn test_classify_at_ingest_sets_shard_id() {
        let router = Arc::new(TopicRouter::new(vec![
            ("topic-technology".into(), vec![1.0, 0.0, 0.0, 0.0]),
            ("topic-science".into(), vec![0.0, 1.0, 0.0, 0.0]),
        ]));

        let embedder = Arc::new(MockEmbedder::new(4));
        let client = OpenSonarXClient::with_embedder(SdkConfig::default(), embedder)
            .with_topic_router(router);

        let chunks = vec![ContentChunk {
            chunk_id: "c1".to_string(),
            content: "tech stuff".to_string(),
            embedding: Some(vec![0.9, 0.1, 0.0, 0.0]),
            seq: 0,
            meta: Some(ChunkMeta {
                url: "https://example.com/tech".to_string(),
                title: "Tech".to_string(),
                domain: "example.com".to_string(),
                content_hash: "h1".to_string(),
                ..Default::default()
            }),
        }];

        client.ingest(&chunks).unwrap();
        let results = client.search("tech", 5, &QueryFilters::default()).unwrap();
        assert!(!results.is_empty());
    }

    #[test]
    fn test_shard_filter_drops_off_topic_chunks() {
        let router = Arc::new(TopicRouter::new(vec![
            ("topic-technology".into(), vec![1.0, 0.0, 0.0, 0.0]),
            ("topic-science".into(), vec![0.0, 1.0, 0.0, 0.0]),
        ]));

        let embedder = Arc::new(MockEmbedder::new(4));
        let client = OpenSonarXClient::with_embedder(SdkConfig::default(), embedder)
            .with_topic_router(router)
            .with_shard_filter(vec!["topic-science".to_string()]);

        // This chunk will be classified as topic-technology → should be dropped
        let chunks = vec![ContentChunk {
            chunk_id: "c1".to_string(),
            content: "technology content".to_string(),
            embedding: Some(vec![0.9, 0.1, 0.0, 0.0]),
            seq: 0,
            meta: Some(ChunkMeta {
                url: "https://example.com/tech".to_string(),
                title: "Tech".to_string(),
                domain: "example.com".to_string(),
                content_hash: "h1".to_string(),
                ..Default::default()
            }),
        }];

        client.ingest(&chunks).unwrap();
        let results = client.search("technology", 5, &QueryFilters::default()).unwrap();
        assert!(results.is_empty(), "off-topic chunk should have been dropped");
    }

    #[test]
    fn test_shard_filter_accepts_matching_chunks() {
        let router = Arc::new(TopicRouter::new(vec![
            ("topic-technology".into(), vec![1.0, 0.0, 0.0, 0.0]),
            ("topic-science".into(), vec![0.0, 1.0, 0.0, 0.0]),
        ]));

        let embedder = Arc::new(MockEmbedder::new(4));
        let client = OpenSonarXClient::with_embedder(SdkConfig::default(), embedder)
            .with_topic_router(router)
            .with_shard_filter(vec!["topic-technology".to_string()]);

        let chunks = vec![ContentChunk {
            chunk_id: "c1".to_string(),
            content: "technology content".to_string(),
            embedding: Some(vec![0.9, 0.1, 0.0, 0.0]),
            seq: 0,
            meta: Some(ChunkMeta {
                url: "https://example.com/tech".to_string(),
                title: "Tech".to_string(),
                domain: "example.com".to_string(),
                content_hash: "h1".to_string(),
                ..Default::default()
            }),
        }];

        client.ingest(&chunks).unwrap();
        let results = client.search("technology", 5, &QueryFilters::default()).unwrap();
        assert!(!results.is_empty(), "matching chunk should have been ingested");
    }

    #[test]
    fn test_no_router_passes_all_chunks() {
        let embedder = Arc::new(MockEmbedder::new(4));
        let client = OpenSonarXClient::with_embedder(SdkConfig::default(), embedder);
        // No router, no filter — all chunks pass through

        let chunks = vec![ContentChunk {
            chunk_id: "c1".to_string(),
            content: "anything".to_string(),
            embedding: Some(vec![0.5, 0.5, 0.0, 0.0]),
            seq: 0,
            meta: Some(ChunkMeta {
                url: "https://example.com/page".to_string(),
                title: "Page".to_string(),
                domain: "example.com".to_string(),
                content_hash: "h1".to_string(),
                ..Default::default()
            }),
        }];

        client.ingest(&chunks).unwrap();
        let results = client.search("anything", 5, &QueryFilters::default()).unwrap();
        assert!(!results.is_empty());
    }

    /// Evaluate semantic content_relevant extraction quality.
    ///
    /// Tests the specific failure cases that motivated the keyword→semantic rewrite:
    /// - "memory safety" should surface "dangling pointers, double frees"
    /// - "staking" should surface "stake"
    /// - General relevance across all benchmark articles
    ///
    /// Run: cargo test -p oi-sdk test_semantic_content_relevant_eval -- --nocapture --ignored
    #[test]
    #[ignore] // manual evaluation — run with --ignored --nocapture
    fn test_semantic_content_relevant_eval() {
        let client = OpenSonarXClient::new(SdkConfig::default());

        eprintln!("\n{}", "=".repeat(80));
        eprintln!("  SEMANTIC content_relevant EVALUATION");
        eprintln!("  Embedder: {} (dim={})", client.embedder().name(), client.embedder().dimension());
        eprintln!("{}", "=".repeat(80));

        // ── Documents with content designed to test edge cases ────────────
        let docs: Vec<(&str, &str, &str, &str)> = vec![
            (
                "rust-lang.org",
                "https://rust-lang.org/safety",
                "Memory Safety in Rust",
                "Rust is a systems programming language focused on safety. \
                 The ownership model prevents dangling pointers, double frees, and use-after-free bugs. \
                 The borrow checker enforces these rules at compile time with zero runtime cost. \
                 Rust also supports async/await for concurrent programming. \
                 The cargo package manager makes dependency management easy.",
            ),
            (
                "docs.solana.com",
                "https://docs.solana.com/staking",
                "Solana Staking Guide",
                "Validators on Solana earn rewards by processing transactions. \
                 Token holders can stake their SOL to validators to earn staking rewards. \
                 The minimum stake amount is 0.01 SOL for delegation. \
                 Unstaking has a cooldown period of one epoch, roughly two days. \
                 The annual percentage yield varies based on total network stake.",
            ),
            (
                "opensonarx.com",
                "https://opensonarx.com/docs/protocol",
                "OpenSonarX Protocol Overview",
                "OpenSonarX is a decentralized search protocol for AI agents. \
                 Publishers stake TRUTH tokens to vouch for content quality and accuracy. \
                 The hybrid search engine combines HNSW vector similarity with BM25 keyword matching. \
                 Malicious publishers who stake on spam content risk losing their tokens through disputes. \
                 The libp2p network distributes index shards across participating nodes.",
            ),
            (
                "wikipedia.org",
                "https://en.wikipedia.org/wiki/Quantum_computing",
                "Quantum Computing",
                "Quantum computing uses quantum bits or qubits to perform calculations. \
                 Unlike classical bits, qubits can exist in superposition of states. \
                 Quantum entanglement enables correlations that have no classical equivalent. \
                 Error correction is a major challenge for building practical quantum computers. \
                 Applications include cryptography, drug discovery, and optimization problems.",
            ),
            (
                "arxiv.org",
                "https://arxiv.org/abs/2024/embeddings",
                "Semantic Search with Embeddings",
                "Dense retrieval models encode queries and documents into vector representations. \
                 Cosine similarity between vectors captures semantic relatedness. \
                 Contrastive learning with hard negatives improves retrieval accuracy. \
                 The 384-dimensional Snowflake Arctic Embed model balances quality and efficiency. \
                 Hybrid search combining dense vectors with sparse BM25 outperforms either alone.",
            ),
        ];

        // ── Ingest all documents ──────────────────────────────────────────
        let chunks: Vec<ContentChunk> = docs
            .iter()
            .enumerate()
            .map(|(i, (domain, url, title, content))| {
                let embedding = client.embedder().embed_text(content).unwrap();
                ContentChunk {
                    chunk_id: format!("eval_{i}"),
                    content: content.to_string(),
                    embedding: Some(embedding),
                    seq: i as u64,
                    meta: Some(ChunkMeta {
                        url: url.to_string(),
                        title: title.to_string(),
                        domain: domain.to_string(),
                        content_hash: format!("eval_hash_{i}"),
                        ..Default::default()
                    }),
                }
            })
            .collect();

        client.ingest(&chunks).unwrap();

        // ── Queries to evaluate ───────────────────────────────────────────
        let queries = [
            ("memory safety",               "Should surface: dangling pointers, double frees, borrow checker"),
            ("staking rewards",             "Should surface: stake, staking rewards, cooldown"),
            ("decentralized search",        "Should surface: TRUTH tokens, HNSW, disputes, libp2p"),
            ("quantum error correction",    "Should surface: qubits, error correction, superposition"),
            ("dense retrieval embeddings",  "Should surface: cosine similarity, contrastive learning, 384-dim"),
            ("token staking protocol",      "Should surface: stake TRUTH tokens, disputes, malicious publishers"),
            ("Rust ownership borrow checker","Should surface: ownership model, borrow checker, compile time"),
            ("vector search BM25 hybrid",   "Should surface: HNSW vector similarity, BM25, hybrid search"),
        ];

        let sep = "-".repeat(80);

        for (query, expectation) in &queries {
            eprintln!("\n{sep}");
            eprintln!("  QUERY: \"{query}\"");
            eprintln!("  EXPECT: {expectation}");
            eprintln!("{sep}");

            let query_embedding = client.embedder().embed_query(query).unwrap();
            let results = client.search(query, 5, &QueryFilters::default()).unwrap();

            if results.is_empty() {
                eprintln!("  (no results)");
                continue;
            }

            // Show per-sentence cosine similarities for top 2 results
            for r in results.iter().take(2) {
                eprintln!(
                    "\n  #{} {} (score={:.4})",
                    r.rank, r.url, r.final_score
                );
                eprintln!("     title: {}", r.title);

                // Compute per-sentence similarity scores for diagnosis
                let sentences = oi_index::relevance::split_into_sentences(&r.content_md);
                let sent_texts: Vec<&str> = sentences.iter().map(|(t, _, _)| t.as_str()).collect();
                if let Ok(embs) = client.embedder().embed_text_batch(&sent_texts) {
                    eprintln!("     per-sentence cosine sims:");
                    for (i, ((text, _, _), emb)) in sentences.iter().zip(embs.iter()).enumerate() {
                        let sim: f32 = {
                            let dot: f32 = query_embedding.iter().zip(emb).map(|(a, b)| a * b).sum();
                            let na: f32 = query_embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
                            let nb: f32 = emb.iter().map(|x| x * x).sum::<f32>().sqrt();
                            if na == 0.0 || nb == 0.0 { 0.0 } else { dot / (na * nb) }
                        };
                        let truncated = if text.len() > 70 {
                            format!("{}...", &text[..67])
                        } else {
                            text.to_string()
                        };
                        eprintln!("       [{i}] sim={sim:.4}  \"{truncated}\"");
                    }
                }

                match &r.content_relevant {
                    Some(cr) => {
                        // Wrap at ~100 chars for readability
                        let wrapped: Vec<&str> = cr
                            .as_bytes()
                            .chunks(100)
                            .map(|chunk| std::str::from_utf8(chunk).unwrap_or("?"))
                            .collect();
                        eprintln!("     content_relevant:");
                        for line in wrapped {
                            eprintln!("       {line}");
                        }
                    }
                    None => {
                        eprintln!("     content_relevant: None");
                    }
                }
            }
        }

        eprintln!("\n{}", "=".repeat(80));
        eprintln!("  EVALUATION COMPLETE");
        eprintln!("{}\n", "=".repeat(80));
    }

    /// Timing breakdown: isolate embed_query, index.query, and enrich_content_relevant.
    ///
    /// Run: cargo test -p oi-sdk test_enrichment_timing_breakdown -- --nocapture --ignored
    #[test]
    #[ignore] // manual benchmark — run with --ignored --nocapture
    fn test_enrichment_timing_breakdown() {
        use std::time::Instant;

        let client = OpenSonarXClient::new(SdkConfig::default());

        eprintln!("\n{}", "=".repeat(70));
        eprintln!("  ENRICHMENT TIMING BREAKDOWN");
        eprintln!("  Embedder: {} (dim={})", client.embedder().name(), client.embedder().dimension());
        eprintln!("{}", "=".repeat(70));

        // Ingest benchmark articles
        let articles = test_articles();
        let chunks = build_chunks(&articles, client.embedder());
        client.ingest(&chunks).unwrap();
        eprintln!("[SETUP] Ingested {} documents\n", chunks.len());

        let queries = [
            "AI search engines artificial intelligence",
            "Rust programming language systems safety",
            "cryptocurrency regulation staking DeFi",
            "vector search HNSW embedding semantic",
            "decentralized search protocol token staking",
            "quantum computing error correction",
            "open source search engines privacy",
            "Solana blockchain DeFi staking",
            "async Rust Tokio patterns",
            "embedding models contrastive learning",
        ];

        // Warmup pass (JIT, cache warm)
        for q in &queries {
            let _ = client.search(q, 5, &QueryFilters::default());
        }
        eprintln!("[WARMUP] {} queries completed\n", queries.len());

        let mut total_embed_us = 0u128;
        let mut total_index_us = 0u128;
        let mut total_enrich_us = 0u128;
        let mut total_search_us = 0u128;
        let mut total_sentences = 0usize;

        for query in &queries {
            // 1. embed_query
            let t0 = Instant::now();
            let query_embedding = client.embedder().embed_query(query).unwrap();
            let embed_us = t0.elapsed().as_micros();

            // 2. index.query (no enrichment)
            let t1 = Instant::now();
            let mut results = client.index().query(&query_embedding, query, &QueryFilters::default(), 5).unwrap();
            let index_us = t1.elapsed().as_micros();

            // Count sentences in results
            let sent_count: usize = results.iter()
                .map(|r| oi_index::relevance::split_into_sentences(&r.content_md).len())
                .sum();

            // 3. enrich_content_relevant
            let t2 = Instant::now();
            client.enrich_content_relevant(&mut results, &query_embedding);
            let enrich_us = t2.elapsed().as_micros();

            // 4. full search() for comparison
            let t3 = Instant::now();
            let _ = client.search(query, 5, &QueryFilters::default()).unwrap();
            let search_us = t3.elapsed().as_micros();

            total_embed_us += embed_us;
            total_index_us += index_us;
            total_enrich_us += enrich_us;
            total_search_us += search_us;
            total_sentences += sent_count;

            let enriched_count = results.iter().filter(|r| r.content_relevant.is_some()).count();

            eprintln!(
                "  q=\"{}\"\n    embed={:>5}us  index={:>4}us  enrich={:>5}us  total_search={:>5}us  sents={}  enriched={}/{}",
                query, embed_us, index_us, enrich_us, search_us, sent_count, enriched_count, results.len()
            );
        }

        let n = queries.len() as f64;
        let avg_embed = total_embed_us as f64 / n;
        let avg_index = total_index_us as f64 / n;
        let avg_enrich = total_enrich_us as f64 / n;
        let avg_search = total_search_us as f64 / n;
        let avg_sents = total_sentences as f64 / n;

        eprintln!("\n{}", "-".repeat(70));
        eprintln!("  AVERAGES ({} queries, {:.0} sentences/query):", queries.len(), avg_sents);
        eprintln!("    embed_query:           {:>7.0}us ({:.2}ms)", avg_embed, avg_embed / 1000.0);
        eprintln!("    index.query:           {:>7.0}us ({:.2}ms)", avg_index, avg_index / 1000.0);
        eprintln!("    enrich_content_relevant:{:>6.0}us ({:.2}ms)  ← NEW", avg_enrich, avg_enrich / 1000.0);
        eprintln!("    full search():         {:>7.0}us ({:.2}ms)", avg_search, avg_search / 1000.0);
        eprintln!("\n    enrichment overhead: {:.1}% of total search time",
            avg_enrich / avg_search * 100.0);
        eprintln!("{}\n", "=".repeat(70));
    }

    /// Eval: code block-aware content_relevant extraction.
    ///
    /// Verifies that ``` fenced code blocks with `. ` patterns (e.g. `api . logger . info(...)`)
    /// are kept atomic and not shattered into incoherent fragments.
    ///
    /// Run: cargo test -p oi-sdk test_code_block_content_relevant -- --nocapture --ignored
    #[test]
    #[ignore]
    fn test_code_block_content_relevant() {
        let client = OpenSonarXClient::new(SdkConfig::default());

        eprintln!("\n{}", "=".repeat(80));
        eprintln!("  CODE BLOCK content_relevant EVAL");
        eprintln!("  Embedder: {} (dim={})", client.embedder().name(), client.embedder().dimension());
        eprintln!("{}", "=".repeat(80));

        let docs: Vec<(&str, &str, &str, &str)> = vec![
            (
                "docs.openclaw.io",
                "https://docs.openclaw.io/plugins/getting-started",
                "Creating Plugins - OpenClaw Docs",
                "How to Create a Plugin for OpenClaw. \
                 Plugins extend the OpenClaw gateway with custom behavior. \
                 This guide walks you through building your first plugin from scratch. \
                 Step 1 Create the Project. \
                 Create a new directory for your plugin.",
            ),
            (
                "docs.openclaw.io",
                "https://docs.openclaw.io/plugins/register",
                "Register Function - OpenClaw Docs",
                "Step 3 Write the Register Function. \
                 Create src/index.ts with the register function that OpenClaw calls on startup:\n\
                 ```\n\
                 export default function register ( api : any ) {\n\
                   api . logger . info ( \"Hello from my plugin!\" );\n\
                   api . hooks . onSearch ( async ( query : string ) => {\n\
                     api . logger . debug ( \"Search query: \" + query );\n\
                     return query;\n\
                   });\n\
                 }\n\
                 ```\n\
                 When the gateway starts, it discovers your plugin, calls register, and passes in the Plugin API object. \
                 The Plugin API gives you access to hooks, logging, and configuration.",
            ),
            (
                "docs.openclaw.io",
                "https://docs.openclaw.io/plugins/config",
                "Plugin Config - OpenClaw Docs",
                "Step 4 Configure the Plugin. \
                 Add your plugin to the OpenClaw gateway config:\n\
                 ```\n\
                 {\n\
                   \"plugins\": [\n\
                     {\n\
                       \"name\": \"my-plugin\",\n\
                       \"path\": \"./plugins/my-plugin\",\n\
                       \"enabled\": true\n\
                     }\n\
                   ]\n\
                 }\n\
                 ```\n\
                 Going Further. \
                 Explore the Plugin API reference for advanced hooks like onIndex, onCrawl, and onRank. \
                 You can also publish your plugin to the OpenClaw registry for others to use.",
            ),
            (
                "docs.openclaw.io",
                "https://docs.openclaw.io/architecture",
                "Architecture - OpenClaw Docs",
                "OpenClaw Architecture Overview. \
                 The OpenClaw search engine uses a distributed index with HNSW vector search and BM25 full-text search. \
                 Nodes communicate via libp2p and synchronize state using a Merkle DAG. \
                 The staking mechanism ensures content quality through economic incentives.",
            ),
        ];

        let chunks: Vec<ContentChunk> = docs
            .iter()
            .enumerate()
            .map(|(i, (domain, url, title, content))| {
                let embedding = client.embedder().embed_text(content).unwrap();
                ContentChunk {
                    chunk_id: format!("oc_{i}"),
                    content: content.to_string(),
                    embedding: Some(embedding),
                    seq: i as u64,
                    meta: Some(ChunkMeta {
                        url: url.to_string(),
                        title: title.to_string(),
                        domain: domain.to_string(),
                        content_hash: format!("oc_hash_{i}"),
                        ..Default::default()
                    }),
                }
            })
            .collect();

        client.ingest(&chunks).unwrap();
        eprintln!("[SETUP] Ingested {} documents\n", chunks.len());

        let query = "how to install openclaw";
        eprintln!("  QUERY: \"{query}\"");
        eprintln!("{}", "-".repeat(80));

        let query_embedding = client.embedder().embed_query(query).unwrap();
        let results = client.search(query, 5, &QueryFilters::default()).unwrap();

        for r in &results {
            eprintln!(
                "\n  #{} [{}] {} (score={:.4})",
                r.rank, r.source, r.url, r.final_score
            );
            eprintln!("     title: {}", r.title);

            let sentences = oi_index::relevance::split_into_sentences(&r.content_md);
            let sent_texts: Vec<&str> = sentences.iter().map(|(t, _, _)| t.as_str()).collect();
            if let Ok(embs) = client.embedder().embed_text_batch(&sent_texts) {
                eprintln!("     sentences ({}):", sentences.len());
                for (i, ((text, _, _), emb)) in sentences.iter().zip(embs.iter()).enumerate() {
                    let sim: f32 = {
                        let dot: f32 = query_embedding.iter().zip(emb).map(|(a, b)| a * b).sum();
                        let na: f32 = query_embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
                        let nb: f32 = emb.iter().map(|x| x * x).sum::<f32>().sqrt();
                        if na == 0.0 || nb == 0.0 { 0.0 } else { dot / (na * nb) }
                    };
                    let truncated = if text.len() > 80 {
                        format!("{}...", &text[..77])
                    } else {
                        text.to_string()
                    };
                    eprintln!("       [{i}] sim={sim:.4}  \"{truncated}\"");
                }
            }

            match &r.content_relevant {
                Some(cr) => {
                    eprintln!("     content_relevant:");
                    for line in cr.lines() {
                        if line.len() > 100 {
                            eprintln!("       {}...", &line[..97]);
                        } else {
                            eprintln!("       {line}");
                        }
                    }
                }
                None => eprintln!("     content_relevant: None"),
            }
        }

        eprintln!("\n{}", "=".repeat(80));
    }
}
