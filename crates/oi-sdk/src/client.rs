use crate::answer::{AnswerPolicy, AnswerTrace, AnsweredSearch};
use crate::config::SdkConfig;
use crate::evidence::{ClaimStance, apply_claim_stance, apply_decision_policy};
use crate::leaderboard::InMemoryLeaderboard;
use crate::source::source_family;
use crate::wallet::Wallet;
use oi_index::{HybridIndex, Storage};
use oi_staking::InMemoryStakeRegistry;
use oi_types::traits::{SearchIndex, StakeRegistry};
use oi_types::{
    ClaimExtractor, ContentChunk, EmbeddingPipeline, QueryFilters, QueryLeaderboard,
    ResponsivenessJudge, Result, SearchEngine, SearchParams, SearchResultItem, StakeInfo,
    StanceJudge, TopicRouter,
};
use std::sync::Arc;

const MAX_DISCOVERY_POOL: usize = 25;

fn discovery_pool_limit(top_k: usize, configured_providers: usize) -> usize {
    top_k
        .saturating_mul(configured_providers.max(1))
        .min(MAX_DISCOVERY_POOL)
        .max(top_k)
}

struct CrawlFlight {
    complete: std::sync::Mutex<bool>,
    ready: std::sync::Condvar,
}

struct CachedAnsweredSearch {
    stored_at: std::time::Instant,
    answered: crate::answer::AnsweredSearch,
}

struct AnswerFlightLeader<'a> {
    key: String,
    flight: Arc<CrawlFlight>,
    flights: &'a std::sync::Mutex<std::collections::HashMap<String, Arc<CrawlFlight>>>,
}

impl Drop for AnswerFlightLeader<'_> {
    fn drop(&mut self) {
        // Remove the flight before waking followers. A follower immediately
        // re-enters `search_answered`; it must see retained evidence or become
        // a new leader after an errored acquisition, never attach itself to a
        // completed entry and recurse in a tight loop.
        if let Ok(mut flights) = self.flights.lock()
            && flights
                .get(&self.key)
                .is_some_and(|active| Arc::ptr_eq(active, &self.flight))
        {
            flights.remove(&self.key);
        }
        if let Ok(mut complete) = self.flight.complete.lock() {
            *complete = true;
            self.flight.ready.notify_all();
        }
    }
}

enum AnswerFlightEntry<'a> {
    Leader(AnswerFlightLeader<'a>),
    Follower(Arc<CrawlFlight>),
}

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
    /// Network fetches may run concurrently across answered searches, but
    /// index persistence must not write the same snapshot files concurrently.
    crawl_persist: std::sync::Mutex<()>,
    /// Coalesce the same live URL across concurrent answer requests. Followers
    /// wait for the leader's bounded crawl and then read the shared index.
    crawl_flights: std::sync::Mutex<std::collections::HashMap<String, Arc<CrawlFlight>>>,
    /// Coalesce concurrent identical answered searches before any fetch or
    /// model call.
    answer_flights: std::sync::Mutex<std::collections::HashMap<String, Arc<CrawlFlight>>>,
    /// Very short-lived, process-local reuse of a completed answered search.
    /// Absorbs bursts of the same request after the leader has left its
    /// in-flight slot; empty or failed acquisitions are retried.
    answer_cache: std::sync::Mutex<std::collections::HashMap<String, CachedAnsweredSearch>>,
    /// Reads a request and its retained passages and proposes claims with
    /// quotes. Without one, `search_answered` returns the slate unanswered.
    claim_extractor: Option<Arc<dyn ClaimExtractor>>,
    /// Labels each retained passage's stance toward each proposed claim.
    stance_judge: Option<Arc<dyn StanceJudge>>,
    /// Decides whether each proposed claim answers the request as asked.
    responsiveness_judge: Option<Arc<dyn ResponsivenessJudge>>,
    answer_policy: AnswerPolicy,
    /// Usage attribution → creator credits (the payoff leg of the market).
    creator_ledger: crate::creator_ledger::CreatorLedger,
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
        tracing::error!(
            "USING MOCK EMBEDDER — search results will be semantically random! Build with --features onnx for real embeddings."
        );
        Arc::new(oi_embed::MockEmbedder::default())
    }

    /// Create a new client with a custom embedding pipeline.
    ///
    /// If `local_index_path` is set and contains saved index segments,
    /// the index is loaded from disk. Otherwise a fresh index is created.
    pub fn with_embedder(config: SdkConfig, embedder: Arc<dyn EmbeddingPipeline>) -> Self {
        let embedder: Arc<dyn EmbeddingPipeline> =
            Arc::new(oi_embed::CachedEmbedder::new(embedder, 10_000));
        let dim = embedder.dimension();
        let stake_registry = Arc::new(InMemoryStakeRegistry::new());
        Self::load_stakes(&config, &stake_registry);

        let storage = config
            .local_index_path
            .as_ref()
            .and_then(|path| Storage::new(path).ok());

        // Fresh indexes record which embedding model their vectors come from;
        // loaded indexes are checked against the current model — mismatched
        // vectors don't error, they silently return garbage similarity.
        let fresh_config = oi_index::hybrid::HybridConfig {
            embedder_name: embedder.name().to_string(),
            ..Default::default()
        };
        let index = if let Some(ref s) = storage
            && s.segment_exists("hnsw.bin")
        {
            match HybridIndex::load(s) {
                Ok(idx) => {
                    tracing::info!("Loaded index from disk ({} docs)", idx.doc_count());
                    let recorded = idx.embedder_name();
                    if !recorded.is_empty() && recorded != embedder.name() {
                        tracing::error!(
                            "EMBEDDER MISMATCH: index was built with '{recorded}' but the \
                             current model is '{}'. Vector similarity will be meaningless \
                             until you run `oi index reembed`.",
                            embedder.name()
                        );
                    }
                    Arc::new(
                        idx.with_stake_registry(stake_registry.clone() as Arc<dyn StakeRegistry>),
                    )
                }
                Err(e) => {
                    tracing::warn!("Failed to load index from disk, starting fresh: {e}");
                    Arc::new(
                        HybridIndex::with_config(dim, fresh_config)
                            .with_stake_registry(stake_registry.clone() as Arc<dyn StakeRegistry>),
                    )
                }
            }
        } else {
            Arc::new(
                HybridIndex::with_config(dim, fresh_config)
                    .with_stake_registry(stake_registry.clone() as Arc<dyn StakeRegistry>),
            )
        };

        let wallet = config
            .wallet_path
            .as_ref()
            .and_then(|p| Wallet::from_path(p).ok());

        let search_engines = crate::engines::default_engines(&config);
        let leaderboard: Arc<dyn QueryLeaderboard> = Arc::new(InMemoryLeaderboard::new());

        let config_creator_ledger_path = config.creator_ledger_path.clone();
        let answer_model = config
            .answer_model
            .clone()
            .map(|model| Arc::new(crate::chat_model::ChatModel::new(model)));
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
            crawl_persist: std::sync::Mutex::new(()),
            crawl_flights: std::sync::Mutex::new(std::collections::HashMap::new()),
            answer_flights: std::sync::Mutex::new(std::collections::HashMap::new()),
            answer_cache: std::sync::Mutex::new(std::collections::HashMap::new()),
            claim_extractor: answer_model
                .clone()
                .map(|model| model as Arc<dyn ClaimExtractor>),
            stance_judge: answer_model
                .clone()
                .map(|model| model as Arc<dyn StanceJudge>),
            responsiveness_judge: answer_model.map(|model| model as Arc<dyn ResponsivenessJudge>),
            answer_policy: AnswerPolicy::default(),
            creator_ledger: crate::creator_ledger::CreatorLedger::load(config_creator_ledger_path),
        }
    }

    /// Create a client over an EXISTING shared index (e.g. the resident
    /// node's `Arc<HybridIndex>`, kept live by the integrated sentinel).
    /// The client adds the blended-search layer — external engines,
    /// crawl-on-query learning, spell correction — on top of that index
    /// without loading a second copy. No storage is attached: persistence
    /// stays with the index's owner (`save_index` is a no-op here).
    pub fn with_shared_index(config: SdkConfig, index: Arc<HybridIndex>) -> Self {
        let embedder: Arc<dyn EmbeddingPipeline> = Arc::new(oi_embed::CachedEmbedder::new(
            Self::default_embedder(),
            10_000,
        ));
        let stake_registry = Arc::new(InMemoryStakeRegistry::new());
        Self::load_stakes(&config, &stake_registry);
        let wallet = config
            .wallet_path
            .as_ref()
            .and_then(|p| Wallet::from_path(p).ok());
        let search_engines = crate::engines::default_engines(&config);
        let leaderboard: Arc<dyn QueryLeaderboard> = Arc::new(InMemoryLeaderboard::new());
        let config_creator_ledger_path = config.creator_ledger_path.clone();
        let answer_model = config
            .answer_model
            .clone()
            .map(|model| Arc::new(crate::chat_model::ChatModel::new(model)));
        Self {
            config,
            index,
            stake_registry,
            embedder,
            wallet,
            storage: None,
            search_engines,
            leaderboard,
            topic_router: None,
            shard_filter: Vec::new(),
            crawl_queue: std::sync::Mutex::new(Vec::new()),
            crawl_persist: std::sync::Mutex::new(()),
            crawl_flights: std::sync::Mutex::new(std::collections::HashMap::new()),
            answer_flights: std::sync::Mutex::new(std::collections::HashMap::new()),
            answer_cache: std::sync::Mutex::new(std::collections::HashMap::new()),
            claim_extractor: answer_model
                .clone()
                .map(|model| model as Arc<dyn ClaimExtractor>),
            stance_judge: answer_model
                .clone()
                .map(|model| model as Arc<dyn StanceJudge>),
            responsiveness_judge: answer_model.map(|model| model as Arc<dyn ResponsivenessJudge>),
            answer_policy: AnswerPolicy::default(),
            creator_ledger: crate::creator_ledger::CreatorLedger::load(config_creator_ledger_path),
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
    pub fn with_reputation_provider(
        mut self,
        provider: Arc<dyn oi_types::ReputationProvider>,
    ) -> Self {
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

    /// Attach the model that proposes claims and quotes for `search_answered`.
    pub fn with_claim_extractor(mut self, extractor: Arc<dyn ClaimExtractor>) -> Self {
        self.claim_extractor = Some(extractor);
        self
    }

    /// Attach the model that judges passage stance toward proposed claims.
    pub fn with_stance_judge(mut self, judge: Arc<dyn StanceJudge>) -> Self {
        self.stance_judge = Some(judge);
        self
    }

    /// Attach the model that decides whether a claim answers the request.
    pub fn with_responsiveness_judge(mut self, judge: Arc<dyn ResponsivenessJudge>) -> Self {
        self.responsiveness_judge = Some(judge);
        self
    }

    /// Replace the numeric answer policy.
    pub fn with_answer_policy(mut self, policy: AnswerPolicy) -> Self {
        self.answer_policy = policy;
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
                tracing::debug!(
                    "embed_text_batch failed for content_relevant, keeping existing: {e}"
                );
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
            let sentence_embeddings: Vec<Vec<f32>> =
                all_embeddings[emb_idx..emb_idx + count].to_vec();
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

        // Ownership proof: does the domain publish oi-verify=<wallet> in a
        // TXT record? Anyone can bond capital to any name — only this proof
        // ties the bond to the domain owner. Unverified stakes are accepted
        // (the bond is real and visible) but their verdict caps at "neutral".
        // Without a wallet there is nothing to prove (and no network call).
        let dns_verified = self.wallet.is_some()
            && crate::dns_verify::verify_domain_ownership(domain, &wallet_addr);
        if !dns_verified {
            tracing::warn!(
                "Domain ownership not verified for {domain}: publish TXT record \
                 '{}' to unlock the trusted verdict tier",
                crate::dns_verify::expected_txt(&wallet_addr)
            );
        }

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
            dns_verified,
            reputation: 1.0,
            entity: None,
            disputed: false,
        };
        self.stake_registry.add_stake(info)?;
        self.save_stakes();

        // Return a stub tx hash
        let tx_hash = format!("stub_tx_{domain}_{amount}");
        Ok(tx_hash)
    }

    /// Persist the stake registry (off-chain mirror; in web3 mode the chain
    /// remains the source of truth) so stakes survive across processes —
    /// without this, trust verdicts silently reset on every CLI invocation.
    /// No-op when `config.stakes_path` is unset (no hidden home-dir writes
    /// for library consumers).
    fn save_stakes(&self) {
        use oi_types::traits::StakeRegistry;
        let Some(ref path) = self.config.stakes_path else {
            return;
        };
        let path = std::path::Path::new(path);
        let Ok(stakes) = self.stake_registry.list_active_stakes() else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match serde_json::to_vec_pretty(&stakes) {
            Ok(bytes) => {
                if let Err(e) = std::fs::write(path, bytes) {
                    tracing::warn!("failed to persist stakes: {e}");
                }
            }
            Err(e) => tracing::warn!("failed to serialize stakes: {e}"),
        }
    }

    /// Load persisted stakes into the registry (called at client build).
    /// No-op when `stakes_path` is unset.
    fn load_stakes(config: &SdkConfig, registry: &InMemoryStakeRegistry) {
        let Some(ref path) = config.stakes_path else {
            return;
        };
        Self::load_stakes_from(path, registry);
    }

    /// Load a persisted stake mirror file into a registry. Public so hosts
    /// that build their own index (the P2P node) can attach the same stake
    /// state — without it, staked domains verdict as "unverified" on that
    /// surface and min_trust filters everything out.
    pub fn load_stakes_from(path: &str, registry: &InMemoryStakeRegistry) {
        let Ok(bytes) = std::fs::read(path) else {
            return;
        };
        match serde_json::from_slice::<Vec<StakeInfo>>(&bytes) {
            Ok(stakes) => {
                for info in stakes {
                    let _ = registry.add_stake(info);
                }
            }
            Err(e) => tracing::warn!("failed to parse stakes file: {e}"),
        }
    }

    /// Initiate unstaking for a domain (7-day cooldown).
    ///
    /// Sets the cooldown timestamp on the stake entry. Returns a stub tx hash.
    pub fn unstake(&self, domain: &str) -> Result<String> {
        // Remove from registry (simplified -- production would set cooldown)
        self.stake_registry.remove_stake(domain)?;
        self.save_stakes();
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

    /// Number of documents currently retained by the local index.
    pub fn indexed_document_count(&self) -> usize {
        self.index.doc_count()
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
    /// compete on the same semantic/lexical scale as local results. Economic
    /// metadata never changes Stage-1 relevance; claim-specific conviction can
    /// enter only through a calibrated Stage-2 OpenUncertainty assessment.
    ///
    /// Returns `(results, sources_used)`.
    pub fn search_blended(
        &self,
        query: &str,
        top_k: usize,
        filters: &QueryFilters,
    ) -> Result<(Vec<SearchResultItem>, Vec<String>)> {
        self.search_blended_internal(query, top_k, filters, true)
    }

    fn search_blended_internal(
        &self,
        query: &str,
        top_k: usize,
        filters: &QueryFilters,
        record_usage: bool,
    ) -> Result<(Vec<SearchResultItem>, Vec<String>)> {
        if top_k == 0 {
            return Ok((Vec::new(), Vec::new()));
        }

        // Spell-correct from the indexed vocabulary ("javascrpt promisses" →
        // "javascript promises") — a misspelled query otherwise embeds
        // off-target for local retrieval. The ORIGINAL query still goes to
        // external engines: they have far larger correction vocabularies, so
        // a small-corpus miscorrection must never leak outward.
        let corrected = self.correct_query(query);
        let retrieval_query = corrected.as_deref().unwrap_or(query);

        // Compute query embedding once — used for both local search and external re-scoring
        let query_embedding = self.embedder.embed_query(retrieval_query)?;
        // Retrieval depth and presentation depth are intentionally separate.
        // Asking for five displayed results must not mean that only five local
        // candidates survive to the shared-scale rerank.
        let candidate_k = retrieval_candidate_depth(top_k);
        let mut local_results =
            self.index
                .query(&query_embedding, retrieval_query, filters, candidate_k)?;
        let mut sources_used = Vec::new();
        if local_results.iter().any(|result| result.source == "local") {
            sources_used.push("local".to_string());
        }

        let local_count = local_results.len();
        apply_blend_scores(
            &mut local_results,
            local_count,
            query,
            &query_embedding,
            self.embedder.as_ref(),
        );
        local_results.sort_by(|a, b| b.final_score.total_cmp(&a.final_score));

        // A min_trust floor means "vetted results only": external engine
        // results are unvetted by definition (verdict "external"), so the
        // blend is local-only — the index has already enforced the floor.
        // Also the no-engines backward-compat path.
        let local_only = self.search_engines.is_empty() || filters.min_trust.is_some();
        // Local-coverage fast path: when the index already covers the query
        // well, skip the ~1s external engine round-trip entirely. Coverage is
        // judged on the same absolute blend score the merge would compute, so
        // this changes latency, not ranking.
        let strong_coverage = local_results.len() >= top_k
            && local_results
                .first()
                .is_some_and(|r| r.final_score >= LOCAL_COVERAGE_TOP1)
            && local_results
                .get(top_k - 1)
                .is_some_and(|r| r.final_score >= LOCAL_COVERAGE_KTH);
        if local_only || strong_coverage {
            local_results.truncate(top_k);
            self.enrich_content_relevant(&mut local_results, &query_embedding);
            apply_decision_policy(query, &mut local_results);
            if record_usage {
                self.record_leaderboard(&local_results);
                self.attribute_usage(&local_results);
            }
            return Ok((local_results, sources_used));
        }

        self.blend_with_external(
            query,
            top_k,
            filters,
            local_results,
            query_embedding,
            sources_used,
            record_usage,
        )
    }

    /// Search, then answer from retained evidence.
    ///
    /// Retrieval is the same stake-blind path as `search_blended`. With a
    /// claim extractor configured, result pages that have no retained text
    /// are fetched in the foreground, the extractor proposes claims with
    /// quotes, and the stance judge labels each retained passage against each
    /// claim. Every quote is then located verbatim in retained text before it
    /// can be cited (`crate::answer`). Without an extractor the slate itself
    /// is the answer. Usage is attributed exactly once, to the final slate.
    pub fn search_answered(
        &self,
        query: &str,
        top_k: usize,
        filters: &QueryFilters,
    ) -> Result<AnsweredSearch> {
        let total_started = std::time::Instant::now();
        let elapsed_ms = |since: std::time::Instant| since.elapsed().as_secs_f64() * 1_000.0;

        let cache_key = answered_search_cache_key(query, top_k, filters);
        let reuse = |mut cached: AnsweredSearch| {
            cached.trace = AnswerTrace {
                evidence_checklist: cached.trace.evidence_checklist.take(),
                extractor_model: cached.trace.extractor_model.take(),
                stance_model: cached.trace.stance_model.take(),
                fresh_answer_cache_hit: true,
                total_ms: elapsed_ms(total_started),
                ..Default::default()
            };
            cached
        };
        if let Some(cached) = self.cached_answered_search(&cache_key) {
            return Ok(reuse(cached));
        }
        // Coalesce concurrent identical requests: one leader acquires
        // evidence and calls the models; followers reuse its decision.
        let _flight = match self.enter_answer_flight(cache_key.clone()) {
            AnswerFlightEntry::Leader(guard) => Some(guard),
            AnswerFlightEntry::Follower(flight) => {
                self.wait_for_answer_flight(&flight);
                if let Some(cached) = self.cached_answered_search(&cache_key) {
                    return Ok(reuse(cached));
                }
                None
            }
        };

        let mut trace = AnswerTrace::default();
        let started = std::time::Instant::now();
        let (mut results, sources_used) =
            self.search_blended_internal(query, top_k, filters, false)?;
        trace.retrieval_ms = elapsed_ms(started);

        let policy = &self.answer_policy;
        let mut hydrated_pages = 0;
        let (mut answer, checklist) = match &self.claim_extractor {
            None => crate::answer::results_only(policy),
            Some(extractor) => {
                let started = std::time::Instant::now();
                let urls: Vec<String> = results
                    .iter()
                    .take(policy.max_passages)
                    .filter(|result| result.content_md.trim().is_empty())
                    .map(|result| result.url.clone())
                    .take(policy.max_hydrations)
                    .collect();
                trace.hydration_requested = urls.len();
                if !urls.is_empty() {
                    self.remove_pending_crawl_urls(&urls);
                    hydrated_pages = self.process_crawl_urls(urls.clone());
                    self.attach_retained_content(query, &mut results, &urls)?;
                }
                trace.hydration_ms = elapsed_ms(started);

                let passages = crate::answer::build_passages(&results, policy);
                let retained: Vec<_> = passages
                    .iter()
                    .filter(|passage| passage.retained)
                    .cloned()
                    .collect();
                trace.passages_read = passages.len();
                trace.retained_passages = retained.len();
                if passages.is_empty() {
                    // Nothing to read: no model is called for an empty slate.
                    crate::answer::results_only(policy)
                } else {
                    trace.extractor_model = Some(extractor.model_version());
                    let started = std::time::Instant::now();
                    let extraction = extractor.extract(query, &passages);
                    trace.extraction_ms = elapsed_ms(started);
                    match extraction {
                        Err(error) => {
                            trace.model_error = Some(error.to_string());
                            crate::answer::results_only(policy)
                        }
                        Ok(mut extraction) => {
                            trace.proposed_claims = extraction.claims.len();
                            crate::answer::merge_repeated_claims(&mut extraction);
                            let started = std::time::Instant::now();
                            let claims: Vec<&str> = extraction
                                .claims
                                .iter()
                                .take(policy.max_claims)
                                .map(|claim| claim.text.as_str())
                                .collect();
                            // Stance and responsiveness are independent
                            // judgments; run them side by side.
                            let (stance, responsive) = std::thread::scope(|scope| {
                                let stance = self.stance_judge.as_ref().map(|judge| {
                                    scope.spawn(|| judge_claims(judge.as_ref(), &claims, &retained))
                                });
                                let responsive = self
                                    .responsiveness_judge
                                    .as_ref()
                                    .filter(|_| !claims.is_empty())
                                    .map(|judge| judge.answers_request(query, &claims));
                                (
                                    stance.map(|call| {
                                        call.join().unwrap_or_else(|_| {
                                            Err(oi_types::Error::Internal(
                                                "stance judge panicked".into(),
                                            ))
                                        })
                                    }),
                                    responsive,
                                )
                            });
                            trace.stance_model = self
                                .stance_judge
                                .as_ref()
                                .map(|judge| judge.model_version());
                            trace.responsiveness_model = self
                                .responsiveness_judge
                                .as_ref()
                                .map(|judge| judge.model_version());
                            let judgments = stance.and_then(|judged| {
                                judged
                                    .map_err(|error| trace.model_error = Some(error.to_string()))
                                    .ok()
                            });
                            let verdicts = responsive.and_then(|judged| {
                                judged
                                    .map_err(|error| trace.model_error = Some(error.to_string()))
                                    .ok()
                            });
                            let (responsive, discarded) = crate::answer::responsiveness_decisions(
                                query,
                                &extraction.claims[..claims.len()],
                                verdicts.as_deref(),
                            );
                            trace.unshown_rejections = discarded;
                            trace.stance_ms = elapsed_ms(started);
                            let (answer, checklist, stats) = crate::answer::assemble_answer(
                                &results,
                                &passages,
                                &extraction,
                                crate::answer::ClaimJudgments {
                                    stance: judgments.as_deref(),
                                    responsive: &responsive,
                                },
                                policy,
                            );
                            trace.off_request_claims = answer
                                .claims
                                .iter()
                                .filter(|claim| claim.status == "off_request")
                                .count();
                            trace.rejected_quotes = stats.rejected_quotes;
                            (answer, checklist)
                        }
                    }
                }
            }
        };

        // Stage 2: results cited for or against a claim now carry stance.
        let mut stance: std::collections::HashMap<String, ClaimStance> =
            std::collections::HashMap::new();
        for citation in answer
            .claims
            .iter()
            .filter(|claim| claim.status != "off_request")
            .flat_map(|claim| &claim.citations)
        {
            let entry = stance.entry(citation.url.clone()).or_default();
            if citation.evidence_role == "contradicts" {
                entry.contradicts += 1;
            } else {
                entry.supports += 1;
            }
        }
        if !stance.is_empty() {
            apply_claim_stance(query, &mut results, &stance);
            for citation in answer
                .claims
                .iter_mut()
                .flat_map(|claim| &mut claim.citations)
            {
                citation.decision_rank = results
                    .iter()
                    .find(|result| result.url == citation.url)
                    .map(|result| result.rank)
                    .unwrap_or(0);
            }
        }

        // Structured join: a cited page that prints an official poll
        // registration ID gets that exact registry record attached.
        let cited_urls: std::collections::HashSet<String> = answer
            .claims
            .iter()
            .flat_map(|claim| &claim.citations)
            .map(|citation| citation.url.clone())
            .collect();
        let mut poll_registry = Vec::new();
        let registration_ids =
            crate::pesqele::registration_ids_for_claim_urls(&results, &cited_urls);
        if !registration_ids.is_empty() {
            let started = std::time::Instant::now();
            let cache_dir = self
                .config
                .local_index_path
                .as_deref()
                .map(|path| std::path::Path::new(path).join("pesqele-registry"));
            for registration_id in &registration_ids {
                match crate::pesqele::lookup_with_cache(registration_id, cache_dir.as_deref()) {
                    Ok(Some(record)) => poll_registry.push(record),
                    Ok(None) => {}
                    Err(_) => trace.registry_lookup_errors += 1,
                }
            }
            trace.registry_lookup_ms = elapsed_ms(started);
        }

        trace.unavailable_discovery_providers = self
            .search_engines
            .iter()
            .filter(|engine| !engine.is_available())
            .map(|engine| engine.name().to_string())
            .collect();
        if !trace.unavailable_discovery_providers.is_empty() {
            let diagnostic = "Busca degradada: há provedores de descoberta indisponíveis. Ausência de resultados não comprova ausência de informação; a evidência retida pode estar incompleta.";
            answer.warning = Some(answer.warning.map_or_else(
                || diagnostic.to_string(),
                |warning| format!("{warning} {diagnostic}"),
            ));
        }
        self.record_leaderboard(&results);
        self.attribute_usage(&results);
        trace.evidence_checklist = Some(checklist);
        trace.total_ms = elapsed_ms(total_started);
        let answered = AnsweredSearch {
            query: query.to_string(),
            answer,
            results,
            sources_used,
            hydrated_pages,
            poll_registry,
            trace,
        };
        if !answered.results.is_empty() && answered.trace.model_error.is_none() {
            self.cache_answered_search(cache_key, &answered);
        }
        Ok(answered)
    }

    /// Copy freshly retained content onto slate results for the same URLs.
    /// Scores, ranks and sources are untouched: fetching a page makes its
    /// text citable, it does not make it more relevant.
    fn attach_retained_content(
        &self,
        query: &str,
        results: &mut [SearchResultItem],
        urls: &[String],
    ) -> Result<()> {
        let retained = self.local_evidence_for_exact_urls(query, urls, urls.len())?;
        for result in results.iter_mut() {
            let Some(fetched) = retained.iter().find(|fetched| {
                fetched.url.trim_end_matches('/') == result.url.trim_end_matches('/')
                    && !fetched.content_md.trim().is_empty()
            }) else {
                continue;
            };
            result.content_md = fetched.content_md.clone();
            result.content_relevant = fetched.content_relevant.clone();
            result.content_hash = fetched.content_hash.clone();
            result.extraction_quality = fetched.extraction_quality.clone();
            result.crawled_at = fetched.crawled_at;
            result.published_at = result.published_at.or(fetched.published_at);
            result.facts = fetched.facts.clone();
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn blend_with_external(
        &self,
        query: &str,
        top_k: usize,
        filters: &QueryFilters,
        local_results: Vec<SearchResultItem>,
        query_embedding: Vec<f32>,
        mut sources_used: Vec<String>,
        record_usage: bool,
    ) -> Result<(Vec<SearchResultItem>, Vec<String>)> {
        let params = SearchParams {
            locale: filters.locale.clone(),
            gl: filters.gl.clone(),
            site: filters.site.clone(),
            freshness: filters.freshness.clone(),
        };
        // Stage 1 must preserve enough depth from every configured discovery
        // provider for the final decision. Asking the orchestrator for only
        // `top_k` total candidates means a two-provider interleave keeps
        // roughly half of each provider's top-k. The user-visible slate
        // remains `top_k`; this only widens the bounded internal pool.
        let discovery_limit = discovery_pool_limit(top_k, self.search_engines.len());
        let external_results = crate::engines::search_with_engines(
            &self.search_engines,
            query,
            discovery_limit,
            &params,
        );

        for result in &external_results {
            if !sources_used.contains(&result.source) {
                sources_used.push(result.source.clone());
            }
        }
        if external_results
            .iter()
            .any(|result| result.extraction_quality == "external_cache")
            && !sources_used
                .iter()
                .any(|source| source == "discovery_cache")
        {
            // A short-lived provider-SERP cache, not retained evidence.
            sources_used.push("discovery_cache".to_string());
        }

        let mut merged = merge_results(
            local_results,
            external_results,
            query,
            top_k,
            &self.stake_registry,
            &query_embedding,
            self.embedder.as_ref(),
        );
        self.enrich_content_relevant(&mut merged, &query_embedding);
        apply_decision_policy(query, &mut merged);
        if record_usage {
            self.record_leaderboard(&merged);
            self.attribute_usage(&merged);
        }

        // Crawl-on-query: queue external results for background indexing so
        // every query permanently grows local coverage. Drained by
        // `process_crawl_queue` (callers decide when to pay the crawl cost).
        if self.config.crawl_on_query {
            let pending = self.urls_to_crawl(&merged, self.config.crawl_on_query_max_urls);
            let pending = self.exclude_recently_retained_background_urls(query, pending);
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

    /// Suggest a spelling correction for a query from the indexed vocabulary.
    /// Returns `Some(corrected)` only if a token changed. `search_blended`
    /// applies this automatically; callers can use it to display a
    /// "showing results for ..." note.
    pub fn correct_query(&self, query: &str) -> Option<String> {
        self.index.correct_query(query)
    }

    /// Re-score externally-sourced results (e.g. P2P peer answers) on the
    /// same absolute title+snippet scale `search_blended` uses, so they can
    /// be merged fairly with blended results. Treats all items as carrying
    /// no local BM25 evidence.
    pub fn rescore_extra(&self, query: &str, items: &mut [SearchResultItem]) {
        let Ok(query_embedding) = self.embedder.embed_query(query) else {
            return;
        };
        apply_blend_scores(items, 0, query, &query_embedding, self.embedder.as_ref());
    }

    /// Attribute served LOCAL results to their creators' usage ledger —
    /// an agent consuming this content generated no page view; this record
    /// is what replaces it (see BACKLOG.md Phase 1).
    fn attribute_usage(&self, results: &[SearchResultItem]) {
        let served: Vec<(String, u32, bool)> = results
            .iter()
            .filter(|r| r.source == "local")
            .map(|r| {
                let domain = r
                    .url
                    .strip_prefix("https://")
                    .or_else(|| r.url.strip_prefix("http://"))
                    .and_then(|rest| rest.split('/').next())
                    .unwrap_or("")
                    .to_string();
                (domain, r.rank, r.stake_usd > 0)
            })
            .collect();
        self.creator_ledger.record_query(&served);
    }

    /// Creator usage snapshot: (queries_attributed, per-domain usage sorted
    /// by credits).
    pub fn creator_earnings(&self) -> (u64, Vec<(String, crate::creator_ledger::CreatorUsage)>) {
        self.creator_ledger.snapshot()
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
        self.process_crawl_urls(urls)
    }

    fn remove_pending_crawl_urls(&self, urls: &[String]) {
        if urls.is_empty() {
            return;
        }
        if let Ok(mut queue) = self.crawl_queue.lock() {
            queue.retain(|pending| !urls.contains(pending));
        }
    }

    fn enter_answer_flight(&self, key: String) -> AnswerFlightEntry<'_> {
        let Ok(mut flights) = self.answer_flights.lock() else {
            return AnswerFlightEntry::Leader(AnswerFlightLeader {
                key,
                flight: Arc::new(CrawlFlight {
                    complete: std::sync::Mutex::new(false),
                    ready: std::sync::Condvar::new(),
                }),
                flights: &self.answer_flights,
            });
        };
        if let Some(flight) = flights.get(&key) {
            return AnswerFlightEntry::Follower(Arc::clone(flight));
        }
        let flight = Arc::new(CrawlFlight {
            complete: std::sync::Mutex::new(false),
            ready: std::sync::Condvar::new(),
        });
        flights.insert(key.clone(), Arc::clone(&flight));
        AnswerFlightEntry::Leader(AnswerFlightLeader {
            key,
            flight,
            flights: &self.answer_flights,
        })
    }

    fn wait_for_answer_flight(&self, flight: &Arc<CrawlFlight>) {
        const FOLLOWER_DEADLINE: std::time::Duration = std::time::Duration::from_secs(10);
        let Ok(complete) = flight.complete.lock() else {
            return;
        };
        let _ = flight
            .ready
            .wait_timeout_while(complete, FOLLOWER_DEADLINE, |complete| !*complete);
    }

    fn cached_answered_search(&self, key: &str) -> Option<crate::answer::AnsweredSearch> {
        const ANSWER_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(30);
        let mut cache = self.answer_cache.lock().ok()?;
        cache.retain(|_, entry| entry.stored_at.elapsed() <= ANSWER_CACHE_TTL);
        cache.get(key).map(|entry| entry.answered.clone())
    }

    fn cache_answered_search(&self, key: String, answered: &crate::answer::AnsweredSearch) {
        const MAX_ANSWER_CACHE_ENTRIES: usize = 64;
        let Ok(mut cache) = self.answer_cache.lock() else {
            return;
        };
        if cache.len() >= MAX_ANSWER_CACHE_ENTRIES
            && let Some(oldest) = cache
                .iter()
                .min_by_key(|(_, entry)| entry.stored_at)
                .map(|(key, _)| key.clone())
        {
            cache.remove(&oldest);
        }
        cache.insert(
            key,
            CachedAnsweredSearch {
                stored_at: std::time::Instant::now(),
                answered: answered.clone(),
            },
        );
    }

    /// Fetch and retain one request-owned evidence batch. Unlike the
    /// background queue, this method never drains work belonging to another
    /// concurrent query.
    fn process_crawl_urls(&self, urls: Vec<String>) -> usize {
        if urls.is_empty() {
            return 0;
        }

        let (urls, leader_flights, follower_flights) = self.partition_crawl_flights(urls);
        if urls.is_empty() {
            self.wait_for_crawl_flights(&follower_flights);
            return 0;
        }

        let distiller = Arc::new(oi_crawl::Distiller::new().with_embedder(self.embedder_arc()));
        let pesqele_cache_dir = self
            .config
            .local_index_path
            .as_deref()
            .map(std::path::Path::new)
            .map(|path| path.join("pesqele-registry"));
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
                    use oi_crawl::robots::{RobotsChecker, parse_robots_txt};
                    use oi_types::CrawlOutput;
                    use oi_types::traits::CrawlPipeline;
                    use tokio::task::JoinSet;

                    // Hard bounds: a slow page must never hold a search
                    // process hostage. Distinct publishers are fetched with
                    // bounded concurrency; the per-URL timeout covers both
                    // robots.txt and the page itself.
                    // Foreground evidence acquisition has a serving budget,
                    // not a crawler-daemon budget. Healthy live pages in the
                    // benchmark complete comfortably below this bound; a bot
                    // wall or stalled origin must not add a 15-second tail.
                    const PER_URL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(4);
                    const QUEUE_DEADLINE: std::time::Duration = std::time::Duration::from_secs(5);
                    const MAX_CONCURRENT: usize = 5;
                    let started = std::time::Instant::now();

                    let mut pending = urls.iter().cloned();
                    let mut active: JoinSet<Option<CrawlOutput>> = JoinSet::new();
                    let mut outputs = Vec::new();

                    let launch = |active: &mut JoinSet<Option<CrawlOutput>>, url: String| {
                        let distiller = Arc::clone(&distiller);
                        active.spawn(async move {
                            let target = url.clone();
                            match tokio::time::timeout(PER_URL_TIMEOUT, async move {
                                let mut robots = RobotsChecker::new();
                                if let Some(domain) = url
                                    .strip_prefix("https://")
                                    .or_else(|| url.strip_prefix("http://"))
                                    .and_then(|rest| rest.split('/').next())
                                {
                                    let robots_url = format!("https://{domain}/robots.txt");
                                    if let Ok(res) = oi_crawl::fetcher::fetch_url(&robots_url).await
                                    {
                                        robots.add_rules(domain, parse_robots_txt(&res.html));
                                    }
                                    if !robots.is_allowed(&url) {
                                        tracing::debug!("crawl-on-query: robots.txt blocked {url}");
                                        return None;
                                    }
                                }
                                match distiller.crawl(&url).await {
                                    Ok(out) if !out.chunks.is_empty() => Some(out),
                                    Ok(_) => {
                                        tracing::debug!("crawl-on-query: no content at {url}");
                                        None
                                    }
                                    Err(error) => {
                                        tracing::debug!(
                                            "crawl-on-query: crawl failed for {url}: {error}"
                                        );
                                        None
                                    }
                                }
                            })
                            .await
                            {
                                Ok(output) => output,
                                Err(_) => {
                                    tracing::debug!("crawl-on-query: timed out crawling {target}");
                                    None
                                }
                            }
                        });
                    };

                    for _ in 0..MAX_CONCURRENT {
                        if let Some(url) = pending.next() {
                            launch(&mut active, url);
                        }
                    }

                    while !active.is_empty() {
                        let Some(remaining) = QUEUE_DEADLINE.checked_sub(started.elapsed()) else {
                            tracing::debug!("crawl-on-query: queue deadline reached");
                            break;
                        };
                        match tokio::time::timeout(remaining, active.join_next()).await {
                            Ok(Some(Ok(Some(output)))) => {
                                crate::pesqele::prefetch_from_retained_text(
                                    &output.markdown,
                                    pesqele_cache_dir.clone(),
                                );
                                outputs.push(output);
                            }
                            Ok(Some(Ok(None))) => {}
                            Ok(Some(Err(error))) => {
                                tracing::debug!("crawl-on-query: task failed: {error}")
                            }
                            Ok(None) => break,
                            Err(_) => {
                                tracing::debug!("crawl-on-query: queue deadline reached");
                                break;
                            }
                        }
                        if let Some(url) = pending.next() {
                            launch(&mut active, url);
                        }
                    }
                    active.abort_all();
                    outputs
                })
            })
            .join()
            .unwrap_or_default()
        });

        let mut indexed = 0;
        if let Ok(_persist_guard) = self.crawl_persist.lock() {
            // One crawl batch is one index mutation. Acquiring the HNSW/BM25
            // write locks once materially reduces reader stalls under mixed
            // search+crawl load and keeps URL replacement atomic across the
            // pages fetched for the same answer.
            let chunks = outputs
                .iter()
                .flat_map(|output| output.chunks.iter().cloned())
                .collect::<Vec<_>>();
            if !chunks.is_empty() {
                match self.ingest(&chunks) {
                    Ok(()) => indexed = outputs.len(),
                    Err(e) => tracing::debug!("crawl-on-query: batch ingest failed: {e}"),
                }
            }
            // Index mutations are immediately visible. Wake same-URL
            // followers before serializing the snapshot so a busy disk does
            // not inflate answer latency.
            self.finish_crawl_flights(&leader_flights);
            if indexed > 0
                && let Err(e) = self.save_index()
            {
                tracing::warn!("crawl-on-query: index save failed: {e}");
            }
        } else {
            self.finish_crawl_flights(&leader_flights);
        }
        self.wait_for_crawl_flights(&follower_flights);
        indexed
    }

    fn partition_crawl_flights(
        &self,
        urls: Vec<String>,
    ) -> (
        Vec<String>,
        Vec<(String, Arc<CrawlFlight>)>,
        Vec<Arc<CrawlFlight>>,
    ) {
        let mut unique = std::collections::HashSet::new();
        let Ok(mut flights) = self.crawl_flights.lock() else {
            return (urls, Vec::new(), Vec::new());
        };
        let mut leaders = Vec::new();
        let mut leader_flights = Vec::new();
        let mut followers = Vec::new();
        for url in urls.into_iter().filter(|url| unique.insert(url.clone())) {
            if let Some(flight) = flights.get(&url) {
                followers.push(Arc::clone(flight));
            } else {
                let flight = Arc::new(CrawlFlight {
                    complete: std::sync::Mutex::new(false),
                    ready: std::sync::Condvar::new(),
                });
                flights.insert(url.clone(), Arc::clone(&flight));
                leaders.push(url.clone());
                leader_flights.push((url, flight));
            }
        }
        (leaders, leader_flights, followers)
    }

    fn finish_crawl_flights(&self, leaders: &[(String, Arc<CrawlFlight>)]) {
        for (_, flight) in leaders {
            if let Ok(mut complete) = flight.complete.lock() {
                *complete = true;
                flight.ready.notify_all();
            }
        }
        if let Ok(mut flights) = self.crawl_flights.lock() {
            for (url, leader) in leaders {
                if flights
                    .get(url)
                    .is_some_and(|active| Arc::ptr_eq(active, leader))
                {
                    flights.remove(url);
                }
            }
        }
    }

    fn wait_for_crawl_flights(&self, followers: &[Arc<CrawlFlight>]) {
        const FOLLOWER_DEADLINE: std::time::Duration = std::time::Duration::from_secs(6);
        let started = std::time::Instant::now();
        for flight in followers {
            let Ok(complete) = flight.complete.lock() else {
                continue;
            };
            let Some(remaining) = FOLLOWER_DEADLINE.checked_sub(started.elapsed()) else {
                break;
            };
            let _ = flight
                .ready
                .wait_timeout_while(complete, remaining, |complete| !*complete);
        }
    }

    /// Query external search engines only and return discovered URLs.
    /// Used by frontier seeding to bootstrap the corpus without embedding or local index queries.
    pub fn search_external_only(&self, query: &str, limit: usize) -> Vec<String> {
        if self.search_engines.is_empty() {
            return Vec::new();
        }
        let params = SearchParams::default();
        let results =
            crate::engines::search_with_engines(&self.search_engines, query, limit, &params);
        results.into_iter().map(|r| r.url).collect()
    }

    /// Retrieve only exact, already-selected crawl targets. This is the
    /// bounded fallback for a busy mixed index where freshly hydrated pages
    /// can fall below the global vector window. It never performs discovery.
    fn local_evidence_for_exact_urls(
        &self,
        query: &str,
        urls: &[String],
        limit: usize,
    ) -> Result<Vec<SearchResultItem>> {
        let lookup_query = if query.trim().is_empty() {
            urls.join(" ")
        } else {
            query.to_string()
        };
        let query_embedding = self.embedder.embed_query(&lookup_query)?;
        self.index
            .query_exact_urls(&query_embedding, &lookup_query, urls, limit)
    }

    /// Returns external URLs from blended results that are not yet in the local index.
    /// Used by crawl-on-query to grow the corpus organically.
    pub fn urls_to_crawl(&self, results: &[SearchResultItem], max: usize) -> Vec<String> {
        self.diverse_urls_to_crawl(results, max)
    }

    /// The background flywheel should acquire new URLs and periodically
    /// revalidate old ones, but it must not refetch the same retained page on
    /// every equivalent query in a burst. Foreground claim paths have their
    /// own stricter intent-specific freshness rules; this five-minute guard is
    /// only for opportunistic post-response crawling.
    fn exclude_recently_retained_background_urls(
        &self,
        query: &str,
        mut urls: Vec<String>,
    ) -> Vec<String> {
        const BACKGROUND_RECRAWL_TTL_SECS: u64 = 300;
        if urls.is_empty() {
            return urls;
        }
        let retained = self
            .local_evidence_for_exact_urls(query, &urls, urls.len().saturating_mul(20).max(20))
            .unwrap_or_default();
        let now = unix_time_secs();
        let fresh_urls = retained
            .iter()
            .filter(|result| {
                result.crawled_at.is_some_and(|crawled_at| {
                    now.saturating_sub(crawled_at) <= BACKGROUND_RECRAWL_TTL_SECS
                })
            })
            .map(|result| result.url.trim_end_matches('/').to_string())
            .collect::<std::collections::HashSet<_>>();
        urls.retain(|url| !fresh_urls.contains(url.trim_end_matches('/')));
        urls
    }

    /// Prefer distinct domains when hydrating answer evidence. Retrieval may
    /// legitimately return several pages from one publisher, but crawling all
    /// of them cannot establish independent corroboration.
    fn diverse_urls_to_crawl(&self, results: &[SearchResultItem], max: usize) -> Vec<String> {
        use std::collections::HashSet;

        let mut families = HashSet::new();
        let mut urls = Vec::new();
        for result in results
            .iter()
            .filter(|result| result.source != "local" && result.source != "cached")
        {
            let family = source_family(&result.url);
            if families.insert(family) {
                urls.push(result.url.clone());
                if urls.len() == max {
                    break;
                }
            }
        }
        urls
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
/// The local index preserves its raw query/chunk cosine separately from its
/// max-normalized internal fusion score. At blend time external title+snippet
/// text is embedded into that same absolute vector space, while locals reuse
/// the raw HNSW cosine. Locals keep a small normalized BM25 term-match hint;
/// externals carry none. This stops an irrelevant but "locally best" result
/// from outranking a genuinely relevant external one without re-embedding the
/// entire local candidate pool. Stake never moves the score.
fn merge_results(
    local: Vec<SearchResultItem>,
    external: Vec<SearchResultItem>,
    query: &str,
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

    // Embed only external title+snippet text; local raw HNSW cosine is already
    // on the shared absolute scale.
    apply_blend_scores(&mut merged, local_count, query, query_embedding, embedder);

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

fn unix_time_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Local-coverage fast path thresholds: the external engine round-trip is
/// skipped only when the best local hit is strongly on-topic (top-1) AND the
/// whole top_k is solid (k-th). A high top-1 bar prevents a page of merely
/// same-domain results (e.g. Rust std docs for "rust borrow checker") from
/// suppressing the external answer.
const LOCAL_COVERAGE_TOP1: f64 = 0.70;
const LOCAL_COVERAGE_KTH: f64 = 0.58;
/// Candidate pool used before shared-scale and intent-aware reranking.
/// Keeps normal result pages bounded while allowing large explicit requests.
pub fn retrieval_candidate_depth(top_k: usize) -> usize {
    top_k.saturating_mul(10).max(50).min(top_k.max(100))
}

/// Re-score results on the shared absolute scale. Local results already carry
/// the absolute query/chunk cosine returned by HNSW; only external snippets
/// need an embedding here. This avoids re-embedding 50 local candidates per
/// request while keeping both sides in the same model/vector space. The first
/// `local_count` items retain a small query-normalized BM25 hint; external
/// candidates carry none.
fn apply_blend_scores(
    results: &mut [SearchResultItem],
    local_count: usize,
    query: &str,
    query_embedding: &[f32],
    embedder: &dyn EmbeddingPipeline,
) {
    if results.is_empty() {
        return;
    }
    let texts: Vec<String> = results
        .iter()
        .skip(local_count)
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

    // Title/snippet lexical coverage is computed for BOTH local and web
    // candidates. Local BM25 is query-normalized against only the local pool,
    // so it is retained as a small retrieval hint rather than an asymmetric
    // advantage that web discoveries can never earn.
    const SEMANTIC_W: f64 = 0.82;
    const LEXICAL_W: f64 = 0.16;
    const LOCAL_BM25_W: f64 = 0.02;
    for (i, result) in results.iter_mut().enumerate() {
        let semantic_score = if i < local_count {
            result.semantic_score.max(0.0)
        } else {
            embeddings
                .get(i - local_count)
                .map(|emb| cosine_similarity(query_embedding, emb).max(0.0) as f64)
                .unwrap_or(0.0)
        };
        let bm25 = if i < local_count {
            result.bm25_score.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let lexical = lexical_relevance(query, result);
        result.semantic_score = semantic_score;
        result.final_score =
            SEMANTIC_W * semantic_score + LEXICAL_W * lexical + LOCAL_BM25_W * bm25;
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

/// Judge every claim against the retained passages, one model call per
/// claim, concurrently. Each call is independent, so the answer waits for the
/// slowest call instead of the sum of all of them.
fn judge_claims(
    judge: &dyn StanceJudge,
    claims: &[&str],
    retained: &[oi_types::EvidencePassage],
) -> Result<Vec<Vec<oi_types::StanceJudgment>>> {
    if retained.is_empty() {
        // No retained text can support or contradict anything.
        return Ok(claims.iter().map(|_| Vec::new()).collect());
    }
    if let [claim] = claims {
        return Ok(vec![judge.judge(claim, retained)?]);
    }
    std::thread::scope(|scope| {
        let calls: Vec<_> = claims
            .iter()
            .map(|claim| scope.spawn(move || judge.judge(claim, retained)))
            .collect();
        calls
            .into_iter()
            .map(|call| {
                call.join().unwrap_or_else(|_| {
                    Err(oi_types::Error::Internal("stance judge panicked".into()))
                })
            })
            .collect()
    })
}

/// Identical requests share one answer flight and one short-lived cache
/// entry. The key is the request itself: no rewriting, no equivalence table.
fn answered_search_cache_key(query: &str, top_k: usize, filters: &QueryFilters) -> String {
    let filters = serde_json::to_string(filters).unwrap_or_default();
    format!("{}:k={top_k}:filters={filters}", query.trim())
}

/// Shared lexical score for local and external candidates: the share of
/// query terms present in the title and in the title plus body surface.
fn lexical_relevance(query: &str, result: &SearchResultItem) -> f64 {
    let query_terms = crate::text::terms(query);
    if query_terms.is_empty() {
        return 0.0;
    }

    let title_terms = crate::text::terms(&result.title);
    let body = if result.snippet.is_empty() {
        result.content_md.chars().take(400).collect::<String>()
    } else {
        result.snippet.clone()
    };
    let mut all_terms = title_terms.clone();
    all_terms.extend(crate::text::terms(&body));

    let title_coverage = crate::text::term_coverage(&query_terms, &title_terms);
    let all_coverage = crate::text::term_coverage(&query_terms, &all_terms);
    0.65 * title_coverage + 0.35 * all_coverage
}

#[cfg(test)]
mod tests {
    use super::*;
    use oi_embed::MockEmbedder;
    use oi_types::ChunkMeta;

    struct NoDocumentReembed;

    impl EmbeddingPipeline for NoDocumentReembed {
        fn embed_text(&self, _text: &str) -> Result<Vec<f32>> {
            panic!("local blend must not re-embed document text")
        }

        fn dimension(&self) -> usize {
            2
        }

        fn name(&self) -> &str {
            "no-document-reembed"
        }
    }

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
    fn discovery_pool_preserves_top_k_depth_per_provider_with_a_hard_cap() {
        assert_eq!(discovery_pool_limit(5, 1), 5);
        assert_eq!(discovery_pool_limit(5, 2), 10);
        assert_eq!(discovery_pool_limit(5, 3), 15);
        assert_eq!(discovery_pool_limit(10, 8), MAX_DISCOVERY_POOL);
        assert_eq!(discovery_pool_limit(30, 2), 30);
    }

    #[test]
    fn concurrent_crawl_batches_coalesce_the_same_url() {
        let client = OpenSonarXClient::with_embedder(
            SdkConfig::default(),
            Arc::new(MockEmbedder::default()),
        );
        let url = "https://example.test/evidence".to_string();
        let (first_urls, first_leaders, first_followers) =
            client.partition_crawl_flights(vec![url.clone(), url.clone()]);
        assert_eq!(first_urls, vec![url.clone()]);
        assert_eq!(first_leaders.len(), 1);
        assert!(first_followers.is_empty());

        let (second_urls, second_leaders, second_followers) =
            client.partition_crawl_flights(vec![url]);
        assert!(second_urls.is_empty());
        assert!(second_leaders.is_empty());
        assert_eq!(second_followers.len(), 1);

        client.finish_crawl_flights(&first_leaders);
        client.wait_for_crawl_flights(&second_followers);
        assert!(*second_followers[0].complete.lock().unwrap());
    }

    #[test]
    fn test_local_candidate_depth_is_independent_from_display_limit() {
        assert_eq!(super::retrieval_candidate_depth(1), 50);
        assert_eq!(super::retrieval_candidate_depth(5), 50);
        assert_eq!(super::retrieval_candidate_depth(10), 100);
        assert_eq!(super::retrieval_candidate_depth(20), 100);
        assert_eq!(super::retrieval_candidate_depth(200), 200);
    }

    #[test]
    fn local_blend_reuses_absolute_hnsw_semantic_score() {
        let mut results = vec![coverage_result(
            "https://example.test/rust",
            "Rust",
            "Rust programming language",
        )];
        results[0].semantic_score = 0.73;
        results[0].bm25_score = 0.5;
        super::apply_blend_scores(&mut results, 1, "rust", &[1.0, 0.0], &NoDocumentReembed);
        assert_eq!(results[0].semantic_score, 0.73);
        assert!(results[0].final_score > 0.6);
    }

    fn coverage_result(url: &str, title: &str, content_md: &str) -> SearchResultItem {
        SearchResultItem {
            result_id: url.to_string(),
            rank: 1,
            url: url.to_string(),
            title: title.to_string(),
            snippet: content_md.chars().take(160).collect(),
            content_md: content_md.to_string(),
            semantic_score: 0.8,
            bm25_score: 0.8,
            final_score: 0.8,
            stake_truth: 0,
            stake_usd: 0,
            stake_type: "domain".to_string(),
            reputation: 0.0,
            content_hash: url.to_string(),
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
            decision: None,
        }
    }

    #[test]
    fn exact_url_lookup_separates_pages_of_one_shared_domain() {
        let embedder = Arc::new(MockEmbedder::new(16));
        let client = OpenSonarXClient::with_embedder(SdkConfig::default(), embedder.clone());
        let dengue_url = "https://www.gov.br/saude/pt-br/assuntos/saude-de-a-a-z/d/dengue";
        let tax_url = "https://www.gov.br/receitafederal/pt-br/assuntos/imposto-de-renda";
        let documents = [
            ("dengue", dengue_url, "Dengue: sinais e sintomas de alarme"),
            ("tax", tax_url, "Imposto de renda e declaração anual"),
        ];
        let chunks = documents
            .iter()
            .map(|(id, url, content)| ContentChunk {
                chunk_id: (*id).to_string(),
                content: (*content).to_string(),
                embedding: Some(embedder.embed_text(content).unwrap()),
                seq: 0,
                meta: Some(ChunkMeta {
                    url: (*url).to_string(),
                    title: (*content).to_string(),
                    domain: "www.gov.br".to_string(),
                    content_hash: format!("{id}-hash"),
                    entity: oi_types::entity::entity_from_url(url),
                    ..Default::default()
                }),
            })
            .collect::<Vec<_>>();
        client.ingest(&chunks).unwrap();

        let evidence = client
            .local_evidence_for_exact_urls(
                "dengue sinais e sintomas de alarme",
                &[dengue_url.to_string()],
                10,
            )
            .unwrap();
        assert_eq!(evidence.len(), 1);
        assert_eq!(evidence[0].url, dengue_url);
    }

    use oi_types::{
        ClaimExtraction, EvidencePassage, ProposedClaim, ProposedQuote, RequestKind,
        StanceJudgment, StanceLabel,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    const FIRST_URL: &str = "https://news.test/acme";
    const SECOND_URL: &str = "https://reference.test/acme";
    const FIRST_TEXT: &str = "Acme history. The company was started by Ada Example in Recife.";
    const SECOND_TEXT: &str = "Acme profile. Records show the company was started by Ada Example.";
    const SPAN: &str = "company was started by Ada Example";

    /// Proposes `claims` claims, each quoting `quote` from every retained
    /// passage.
    struct ScriptedExtractor {
        quote: &'static str,
        calls: AtomicUsize,
        fail: bool,
        claims: usize,
        /// Declare that the claim reads "Acme" in the request as "company".
        substitutes: bool,
    }

    impl ScriptedExtractor {
        fn quoting(quote: &'static str) -> Arc<Self> {
            Arc::new(Self {
                quote,
                calls: AtomicUsize::new(0),
                fail: false,
                claims: 1,
                substitutes: false,
            })
        }
    }

    impl ClaimExtractor for ScriptedExtractor {
        fn extract(&self, _query: &str, passages: &[EvidencePassage]) -> Result<ClaimExtraction> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                return Err(oi_types::Error::Internal("model unavailable".into()));
            }
            Ok(ClaimExtraction {
                request_kind: RequestKind::Question,
                claims: (0..self.claims)
                    .map(|index| ProposedClaim {
                        text: if index == 0 {
                            "Ada Example started the company.".to_string()
                        } else {
                            format!("Ada Example started the company ({index}).")
                        },
                        quotes: passages
                            .iter()
                            .filter(|passage| passage.retained)
                            .map(|passage| ProposedQuote {
                                passage_id: passage.id.clone(),
                                quote: self.quote.to_string(),
                            })
                            .collect(),
                        substitution: self.substitutes.then(|| oi_types::TermSubstitution {
                            request_term: "Acme".to_string(),
                            claim_term: "company".to_string(),
                        }),
                    })
                    .collect(),
                ..Default::default()
            })
        }

        fn model_version(&self) -> String {
            "scripted-extractor".to_string()
        }
    }

    /// Labels every passage from `contradicting_host` as a contradiction and
    /// every other passage as support, quoting `SPAN`.
    struct ScriptedJudge {
        contradicting_host: Option<&'static str>,
    }

    impl StanceJudge for ScriptedJudge {
        fn judge(&self, _claim: &str, passages: &[EvidencePassage]) -> Result<Vec<StanceJudgment>> {
            Ok(passages
                .iter()
                .map(|passage| StanceJudgment {
                    passage_id: passage.id.clone(),
                    label: if self
                        .contradicting_host
                        .is_some_and(|host| passage.url.contains(host))
                    {
                        StanceLabel::Contradicts
                    } else {
                        StanceLabel::Supports
                    },
                    quote: Some(SPAN.to_string()),
                })
                .collect())
        }

        fn model_version(&self) -> String {
            "scripted-judge".to_string()
        }
    }

    /// Gives every claim the same verdict.
    struct ScriptedResponsiveness {
        answers: bool,
    }

    impl ResponsivenessJudge for ScriptedResponsiveness {
        fn answers_request(
            &self,
            _query: &str,
            claims: &[&str],
        ) -> Result<Vec<oi_types::ResponsivenessVerdict>> {
            // A rejection names "Robotics" in the request and "company" in
            // the claim, both of which occur literally in the test inputs.
            Ok(vec![
                oi_types::ResponsivenessVerdict {
                    answers: self.answers,
                    request_term: (!self.answers).then(|| "Robotics".to_string()),
                    claim_term: (!self.answers).then(|| "company".to_string()),
                };
                claims.len()
            ])
        }

        fn model_version(&self) -> String {
            "scripted-responsiveness".to_string()
        }
    }

    fn client_with_two_sources() -> OpenSonarXClient {
        let embedder = Arc::new(MockEmbedder::new(16));
        let client = OpenSonarXClient::with_embedder(SdkConfig::default(), embedder.clone())
            .with_search_engines(Vec::new());
        let chunks = [(FIRST_URL, FIRST_TEXT), (SECOND_URL, SECOND_TEXT)]
            .iter()
            .enumerate()
            .map(|(index, (url, content))| ContentChunk {
                chunk_id: format!("c{index}"),
                content: (*content).to_string(),
                embedding: Some(embedder.embed_text(content).unwrap()),
                seq: 0,
                meta: Some(ChunkMeta {
                    url: (*url).to_string(),
                    title: "Acme".to_string(),
                    domain: crate::source::host(url),
                    content_hash: format!("hash-{index}"),
                    ..Default::default()
                }),
            })
            .collect::<Vec<_>>();
        client.ingest(&chunks).unwrap();
        client
    }

    #[test]
    fn answered_search_without_a_model_returns_the_slate_unanswered() {
        let client = client_with_two_sources();
        let answered = client
            .search_answered("who started Acme", 5, &QueryFilters::default())
            .unwrap();
        assert_eq!(answered.answer.status, "results_only");
        assert_eq!(answered.answer.intent, "unclassified");
        assert!(answered.answer.claims.is_empty());
        assert_eq!(answered.results.len(), 2);
        assert_eq!(answered.trace.extractor_model, None);
        assert_eq!(answered.trace.passages_read, 0);
        for (index, result) in answered.results.iter().enumerate() {
            let decision = result.decision.as_ref().unwrap();
            assert_eq!(decision.retrieval_rank, index as u32 + 1);
            assert_eq!(decision.decision_rank, index as u32 + 1);
        }
    }

    #[test]
    fn answered_search_cites_only_verified_quotes_from_two_families() {
        let client = client_with_two_sources()
            .with_claim_extractor(ScriptedExtractor::quoting(SPAN))
            .with_stance_judge(Arc::new(ScriptedJudge {
                contradicting_host: None,
            }))
            .with_responsiveness_judge(Arc::new(ScriptedResponsiveness { answers: true }));
        let answered = client
            .search_answered("who started Acme", 5, &QueryFilters::default())
            .unwrap();
        assert_eq!(answered.answer.status, "extractive");
        assert_eq!(answered.answer.intent, "question");
        assert_eq!(answered.answer.summary, "Ada Example started the company.");
        let claim = &answered.answer.claims[0];
        assert_eq!(claim.status, "corroborated");
        assert_eq!(claim.citations.len(), 2);
        for citation in &claim.citations {
            assert_eq!(citation.evidence_role, "supports");
            assert_eq!(citation.evidence_excerpt.as_deref(), Some(SPAN));
            let cited = answered
                .results
                .iter()
                .find(|result| result.url == citation.url)
                .unwrap();
            assert_eq!(cited.rank, citation.decision_rank);
            assert!(cited.decision.as_ref().unwrap().support > 0.0);
        }
        let trace = &answered.trace;
        assert_eq!(trace.extractor_model.as_deref(), Some("scripted-extractor"));
        assert_eq!(trace.stance_model.as_deref(), Some("scripted-judge"));
        assert_eq!(trace.retained_passages, 2);
        assert_eq!(trace.rejected_quotes, 0);
        assert_eq!(trace.hydration_requested, 0);
        let checklist = trace.evidence_checklist.as_ref().unwrap();
        assert!(checklist.complete_answer_supported);
        assert_eq!(checklist.policy_id, AnswerPolicy::default().policy_id);
    }

    #[test]
    fn answered_search_rejects_a_quote_the_retained_text_does_not_contain() {
        let client = client_with_two_sources().with_claim_extractor(ScriptedExtractor::quoting(
            "the company was started by Mallory Attacker",
        ));
        let answered = client
            .search_answered("who started Acme", 5, &QueryFilters::default())
            .unwrap();
        assert_eq!(answered.answer.status, "insufficient_evidence");
        assert!(answered.answer.claims.is_empty());
        assert_eq!(answered.trace.proposed_claims, 1);
        assert_eq!(answered.trace.rejected_quotes, 2);
        assert!(
            answered
                .trace
                .evidence_checklist
                .as_ref()
                .unwrap()
                .critical_missing
        );
    }

    #[test]
    fn answered_search_reports_a_contradicting_source_as_contested() {
        let client = client_with_two_sources()
            .with_claim_extractor(ScriptedExtractor::quoting(SPAN))
            .with_stance_judge(Arc::new(ScriptedJudge {
                contradicting_host: Some("reference.test"),
            }));
        let answered = client
            .search_answered("who started Acme", 5, &QueryFilters::default())
            .unwrap();
        assert_eq!(answered.answer.status, "insufficient_evidence");
        let claim = &answered.answer.claims[0];
        assert_eq!(claim.status, "contested");
        assert!(
            claim
                .citations
                .iter()
                .any(|c| c.evidence_role == "contradicts")
        );
        let contradicting = answered
            .results
            .iter()
            .find(|result| result.url == SECOND_URL)
            .unwrap();
        assert!(contradicting.decision.as_ref().unwrap().conflict > 0.0);
    }

    #[test]
    fn answered_search_sets_aside_a_supported_claim_that_misses_the_request() {
        let client = client_with_two_sources()
            .with_claim_extractor(ScriptedExtractor::quoting(SPAN))
            .with_stance_judge(Arc::new(ScriptedJudge {
                contradicting_host: None,
            }))
            .with_responsiveness_judge(Arc::new(ScriptedResponsiveness { answers: false }));
        let answered = client
            .search_answered("who started Acme Robotics", 5, &QueryFilters::default())
            .unwrap();
        assert_eq!(answered.answer.status, "insufficient_evidence");
        assert_eq!(answered.answer.claims[0].status, "off_request");
        assert_eq!(answered.trace.off_request_claims, 1);
        assert_eq!(
            answered.trace.responsiveness_model.as_deref(),
            Some("scripted-responsiveness")
        );
        // Sources of an off-request claim gain no Stage-2 support.
        assert!(
            answered
                .results
                .iter()
                .all(|result| result.decision.as_ref().unwrap().support == 0.0)
        );
    }

    #[test]
    fn a_substitution_declared_by_the_extractor_keeps_the_claim_out_of_the_answer() {
        let client = client_with_two_sources().with_claim_extractor(Arc::new(ScriptedExtractor {
            quote: SPAN,
            calls: AtomicUsize::new(0),
            fail: false,
            claims: 1,
            substitutes: true,
        }));
        let answered = client
            .search_answered("who started Acme", 5, &QueryFilters::default())
            .unwrap();
        assert_eq!(answered.answer.status, "insufficient_evidence");
        assert_eq!(answered.answer.claims[0].status, "off_request");
        assert_eq!(answered.trace.off_request_claims, 1);
        assert_eq!(answered.trace.unshown_rejections, 0);
    }

    #[test]
    fn answered_search_survives_a_model_failure_and_does_not_cache_it() {
        let extractor = Arc::new(ScriptedExtractor {
            quote: SPAN,
            calls: AtomicUsize::new(0),
            fail: true,
            claims: 1,
            substitutes: false,
        });
        let client = client_with_two_sources().with_claim_extractor(extractor.clone());
        for _ in 0..2 {
            let answered = client
                .search_answered("who started Acme", 5, &QueryFilters::default())
                .unwrap();
            assert_eq!(answered.answer.status, "results_only");
            assert_eq!(answered.results.len(), 2);
            assert!(
                answered
                    .trace
                    .model_error
                    .as_deref()
                    .unwrap()
                    .contains("model unavailable")
            );
            assert!(!answered.trace.fresh_answer_cache_hit);
        }
        assert_eq!(extractor.calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn identical_answered_search_reuses_the_cached_decision() {
        let extractor = ScriptedExtractor::quoting(SPAN);
        let client = client_with_two_sources()
            .with_claim_extractor(extractor.clone())
            .with_stance_judge(Arc::new(ScriptedJudge {
                contradicting_host: None,
            }));
        let first = client
            .search_answered("who started Acme", 5, &QueryFilters::default())
            .unwrap();
        let second = client
            .search_answered("who started Acme", 5, &QueryFilters::default())
            .unwrap();
        assert!(!first.trace.fresh_answer_cache_hit);
        assert!(second.trace.fresh_answer_cache_hit);
        assert_eq!(second.answer.status, "extractive");
        assert!(second.trace.evidence_checklist.is_some());
        assert_eq!(extractor.calls.load(Ordering::SeqCst), 1);

        // A different request is a different key: nothing is rewritten.
        client
            .search_answered("who started Acme?", 5, &QueryFilters::default())
            .unwrap();
        assert_eq!(extractor.calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn answered_search_calls_no_model_when_there_is_nothing_to_read() {
        let extractor = ScriptedExtractor::quoting(SPAN);
        let client =
            OpenSonarXClient::with_embedder(SdkConfig::default(), Arc::new(MockEmbedder::new(16)))
                .with_search_engines(Vec::new())
                .with_claim_extractor(extractor.clone());
        let answered = client
            .search_answered("who started Acme", 5, &QueryFilters::default())
            .unwrap();
        assert!(answered.results.is_empty());
        assert_eq!(answered.answer.status, "results_only");
        assert_eq!(answered.trace.extractor_model, None);
        assert_eq!(extractor.calls.load(Ordering::SeqCst), 0);
    }

    /// Supports every passage and records how many calls overlapped.
    struct OverlapJudge {
        active: AtomicUsize,
        peak: AtomicUsize,
        calls: AtomicUsize,
    }

    impl StanceJudge for OverlapJudge {
        fn judge(&self, _claim: &str, passages: &[EvidencePassage]) -> Result<Vec<StanceJudgment>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(active, Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(100));
            self.active.fetch_sub(1, Ordering::SeqCst);
            Ok(passages
                .iter()
                .map(|passage| StanceJudgment {
                    passage_id: passage.id.clone(),
                    label: StanceLabel::Supports,
                    quote: Some(SPAN.to_string()),
                })
                .collect())
        }

        fn model_version(&self) -> String {
            "overlap-judge".to_string()
        }
    }

    #[test]
    fn stance_for_several_claims_is_judged_concurrently() {
        let judge = Arc::new(OverlapJudge {
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            calls: AtomicUsize::new(0),
        });
        let client = client_with_two_sources()
            .with_claim_extractor(Arc::new(ScriptedExtractor {
                quote: SPAN,
                calls: AtomicUsize::new(0),
                fail: false,
                claims: 3,
                substitutes: false,
            }))
            .with_stance_judge(judge.clone());
        let answered = client
            .search_answered("who started Acme", 5, &QueryFilters::default())
            .unwrap();
        assert_eq!(answered.answer.claims.len(), 3);
        assert!(
            answered
                .answer
                .claims
                .iter()
                .all(|claim| claim.status == "corroborated")
        );
        assert_eq!(judge.calls.load(Ordering::SeqCst), 3);
        assert_eq!(judge.peak.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn no_stance_call_is_made_without_retained_text() {
        struct NeverJudge;
        impl StanceJudge for NeverJudge {
            fn judge(&self, _: &str, _: &[EvidencePassage]) -> Result<Vec<StanceJudgment>> {
                panic!("judge must not be called without retained passages")
            }
            fn model_version(&self) -> String {
                "never".to_string()
            }
        }
        let judged = super::judge_claims(&NeverJudge, &["a claim", "another claim"], &[]).unwrap();
        assert_eq!(judged.len(), 2);
        assert!(judged.iter().all(Vec::is_empty));
    }

    #[test]
    fn blended_search_is_identical_for_requests_that_differ_only_in_wording() {
        // No request is special-cased: the slate depends on the index and the
        // embedder alone, and source order is the retrieval order.
        let client = client_with_two_sources();
        let (results, sources) = client
            .search_blended("who started Acme", 5, &QueryFilters::default())
            .unwrap();
        assert_eq!(sources, vec!["local".to_string()]);
        assert_eq!(results.len(), 2);
        assert!(results[0].final_score >= results[1].final_score);
        assert!(results.iter().all(|result| result.decision.is_some()));
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

        let embedding = client
            .embedder()
            .embed_text("rust systems programming language")
            .unwrap();
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
                embedding: Some(
                    client
                        .embedder()
                        .embed_text("python data science machine learning")
                        .unwrap(),
                ),
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

        let embedding = client
            .embedder()
            .embed_text("rust systems programming")
            .unwrap();
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

        let mut client = OpenSonarXClient::new(SdkConfig::default());
        // This is a deterministic local-index benchmark, not a live SERP
        // integration test. Live engines are exercised by `oi-bench
        // live-search`; keeping them here made the assertion depend on network
        // reachability and on whatever the public engine returned that day.
        client.search_engines.clear();

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
        eprintln!(
            "\n[INGEST] {} documents in {:?}",
            chunks.len(),
            ingest_elapsed
        );

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
        eprintln!(
            "[STAKE]  {} domains staked in {:?}",
            domains_to_stake.len(),
            stake_elapsed
        );

        // Verify stakes
        let stakes = client.list_stakes().unwrap();
        assert_eq!(stakes.len(), domains_to_stake.len());
        eprintln!("[STAKE]  Active stakes: {}", stakes.len());
        for s in &stakes {
            eprintln!(
                "         {} — ${:.2} ({} lamports)",
                s.domain, s.amount_usd, s.amount_lamports
            );
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
        eprintln!(
            "  Total time:     {total_local_us}us ({:.3}ms)",
            total_local_us as f64 / 1000.0
        );
        eprintln!(
            "  Average/query:  {avg_local_us:.1}us ({:.3}ms)",
            avg_local_us / 1000.0
        );

        // ── search_with_fallback should NOT hit external engines ─────────
        eprintln!("\n--- search_with_fallback (should return local, no external call) ---");
        let start = Instant::now();
        let results = client
            .search_with_fallback(
                "decentralized search protocol staking",
                5,
                &QueryFilters::default(),
            )
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
        assert!(
            !top.is_empty(),
            "leaderboard should have entries after queries"
        );

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
        use crate::engines::duckduckgo::DuckDuckGo;
        use std::time::Instant;

        let client = OpenSonarXClient::new(SdkConfig::default());

        let articles = test_articles();
        let chunks = build_chunks(&articles, client.embedder());
        client.ingest(&chunks).unwrap();

        for (domain, _url, _title, _content) in &articles {
            client.stake(domain, 25_000_000).unwrap();
        }

        let query = "Rust programming language";

        eprintln!("\n{}", "=".repeat(70));
        eprintln!(
            "  COMPARISON: Local Index ({}) vs DuckDuckGo",
            client.embedder().name()
        );
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
        use crate::engines::duckduckgo::DuckDuckGo;
        use oi_embed::MiniLmEmbedder;
        use std::time::Instant;

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
        eprintln!(
            "[MODEL] Loaded in {:?} (dim={})",
            model_elapsed,
            embedder.dimension()
        );

        let config = SdkConfig::default();
        let client = OpenSonarXClient::with_embedder(config, embedder);

        // ── Ingest ──────────────────────────────────────────────────────
        let articles = test_articles();

        let ingest_start = Instant::now();
        let chunks = build_chunks(&articles, client.embedder());
        let embed_elapsed = ingest_start.elapsed();
        eprintln!(
            "[EMBED] {} documents embedded in {:?}",
            chunks.len(),
            embed_elapsed
        );

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
                    r.rank, r.semantic_score, r.bm25_score, r.final_score, r.stake_usd, r.url
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
        eprintln!(
            "  Model:           {} (384-dim, INT8 quantized)",
            client.embedder().name()
        );
        eprintln!("  Model load:      {:?}", model_elapsed);
        eprintln!("  Documents:       {}", articles.len());
        eprintln!("  Queries:         {}", queries.len());
        eprintln!(
            "  Avg embed:       {avg_embed:.0}us ({:.3}ms)",
            avg_embed / 1000.0
        );
        eprintln!(
            "  Avg search:      {avg_total:.0}us ({:.3}ms) (embed + index)",
            avg_total / 1000.0
        );
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
        eprintln!(
            "\n--- LOCAL INDEX ({} + BM25) ---",
            client.embedder().name()
        );
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
        eprintln!(
            "  Time: {:?} | Embedder: {}",
            elapsed,
            client.embedder().name()
        );
    }

    /// Head-to-head: OpenSonarX (MiniLM) vs DuckDuckGo vs Serper (Google)
    ///
    /// Run with:
    ///   SERPER_API_KEY=<key> cargo test -p oi-sdk --features onnx test_opensonarx_vs_ddg_vs_serper -- --nocapture --ignored
    #[cfg(feature = "onnx")]
    #[test]
    #[ignore] // requires network + SERPER_API_KEY
    fn test_opensonarx_vs_ddg_vs_serper() {
        use crate::engines::duckduckgo::DuckDuckGo;
        use crate::engines::serper::Serper;
        use oi_embed::MiniLmEmbedder;
        use std::time::Instant;

        let serper_key =
            std::env::var("SERPER_API_KEY").expect("Set SERPER_API_KEY env var to run this test");

        // ── Setup ───────────────────────────────────────────────────────
        let embedder = Arc::new(MiniLmEmbedder::new().expect("Failed to load MiniLM model"));
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
        eprintln!(
            "  Model: {} | Index: {} docs | Stakes: {} domains",
            client.embedder().name(),
            articles.len(),
            domains_to_stake.len()
        );
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

            eprintln!(
                "\n  OPENINDEX ({:.2}ms):",
                oi_elapsed.as_micros() as f64 / 1000.0
            );
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
            "OPENINDEX",
            oi_avg / 1000.0,
            oi_total_us as f64 / 1000.0
        );
        eprintln!(
            "  {:.<20} avg {:.0}ms   total {:.0}ms",
            "DUCKDUCKGO",
            ddg_avg / 1000.0,
            ddg_total_us as f64 / 1000.0
        );
        eprintln!(
            "  {:.<20} avg {:.0}ms   total {:.0}ms",
            "SERPER/GOOGLE",
            serper_avg / 1000.0,
            serper_total_us as f64 / 1000.0
        );
        eprintln!("{thin}");
        eprintln!(
            "  OpenSonarX is ~{:.0}x faster than DDG, ~{:.0}x faster than Serper",
            ddg_avg / oi_avg.max(1.0),
            serper_avg / oi_avg.max(1.0),
        );
        eprintln!(
            "\n  NOTE: OpenSonarX searches a local index of {} curated, staked documents.",
            articles.len()
        );
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
                decision: None,
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
                decision: None,
            })
            .collect();

        let merged = super::merge_results(
            local,
            external,
            "example",
            10,
            &registry,
            &query_emb,
            embedder.as_ref(),
        );

        // Count per domain
        let mut domain_counts: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        for r in &merged {
            *domain_counts
                .entry(super::extract_domain(&r.url))
                .or_insert(0) += 1;
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
        let results = client
            .search("technology", 5, &QueryFilters::default())
            .unwrap();
        assert!(
            results.is_empty(),
            "off-topic chunk should have been dropped"
        );
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
        let results = client
            .search("technology", 5, &QueryFilters::default())
            .unwrap();
        assert!(
            !results.is_empty(),
            "matching chunk should have been ingested"
        );
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
        let results = client
            .search("anything", 5, &QueryFilters::default())
            .unwrap();
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
        eprintln!(
            "  Embedder: {} (dim={})",
            client.embedder().name(),
            client.embedder().dimension()
        );
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
            (
                "memory safety",
                "Should surface: dangling pointers, double frees, borrow checker",
            ),
            (
                "staking rewards",
                "Should surface: stake, staking rewards, cooldown",
            ),
            (
                "decentralized search",
                "Should surface: TRUTH tokens, HNSW, disputes, libp2p",
            ),
            (
                "quantum error correction",
                "Should surface: qubits, error correction, superposition",
            ),
            (
                "dense retrieval embeddings",
                "Should surface: cosine similarity, contrastive learning, 384-dim",
            ),
            (
                "token staking protocol",
                "Should surface: stake TRUTH tokens, disputes, malicious publishers",
            ),
            (
                "Rust ownership borrow checker",
                "Should surface: ownership model, borrow checker, compile time",
            ),
            (
                "vector search BM25 hybrid",
                "Should surface: HNSW vector similarity, BM25, hybrid search",
            ),
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
                eprintln!("\n  #{} {} (score={:.4})", r.rank, r.url, r.final_score);
                eprintln!("     title: {}", r.title);

                // Compute per-sentence similarity scores for diagnosis
                let sentences = oi_index::relevance::split_into_sentences(&r.content_md);
                let sent_texts: Vec<&str> = sentences.iter().map(|(t, _, _)| t.as_str()).collect();
                if let Ok(embs) = client.embedder().embed_text_batch(&sent_texts) {
                    eprintln!("     per-sentence cosine sims:");
                    for (i, ((text, _, _), emb)) in sentences.iter().zip(embs.iter()).enumerate() {
                        let sim: f32 = {
                            let dot: f32 =
                                query_embedding.iter().zip(emb).map(|(a, b)| a * b).sum();
                            let na: f32 = query_embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
                            let nb: f32 = emb.iter().map(|x| x * x).sum::<f32>().sqrt();
                            if na == 0.0 || nb == 0.0 {
                                0.0
                            } else {
                                dot / (na * nb)
                            }
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
        eprintln!(
            "  Embedder: {} (dim={})",
            client.embedder().name(),
            client.embedder().dimension()
        );
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
            let mut results = client
                .index()
                .query(&query_embedding, query, &QueryFilters::default(), 5)
                .unwrap();
            let index_us = t1.elapsed().as_micros();

            // Count sentences in results
            let sent_count: usize = results
                .iter()
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

            let enriched_count = results
                .iter()
                .filter(|r| r.content_relevant.is_some())
                .count();

            eprintln!(
                "  q=\"{}\"\n    embed={:>5}us  index={:>4}us  enrich={:>5}us  total_search={:>5}us  sents={}  enriched={}/{}",
                query,
                embed_us,
                index_us,
                enrich_us,
                search_us,
                sent_count,
                enriched_count,
                results.len()
            );
        }

        let n = queries.len() as f64;
        let avg_embed = total_embed_us as f64 / n;
        let avg_index = total_index_us as f64 / n;
        let avg_enrich = total_enrich_us as f64 / n;
        let avg_search = total_search_us as f64 / n;
        let avg_sents = total_sentences as f64 / n;

        eprintln!("\n{}", "-".repeat(70));
        eprintln!(
            "  AVERAGES ({} queries, {:.0} sentences/query):",
            queries.len(),
            avg_sents
        );
        eprintln!(
            "    embed_query:           {:>7.0}us ({:.2}ms)",
            avg_embed,
            avg_embed / 1000.0
        );
        eprintln!(
            "    index.query:           {:>7.0}us ({:.2}ms)",
            avg_index,
            avg_index / 1000.0
        );
        eprintln!(
            "    enrich_content_relevant:{:>6.0}us ({:.2}ms)  ← NEW",
            avg_enrich,
            avg_enrich / 1000.0
        );
        eprintln!(
            "    full search():         {:>7.0}us ({:.2}ms)",
            avg_search,
            avg_search / 1000.0
        );
        eprintln!(
            "\n    enrichment overhead: {:.1}% of total search time",
            avg_enrich / avg_search * 100.0
        );
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
        eprintln!(
            "  Embedder: {} (dim={})",
            client.embedder().name(),
            client.embedder().dimension()
        );
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
                        if na == 0.0 || nb == 0.0 {
                            0.0
                        } else {
                            dot / (na * nb)
                        }
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
