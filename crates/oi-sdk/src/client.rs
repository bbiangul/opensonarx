use crate::config::SdkConfig;
use crate::evidence::{apply_decision_policy, apply_decision_policy_with_registry};
use crate::leaderboard::InMemoryLeaderboard;
use crate::navigation_cache::NavigationCache;
use crate::query_intent::{
    QueryIntent, apply_intent_adjustments, classify_query, current_query_allows_undated_display,
    diligence_candidate_matches_context, diligence_context_queries, diligence_context_terms,
    entity_words, exact_entity_host_match, external_discovery_queries,
    factual_answerability_coverage, is_current_office_holder_query, is_live_value_query,
    is_poll_query, is_recommendation_query, is_reference_exploration_query,
    local_coverage_satisfies_intent, navigation_domain_guesses, navigation_entity_host_match,
    navigation_entity_key, navigation_title_supports_entity, official_registry_entry_for_query,
    official_state_source_urls, prefers_global_english_news, primary_source_url_matches_claim,
    primary_source_url_matches_query, retain_answered_serving_scope, retain_intent_scope,
    source_family, suspicious_transaction_path, transaction_destination_key,
    transaction_destination_matches_query, transaction_route_paths,
};
use crate::wallet::Wallet;
use oi_index::{HybridIndex, Storage};
use oi_staking::InMemoryStakeRegistry;
use oi_types::traits::{SearchIndex, StakeRegistry};
use oi_types::{
    ContentChunk, EmbeddingPipeline, QueryFilters, QueryLeaderboard, Result, SearchEngine,
    SearchParams, SearchResultItem, StakeInfo, TopicRouter,
};
use std::sync::Arc;
use ureq::ResponseExt;

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
    /// A bounded factual-reference corpus used only when general web
    /// discovery cannot provide two relation-specific search leads. It is
    /// intentionally separate so it never counts as web-provider redundancy.
    factual_reference_engines: Vec<Arc<dyn SearchEngine>>,
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
    /// Coalesce equivalent claim-acquisition workloads before URL selection.
    /// URL-level flights are insufficient for concurrent paraphrases because
    /// each variant can rank and crawl a different five-page slice.
    answer_flights: std::sync::Mutex<std::collections::HashMap<String, Arc<CrawlFlight>>>,
    /// Very short-lived, process-local reuse of a completed claim decision.
    /// This absorbs bursty paraphrases after the leader has already left its
    /// in-flight slot. Only positive poll answers and diligence slates with
    /// retained results are eligible; failed/empty acquisition is retried.
    answer_cache: std::sync::Mutex<std::collections::HashMap<String, CachedAnsweredSearch>>,
    /// Canonical first-party destinations discovered externally. This is
    /// metadata-only and is never used as factual evidence.
    navigation_cache: std::sync::Mutex<NavigationCache>,
    /// Coalesce bounded first-party action-route probes across equivalent
    /// concurrent queries (for example, login/entrar/acessar Claude).
    transaction_flights: std::sync::Mutex<std::collections::HashMap<String, Arc<CrawlFlight>>>,
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
        let factual_reference_engines = crate::engines::factual_reference_engines(&config);
        let leaderboard: Arc<dyn QueryLeaderboard> = Arc::new(InMemoryLeaderboard::new());

        let config_creator_ledger_path = config.creator_ledger_path.clone();
        let navigation_cache = NavigationCache::load(config.local_index_path.as_deref());
        Self {
            config,
            index,
            stake_registry,
            embedder,
            wallet,
            storage,
            search_engines,
            factual_reference_engines,
            leaderboard,
            topic_router: None,
            shard_filter: Vec::new(),
            crawl_queue: std::sync::Mutex::new(Vec::new()),
            crawl_persist: std::sync::Mutex::new(()),
            crawl_flights: std::sync::Mutex::new(std::collections::HashMap::new()),
            answer_flights: std::sync::Mutex::new(std::collections::HashMap::new()),
            answer_cache: std::sync::Mutex::new(std::collections::HashMap::new()),
            navigation_cache: std::sync::Mutex::new(navigation_cache),
            transaction_flights: std::sync::Mutex::new(std::collections::HashMap::new()),
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
        let factual_reference_engines = crate::engines::factual_reference_engines(&config);
        let leaderboard: Arc<dyn QueryLeaderboard> = Arc::new(InMemoryLeaderboard::new());
        let config_creator_ledger_path = config.creator_ledger_path.clone();
        let navigation_cache = NavigationCache::load(config.local_index_path.as_deref());
        Self {
            config,
            index,
            stake_registry,
            embedder,
            wallet,
            storage: None,
            search_engines,
            factual_reference_engines,
            leaderboard,
            topic_router: None,
            shard_filter: Vec::new(),
            crawl_queue: std::sync::Mutex::new(Vec::new()),
            crawl_persist: std::sync::Mutex::new(()),
            crawl_flights: std::sync::Mutex::new(std::collections::HashMap::new()),
            answer_flights: std::sync::Mutex::new(std::collections::HashMap::new()),
            answer_cache: std::sync::Mutex::new(std::collections::HashMap::new()),
            navigation_cache: std::sync::Mutex::new(navigation_cache),
            transaction_flights: std::sync::Mutex::new(std::collections::HashMap::new()),
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
        // A caller replacing engines expects a closed, deterministic provider
        // set (especially tests and embedded deployments).
        self.factual_reference_engines.clear();
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
        let original_query = query;
        let corrected = self.correct_query(query);
        let retrieval_query = corrected.as_deref().unwrap_or(query);

        // Compute query embedding once — used for both local search and external re-scoring
        let query_embedding = self.embedder.embed_query(retrieval_query)?;
        // Retrieval depth and presentation depth are intentionally separate.
        // Asking for five displayed results must not mean that only five local
        // candidates survive to the shared-scale and intent-aware rerank.
        let candidate_k = retrieval_candidate_depth(top_k);
        let mut local_results =
            self.index
                .query(&query_embedding, retrieval_query, filters, candidate_k)?;
        let mut sources_used = Vec::new();

        // Query-relative authority entrypoints are bounded registry joins, so
        // do not let a growing global candidate window hide an already-crawled
        // official page and trigger an unnecessary provider round trip. Reuse
        // the embedding already computed for this request; the normal shared-
        // scale scoring and intent gates below still decide presentation.
        let registered_entrypoints = official_state_source_urls(original_query);
        if !registered_entrypoints.is_empty() {
            let exact_authority = self.index.query_exact_urls(
                &query_embedding,
                retrieval_query,
                &registered_entrypoints,
                registered_entrypoints.len().max(1),
            )?;
            if !exact_authority.is_empty() {
                prefer_exact_evidence_for_same_urls(&mut local_results, &exact_authority);
                local_results.extend(exact_authority);
            }
        }

        // Spell correction is a retrieval aid only. Intent, entity identity,
        // lexical ranking, and navigation-cache keys must remain owned by the
        // user's original query; a corpus-specific correction must never turn
        // "site oficial chatgpt" into another destination namespace.
        let original_intent = classify_query(original_query);
        if matches!(
            original_intent,
            QueryIntent::Navigational | QueryIntent::Transactional
        ) {
            let key = if original_intent == QueryIntent::Transactional {
                transaction_destination_key(original_query).unwrap_or_default()
            } else {
                navigation_entity_key(original_query)
            };
            let cached = self
                .navigation_cache
                .lock()
                .ok()
                .and_then(|cache| cache.get(&key))
                // Cache entries are discovery metadata, not permanent truth.
                // Re-evaluate them against the current canonical-destination
                // policy so an old subdomain decision cannot poison resident
                // ranking after the policy is tightened.
                .filter(|cached| {
                    local_coverage_satisfies_intent(original_query, std::slice::from_ref(cached))
                });
            if let Some(cached) = cached
                && !local_results.iter().any(|result| result.url == cached.url)
            {
                local_results.push(cached);
                sources_used.push("navigation_cache".to_string());
            }
            if original_intent == QueryIntent::Navigational {
                let derived = local_results.iter().find_map(|result| {
                    canonical_entity_root_from_subdomain(original_query, &result.url).map(|root| {
                        let mut destination = result.clone();
                        destination.result_id = format!("navigation-derived:{key}");
                        destination.url = root;
                        destination.content_md.clear();
                        destination.content_relevant = None;
                        destination.extraction_quality = "discovery_metadata".to_string();
                        destination.source = "navigation_derived".to_string();
                        destination.crawled_at = None;
                        destination.published_at = None;
                        destination.facts = None;
                        destination.decision = None;
                        destination
                    })
                });
                if let Some(derived) = derived
                    && !local_results.iter().any(|result| result.url == derived.url)
                {
                    local_results.push(derived);
                    if !sources_used
                        .iter()
                        .any(|source| source == "navigation_derived")
                    {
                        sources_used.push("navigation_derived".to_string());
                    }
                }
            }
        }

        // A root page can truthfully expose an embedded login control, but a
        // verified conventional first-party route is a better transactional
        // destination. Probe only origins already tied to the requested
        // entity, coalesce equivalent concurrent queries, and retain the
        // result as metadata rather than factual evidence.
        if original_intent == QueryIntent::Transactional
            && !self.search_engines.is_empty()
            && filters.min_trust.is_none()
            && let Some(destination) =
                self.resolve_transaction_destination(original_query, &local_results)
            && !local_results
                .iter()
                .any(|result| result.url == destination.url)
        {
            local_results.push(destination);
            sources_used.push("transaction_route".to_string());
        }
        for result in local_results.iter_mut().filter(|result| {
            result.extraction_quality == "verified_transaction_route"
                && result.semantic_score == 0.0
        }) {
            let surface = format!("{} {}", result.title, result.snippet);
            if let Ok(destination_embedding) = self.embedder.embed_text(&surface) {
                result.semantic_score =
                    cosine_similarity(&query_embedding, &destination_embedding) as f64;
            }
        }

        if !local_results.is_empty() {
            if local_results.iter().any(|result| result.source == "local") {
                sources_used.push("local".to_string());
            }
        }

        // A min_trust floor means "vetted results only": external engine
        // results are unvetted by definition (verdict "external"), so the
        // blend is local-only — the index has already enforced the floor.
        // Also the no-engines backward-compat path.
        if self.search_engines.is_empty() || filters.min_trust.is_some() {
            let mut local_results = local_results;
            let n = local_results.len();
            apply_blend_scores(
                &mut local_results,
                n,
                original_query,
                &query_embedding,
                self.embedder.as_ref(),
            );
            apply_intent_adjustments(original_query, &mut local_results);
            retain_intent_scope(original_query, &mut local_results);
            local_results.sort_by(|a, b| b.final_score.total_cmp(&a.final_score));
            local_results.truncate(top_k);
            for (i, r) in local_results.iter_mut().enumerate() {
                r.rank = (i + 1) as u32;
            }
            if classify_query(original_query) == QueryIntent::Exploratory {
                self.enrich_content_relevant(&mut local_results, &query_embedding);
            }
            apply_decision_policy(original_query, &mut local_results);
            if record_usage {
                self.record_leaderboard(&local_results);
                self.attribute_usage(&local_results);
            }
            return Ok((local_results, sources_used));
        }

        // Local-coverage fast path: when the index already answers the query
        // well, skip the ~1s external engine round-trip entirely. Coverage is
        // judged on the same absolute blend score the merge would compute, so
        // this changes latency, not ranking. Threshold: a full top_k where even
        // the weakest hit is solidly on-topic (absolute cosine blend >= 0.60).
        if !local_results.is_empty() {
            let mut local_results = local_results;
            let n = local_results.len();
            apply_blend_scores(
                &mut local_results,
                n,
                original_query,
                &query_embedding,
                self.embedder.as_ref(),
            );
            apply_intent_adjustments(original_query, &mut local_results);
            retain_intent_scope(original_query, &mut local_results);
            local_results.sort_by(|a, b| b.final_score.total_cmp(&a.final_score));
            let inspected = top_k.min(local_results.len());
            let intent = classify_query(original_query);
            let intent_coverage = if intent == QueryIntent::Factual {
                factual_local_coverage(original_query, &local_results[..inspected])
            } else {
                local_coverage_satisfies_intent(original_query, &local_results[..inspected])
            };
            let full_strong_coverage = local_results.len() >= top_k
                && local_results
                    .first()
                    .is_some_and(|r| r.final_score >= LOCAL_COVERAGE_TOP1)
                && local_results
                    .get(top_k - 1)
                    .is_some_and(|r| r.final_score >= LOCAL_COVERAGE_KTH)
                && intent_coverage;
            // A resolved navigational destination is useful even if a young
            // local index has fewer than `top_k` secondary links. Do not pay a
            // multi-second web round trip merely to pad the slate.
            let partial_navigational_coverage =
                intent == QueryIntent::Navigational && !local_results.is_empty() && intent_coverage;
            // An action query with a resolved first-party destination should
            // not pay an external round trip merely to pad secondary links.
            let partial_transactional_coverage = intent == QueryIntent::Transactional
                && !local_results.is_empty()
                && local_results
                    .first()
                    .is_some_and(|r| r.final_score >= LOCAL_COVERAGE_TOP1)
                && intent_coverage;
            // A category-level tool query needs alternatives, not merely one
            // entity destination. Three retained functional domains are the
            // minimum resident slate used by the live quality contract.
            let tool_depth = top_k.min(3);
            let partial_tool_coverage = intent == QueryIntent::Tool
                && local_results.len() >= tool_depth
                && local_results
                    .first()
                    .is_some_and(|r| r.final_score >= LOCAL_COVERAGE_TOP1)
                && local_results
                    .get(tool_depth - 1)
                    .is_some_and(|r| r.final_score >= LOCAL_COVERAGE_KTH)
                && intent_coverage;
            // For factual queries, three retained, independently sourced,
            // answer-bearing pages are stronger coverage than five weakly
            // related links. This is the warm evidence path, not padding.
            let factual_depth = top_k.min(3);
            let factual_bounded_abstention = if intent == QueryIntent::Factual
                // This branch never emits a factual claim: it only preserves
                // an explicit, evidence-backed abstention when a retained
                // founder biography says the enumeration is partial. One
                // recent family is enough to know that returning the observed
                // names as a complete list would be unsafe. Requiring two
                // made every process restart repeat a provider+crawl cascade
                // even though the only honest answer remained "incomplete".
                && recently_crawled_source_families(
                    &local_results[..inspected],
                    FACTUAL_REVALIDATION_TTL_SECS,
                ) >= 1
            {
                let local_answer = crate::answer::construct_answer(
                    original_query,
                    QueryIntent::Factual,
                    &local_results[..inspected],
                );
                crate::answer::factual_answer_is_bounded_abstention(
                    original_query,
                    &local_results[..inspected],
                    &local_answer,
                )
            } else {
                false
            };
            let partial_factual_coverage = factual_bounded_abstention
                || (intent == QueryIntent::Factual
                    && local_results.len() >= factual_depth
                    && local_results
                        .first()
                        .is_some_and(|r| r.final_score >= LOCAL_COVERAGE_TOP1)
                    && local_results
                        .get(factual_depth - 1)
                        .is_some_and(|r| r.final_score >= LOCAL_COVERAGE_KTH)
                    && intent_coverage);
            // A poll answer already requires two fresh editorial families
            // with a compatible retained numeric claim. Requiring a third
            // high-scoring page here forced a live provider round-trip even
            // during the five-minute revalidation window. Generic news keeps
            // the three-result depth because it has no equivalent claim gate.
            let current_depth = if is_poll_query(original_query) {
                top_k.min(2)
            } else {
                top_k.min(3)
            };
            let current_claim_coverage = !is_poll_query(original_query)
                || crate::answer::construct_answer(
                    original_query,
                    QueryIntent::CurrentEvents,
                    &local_results[..inspected],
                )
                .status
                    == "extractive";
            let partial_current_coverage = intent == QueryIntent::CurrentEvents
                && local_results.len() >= current_depth
                && intent_coverage
                && current_claim_coverage;
            // A versioned query-relative authority entrypoint is a complete
            // resident destination for high-stakes guidance. External search
            // would only pad the slate; the answered path below independently
            // checks its exact URL/content TTL and refetches it when stale.
            let partial_registered_authority_coverage = intent == QueryIntent::Exploratory
                && !official_state_source_urls(original_query).is_empty()
                && local_results.iter().any(|result| {
                    result.source == "local"
                        && !result.content_md.trim().is_empty()
                        && result.final_score >= LOCAL_COVERAGE_KTH
                        && official_registry_entry_for_query(original_query, &result.url)
                });
            let strong_coverage = full_strong_coverage
                || partial_navigational_coverage
                || partial_transactional_coverage
                || partial_tool_coverage
                || partial_factual_coverage
                || partial_current_coverage
                || partial_registered_authority_coverage;
            if strong_coverage {
                if (partial_navigational_coverage
                    || partial_transactional_coverage
                    || partial_tool_coverage
                    || (partial_factual_coverage && !factual_bounded_abstention))
                    && !full_strong_coverage
                {
                    local_results.retain(|result| result.final_score >= LOCAL_COVERAGE_KTH);
                }
                local_results.truncate(top_k);
                for (i, r) in local_results.iter_mut().enumerate() {
                    r.rank = (i + 1) as u32;
                }
                if intent == QueryIntent::Exploratory {
                    self.enrich_content_relevant(&mut local_results, &query_embedding);
                }
                apply_decision_policy(original_query, &mut local_results);
                if record_usage {
                    self.record_leaderboard(&local_results);
                    self.attribute_usage(&local_results);
                }
                return Ok((local_results, sources_used));
            }
            // Weak coverage — fall through to the external blend. Blend scores
            // are recomputed in merge (the embedder cache makes that free).
            return self.blend_with_external(
                original_query,
                top_k,
                filters,
                local_results,
                query_embedding,
                sources_used,
                record_usage,
            );
        }

        self.blend_with_external(
            original_query,
            top_k,
            filters,
            local_results,
            query_embedding,
            sources_used,
            record_usage,
        )
    }

    /// Re-query only retained content after a synchronous crawl batch.
    ///
    /// External engines discover URLs; they do not need to be called again
    /// merely because those URLs were just hydrated into the local index. A
    /// local-only refresh avoids repeated provider deadlines and guarantees
    /// that answer construction sees retained content rather than SERP text.
    fn search_retained_evidence(
        &self,
        query: &str,
        top_k: usize,
        filters: &QueryFilters,
    ) -> Result<(Vec<SearchResultItem>, Vec<String>)> {
        if top_k == 0 {
            return Ok((Vec::new(), Vec::new()));
        }
        let corrected = self.correct_query(query);
        let retrieval_query = corrected.as_deref().unwrap_or(query);
        let query_embedding = self.embedder.embed_query(retrieval_query)?;
        let candidate_k = retrieval_candidate_depth(top_k);
        let mut results =
            self.index
                .query(&query_embedding, retrieval_query, filters, candidate_k)?;
        let local_count = results.len();
        apply_blend_scores(
            &mut results,
            local_count,
            query,
            &query_embedding,
            self.embedder.as_ref(),
        );
        apply_intent_adjustments(query, &mut results);
        retain_intent_scope(query, &mut results);
        results.sort_by(|left, right| right.final_score.total_cmp(&left.final_score));
        results.truncate(top_k);
        for (index, result) in results.iter_mut().enumerate() {
            result.rank = (index + 1) as u32;
        }
        apply_decision_policy(query, &mut results);
        let sources = (!results.is_empty())
            .then(|| vec!["local".to_string()])
            .unwrap_or_default();
        Ok((results, sources))
    }

    /// Search plus conservative answer construction.
    ///
    /// Claim-bearing queries hydrate the crawl queue before the final decision
    /// pass so a cold web snippet cannot be presented as retained evidence.
    /// Navigational and transactional queries keep the low-latency path. Usage
    /// is attributed exactly once, to the final slate.
    pub fn search_answered(
        &self,
        query: &str,
        top_k: usize,
        filters: &QueryFilters,
    ) -> Result<crate::answer::AnsweredSearch> {
        let total_started = std::time::Instant::now();
        let intent = classify_query(query);
        if top_k == 0 {
            let answer_started = std::time::Instant::now();
            let answer = crate::answer::construct_answer(query, intent, &[]);
            let checklist = crate::checklist::evaluate(query, &answer, &[]);
            return Ok(crate::answer::AnsweredSearch {
                query: query.to_string(),
                answer,
                results: Vec::new(),
                sources_used: Vec::new(),
                search_passes: 0,
                hydrated_pages: 0,
                poll_registry: Vec::new(),
                market_quotes: Vec::new(),
                trace: crate::answer::AnswerTrace {
                    evidence_checklist: Some(checklist),
                    answer_construction_ms: answer_started.elapsed().as_secs_f64() * 1_000.0,
                    total_ms: total_started.elapsed().as_secs_f64() * 1_000.0,
                    ..Default::default()
                },
            });
        }
        if let Some(canonical_query) = canonical_equivalent_query(query, intent)
            && query != canonical_query
        {
            // Semantically identical multilingual/paraphrased requests share
            // the same production retrieval path. The original wording is
            // restored in the response; evidence policy is owned by the
            // canonical intent, not by whichever lexical variant happened to
            // reach a keyless provider first.
            let mut answered = self.search_answered(canonical_query, top_k, filters)?;
            answered.query = query.to_string();
            return Ok(answered);
        }
        if let Some(canonical_query) = canonical_answer_query(query, intent)
            && query != canonical_query
        {
            // These broad Brazilian presidential-poll phrasings have the same
            // jurisdiction, office, time intent, and requested scenario set.
            // Route them through one canonical evidence decision so a cold
            // concurrent burst cannot re-rank the same retained pages into
            // separate acquisition cascades. Explicit first/second-round
            // queries are deliberately excluded below.
            let mut answered = self.search_answered(canonical_query, top_k, filters)?;
            answered.query = query.to_string();
            return Ok(answered);
        }
        let answer_cache_key = answered_search_cache_key(query, intent, top_k, filters);
        if let Some(cache_key) = answer_cache_key.as_deref()
            && let Some(mut answered) = self.cached_answered_search(cache_key)
        {
            answered.query = query.to_string();
            answered.search_passes = 0;
            answered.hydrated_pages = 0;
            answered.trace = crate::answer::AnswerTrace {
                evidence_checklist: answered.trace.evidence_checklist.clone(),
                total_ms: total_started.elapsed().as_secs_f64() * 1_000.0,
                fresh_answer_cache_hit: true,
                ..Default::default()
            };
            self.record_leaderboard(&answered.results);
            self.attribute_usage(&answered.results);
            return Ok(answered);
        }
        let _answer_flight_guard = if let Some(key) = answer_acquisition_key(query, intent) {
            match self.enter_answer_flight(key) {
                AnswerFlightEntry::Leader(guard) => Some(guard),
                AnswerFlightEntry::Follower(flight) => {
                    self.wait_for_answer_flight(&flight);
                    // The leader has completed its bounded production path
                    // and published any retained evidence to the shared
                    // index. Re-enter with the original wording so ranking,
                    // scope, and answer semantics remain query-specific.
                    return self.search_answered(query, top_k, filters);
                }
            }
        } else {
            None
        };
        let claim_bearing = matches!(
            intent,
            QueryIntent::DueDiligence
                | QueryIntent::CurrentEvents
                | QueryIntent::Factual
                | QueryIntent::Exploratory
        );
        let synchronous_hydration = answer_requires_synchronous_hydration(query, intent);
        let evidence_k = if claim_bearing {
            top_k.max(40).min(50)
        } else {
            top_k
        };
        let initial_retrieval_started = std::time::Instant::now();
        let (search_result, market_quote_lookup) = if is_live_value_query(query) {
            std::thread::scope(|scope| {
                let market_lookup = scope.spawn(|| {
                    crate::market_data::lookup_for_query(
                        query,
                        filters.locale.as_deref(),
                        filters.gl.as_deref(),
                    )
                });
                let search_result = self.search_blended_internal(query, evidence_k, filters, false);
                let lookup = market_lookup.join().unwrap_or_else(|_| {
                    let mut lookup = crate::market_data::MarketQuoteLookup::default();
                    lookup
                        .errors
                        .push("market-data lookup worker panicked".to_string());
                    lookup
                });
                (search_result, lookup)
            })
        } else {
            (
                self.search_blended_internal(query, evidence_k, filters, false),
                crate::market_data::MarketQuoteLookup::default(),
            )
        };
        let (mut results, mut sources_used) = search_result?;
        for record in &market_quote_lookup.records {
            let source = format!("{}_market_data", record.provider);
            if !sources_used.contains(&source) {
                sources_used.push(source);
            }
        }
        let structured_market_quote_ready = is_live_value_query(query)
            && crate::market_data::validated_quote_records(
                &market_quote_lookup.records,
                crate::market_data::unix_time_ms(),
            )
            .is_some();
        let mut search_passes = 1;
        let mut retrieval_expansion_queries = Vec::new();
        if results.is_empty()
            && !self.search_engines.is_empty()
            && !structured_market_quote_ready
            // Diligence already issues a neutral entity query plus an
            // adverse-evidence query against the recent corpus, followed by
            // bounded namespace recovery when necessary. Repeating that
            // entire matrix as a second "news retry" doubled provider tails
            // without adding a distinct evidence source.
            && intent != QueryIntent::DueDiligence
        {
            // A keyless provider can transiently return an empty bot-wall
            // response. For factual and diligence queries, the provider's
            // news corpus is also a useful independent fallback when its web
            // corpus has no answer-bearing surface (common for newly launched
            // entities and facts that are suddenly newsworthy). The original
            // intent-scope and evidence gates still apply after discovery.
            let news_retry_filters =
                matches!(intent, QueryIntent::Factual | QueryIntent::DueDiligence).then(|| {
                    let mut retry = filters.clone();
                    if retry.freshness.is_none() {
                        retry.freshness = Some("month".to_string());
                    }
                    retry
                });
            let retry_filters = news_retry_filters.as_ref().unwrap_or(filters);
            let (retry_results, retry_sources) =
                self.search_blended_internal(query, evidence_k, retry_filters, false)?;
            search_passes += 1;
            if !retry_results.is_empty() {
                results = retry_results;
            }
            for source in retry_sources {
                if !sources_used.contains(&source) {
                    sources_used.push(source);
                }
            }
        }
        let reference_depth = top_k.min(3);
        if results.len() < reference_depth
            && intent == QueryIntent::Exploratory
            && is_reference_exploration_query(query)
            && !self.factual_reference_engines.is_empty()
        {
            // A definitional query can be empty or shallow when keyless web
            // providers are rate-limited. The bounded MediaWiki API completes
            // the display slate while preserving any retained local result.
            // It is a reference-corpus fallback, never a redundant web
            // provider or independent corroboration for claim-bearing answers.
            let reference_params = SearchParams {
                locale: filters.locale.clone(),
                gl: filters.gl.clone(),
                site: None,
                freshness: None,
            };
            let reference_results = crate::engines::search_with_engines(
                &self.factual_reference_engines,
                query,
                evidence_k.min(10),
                &reference_params,
            );
            search_passes += 1;
            retrieval_expansion_queries.push(format!("wikipedia_reference:{query}"));
            if !reference_results.is_empty() {
                let query_embedding = self.embedder.embed_query(query)?;
                results = merge_results(
                    std::mem::take(&mut results),
                    reference_results,
                    query,
                    evidence_k,
                    &self.stake_registry,
                    &query_embedding,
                    self.embedder.as_ref(),
                );
                apply_decision_policy(query, &mut results);
                if !results.is_empty() {
                    sources_used.push("wikipedia_reference".to_string());
                    if self.config.crawl_on_query {
                        let pending = self.exclude_recently_retained_background_urls(
                            query,
                            self.urls_to_crawl(&results, self.config.crawl_on_query_max_urls),
                        );
                        if let Ok(mut queue) = self.crawl_queue.lock() {
                            for url in pending {
                                if !queue.contains(&url) {
                                    queue.push(url);
                                }
                            }
                        }
                    }
                }
            }
        }
        let recommendation_depth = top_k.min(3);
        if is_recommendation_query(query)
            && !recommendation_slate_is_sufficient(&results, recommendation_depth)
        {
            // Recommendation SERPs are unusually sensitive to language and
            // commercial-query phrasing. A keyless provider may return a bot
            // wall or retailer taxonomy for the literal wording while a
            // purpose-preserving comparison rewrite exposes review sources.
            // These are discovery rewrites only: the original query still
            // owns the answer warning and no product is synthesized.
            let mut recommendation_filters = filters.clone();
            if recommendation_filters.freshness.is_none() {
                // Bing's ordinary keyless web surface often tokenizes
                // "best laptops" into dictionary or retailer results. Its
                // news vertical exposes dated review/editorial pages while
                // preserving the exact product-purpose query. This remains a
                // discovery lead; the original query's crawl and commercial-
                // dependency gates still decide what can be displayed.
                recommendation_filters.freshness = Some("1y".to_string());
            }
            let mut best_family_depth = recommendation_source_family_count(&results);
            let mut expanded = false;
            for rewrite in recommendation_recovery_queries(query) {
                let (retry_results, retry_sources) = self.search_blended_internal(
                    &rewrite,
                    evidence_k,
                    &recommendation_filters,
                    false,
                )?;
                search_passes += 1;
                retrieval_expansion_queries.push(rewrite);
                for source in retry_sources {
                    if !sources_used.contains(&source) {
                        sources_used.push(source);
                    }
                }
                let retry_family_depth = recommendation_source_family_count(&retry_results);
                if retry_family_depth > best_family_depth
                    || (retry_family_depth == best_family_depth
                        && retry_results.len() > results.len())
                {
                    best_family_depth = retry_family_depth;
                    results = retry_results;
                    expanded = true;
                }
                if best_family_depth >= recommendation_depth {
                    break;
                }
            }
            if expanded {
                sources_used.push("recommendation_query_expansion".to_string());
            }
        }
        // Preserve the broad, intent-scoped candidate slate separately from
        // the stricter answer-bearing slate below. A factual answer may need
        // to abstain because only one complete source survives claim gates;
        // that must not erase other relevant discovery results from the UI.
        let official_entrypoints = official_state_source_urls(query);
        // A validated market quote is already backed by two typed,
        // timestamped provider observations. Crawl ordinary price pages in
        // the background for index coverage, but do not block the quote on a
        // second, weaker HTML hydration path.
        let mut resident_authority_evidence = if official_entrypoints.is_empty() {
            Vec::new()
        } else {
            let requested = official_entrypoints
                .iter()
                .map(|url| url.trim_end_matches('/'))
                .collect::<std::collections::HashSet<_>>();
            results
                .iter()
                .filter(|result| {
                    result.source == "local"
                        && requested.contains(result.url.trim_end_matches('/'))
                        && !result.content_md.trim().is_empty()
                })
                .cloned()
                .collect::<Vec<_>>()
        };
        if resident_authority_evidence.is_empty() && !official_entrypoints.is_empty() {
            // A large mixed index may omit the registered entrypoint from the
            // broad candidate window. Pay for the exact-URL query only then;
            // the common resident path reuses the already-ranked local chunk.
            resident_authority_evidence =
                self.local_evidence_for_exact_urls(query, &official_entrypoints, 20)?;
        }
        // Registry entries for one topic are bounded alternatives, not a
        // checklist of mandatory sources. One recently crawled exact
        // entrypoint is enough to avoid refetching a broken/redirected sibling;
        // answer-quality gates still decide whether its content is usable.
        let resident_authority_is_fresh = retained_exact_url_is_fresh(
            &resident_authority_evidence,
            &official_entrypoints,
            unix_time_secs(),
            3_600,
        );
        let foreground_hydration = (synchronous_hydration && !structured_market_quote_ready)
            || (!official_entrypoints.is_empty() && !resident_authority_is_fresh);
        let initial_discovery_results = foreground_hydration.then(|| results.clone());
        let mut factual_display_fallback = (intent == QueryIntent::Factual).then(|| {
            results
                .iter()
                .filter(|result| {
                    crate::answer::factual_discovery_surface_matches_relation(query, result)
                })
                .cloned()
                .collect::<Vec<_>>()
        });
        let current_display_fallback =
            (intent == QueryIntent::CurrentEvents && !is_poll_query(query)).then(|| {
                let mut candidates = results.clone();
                // A live quote or office-holder page can be continuously
                // updated and legitimately have no article publication date.
                // Preserve those scoped discovery leads for display while the
                // answer still abstains; ordinary news keeps the dated gate.
                if !current_query_allows_undated_display(query) {
                    retain_answered_serving_scope(query, &mut candidates);
                }
                candidates
            });
        let mut trace = crate::answer::AnswerTrace {
            initial_retrieval_ms: initial_retrieval_started.elapsed().as_secs_f64() * 1_000.0,
            retrieval_expansion_queries,
            market_quote_lookup_ms: market_quote_lookup.lookup_ms,
            market_quote_records: market_quote_lookup.records.len(),
            market_quote_errors: market_quote_lookup.errors.len(),
            ..Default::default()
        };
        let mut hydrated_pages = 0;
        let mut supplemental_answer_evidence = Vec::new();

        let current_local_fast_path_is_fresh = intent == QueryIntent::CurrentEvents
            && !sources_used.is_empty()
            && sources_used.iter().all(|source| source == "local")
            && local_coverage_satisfies_intent(query, &results)
            && (!is_poll_query(query)
                || crate::answer::construct_answer(query, intent, &results).status == "extractive");
        trace.fresh_local_evidence_hit =
            current_local_fast_path_is_fresh || resident_authority_is_fresh;

        let answer_urls = if foreground_hydration && !current_local_fast_path_is_fresh {
            // Answer construction may need more evidence than the background
            // flywheel. When the query has curated official entrypoints,
            // hydrate only that bounded authority set in the foreground.
            // Filling the same batch with arbitrary discovery results adds
            // latency and can dilute the high-stakes evidence surface.
            let hydration_limit = 5;
            let mut answer_urls = official_entrypoints.clone();
            if official_entrypoints.is_empty() {
                let scheduled_urls = if intent == QueryIntent::CurrentEvents && is_poll_query(query)
                {
                    // Use the same round-balanced scheduler on the first
                    // hydration batch. Otherwise five generic/high-ranked
                    // pages can all describe one scenario and force a second
                    // five-page completion crawl after the first batch has
                    // already returned.
                    poll_urls_to_crawl(&results, hydration_limit)
                } else {
                    self.answer_urls_to_crawl(query, &results, hydration_limit)
                };
                for url in scheduled_urls {
                    if !answer_urls.contains(&url) {
                        answer_urls.push(url);
                    }
                }
            }
            answer_urls.truncate(hydration_limit);
            if !official_entrypoints.is_empty()
                && !sources_used
                    .iter()
                    .any(|source| source == "official_authority_registry")
            {
                sources_used.push("official_authority_registry".to_string());
            }
            trace.initial_hydration_requested = answer_urls.len();
            answer_urls
        } else {
            Vec::new()
        };

        let hydrated_answer_urls = !answer_urls.is_empty();
        if hydrated_answer_urls {
            let initially_attempted_urls = answer_urls.clone();
            self.remove_pending_crawl_urls(&answer_urls);
            let hydration_started = std::time::Instant::now();
            hydrated_pages = self.process_crawl_urls(answer_urls);
            trace.initial_hydration_ms = hydration_started.elapsed().as_secs_f64() * 1_000.0;
            // A same-URL crawl may have been coalesced behind another
            // request, in which case this request learned zero pages itself
            // but the shared index changed. Always refresh after a requested
            // hydration batch.
            let refresh_started = std::time::Instant::now();
            let (hydrated_results, hydrated_sources) =
                self.search_retained_evidence(query, evidence_k, filters)?;
            trace.retrieval_refresh_ms += refresh_started.elapsed().as_secs_f64() * 1_000.0;
            results = hydrated_results;
            for source in hydrated_sources {
                if !sources_used.contains(&source) {
                    sources_used.push(source);
                }
            }
            if !official_entrypoints.is_empty() {
                // A large resident index can rank an exact, freshly hydrated
                // authority page below the global candidate window. Retrieve
                // within its registered domain and keep only the exact
                // versioned entrypoint (or descendants) before normal topic
                // scope and decision policy run. The registry identifies
                // where to verify; it does not encode the answer.
                let mut authority_evidence = self
                    .local_evidence_for_urls(query, &official_entrypoints, 20)?
                    .into_iter()
                    .filter(|result| official_registry_entry_for_query(query, &result.url))
                    .collect::<Vec<_>>();
                if authority_evidence.is_empty() {
                    authority_evidence =
                        self.local_evidence_for_exact_urls(query, &official_entrypoints, 20)?;
                }
                if !authority_evidence.is_empty() {
                    prefer_exact_evidence_for_same_urls(&mut results, &authority_evidence);
                    let query_embedding = self.embedder.embed_query(query)?;
                    results = merge_results(
                        results,
                        authority_evidence,
                        query,
                        evidence_k,
                        &self.stake_registry,
                        &query_embedding,
                        self.embedder.as_ref(),
                    );
                    // The exact entity join happens after the normal local
                    // refresh, so its newly appended records have not yet
                    // passed Stage 2. Re-run the deterministic policy over
                    // the combined slate before anything is displayed.
                    apply_decision_policy(query, &mut results);
                }
            }
            search_passes += 1;

            // One provider page set can be entirely blocked, paywalled, or
            // non-distillable. Retry at most once with the next already
            // discovered URLs; do not issue another search and do not serve
            // the original snippets as evidence.
            if results.is_empty()
                && let Some(initial_discovery_results) = &initial_discovery_results
            {
                let remaining_candidates = initial_discovery_results
                    .iter()
                    .filter(|result| !initially_attempted_urls.contains(&result.url))
                    .cloned()
                    .collect::<Vec<_>>();
                let retry_urls = self.answer_urls_to_crawl(query, &remaining_candidates, 5);
                if !retry_urls.is_empty() {
                    trace.initial_hydration_requested += retry_urls.len();
                    self.remove_pending_crawl_urls(&retry_urls);
                    let retry_hydration_started = std::time::Instant::now();
                    hydrated_pages += self.process_crawl_urls(retry_urls);
                    trace.initial_hydration_ms +=
                        retry_hydration_started.elapsed().as_secs_f64() * 1_000.0;
                    let retry_refresh_started = std::time::Instant::now();
                    let (retry_results, retry_sources) =
                        self.search_retained_evidence(query, evidence_k, filters)?;
                    trace.retrieval_refresh_ms +=
                        retry_refresh_started.elapsed().as_secs_f64() * 1_000.0;
                    results = retry_results;
                    for source in retry_sources {
                        if !sources_used.contains(&source) {
                            sources_used.push(source);
                        }
                    }
                    search_passes += 1;
                }
            }
        }
        if !hydrated_answer_urls && !resident_authority_evidence.is_empty() {
            // The registered authority page was revalidated recently. Attach
            // its claim-focused chunk directly from the resident index rather
            // than refetching the same URL on every exploratory query.
            prefer_exact_evidence_for_same_urls(&mut results, &resident_authority_evidence);
            let query_embedding = self.embedder.embed_query(query)?;
            results = merge_results(
                results,
                resident_authority_evidence,
                query,
                evidence_k,
                &self.stake_registry,
                &query_embedding,
                self.embedder.as_ref(),
            );
            apply_decision_policy(query, &mut results);
            if !sources_used
                .iter()
                .any(|source| source == "official_authority_registry")
            {
                sources_used.push("official_authority_registry".to_string());
            }
        }

        self.restore_factual_chunks(query, &mut results, evidence_k, &mut trace)?;
        retain_answered_serving_scope(query, &mut results);
        let provisional_answer_started = std::time::Instant::now();
        let provisional_answer = crate::answer::construct_answer(query, intent, &results);
        trace.answer_construction_ms +=
            provisional_answer_started.elapsed().as_secs_f64() * 1_000.0;
        let needs_completion = intent == QueryIntent::Factual
            && (provisional_answer.status == "insufficient_evidence"
                || crate::answer::factual_answer_may_be_incomplete(
                    query,
                    &results,
                    &provisional_answer,
                )
                || crate::answer::factual_answer_is_bounded_abstention(
                    query,
                    &results,
                    &provisional_answer,
                ));
        let needs_primary_source = intent == QueryIntent::Factual
            && ((provisional_answer.status == "extractive"
                && !provisional_answer.claims.iter().any(|claim| {
                    claim.citations.iter().any(|citation| {
                        primary_source_url_matches_claim(query, &claim.text, &citation.url)
                    })
                }))
                || (provisional_answer.status == "insufficient_evidence"
                    && crate::answer::factual_candidate_suggests_primary_authority(
                        query, &results,
                    )));
        let stable_factual_abstention = intent == QueryIntent::Factual
            && provisional_answer.status == "insufficient_evidence"
            && !needs_primary_source
            && recently_crawled_source_families(&results, FACTUAL_REVALIDATION_TTL_SECS) >= 2
            && results
                .iter()
                .filter(|result| {
                    crate::answer::factual_discovery_surface_matches_relation(query, result)
                })
                .take(2)
                .count()
                >= 2
            && results.iter().any(|result| {
                crate::answer::factual_public_surface_matches_relation(query, result)
            })
            && results
                .iter()
                .any(|result| crate::answer::best_factual_sentence(query, result).is_some());
        if stable_factual_abstention {
            // This is a fresh negative serving decision, not a negative fact:
            // at least two retained source families were examined and the UI
            // still has two relation-specific audit trails, but the claim gate
            // could not establish a safe answer. Reuse that abstention during
            // the stable-fact TTL instead of retrying providers on every
            // process restart.
            trace.fresh_local_evidence_hit = true;
        }
        let needs_poll_scenario_expansion = intent == QueryIntent::CurrentEvents
            && crate::answer::poll_answer_needs_scenario_expansion(query, &provisional_answer);
        let poll_primary_scenario_queries = if intent == QueryIntent::CurrentEvents {
            crate::answer::poll_primary_scenario_corroboration_queries(
                query,
                &results,
                &provisional_answer,
            )
        } else {
            Vec::new()
        };
        let needs_poll_primary_scenario_audit = !poll_primary_scenario_queries.is_empty();
        trace.primary_source_expansion = needs_primary_source;
        trace.poll_scenario_expansion = needs_poll_scenario_expansion;
        trace.poll_primary_scenario_audit = needs_poll_primary_scenario_audit;
        let mut corroboration_queries = if !stable_factual_abstention
            && (provisional_answer.status == "insufficient_evidence"
                || needs_completion
                || needs_primary_source
                || needs_poll_scenario_expansion
                || needs_poll_primary_scenario_audit)
        {
            match intent {
                QueryIntent::CurrentEvents => {
                    let mut queries = Vec::new();
                    queries.extend(crate::answer::current_office_holder_corroboration_queries(
                        query, &results,
                    ));
                    if needs_poll_scenario_expansion {
                        queries.extend(crate::answer::poll_scenario_completion_queries(
                            query,
                            &results,
                            &provisional_answer,
                        ));
                    }
                    queries.extend(poll_primary_scenario_queries);
                    if queries.is_empty() {
                        queries.extend(crate::answer::poll_corroboration_queries(query, &results));
                    }
                    queries.sort();
                    queries.dedup();
                    queries
                }
                QueryIntent::Factual => {
                    crate::answer::factual_corroboration_queries(query, &results)
                }
                _ => Vec::new(),
            }
        } else {
            Vec::new()
        };
        if provisional_answer.status == "insufficient_evidence"
            && canonical_answer_query(query, intent).is_some()
        {
            let official_discovery_started = std::time::Instant::now();
            let registry_context = crate::answer::poll_registry_context_terms(&results);
            // The institute observed in retained editorial titles is already
            // a valid discovery hint. Do not make this query contingent on
            // the registry listing endpoint: exact IDs add recall, but a
            // transient empty JSF response must not erase the editorial
            // fallback.
            if !registry_context.is_empty() {
                corroboration_queries.push(format!(
                    "{} pesquisa eleitoral presidente resultado geral primeiro segundo turno",
                    registry_context.join(" ")
                ));
            }
            // Once a retained editorial surface identifies the institute,
            // its bounded institute-specific query is both faster and more
            // selective than blocking on a broad TSE listing plus several ID
            // disjunctions. The exact registration is still mandatory at the
            // final registry join. Use broad official enumeration only when
            // the corpus gives us no institute context at all.
            if registry_context.is_empty() {
                match crate::pesqele::discover_brazil_presidential_registration_ids_for_context(
                    &registry_context,
                ) {
                    Ok(registration_ids) => {
                        let bounded_ids = registration_ids.into_iter().take(12).collect::<Vec<_>>();
                        for ids in bounded_ids.chunks(4) {
                            let disjunction = ids
                                .iter()
                                .map(|id| format!("\"{id}\""))
                                .collect::<Vec<_>>()
                                .join(" OR ");
                            corroboration_queries.push(format!(
                                "({disjunction}) pesquisa eleitoral presidente resultado"
                            ));
                        }
                    }
                    Err(error) => tracing::warn!(
                        error,
                        "official PesqEle discovery could not expand poll evidence"
                    ),
                }
            }
            corroboration_queries.sort();
            corroboration_queries.dedup();
            let discovery_ms = official_discovery_started.elapsed().as_secs_f64() * 1_000.0;
            trace.corroboration_discovery_ms += discovery_ms;
            trace.poll_registration_discovery_ms += discovery_ms;
        }
        if !corroboration_queries.is_empty() {
            trace.corroboration_queries = corroboration_queries.clone();
            // Evidence expansion is claim-specific: use the leading retained
            // percentage pair or factual answer entity to look for the same
            // claim in other domains.
            // This is a discovery pass only; the original query still owns
            // final relevance and decision ranking.
            let corroboration_discovery_started = std::time::Instant::now();
            let mut result_sets = std::thread::scope(|scope| {
                let handles: Vec<_> = corroboration_queries
                    .iter()
                    .map(|corroboration_query| {
                        scope.spawn(|| {
                            self.discover_external_candidates(
                                corroboration_query,
                                evidence_k,
                                filters,
                            )
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .filter_map(|handle| handle.join().ok())
                    .collect::<Vec<_>>()
            });
            let discovery_ms = corroboration_discovery_started.elapsed().as_secs_f64() * 1_000.0;
            trace.corroboration_discovery_ms += discovery_ms;
            trace.corroboration_provider_discovery_ms += discovery_ms;
            // Interleave by engine rank so one rewrite cannot consume every
            // hydration slot before another rewrite contributes a domain.
            let mut corroboration_results = Vec::new();
            let max_depth = result_sets.iter().map(Vec::len).max().unwrap_or(0);
            for rank in 0..max_depth {
                for result_set in &mut result_sets {
                    if rank < result_set.len() {
                        let candidate = result_set[rank].clone();
                        if !corroboration_results
                            .iter()
                            .any(|result: &SearchResultItem| result.url == candidate.url)
                        {
                            corroboration_results.push(candidate);
                        }
                    }
                }
            }
            trace.corroboration_discovered_candidates = corroboration_results.len();
            trace.corroboration_discovered_urls = corroboration_results
                .iter()
                .map(|result| result.url.clone())
                .collect();
            // A numeric claim rewrite (for example `"39%" "33%"`) can
            // retrieve number encyclopedias, games, or search-engine utility
            // pages. Reapply the ORIGINAL query's intent scope before any URL
            // is allowed to consume a foreground crawl slot or enter the
            // retained corpus.
            let current_office_name_leads = corroboration_results
                .iter()
                .filter(|candidate| {
                    crate::answer::current_office_holder_discovery_surface_matches(
                        query, &results, candidate,
                    )
                })
                .cloned()
                .collect::<Vec<_>>();
            retain_intent_scope(query, &mut corroboration_results);
            for candidate in current_office_name_leads {
                if corroboration_results
                    .iter()
                    .all(|result| result.url != candidate.url)
                {
                    corroboration_results.push(candidate);
                }
            }
            trace.corroboration_retained_candidates = corroboration_results.len();
            trace.corroboration_retained_urls = corroboration_results
                .iter()
                .map(|result| result.url.clone())
                .collect();
            if intent == QueryIntent::Factual
                && !self.factual_reference_engines.is_empty()
                && let Some(reference_query) = factual_reference_query(query)
            {
                // This is an auditable discovery fallback, not corroboration:
                // several Wikipedia pages still share one source family and
                // can never satisfy the two-independent-source claim gate.
                let reference_params = SearchParams {
                    locale: Some("en-US".to_string()),
                    gl: Some("us".to_string()),
                    site: None,
                    freshness: None,
                };
                let reference_results = crate::engines::search_with_engines(
                    &self.factual_reference_engines,
                    &reference_query,
                    evidence_k.min(10),
                    &reference_params,
                );
                trace.factual_reference_results = reference_results.len();
                for candidate in reference_results {
                    if corroboration_results
                        .iter()
                        .all(|result| result.url != candidate.url)
                    {
                        corroboration_results.push(candidate);
                    }
                }
                retain_intent_scope(query, &mut corroboration_results);
            }
            if let Some(fallback) = &mut factual_display_fallback {
                for result in corroboration_results.iter().filter(|result| {
                    crate::answer::factual_discovery_surface_matches_relation(query, result)
                }) {
                    if fallback.iter().all(|candidate| candidate.url != result.url) {
                        fallback.push(result.clone());
                    }
                }
                trace.factual_display_fallback_candidates = fallback.len();
            }
            if intent == QueryIntent::CurrentEvents && is_poll_query(query) {
                // Several bounded rewrites are interleaved above. Under a
                // provider burst, generic/registry rewrites can otherwise
                // consume all five crawl slots before the result whose public
                // surface contains the exact quoted vote pair. This score is
                // recall-only: it chooses verification order and cannot make
                // a snippet or title into retained evidence.
                corroboration_results.sort_by(|left, right| {
                    quoted_discovery_surface_score(&corroboration_queries, right).cmp(
                        &quoted_discovery_surface_score(&corroboration_queries, left),
                    )
                });
            } else if intent == QueryIntent::Factual {
                // Put relation-specific leads ahead of merely entity-adjacent
                // pages for the bounded crawl budget. This changes only
                // verification order; provider snippets remain non-evidence.
                corroboration_results.sort_by_key(|result| {
                    std::cmp::Reverse(crate::answer::factual_discovery_surface_matches_relation(
                        query, result,
                    ))
                });
            }
            search_passes += 1;
            for result in &corroboration_results {
                if !sources_used.contains(&result.source) {
                    sources_used.push(result.source.clone());
                }
            }
            // Founder-list completion is best effort and remains fail-closed.
            // If the initial five-page hydration already consumed 3.5
            // seconds, another sequential five-page crawl can violate the
            // five-second foreground hydration SLO without making a partial
            // enumeration safe. The 500 ms headroom is intentional: provider
            // and index refresh work also sits on the critical path. Keep the
            // discovered leads in the short-lived discovery cache for a later
            // retry and abstain on the evidence already retained.
            // Primary-source repair and
            // poll scenario repair are never skipped by this budget.
            let factual_completion_budget_exhausted = factual_completion_hydration_budget_exhausted(
                intent,
                needs_completion,
                needs_primary_source,
                trace.initial_hydration_ms,
            );
            trace.corroboration_hydration_budget_exhausted = factual_completion_budget_exhausted;
            let corroboration_urls = if factual_completion_budget_exhausted {
                Vec::new()
            } else if needs_primary_source {
                let primary_claim_texts = if provisional_answer.claims.is_empty() {
                    results
                        .iter()
                        .filter_map(|result| crate::answer::best_factual_sentence(query, result))
                        .collect::<Vec<_>>()
                } else {
                    provisional_answer
                        .claims
                        .iter()
                        .map(|claim| claim.text.clone())
                        .collect::<Vec<_>>()
                };
                let primary_results = corroboration_results
                    .iter()
                    .filter(|result| {
                        primary_claim_texts.iter().any(|claim| {
                            primary_source_url_matches_claim(query, claim, &result.url)
                        })
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                self.diverse_urls_to_crawl(&primary_results, 1)
            } else if intent == QueryIntent::Factual && needs_completion {
                // Completion usually needs one missing independent relation
                // page, not five arbitrary entity pages. Prefer a missing
                // primary record and relation-legible leads, exclude source
                // families that already yielded answer-bearing retained
                // content, and keep the second foreground batch bounded.
                self.factual_completion_urls_to_crawl(query, &results, &corroboration_results, 3)
            } else if intent == QueryIntent::CurrentEvents && is_poll_query(query) {
                if needs_poll_scenario_expansion {
                    // A broad presidential query promises a first-round and
                    // a runoff slot. Each slot needs two reporting families,
                    // so reserve both pairs plus one failure-margin slot.
                    poll_urls_to_crawl(&corroboration_results, 5)
                } else {
                    // A primary-slate audit already has one retained page
                    // that explicitly relates the alternate and unqualified
                    // scenarios. It needs one missing reporting family, not
                    // another complete two-round slate. The quoted-pair sort
                    // above keeps the exact lead first; three domains provide
                    // a bounded crawl-failure margin.
                    self.diverse_urls_to_crawl(&corroboration_results, 3)
                }
            } else if is_current_office_holder_query(query) {
                // Two official entrypoints established the canonical name.
                // One independent editorial family proves corroboration. A
                // provider may expose several usable URLs from only that one
                // family, so allow same-family backup URLs as a bounded crawl-
                // failure margin without counting them as independent claims.
                self.office_urls_to_crawl(&corroboration_results, 3)
            } else {
                self.diverse_urls_to_crawl(&corroboration_results, 5)
            };
            let primary_urls = needs_primary_source.then(|| corroboration_urls.clone());
            let poll_evidence_urls = (intent == QueryIntent::CurrentEvents && is_poll_query(query))
                .then(|| corroboration_urls.clone());
            let office_evidence_request = is_current_office_holder_query(query)
                .then(|| (corroboration_queries.clone(), corroboration_urls.clone()));
            trace.corroboration_hydration_requested = corroboration_urls.len();
            trace.corroboration_hydration_urls = corroboration_urls.clone();
            self.remove_pending_crawl_urls(&corroboration_urls);
            let corroboration_hydration_started = std::time::Instant::now();
            let learned = self.process_crawl_urls(corroboration_urls);
            trace.corroboration_hydration_ms =
                corroboration_hydration_started.elapsed().as_secs_f64() * 1_000.0;
            hydrated_pages += learned;
            if let Some(primary_urls) = primary_urls {
                // Read the exact primary domain even when this request was a
                // crawl follower and therefore ingested no page itself.
                supplemental_answer_evidence =
                    self.local_evidence_for_urls(query, &primary_urls, 10)?;
            }
            if let Some((office_queries, office_urls)) = office_evidence_request {
                // The broad user query may select a generic office-definition
                // chunk from the right article while the exact name↔office
                // sentence lives in another retained chunk. Query each
                // claim-derived rewrite independently: concatenating quoted
                // rewrites made the lexical request artificially impossible
                // on some otherwise valid editorial pages. These passages
                // feed answer construction but do not alter the public slate.
                let mut seen = supplemental_answer_evidence
                    .iter()
                    .map(|result| result.result_id.clone())
                    .collect::<std::collections::HashSet<_>>();
                for office_query in office_queries {
                    let office_evidence =
                        self.local_evidence_for_urls(&office_query, &office_urls, 10)?;
                    supplemental_answer_evidence.extend(
                        office_evidence
                            .into_iter()
                            .filter(|result| seen.insert(result.result_id.clone())),
                    );
                }
            }
            if trace.corroboration_hydration_requested > 0 {
                let refresh_started = std::time::Instant::now();
                let (expanded_results, expanded_sources) =
                    self.search_retained_evidence(query, evidence_k, filters)?;
                trace.retrieval_refresh_ms += refresh_started.elapsed().as_secs_f64() * 1_000.0;
                results = expanded_results;
                for source in expanded_sources {
                    if !sources_used.contains(&source) {
                        sources_used.push(source);
                    }
                }
                if let Some(poll_evidence_urls) = &poll_evidence_urls {
                    // Broad poll wording can rank newly crawled result pages
                    // below the mixed-index window, especially when a fresh
                    // demographic breakdown shares the same institute and
                    // headline pair. Reattach only the exact URLs selected by
                    // the bounded corroboration crawl, then rerun the original
                    // query scope and Stage 2. This improves recall without
                    // allowing SERP snippets to become evidence.
                    let exact_query = corroboration_queries
                        .first()
                        .map(String::as_str)
                        .unwrap_or(query);
                    let exact_poll_evidence =
                        self.local_evidence_for_exact_urls(exact_query, poll_evidence_urls, 20)?;
                    if !exact_poll_evidence.is_empty() {
                        let mut exact_urls = exact_poll_evidence
                            .iter()
                            .map(|result| result.url.clone())
                            .collect::<Vec<_>>();
                        exact_urls.sort();
                        exact_urls.dedup();
                        trace.corroboration_exact_evidence_urls = exact_urls;
                        // The broad refresh may already contain the same URL
                        // represented by a less relevant chunk. URL-level
                        // dedup in `merge_results` would otherwise discard the
                        // claim-focused exact chunk we just retrieved. Replace
                        // only same-URL representatives; source identity and
                        // public ranking remain URL-level.
                        prefer_exact_evidence_for_same_urls(&mut results, &exact_poll_evidence);
                        let query_embedding = self.embedder.embed_query(query)?;
                        results = merge_results(
                            results,
                            exact_poll_evidence,
                            query,
                            evidence_k,
                            &self.stake_registry,
                            &query_embedding,
                            self.embedder.as_ref(),
                        );
                        apply_decision_policy(query, &mut results);
                    }
                }
                search_passes += 1;
            }
        }

        // A cold poll can begin with no corroborated claim at all. The first
        // expansion then establishes one first-round claim, which is too late
        // for the pre-expansion scenario check above. Re-evaluate once and,
        // only when a broad Brazilian presidential answer is now partial,
        // perform one bounded round-completion pass. This is a sequential
        // evidence dependency, not an unbounded retry loop.
        self.restore_factual_chunks(query, &mut results, evidence_k, &mut trace)?;
        retain_answered_serving_scope(query, &mut results);
        if intent == QueryIntent::CurrentEvents
            && is_poll_query(query)
            && !trace.poll_scenario_expansion
        {
            let partial_answer_started = std::time::Instant::now();
            let partial_answer = crate::answer::construct_answer(query, intent, &results);
            trace.answer_construction_ms +=
                partial_answer_started.elapsed().as_secs_f64() * 1_000.0;
            if crate::answer::poll_answer_needs_scenario_expansion(query, &partial_answer) {
                trace.poll_scenario_expansion = true;
                let completion_queries = crate::answer::poll_scenario_completion_queries(
                    query,
                    &results,
                    &partial_answer,
                );
                let completion_discovery_started = std::time::Instant::now();
                let mut result_sets = std::thread::scope(|scope| {
                    let handles: Vec<_> = completion_queries
                        .iter()
                        .map(|completion_query| {
                            scope.spawn(|| {
                                self.discover_external_candidates(
                                    completion_query,
                                    evidence_k,
                                    filters,
                                )
                            })
                        })
                        .collect();
                    handles
                        .into_iter()
                        .filter_map(|handle| handle.join().ok())
                        .collect::<Vec<_>>()
                });
                let discovery_ms = completion_discovery_started.elapsed().as_secs_f64() * 1_000.0;
                trace.corroboration_discovery_ms += discovery_ms;
                trace.poll_scenario_discovery_ms += discovery_ms;

                let mut completion_results = Vec::new();
                let max_depth = result_sets.iter().map(Vec::len).max().unwrap_or(0);
                for rank in 0..max_depth {
                    for result_set in &mut result_sets {
                        if rank < result_set.len() {
                            let candidate = result_set[rank].clone();
                            if !completion_results
                                .iter()
                                .any(|result: &SearchResultItem| result.url == candidate.url)
                            {
                                completion_results.push(candidate);
                            }
                        }
                    }
                }
                retain_intent_scope(query, &mut completion_results);
                search_passes += 1;
                for result in &completion_results {
                    if !sources_used.contains(&result.source) {
                        sources_used.push(result.source.clone());
                    }
                }

                let completion_urls = self.diverse_urls_to_crawl(&completion_results, 3);
                trace.corroboration_hydration_requested += completion_urls.len();
                self.remove_pending_crawl_urls(&completion_urls);
                let completion_hydration_started = std::time::Instant::now();
                hydrated_pages += self.process_crawl_urls(completion_urls);
                trace.corroboration_hydration_ms +=
                    completion_hydration_started.elapsed().as_secs_f64() * 1_000.0;
                if !completion_results.is_empty() {
                    let refresh_started = std::time::Instant::now();
                    let (expanded_results, expanded_sources) =
                        self.search_retained_evidence(query, evidence_k, filters)?;
                    trace.retrieval_refresh_ms += refresh_started.elapsed().as_secs_f64() * 1_000.0;
                    results = expanded_results;
                    for source in expanded_sources {
                        if !sources_used.contains(&source) {
                            sources_used.push(source);
                        }
                    }
                    search_passes += 1;
                }
            }
        }

        self.restore_factual_chunks(query, &mut results, evidence_k, &mut trace)?;
        retain_answered_serving_scope(query, &mut results);
        let mut poll_registry = Vec::new();
        if intent == QueryIntent::CurrentEvents && is_poll_query(query) {
            let registry_started = std::time::Instant::now();
            // First decide which retained pages actually support an
            // extractive claim. Broad retrieval may contain several older
            // polls; their registrations are useful corpus evidence but do
            // not belong in this answer's official join.
            let provisional_poll_answer = crate::answer::construct_answer(query, intent, &results);
            let claim_urls = provisional_poll_answer
                .claims
                .iter()
                .flat_map(|claim| claim.citations.iter())
                .filter(|citation| citation.evidence_role == "claim_text")
                .map(|citation| citation.url.clone())
                .collect::<std::collections::HashSet<_>>();
            let registration_ids =
                crate::pesqele::registration_ids_for_claim_urls(&results, &claim_urls);
            trace.poll_registry_ids = registration_ids.len();
            let registry_cache_dir = self
                .config
                .local_index_path
                .as_deref()
                .map(std::path::Path::new)
                .map(|path| path.join("pesqele-registry"));
            let registry_lookups = std::thread::scope(|scope| {
                let handles: Vec<_> = registration_ids
                    .iter()
                    .map(|registration_id| {
                        scope.spawn(|| {
                            (
                                registration_id.clone(),
                                crate::pesqele::lookup_with_cache(
                                    registration_id,
                                    registry_cache_dir.as_deref(),
                                ),
                            )
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .filter_map(|handle| handle.join().ok())
                    .collect::<Vec<_>>()
            });
            for (registration_id, lookup) in registry_lookups {
                match lookup {
                    Ok(Some(record))
                        if crate::pesqele::record_matches_query_scope(query, &record) =>
                    {
                        poll_registry.push(record)
                    }
                    Ok(Some(record)) => tracing::warn!(
                        registration_id,
                        methodology = %record.methodology,
                        "official poll registration does not match the query's geographic universe"
                    ),
                    Ok(None) => tracing::warn!(
                        registration_id,
                        "retained poll registration was not found in public PesqEle"
                    ),
                    Err(error) => {
                        trace.poll_registry_errors += 1;
                        tracing::warn!(registration_id, error, "public PesqEle lookup failed");
                    }
                }
            }
            poll_registry.sort_by(|left, right| left.registration_id.cmp(&right.registration_id));
            trace.poll_registry_records = poll_registry.len();
            trace.poll_registry_lookup_ms = registry_started.elapsed().as_secs_f64() * 1_000.0;
            if !poll_registry.is_empty()
                && !sources_used.iter().any(|source| source == "tse_pesqele")
            {
                sources_used.push("tse_pesqele".to_string());
            }
            if !poll_registry.is_empty() {
                let applicable_ids = poll_registry
                    .iter()
                    .map(|record| record.registration_id.as_str())
                    .collect::<std::collections::HashSet<_>>();
                results.retain(|result| {
                    let declared = crate::pesqele::registration_ids_for_result(result);
                    declared.is_empty()
                        || declared.iter().any(|registration_id| {
                            applicable_ids.contains(registration_id.as_str())
                        })
                });
            }
            // The ordinary decision pass could only see page-level evidence.
            // Re-run it after the registry join so pages declaring the same
            // TSE registration become one underlying measurement plus
            // editorial derivations. Stage-1 retrieval ranks are preserved.
            apply_decision_policy_with_registry(query, &mut results, &poll_registry);
        }
        if is_current_office_holder_query(query) {
            // A current-office answer has one query-relative primary record
            // from the explicit authority registry. Keep that record ahead
            // of derivative editorial reporting; this is a stable source-role
            // partition, not a generic government-domain trust bonus.
            results.sort_by_key(|result| {
                std::cmp::Reverse(crate::query_intent::authoritative_source_for_query(
                    query,
                    &result.url,
                ))
            });
            for (index, result) in results.iter_mut().enumerate() {
                let rank = (index + 1) as u32;
                result.rank = rank;
                if let Some(decision) = &mut result.decision {
                    decision.decision_rank = rank;
                }
            }
        }
        let answer_started = std::time::Instant::now();
        let (mut answer, checklist) = if supplemental_answer_evidence.is_empty() {
            crate::answer::construct_answer_with_checklist(
                query,
                intent,
                &results,
                &poll_registry,
                &market_quote_lookup.records,
            )
        } else {
            // Preserve ranks from the public result slate. Supplemental
            // claim-specific passages are appended after it; their per-site
            // retrieval ranks are not comparable with the main query.
            let mut answer_results = results.clone();
            let mut seen_urls: std::collections::HashSet<_> = answer_results
                .iter()
                .map(|evidence| evidence.url.clone())
                .collect();
            for mut evidence in supplemental_answer_evidence {
                if seen_urls.insert(evidence.url.clone()) {
                    evidence.rank = (answer_results.len() + 1) as u32;
                    answer_results.push(evidence);
                }
            }
            crate::answer::construct_answer_with_checklist(
                query,
                intent,
                &answer_results,
                &poll_registry,
                &market_quote_lookup.records,
            )
        };
        trace.evidence_checklist = Some(checklist);
        trace.answer_construction_ms += answer_started.elapsed().as_secs_f64() * 1_000.0;
        if intent == QueryIntent::Factual && answer.status == "insufficient_evidence" {
            // An abstention still deserves useful search results, but broad
            // lexical overlap must not surface a different predicate (for
            // example who created Claude when the user asked who created
            // Anthropic). This is a display gate, never claim promotion.
            results.retain(|result| {
                crate::answer::factual_discovery_surface_matches_relation(query, result)
            });
            if let Some(fallback) = factual_display_fallback.take() {
                restore_display_fallback(&mut results, fallback, top_k);
            }
            // Some auditable fallback leads are external discovery records
            // that were not crawled within the foreground budget. They must
            // still pass Stage 2 before being returned, while their snippets
            // remain non-evidence. Recompute the combined slate from a clean,
            // unique retrieval order so every displayed item carries
            // comparable uncertainty metadata.
            for (index, result) in results.iter_mut().enumerate() {
                result.rank = (index + 1) as u32;
                result.decision = None;
            }
            apply_decision_policy(query, &mut results);
            // Prefer a title/snippet that directly exposes the requested
            // relation over a vague title whose retained body is merely
            // auditable. This is a stable display partition after Stage 2;
            // it does not change evidence, scores, or answer claims.
            results.sort_by_key(|result| {
                std::cmp::Reverse(crate::answer::factual_public_surface_matches_relation(
                    query, result,
                ))
            });
            for (index, result) in results.iter_mut().enumerate() {
                let rank = (index + 1) as u32;
                result.rank = rank;
                if let Some(decision) = &mut result.decision {
                    decision.decision_rank = rank;
                }
            }
            trace.factual_display_results = results.len().min(top_k);
        }
        if intent == QueryIntent::Factual && answer.status == "extractive" {
            // Claim evidence and search-result breadth have different jobs.
            // Keep the answer limited to fetched, corroborated citations, but
            // do not collapse a useful search slate to only those two URLs.
            // Strict relation-legible discovery leads may fill the remaining
            // display slots; their snippets remain non-evidence and cannot
            // enter the already-constructed answer.
            if results.len() < top_k
                && let Some(fallback) = factual_display_fallback.take()
            {
                restore_display_fallback(&mut results, fallback, top_k);
                for (index, result) in results.iter_mut().enumerate() {
                    result.rank = (index + 1) as u32;
                    result.decision = None;
                }
                apply_decision_policy(query, &mut results);
                trace.factual_display_results = results.len().min(top_k);
            }
            // If a factual claim already cites a claim-compatible primary
            // authority, keep that authority visible in the public slate.
            // A large mixed index can otherwise retrieve the exact supporting
            // chunk for answer construction yet leave its URL outside the
            // global display window. This is not a generic `.gov` boost: the
            // URL must be an emitted citation and identify the institution in
            // the emitted claim (for example BCB in the Pix origin claim).
            let mut primary_citation_urls = answer
                .claims
                .iter()
                .flat_map(|claim| {
                    claim.citations.iter().filter_map(move |citation| {
                        crate::query_intent::primary_source_url_matches_claim(
                            query,
                            &claim.text,
                            &citation.url,
                        )
                        .then(|| citation.url.clone())
                    })
                })
                .collect::<Vec<_>>();
            primary_citation_urls.sort();
            primary_citation_urls.dedup();
            if !primary_citation_urls.is_empty() {
                let primary_evidence = self.local_evidence_for_exact_urls(
                    query,
                    &primary_citation_urls,
                    primary_citation_urls.len().saturating_mul(4).max(4),
                )?;
                if !primary_evidence.is_empty() {
                    prefer_exact_evidence_for_same_urls(&mut results, &primary_evidence);
                    let query_embedding = self.embedder.embed_query(query)?;
                    results = merge_results(
                        results,
                        primary_evidence,
                        query,
                        evidence_k,
                        &self.stake_registry,
                        &query_embedding,
                        self.embedder.as_ref(),
                    );
                    apply_decision_policy(query, &mut results);
                }
                let primary_urls = primary_citation_urls
                    .iter()
                    .map(String::as_str)
                    .collect::<std::collections::HashSet<_>>();
                results.sort_by_key(|result| {
                    std::cmp::Reverse(primary_urls.contains(result.url.as_str()))
                });
            }
        }
        if intent == QueryIntent::CurrentEvents
            && is_poll_query(query)
            && answer.status == "extractive"
        {
            // The answer is claim-oriented, while the public slate is also a
            // search product. Interleave the retained citation families from
            // each emitted round, then keep one result per source family
            // ahead of derivatives. This preserves the top answer evidence
            // while preventing two pages from one publisher group from
            // crowding independent reporting out of the visible top three.
            prefer_claim_citation_diversity(&mut results, &answer);
        }
        if intent == QueryIntent::CurrentEvents
            && !is_poll_query(query)
            && answer.status == "insufficient_evidence"
            && let Some(fallback) = current_display_fallback
        {
            restore_display_fallback(&mut results, fallback, top_k);
            // The claim layer still abstains, but its summary and uncertainty
            // must describe the same auditable slate the caller receives.
            // External snippets remain discovery-only because the generic
            // current-state constructor never emits a claim.
            answer = crate::answer::construct_answer(query, intent, &results);
            trace.evidence_checklist = Some(crate::checklist::evaluate(query, &answer, &results));
        }
        if is_current_office_holder_query(query) && answer.status == "extractive" {
            // Once the holder claim is resolved, keep the public slate tied
            // to the evidence actually used. A shared corpus can contain an
            // unrelated article (for example about Pix) whose body mentions
            // the president; it must not survive merely because that hidden
            // passage helped broad retrieval.
            let cited_urls = answer
                .claims
                .iter()
                .flat_map(|claim| claim.citations.iter())
                .map(|citation| citation.url.as_str())
                .collect::<std::collections::HashSet<_>>();
            results.retain(|result| {
                crate::query_intent::authoritative_source_for_query(query, &result.url)
                    || cited_urls.contains(result.url.as_str())
            });
        }
        normalize_display_decision_ranks(&mut results);
        results.truncate(top_k);
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
        trace.total_ms = total_started.elapsed().as_secs_f64() * 1_000.0;
        let answered = crate::answer::AnsweredSearch {
            query: query.to_string(),
            answer,
            results,
            sources_used,
            search_passes,
            hydrated_pages,
            poll_registry,
            market_quotes: market_quote_lookup.records,
            trace,
        };
        if let Some(cache_key) = answer_cache_key
            && answered_search_is_cacheable(intent, query, &answered)
        {
            self.cache_answered_search(cache_key, &answered);
        }
        Ok(answered)
    }

    /// External-engine half of `search_blended`: fetch, merge, crawl-queue.
    /// `query` is the user's ORIGINAL text (external engines do their own
    /// spell correction); ranking uses `query_embedding`, which is computed
    /// from the locally-corrected form.
    fn resolve_transaction_destination(
        &self,
        query: &str,
        candidates: &[SearchResultItem],
    ) -> Option<SearchResultItem> {
        const FOLLOWER_DEADLINE: std::time::Duration = std::time::Duration::from_secs(2);

        let key = transaction_destination_key(query)?;
        let cached = self
            .navigation_cache
            .lock()
            .ok()
            .and_then(|cache| cache.get(&key))
            .filter(|result| {
                result.extraction_quality == "verified_transaction_route"
                    && navigation_entity_host_match(query, result)
                    && transaction_destination_matches_query(query, &result.url)
            });
        if cached.is_some() {
            return cached;
        }

        let route_candidates = transaction_route_candidates(query, candidates);
        if route_candidates.is_empty() {
            return None;
        }
        if self
            .navigation_cache
            .lock()
            .ok()
            .is_some_and(|cache| !cache.route_probe_due(&key))
        {
            return None;
        }

        let (leader, flight) = {
            let mut flights = self.transaction_flights.lock().ok()?;
            if let Some(flight) = flights.get(&key) {
                (false, Arc::clone(flight))
            } else {
                let flight = Arc::new(CrawlFlight {
                    complete: std::sync::Mutex::new(false),
                    ready: std::sync::Condvar::new(),
                });
                flights.insert(key.clone(), Arc::clone(&flight));
                (true, flight)
            }
        };

        if !leader {
            if let Ok(complete) = flight.complete.lock() {
                let _ = flight
                    .ready
                    .wait_timeout_while(complete, FOLLOWER_DEADLINE, |complete| !*complete);
            }
            return self
                .navigation_cache
                .lock()
                .ok()
                .and_then(|cache| cache.get(&key))
                .filter(|result| result.extraction_quality == "verified_transaction_route");
        }

        let mut probe_candidates = discover_declared_transaction_routes(query, candidates);
        for candidate in route_candidates {
            if !probe_candidates
                .iter()
                .any(|declared| declared.url == candidate.url)
            {
                probe_candidates.push(candidate);
            }
        }
        let (destination, definitive_miss) = probe_transaction_routes(query, &probe_candidates);
        if let Ok(mut cache) = self.navigation_cache.lock() {
            if let Some(destination) = &destination {
                cache.remember(key.clone(), destination, destination.url.clone());
            } else if definitive_miss {
                cache.remember_route_miss(key.clone());
            }
        }

        if let Ok(mut complete) = flight.complete.lock() {
            *complete = true;
            flight.ready.notify_all();
        }
        if let Ok(mut flights) = self.transaction_flights.lock()
            && flights
                .get(&key)
                .is_some_and(|active| Arc::ptr_eq(active, &flight))
        {
            flights.remove(&key);
        }
        destination
    }

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
        let intent = classify_query(query);
        let global_english_news = prefers_global_english_news(query);
        let params = SearchParams {
            locale: if global_english_news {
                Some("en-US".to_string())
            } else {
                filters.locale.clone()
            },
            gl: if global_english_news {
                Some("us".to_string())
            } else {
                filters.gl.clone()
            },
            site: filters.site.clone(),
            // Current-event retrieval should use the provider's news/fresh
            // corpus even when the caller did not guess a window explicitly.
            // A month is wide enough for sparse topics; final ranking and
            // evidence grouping still decide which dated claim is current.
            freshness: filters.freshness.clone().or_else(|| {
                ((intent == QueryIntent::CurrentEvents
                    && !current_query_allows_undated_display(query))
                    || intent == QueryIntent::DueDiligence)
                    .then(|| "month".to_string())
            }),
        };
        let discovery_queries = external_discovery_queries(query);
        // Stage 1 must preserve enough depth from every configured discovery
        // provider for Stage 2 to make a real reliance choice. Asking the
        // orchestrator for only `top_k` total candidates means a two-provider
        // interleave keeps roughly half of each provider's top-k and discards
        // potentially stronger sources before relevance/trust evaluation.
        // The final user-visible slate remains `top_k`; this only widens the
        // bounded internal candidate pool.
        let discovery_limit = discovery_pool_limit(top_k, self.search_engines.len());

        // Enrich local results in parallel with external engine HTTP calls.
        // Enrichment takes ~28ms (ONNX batch embed of sentences), external engines
        // take ~900ms (HTTP). The ONNX session is released long before merge needs it.
        let (mut local_results, mut external_results) = std::thread::scope(|s| {
            let enrich_handle = s.spawn(|| {
                let mut results = local_results;
                // Navigation, action, and the current conservative diligence
                // policy use URL/title/snippet plus already-retained metadata.
                // Distilling dozens of full local documents before the merge
                // adds seconds without changing those decisions; the final
                // top-k is still enriched below when relevant.
                if !matches!(
                    intent,
                    QueryIntent::Navigational
                        | QueryIntent::Transactional
                        | QueryIntent::Tool
                        | QueryIntent::DueDiligence
                        | QueryIntent::CurrentEvents
                        | QueryIntent::Factual
                ) {
                    self.enrich_content_relevant(&mut results, &query_embedding);
                }
                results
            });

            let external: Vec<SearchResultItem> = std::thread::scope(|query_scope| {
                let handles: Vec<_> = discovery_queries
                    .iter()
                    .map(|discovery_query| {
                        query_scope.spawn(|| {
                            crate::engines::search_with_engines(
                                &self.search_engines,
                                discovery_query,
                                discovery_limit,
                                &params,
                            )
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .filter_map(|handle| handle.join().ok())
                    .flatten()
                    .collect()
            });

            let enriched_local = enrich_handle.join().expect("enrich thread panicked");
            (enriched_local, external)
        });

        if matches!(
            intent,
            QueryIntent::Navigational | QueryIntent::Transactional
        ) && !external_results
            .iter()
            .any(|result| navigation_entity_host_match(query, result))
            && let Some(destination) = resolve_direct_navigation(query)
        {
            external_results.push(destination);
        }

        if intent == QueryIntent::Transactional {
            // An engine may expose an action-looking deep path that is stale
            // or synthesized. Preserve a same-host canonical root alongside
            // it so the transactional safety gate can reject the malformed
            // route without losing the verified first-party destination.
            let derived_root = external_results.iter().find_map(|result| {
                navigation_entity_host_match(query, result)
                    .then(|| canonical_root_url(&result.url))
                    .flatten()
                    .filter(|root| {
                        !external_results
                            .iter()
                            .any(|candidate| candidate.url == *root)
                    })
                    .map(|root| {
                        let mut destination = result.clone();
                        destination.result_id = format!("transaction-root:{root}");
                        destination.url = root;
                        destination.content_md.clear();
                        destination.content_relevant = None;
                        destination.extraction_quality = "discovery_metadata".to_string();
                        destination.source = "navigation_derived".to_string();
                        destination.crawled_at = None;
                        destination.published_at = None;
                        destination.facts = None;
                        destination.decision = None;
                        destination
                    })
            });
            if let Some(derived_root) = derived_root {
                external_results.push(derived_root);
            }

            let route_candidates = local_results
                .iter()
                .chain(external_results.iter())
                .cloned()
                .collect::<Vec<_>>();
            if let Some(destination) =
                self.resolve_transaction_destination(query, &route_candidates)
            {
                // Prefer the fetched marker over an identical unverified SERP
                // URL so intent scoring can distinguish the route check.
                external_results.retain(|result| result.url != destination.url);
                external_results.insert(0, destination);
                if !sources_used
                    .iter()
                    .any(|source| source == "transaction_route")
                {
                    sources_used.push("transaction_route".to_string());
                }
            }
        }

        if intent == QueryIntent::DueDiligence {
            // A newly launched brand may be absent from literal risk-query
            // SERPs while older homonymous funds dominate. Use a likely
            // first-party title only to disambiguate a second discovery pass;
            // the destination itself remains excluded from evidence below.
            let destination = external_results
                .iter()
                .find(|result| exact_entity_host_match(query, result))
                .cloned()
                .or_else(|| {
                    let entity = entity_words(query, intent).join(" ");
                    (!entity.is_empty())
                        .then(|| resolve_direct_navigation(&entity))
                        .flatten()
                });
            if let Some(destination) = destination {
                let context_terms = diligence_context_terms(query, &destination.title);
                let context_queries = diligence_context_queries(query, &destination.title);
                let context_depth = top_k.min(3);
                let initial_context_depth = external_results
                    .iter()
                    .filter(|result| {
                        exact_entity_host_match(query, result)
                            || diligence_candidate_matches_context(query, result, &context_terms)
                    })
                    .take(context_depth)
                    .count();
                // Contextual discovery is a recovery pass, not a mandatory
                // tax on every diligence query. If the literal search already
                // exposes a useful exact-namespace slate, filtering it below
                // is sufficient and a second provider deadline adds no recall.
                if initial_context_depth < context_depth {
                    let mut contextual_results: Vec<_> = std::thread::scope(|scope| {
                        let handles: Vec<_> = context_queries
                            .iter()
                            .map(|context_query| {
                                scope.spawn(|| {
                                    crate::engines::search_with_engines(
                                        &self.search_engines,
                                        context_query,
                                        discovery_limit,
                                        &params,
                                    )
                                })
                            })
                            .collect();
                        handles
                            .into_iter()
                            .filter_map(|handle| handle.join().ok())
                            .flatten()
                            .collect()
                    });
                    external_results.append(&mut contextual_results);
                }

                if !context_terms.is_empty() {
                    // An exact name match is not an identity match. Apply the
                    // namespace derived from the first-party title to both
                    // retained and newly discovered candidates, otherwise a
                    // previously crawled homonym can survive every restart.
                    local_results.retain(|result| {
                        diligence_candidate_matches_context(query, result, &context_terms)
                    });
                    external_results.retain(|result| {
                        exact_entity_host_match(query, result)
                            || diligence_candidate_matches_context(query, result, &context_terms)
                    });
                    if !sources_used
                        .iter()
                        .any(|source| source == "entity_disambiguation")
                    {
                        sources_used.push("entity_disambiguation".to_string());
                    }
                }
            }
        }

        if !external_results.is_empty() {
            // Track which engines contributed
            for r in &external_results {
                if !sources_used.contains(&r.source) {
                    sources_used.push(r.source.clone());
                }
            }
            if external_results
                .iter()
                .any(|result| result.extraction_quality == "external_cache")
                && !sources_used
                    .iter()
                    .any(|source| source == "discovery_cache")
            {
                // This is explicitly a short-lived provider-SERP cache, not
                // retained claim evidence. Claim-bearing answers still crawl
                // and reconstruct from the local index before citing.
                sources_used.push("discovery_cache".to_string());
            }
        }

        if intent == QueryIntent::Navigational {
            let key = navigation_entity_key(query);
            if !key.is_empty()
                && let Some((result, root)) = external_results
                    .iter()
                    .find(|result| navigation_entity_host_match(query, result))
                    .and_then(|result| canonical_root_url(&result.url).map(|root| (result, root)))
                && let Ok(mut cache) = self.navigation_cache.lock()
            {
                cache.remember(key, result, root);
            }
        } else if intent == QueryIntent::Transactional
            && let Some(key) = transaction_destination_key(query)
            && let Some(result) = external_results.iter().find(|result| {
                result.extraction_quality == "verified_transaction_route"
                    && navigation_entity_host_match(query, result)
                    && transaction_destination_matches_query(query, &result.url)
            })
            && let Ok(mut cache) = self.navigation_cache.lock()
        {
            cache.remember(key, result, result.url.clone());
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
        // Local results are already enriched; only enrich external results that
        // made it into the final set (they have empty content_md, so this is a no-op
        // in practice — but handles the case where external results have content).
        if !matches!(
            intent,
            QueryIntent::Navigational
                | QueryIntent::Transactional
                | QueryIntent::Tool
                | QueryIntent::DueDiligence
                | QueryIntent::CurrentEvents
                | QueryIntent::Factual
        ) {
            self.enrich_content_relevant(&mut merged, &query_embedding);
        }
        apply_decision_policy(query, &mut merged);
        if record_usage {
            self.record_leaderboard(&merged);
            self.attribute_usage(&merged);
        }

        // Crawl-on-query: queue external results for background indexing so
        // every query permanently grows local coverage. Drained by
        // `process_crawl_queue` (callers decide when to pay the crawl cost).
        if self.config.crawl_on_query {
            let mut pending = self.urls_to_crawl(&merged, self.config.crawl_on_query_max_urls);
            if intent == QueryIntent::Navigational
                && let Some(root) = merged
                    .iter()
                    .find(|result| navigation_entity_host_match(query, result))
                    .and_then(|result| canonical_root_url(&result.url))
                && !pending.contains(&root)
            {
                // Learn the canonical first-party origin even when the engine
                // initially returns /features or another deep landing page.
                pending.insert(0, root);
                pending.truncate(self.config.crawl_on_query_max_urls);
            }
            pending = self.exclude_recently_retained_background_urls(query, pending);
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

    fn discover_external_candidates(
        &self,
        query: &str,
        limit: usize,
        filters: &QueryFilters,
    ) -> Vec<SearchResultItem> {
        if self.search_engines.is_empty() {
            return Vec::new();
        }
        let params = SearchParams {
            locale: filters.locale.clone(),
            gl: filters.gl.clone(),
            site: filters.site.clone(),
            freshness: filters.freshness.clone().or_else(|| {
                (classify_query(query) == QueryIntent::CurrentEvents).then(|| "month".to_string())
            }),
        };
        crate::engines::search_with_engines(&self.search_engines, query, limit, &params)
    }

    /// Retrieve freshly hydrated evidence within the exact source domains
    /// requested by an answer-expansion pass. This keeps primary evidence
    /// available to answer construction even when a busy, growing shared
    /// index would rank that page below the global evidence window. It does
    /// not alter the user-visible retrieval slate or assign an authority
    /// bonus; the normal entailment and cross-source compatibility gates still
    /// decide whether the page can become a citation.
    fn local_evidence_for_urls(
        &self,
        query: &str,
        urls: &[String],
        per_site_limit: usize,
    ) -> Result<Vec<SearchResultItem>> {
        let query_embedding = self.embedder.embed_query(query)?;
        let mut domains = std::collections::HashSet::new();
        let mut seen_urls = std::collections::HashSet::new();
        let mut evidence = Vec::new();
        for domain in urls.iter().map(|url| extract_domain(url)) {
            if domain.is_empty() || !domains.insert(domain.clone()) {
                continue;
            }
            let filters = QueryFilters {
                site: Some(domain),
                ..Default::default()
            };
            for result in self.index.query(
                &query_embedding,
                query,
                &filters,
                retrieval_candidate_depth(per_site_limit),
            )? {
                if seen_urls.insert(result.url.clone()) {
                    evidence.push(result);
                    if evidence.len() == per_site_limit {
                        return Ok(evidence);
                    }
                }
            }
        }
        Ok(evidence)
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

    /// Predicate-specific selection must not redefine generic metadata/cache
    /// lookups. A fetched page can be fresh without exposing an origin answer.
    fn factual_evidence_for_exact_urls(
        &self,
        query: &str,
        urls: &[String],
        limit: usize,
    ) -> Result<Vec<SearchResultItem>> {
        let query_embedding = self.embedder.embed_query(query)?;
        let mut seen = std::collections::HashSet::new();
        let chunks = self
            .index
            .query_exact_url_chunks(&query_embedding, query, urls, limit)?;
        Ok(chunks
            .into_iter()
            .filter(|result| {
                crate::answer::best_factual_sentence(query, result).is_some()
                    && seen.insert(result.url.clone())
            })
            .collect())
    }

    /// Reapply the evidence-chunk selection after every retrieval refresh,
    /// not only initial hydration. Corroboration may otherwise replace the
    /// complete relation with a semantically higher-ranked partial chunk.
    fn restore_factual_chunks(
        &self,
        query: &str,
        results: &mut Vec<SearchResultItem>,
        limit: usize,
        trace: &mut crate::answer::AnswerTrace,
    ) -> Result<()> {
        if classify_query(query) != QueryIntent::Factual || results.is_empty() {
            return Ok(());
        }
        let started = std::time::Instant::now();
        let urls = results.iter().map(|r| r.url.clone()).collect::<Vec<_>>();
        let retained = self.factual_evidence_for_exact_urls(query, &urls, limit)?;
        trace
            .factual_chunk_selections
            .push(crate::answer::FactualChunkSelectionTrace {
                known_urls: urls.iter().collect::<std::collections::HashSet<_>>().len(),
                relation_bearing_urls: retained.len(),
                reporting_families: retained
                    .iter()
                    .map(|r| crate::query_intent::source_family(&r.url))
                    .collect::<std::collections::HashSet<_>>()
                    .len(),
            });
        prefer_exact_evidence_for_same_urls(results, &retained);
        results.extend(retained);
        apply_decision_policy(query, results);
        trace.retrieval_refresh_ms += started.elapsed().as_secs_f64() * 1_000.0;
        Ok(())
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

    /// Prefer source diversity for current-office corroboration, then fill
    /// the remaining bounded slots with alternate URLs. Multiple pages from
    /// one publisher never increase evidential independence; they only keep
    /// a transient crawl failure from forcing an avoidable abstention.
    fn office_urls_to_crawl(&self, results: &[SearchResultItem], max: usize) -> Vec<String> {
        let mut urls = self.diverse_urls_to_crawl(results, max);
        if urls.len() == max {
            return urls;
        }
        for result in results
            .iter()
            .filter(|result| result.source != "local" && result.source != "cached")
        {
            if !urls.contains(&result.url) {
                urls.push(result.url.clone());
                if urls.len() == max {
                    break;
                }
            }
        }
        urls
    }

    /// Select the smallest useful second factual crawl slate. Retained
    /// answer-bearing source families cannot provide new independence, while
    /// vague entity pages are less useful than discovery surfaces that expose
    /// the requested relation. Provider snippets are used only to schedule a
    /// fetch; they never become answer evidence.
    fn factual_completion_urls_to_crawl(
        &self,
        query: &str,
        retained: &[SearchResultItem],
        discovery: &[SearchResultItem],
        max: usize,
    ) -> Vec<String> {
        if max == 0 {
            return Vec::new();
        }
        let mut families = retained
            .iter()
            .filter(|result| crate::answer::best_factual_sentence(query, result).is_some())
            .map(|result| source_family(&result.url))
            .collect::<std::collections::HashSet<_>>();
        // Ordinary factual claims require two answer-bearing source families.
        // If one is already retained, the second pass should acquire one
        // missing family rather than wait for unrelated tail requests. An
        // incomplete enumeration may still run one additional lead and then
        // abstain if completeness cannot be established.
        let target = 2usize.saturating_sub(families.len()).max(1).min(max);
        let retained_primary = retained.iter().any(|result| {
            crate::answer::best_factual_sentence(query, result).is_some()
                && primary_source_url_matches_query(query, &result.url)
        });
        let external =
            |result: &&SearchResultItem| result.source != "local" && result.source != "cached";
        let mut urls = Vec::new();
        let push = |families: &mut std::collections::HashSet<String>,
                    urls: &mut Vec<String>,
                    result: &SearchResultItem| {
            let family = source_family(&result.url);
            if families.insert(family) {
                urls.push(result.url.clone());
            }
        };

        if !retained_primary
            && let Some(primary) = discovery.iter().filter(external).find(|result| {
                !families.contains(&source_family(&result.url))
                    && primary_source_url_matches_query(query, &result.url)
                    && crate::answer::factual_public_surface_matches_relation(query, result)
            })
        {
            push(&mut families, &mut urls, primary);
        }
        for result in discovery
            .iter()
            .filter(external)
            .filter(|result| crate::answer::factual_public_surface_matches_relation(query, result))
        {
            push(&mut families, &mut urls, result);
            if urls.len() == target {
                return urls;
            }
        }
        // If the provider exposed too few legible surfaces, fill the bounded
        // tail with ordinary in-scope leads. They still have to survive crawl,
        // retention, relation, and corroboration gates.
        for result in discovery.iter().filter(external) {
            push(&mut families, &mut urls, result);
            if urls.len() == target {
                break;
            }
        }
        urls
    }

    /// Foreground evidence acquisition for an answered query. For current
    /// events, revalidate up to two stale retained families before filling the
    /// remaining budget from new discovery. Otherwise a successful recrawl of
    /// only new external URLs leaves the actual cited pages stale forever and
    /// defeats the five-minute serving cache.
    fn answer_urls_to_crawl(
        &self,
        query: &str,
        results: &[SearchResultItem],
        max: usize,
    ) -> Vec<String> {
        let intent = classify_query(query);
        if max == 0 {
            return Vec::new();
        }

        // Give a likely first-party or public-authority record one of the
        // initial factual hydration slots. This is only crawl scheduling: the
        // fetched page must still pass relation, entity, independence, and
        // evidence gates before it can support a claim. Requiring entity/
        // relation coverage prevents an unrelated `.gov` result from being
        // promoted merely because its host looks authoritative.
        if intent == QueryIntent::Factual {
            let external =
                |result: &&SearchResultItem| result.source != "local" && result.source != "cached";
            let primary = results
                .iter()
                .filter(external)
                .filter(|result| {
                    primary_source_url_matches_query(query, &result.url)
                        && factual_answerability_coverage(query, result) >= 0.5
                })
                .max_by(|left, right| {
                    factual_answerability_coverage(query, left)
                        .total_cmp(&factual_answerability_coverage(query, right))
                        .then_with(|| right.rank.cmp(&left.rank))
                });

            let mut families = std::collections::HashSet::new();
            let mut urls = Vec::new();
            if let Some(primary) = primary {
                families.insert(source_family(&primary.url));
                urls.push(primary.url.clone());
            }
            for result in results.iter().filter(external) {
                if families.insert(source_family(&result.url)) {
                    urls.push(result.url.clone());
                    if urls.len() == max {
                        break;
                    }
                }
            }
            return urls;
        }

        if intent != QueryIntent::CurrentEvents {
            return self.diverse_urls_to_crawl(results, max);
        }

        const MAX_QUERY_CACHE_AGE_SECS: u64 = 300;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let mut families = std::collections::HashSet::new();
        let mut urls = Vec::new();
        let retained_limit = max.min(2);
        for result in results.iter().filter(|result| {
            matches!(result.source.as_str(), "local" | "cached")
                && !result.content_md.trim().is_empty()
                && result.crawled_at.is_none_or(|crawled_at| {
                    now.saturating_sub(crawled_at) > MAX_QUERY_CACHE_AGE_SECS
                })
        }) {
            if families.insert(source_family(&result.url)) {
                urls.push(result.url.clone());
                if urls.len() == retained_limit {
                    break;
                }
            }
        }

        for result in results
            .iter()
            .filter(|result| result.source != "local" && result.source != "cached")
        {
            if families.insert(source_family(&result.url)) {
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
    apply_intent_adjustments(query, &mut merged);

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

    retain_intent_scope(query, &mut merged);

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

/// Make a claim-focused exact-URL chunk authoritative over a broad-query
/// chunk from the same already-crawled page. This does not prefer one website
/// over another and does not admit a new URL; it only prevents generic
/// URL-level deduplication from throwing away the passage selected for the
/// current verification task.
fn prefer_exact_evidence_for_same_urls(
    broad_results: &mut Vec<SearchResultItem>,
    exact_evidence: &[SearchResultItem],
) {
    let exact_urls = exact_evidence
        .iter()
        .map(|result| result.url.trim_end_matches('/'))
        .collect::<std::collections::HashSet<_>>();
    broad_results.retain(|result| !exact_urls.contains(result.url.trim_end_matches('/')));
}

fn retained_exact_url_is_fresh(
    evidence: &[SearchResultItem],
    requested_urls: &[String],
    now: u64,
    max_age_secs: u64,
) -> bool {
    !requested_urls.is_empty()
        && requested_urls.iter().any(|requested| {
            evidence.iter().any(|result| {
                result.url.trim_end_matches('/') == requested.trim_end_matches('/')
                    && !result.content_md.trim().is_empty()
                    && result
                        .crawled_at
                        .is_some_and(|crawled_at| now.saturating_sub(crawled_at) <= max_age_secs)
            })
        })
}

fn unix_time_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Keep verified answer evidence first, then restore relevant discovery leads
/// for exploration when the factual answer abstains. Restored candidates are
/// never added to answer citations; their existing discovery/evidence metadata
/// remains visible so consumers can distinguish a lead from retained proof.
fn restore_display_fallback(
    answer_results: &mut Vec<SearchResultItem>,
    fallback: Vec<SearchResultItem>,
    top_k: usize,
) {
    for candidate in fallback {
        if answer_results.len() >= top_k {
            break;
        }
        if answer_results
            .iter()
            .all(|result| result.url != candidate.url)
        {
            answer_results.push(candidate);
        }
    }
    for (index, result) in answer_results.iter_mut().enumerate() {
        result.rank = (index + 1) as u32;
    }
}

/// Filtering can remove higher-ranked Stage-2 candidates after the policy has
/// run. Keep the public decision order contiguous while preserving the
/// original Stage-1 retrieval rank in metadata.
fn normalize_display_decision_ranks(results: &mut [SearchResultItem]) {
    for (index, result) in results.iter_mut().enumerate() {
        let rank = (index + 1) as u32;
        result.rank = rank;
        if let Some(decision) = &mut result.decision {
            decision.decision_rank = rank;
        }
    }
}

fn prefer_claim_citation_diversity(
    results: &mut Vec<SearchResultItem>,
    answer: &crate::answer::SearchAnswer,
) {
    let answered_units = answer
        .claims
        .iter()
        .filter_map(|claim| claim.evidence_lineage.as_ref())
        .flat_map(|lineage| lineage.underlying_unit_ids.iter().cloned())
        .collect::<std::collections::HashSet<_>>();
    let available = results
        .iter()
        .map(|result| result.url.clone())
        .collect::<std::collections::HashSet<_>>();
    let per_claim = answer
        .claims
        .iter()
        .map(|claim| {
            claim
                .citations
                .iter()
                .filter(|citation| {
                    citation.evidence_role == "claim_text"
                        && available.contains(citation.url.as_str())
                })
                .map(|citation| citation.url.clone())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let depth = per_claim.iter().map(Vec::len).max().unwrap_or(0);
    // Stage 2 deliberately elects exactly one representative for every
    // resolved poll measurement. Keep that representative visible before
    // optimizing editorial diversity; dropping it would leave a slate made
    // entirely of zero-independence derivatives even though the answer names
    // the underlying registered unit.
    let mut ordered_urls = results
        .iter()
        .filter(|result| {
            result
                .decision
                .as_ref()
                .and_then(|decision| decision.evidence_lineage.as_ref())
                .is_some_and(|lineage| {
                    lineage.representative
                        && (answered_units.is_empty() || answered_units.contains(&lineage.unit_ref))
                })
        })
        .map(|result| result.url.clone())
        .collect::<Vec<_>>();
    for citation_index in 0..depth {
        for claim_urls in &per_claim {
            if let Some(url) = claim_urls.get(citation_index)
                && !ordered_urls.contains(url)
            {
                ordered_urls.push(url.clone());
            }
        }
    }
    let priority = ordered_urls
        .iter()
        .enumerate()
        .map(|(index, url)| (url.as_str(), index))
        .collect::<std::collections::HashMap<_, _>>();
    results.sort_by_key(|result| {
        priority
            .get(result.url.as_str())
            .copied()
            .unwrap_or(usize::MAX)
    });

    let mut families = std::collections::HashSet::new();
    let mut diverse = Vec::with_capacity(results.len());
    let mut derivatives = Vec::new();
    for result in results.drain(..) {
        if families.insert(source_family(&result.url)) {
            diverse.push(result);
        } else {
            derivatives.push(result);
        }
    }
    diverse.extend(derivatives);
    *results = diverse;
}

/// Local-coverage fast path thresholds: the external engine round-trip is
/// skipped only when the best local hit is strongly on-topic (top-1) AND the
/// whole top_k is solid (k-th). A high top-1 bar prevents a page of merely
/// same-domain results (e.g. Rust std docs for "rust borrow checker") from
/// suppressing the external answer.
const LOCAL_COVERAGE_TOP1: f64 = 0.70;
const LOCAL_COVERAGE_KTH: f64 = 0.58;
// Stable creator/founder facts do not need news-like five-minute refreshes.
// One hour still bounds correction propagation while avoiding repeated live
// crawling for every process restart. Current events and prices use separate,
// substantially shorter policies.
const FACTUAL_REVALIDATION_TTL_SECS: u64 = 3_600;

fn answer_requires_synchronous_hydration(query: &str, intent: QueryIntent) -> bool {
    intent == QueryIntent::Factual
        || (intent == QueryIntent::CurrentEvents
            && (is_poll_query(query) || is_current_office_holder_query(query)))
}

fn recently_crawled_source_families(results: &[SearchResultItem], max_age_seconds: u64) -> usize {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    results
        .iter()
        .filter(|result| !result.content_md.trim().is_empty())
        .filter(|result| {
            result
                .crawled_at
                .is_some_and(|crawled_at| now.saturating_sub(crawled_at) <= max_age_seconds)
        })
        .map(|result| source_family(&result.url))
        .collect::<std::collections::HashSet<_>>()
        .len()
}

/// Candidate pool used before shared-scale and intent-aware reranking.
/// Keeps normal result pages bounded while allowing large explicit requests.
pub fn retrieval_candidate_depth(top_k: usize) -> usize {
    top_k.saturating_mul(10).max(50).min(top_k.max(100))
}

fn answer_acquisition_key(query: &str, intent: QueryIntent) -> Option<String> {
    if let Some(canonical) = canonical_answer_query(query, intent) {
        return Some(format!("poll:{canonical}"));
    }
    if intent == QueryIntent::DueDiligence {
        let entity = entity_words(query, intent).join(" ");
        if !entity.is_empty() {
            return Some(format!("diligence:{entity}"));
        }
    }
    if intent == QueryIntent::Factual {
        let terms = crate::query_intent::factual_query_terms(query);
        let relation = terms
            .iter()
            .find(|term| matches!(term.as_str(), "created" | "founded" | "invented"))?;
        let mut entity = terms
            .iter()
            .filter(|term| !matches!(term.as_str(), "created" | "founded" | "invented"))
            .cloned()
            .collect::<Vec<_>>();
        entity.sort();
        if !entity.is_empty() {
            return Some(format!("factual:{relation}:{}", entity.join(" ")));
        }
    }
    if intent == QueryIntent::CurrentEvents {
        let normalized = crate::query_intent::relevance_terms(query).join(" ");
        if !normalized.is_empty() {
            return Some(format!("current:{normalized}"));
        }
    }
    None
}

fn answered_search_cache_key(
    query: &str,
    intent: QueryIntent,
    top_k: usize,
    filters: &QueryFilters,
) -> Option<String> {
    let acquisition = answer_acquisition_key(query, intent)?;
    let filters = serde_json::to_string(filters).ok()?;
    Some(format!("{acquisition}:k={top_k}:filters={filters}"))
}

fn answered_search_is_cacheable(
    intent: QueryIntent,
    query: &str,
    answered: &crate::answer::AnsweredSearch,
) -> bool {
    match intent {
        QueryIntent::CurrentEvents if is_poll_query(query) => {
            answered.answer.status == "extractive"
                && !answered.answer.claims.is_empty()
                && !answered.poll_registry.is_empty()
        }
        QueryIntent::CurrentEvents if is_live_value_query(query) => {
            answered.answer.status == "extractive"
                && !answered.answer.claims.is_empty()
                && crate::market_data::validated_quote_records(
                    &answered.market_quotes,
                    crate::market_data::unix_time_ms(),
                )
                .is_some()
        }
        QueryIntent::DueDiligence => !answered.results.is_empty(),
        QueryIntent::Factual => {
            (answered.answer.status == "extractive" && !answered.answer.claims.is_empty())
                || crate::answer::factual_answer_is_bounded_abstention(
                    query,
                    &answered.results,
                    &answered.answer,
                )
        }
        QueryIntent::CurrentEvents => !answered.results.is_empty(),
        _ => false,
    }
}

fn factual_completion_hydration_budget_exhausted(
    intent: QueryIntent,
    needs_completion: bool,
    needs_primary_source: bool,
    initial_hydration_ms: f64,
) -> bool {
    const MAX_INITIAL_HYDRATION_BEFORE_BEST_EFFORT_COMPLETION_MS: f64 = 3_500.0;
    intent == QueryIntent::Factual
        && needs_completion
        && !needs_primary_source
        && initial_hydration_ms >= MAX_INITIAL_HYDRATION_BEFORE_BEST_EFFORT_COMPLETION_MS
}

/// Rank a discovery lead by exact quoted fragments already present in the
/// bounded follow-up queries. This never inspects or modifies trust signals;
/// it only prevents a claim-specific result from losing a foreground crawl
/// slot to a generic rewrite returned slightly earlier by the provider.
fn quoted_discovery_surface_score(queries: &[String], result: &SearchResultItem) -> usize {
    let surface_terms = crate::query_intent::relevance_terms(&format!(
        "{} {} {}",
        result.title, result.snippet, result.url
    ));
    queries
        .iter()
        .map(|query| {
            let fragments = query
                .split('"')
                .enumerate()
                .filter(|(index, _)| index % 2 == 1)
                .map(|(_, fragment)| crate::query_intent::relevance_terms(fragment))
                .filter(|terms| !terms.is_empty())
                .collect::<Vec<_>>();
            if fragments.is_empty() {
                return 0;
            }
            let matched = fragments
                .iter()
                .filter(|fragment| fragment.iter().all(|term| surface_terms.contains(term)))
                .count();
            if matched == fragments.len() {
                100 + matched * 10
            } else {
                matched
            }
        })
        .max()
        .unwrap_or(0)
}

/// Select a bounded poll-verification slate that can actually satisfy the
/// two-family gate for both first and second round. Public titles/snippets are
/// used only to balance crawl scheduling and never enter answer evidence.
fn poll_urls_to_crawl(results: &[SearchResultItem], max: usize) -> Vec<String> {
    if max == 0 {
        return Vec::new();
    }
    let external = results
        .iter()
        .filter(|result| result.source != "local" && result.source != "cached")
        .collect::<Vec<_>>();
    let mut urls = Vec::new();
    let mut selected_urls = std::collections::HashSet::new();
    let mut all_families = std::collections::HashSet::new();
    let mut first_families = std::collections::HashSet::new();
    let mut second_families = std::collections::HashSet::new();

    for wanted_first in [true, false] {
        let mut round_candidates = external
            .iter()
            .copied()
            .filter(|result| {
                let (is_first, is_second) = poll_discovery_surface_rounds(result);
                if wanted_first { is_first } else { is_second }
            })
            .collect::<Vec<_>>();
        round_candidates
            .sort_by_key(|result| std::cmp::Reverse(poll_discovery_surface_score(result)));
        for result in round_candidates {
            let (is_first, is_second) = poll_discovery_surface_rounds(result);
            let family = source_family(&result.url);
            let already_covered = if wanted_first {
                first_families.contains(&family)
            } else {
                second_families.contains(&family)
            };
            if already_covered {
                continue;
            }
            if !selected_urls.contains(&result.url) {
                if urls.len() == max {
                    break;
                }
                selected_urls.insert(result.url.clone());
                all_families.insert(family.clone());
                urls.push(result.url.clone());
            }
            if is_first {
                first_families.insert(family.clone());
            }
            if is_second {
                second_families.insert(family);
            }
            let covered = if wanted_first {
                first_families.len()
            } else {
                second_families.len()
            };
            if covered >= 2 {
                break;
            }
        }
    }

    // Preserve one bounded crawl-failure margin and provide useful fallback
    // for providers whose surfaces omit an explicit round label.
    for result in external {
        if urls.len() == max {
            break;
        }
        let family = source_family(&result.url);
        if selected_urls.insert(result.url.clone()) && all_families.insert(family) {
            urls.push(result.url.clone());
        }
    }
    urls
}

fn poll_discovery_surface_rounds(result: &SearchResultItem) -> (bool, bool) {
    let surface = format!("{} {} {}", result.title, result.snippet, result.url).to_lowercase();
    let first = [
        "primeiro turno",
        "1º turno",
        "1° turno",
        "1o turno",
        "1 turno",
        "first round",
    ]
    .iter()
    .any(|cue| surface.contains(cue));
    let second = [
        "segundo turno",
        "2º turno",
        "2° turno",
        "2o turno",
        "2 turno",
        "second round",
        "runoff",
    ]
    .iter()
    .any(|cue| surface.contains(cue));
    (first, second)
}

fn poll_discovery_surface_score(result: &SearchResultItem) -> usize {
    let surface = format!("{} {} {}", result.title, result.snippet, result.url).to_lowercase();
    let percent_pairs = surface.matches('%').count().min(3);
    let named_institute = [
        "datafolha",
        "quaest",
        "parana pesquisas",
        "atlasintel",
        "nexus",
    ]
    .iter()
    .any(|institute| surface.contains(institute));
    percent_pairs * 20 + usize::from(named_institute) * 10
}

fn canonical_answer_query(query: &str, intent: QueryIntent) -> Option<&'static str> {
    if intent != QueryIntent::CurrentEvents || !is_poll_query(query) {
        return None;
    }
    let folded = query.to_lowercase();
    if [
        "primeiro turno",
        "segundo turno",
        "1º turno",
        "2º turno",
        "1o turno",
        "2o turno",
    ]
    .iter()
    .any(|cue| folded.contains(cue))
    {
        return None;
    }
    let terms = crate::query_intent::relevance_terms(query);
    let brazil = terms
        .iter()
        .any(|term| matches!(term.as_str(), "brasil" | "brazil" | "brasileira"));
    let presidential = terms.iter().any(|term| {
        matches!(
            term.as_str(),
            "presidente" | "presidencia" | "presidencial" | "presidential"
        )
    });
    (brazil && presidential).then_some("pesquisa eleitoral presidente brasil hoje")
}

fn canonical_equivalent_query(query: &str, intent: QueryIntent) -> Option<&'static str> {
    let terms = crate::query_intent::relevance_terms(query);
    if intent == QueryIntent::CurrentEvents && is_live_value_query(query) {
        let bitcoin = terms
            .iter()
            .any(|term| matches!(term.as_str(), "bitcoin" | "btc"));
        let explicit_currency = terms.iter().any(|term| {
            matches!(
                term.as_str(),
                "brl" | "dolar" | "dollar" | "real" | "reais" | "usd"
            )
        });
        if bitcoin && !explicit_currency {
            return Some("preço do bitcoin hoje");
        }
    }
    if intent == QueryIntent::CurrentEvents && is_current_office_holder_query(query) {
        let brazil = terms
            .iter()
            .any(|term| matches!(term.as_str(), "brasil" | "brazil"));
        let president = terms
            .iter()
            .any(|term| matches!(term.as_str(), "president" | "presidente"));
        if brazil && president {
            return Some("quem é o presidente do Brasil?");
        }
    }

    if intent == QueryIntent::Exploratory && is_recommendation_query(query) {
        let portable_computer = terms.iter().any(|term| {
            matches!(
                term.as_str(),
                "laptop" | "laptops" | "notebook" | "notebooks"
            )
        });
        let software_development = terms.iter().any(|term| {
            matches!(
                term.as_str(),
                "coding"
                    | "desenvolvedor"
                    | "desenvolvedores"
                    | "desenvolvimento"
                    | "programacao"
                    | "programar"
                    | "programming"
                    | "software"
            )
        });
        if portable_computer && software_development {
            return Some("melhor notebook para programar em 2026");
        }
    }

    if intent == QueryIntent::Exploratory && crate::query_intent::is_implicit_location_query(query)
    {
        let japanese = terms.iter().any(|term| {
            matches!(
                term.as_str(),
                "japanese" | "japonesa" | "japones" | "japonesas" | "japoneses"
            )
        });
        let restaurant = terms.iter().any(|term| {
            matches!(
                term.as_str(),
                "comida" | "restaurant" | "restaurants" | "restaurante" | "restaurantes"
            )
        });
        if japanese && restaurant {
            return Some("restaurantes japoneses perto de mim");
        }
    }

    if intent == QueryIntent::Exploratory && is_reference_exploration_query(query) {
        let quantum = terms
            .iter()
            .any(|term| matches!(term.as_str(), "quantum" | "quantica" | "quantico"));
        let computing = terms
            .iter()
            .any(|term| matches!(term.as_str(), "computacao" | "computing"));
        if quantum && computing {
            return Some("o que é computação quântica");
        }
    }

    if intent == QueryIntent::Exploratory
        && terms.iter().any(|term| term == "dengue")
        && terms.iter().any(|term| {
            matches!(
                term.as_str(),
                "alarme"
                    | "grave"
                    | "graves"
                    | "medica"
                    | "medico"
                    | "procurar"
                    | "sinal"
                    | "sinais"
                    | "sintoma"
                    | "sintomas"
            )
        })
    {
        return Some("sintomas de dengue quando ir ao médico");
    }
    None
}

fn recommendation_recovery_queries(query: &str) -> Vec<String> {
    let terms = crate::query_intent::relevance_terms(query);
    let portable_computer = terms.iter().any(|term| {
        matches!(
            term.as_str(),
            "laptop" | "laptops" | "notebook" | "notebooks"
        )
    });
    let software_development = terms.iter().any(|term| {
        matches!(
            term.as_str(),
            "coding"
                | "desenvolvedor"
                | "desenvolvedores"
                | "desenvolvimento"
                | "programacao"
                | "programar"
                | "programming"
                | "software"
        )
    });
    if portable_computer && software_development {
        return vec![
            "best laptops for programming".to_string(),
            "notebook programação comprar".to_string(),
        ];
    }

    let cleaned = query
        .split_whitespace()
        .filter(|token| {
            let normalized = crate::query_intent::relevance_terms(token);
            !normalized.iter().any(|term| {
                matches!(term.as_str(), "best" | "melhor" | "melhores")
                    || (term.len() == 4 && term.chars().all(|character| character.is_ascii_digit()))
            })
        })
        .collect::<Vec<_>>()
        .join(" ");
    if cleaned.trim().is_empty() {
        Vec::new()
    } else {
        vec![format!("{cleaned} comparação review")]
    }
}

fn recommendation_source_family_count(results: &[SearchResultItem]) -> usize {
    results
        .iter()
        .map(|result| source_family(&result.url))
        .filter(|family| !family.is_empty())
        .collect::<std::collections::HashSet<_>>()
        .len()
}

fn recommendation_slate_is_sufficient(
    results: &[SearchResultItem],
    required_families: usize,
) -> bool {
    required_families == 0
        || (results.len() >= required_families
            && recommendation_source_family_count(results) >= required_families)
}

fn factual_reference_query(query: &str) -> Option<String> {
    let terms = crate::query_intent::factual_query_terms(query);
    let relation = terms
        .iter()
        .find(|term| matches!(term.as_str(), "created" | "founded" | "invented"))?;
    let entity_terms = terms
        .iter()
        .filter(|term| !matches!(term.as_str(), "created" | "founded" | "invented"))
        .cloned()
        .collect::<Vec<_>>();
    // A single short token such as `Pix` is highly ambiguous in a global
    // encyclopedia search (`PIX System`, `Moon Pix`, Cisco PIX...). The
    // general web path already has locale and authority signals; injecting
    // this corpus would spend crawl budget on homonyms. Longer or multi-token
    // entities such as Anthropic remain eligible.
    if entity_terms.is_empty() || (entity_terms.len() == 1 && entity_terms[0].chars().count() <= 3)
    {
        return None;
    }
    let entity = entity_terms.join(" ");
    let context = if relation == "invented" || query.to_ascii_lowercase().contains("invent") {
        "inventor"
    } else {
        "founders"
    };
    Some(format!("{entity} {context}"))
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

/// Shared lexical score for local and external candidates.
///
fn lexical_relevance(query: &str, result: &SearchResultItem) -> f64 {
    let query_terms = crate::query_intent::relevance_terms(query);
    if query_terms.is_empty() {
        return 0.0;
    }

    let title_terms = crate::query_intent::relevance_terms(&result.title);
    let body = if result.snippet.is_empty() {
        result.content_md.chars().take(400).collect::<String>()
    } else {
        result.snippet.clone()
    };
    let mut all_terms = title_terms.clone();
    all_terms.extend(crate::query_intent::relevance_terms(&body));

    let title_coverage = term_coverage(&query_terms, &title_terms);
    let all_coverage = term_coverage(&query_terms, &all_terms);
    0.65 * title_coverage + 0.35 * all_coverage
}

fn factual_local_coverage(query: &str, results: &[SearchResultItem]) -> bool {
    let Some(top) = results.first() else {
        return false;
    };
    let answer = crate::answer::construct_answer(query, QueryIntent::Factual, results);
    lexical_relevance(query, top) >= 0.60 && answer.status == "extractive"
}

fn term_coverage(query_terms: &[String], candidate_terms: &[String]) -> f64 {
    let matches = query_terms
        .iter()
        .filter(|query_term| {
            candidate_terms.iter().any(|candidate_term| {
                query_term == &candidate_term
                    || (query_term.len() >= 5
                        && candidate_term.len() >= 5
                        && (query_term.starts_with(candidate_term.as_str())
                            || candidate_term.starts_with(query_term.as_str())))
            })
        })
        .count();
    matches as f64 / query_terms.len().max(1) as f64
}

fn canonical_root_url(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    if !matches!(scheme, "http" | "https") {
        return None;
    }
    let host = rest.split('/').next()?.trim();
    (!host.is_empty()).then(|| format!("{scheme}://{host}/"))
}

#[derive(Clone)]
struct TransactionRouteCandidate {
    url: String,
    label: String,
    declared_by_first_party: bool,
}

fn transaction_route_candidates(
    query: &str,
    results: &[SearchResultItem],
) -> Vec<TransactionRouteCandidate> {
    let mut candidates = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for result in results {
        if !navigation_entity_host_match(query, result) {
            continue;
        }
        if !suspicious_transaction_path(query, &result.url)
            && transaction_destination_matches_query(query, &result.url)
            && seen.insert(result.url.clone())
        {
            candidates.push(TransactionRouteCandidate {
                url: result.url.clone(),
                label: result.title.clone(),
                declared_by_first_party: false,
            });
        }
        if let Some(root) = canonical_root_url(&result.url) {
            for path in transaction_route_paths(query) {
                let candidate = format!("{}{}", root.trim_end_matches('/'), path);
                if seen.insert(candidate.clone()) {
                    candidates.push(TransactionRouteCandidate {
                        url: candidate,
                        label: query.to_string(),
                        declared_by_first_party: false,
                    });
                }
            }
        }
        if candidates.len() >= 6 {
            break;
        }
    }
    candidates
}

/// Follow the action links explicitly published by an entity-matching root.
/// This supplies provenance that a guessed conventional path alone cannot:
/// the first-party destination itself declared where the action lives.
fn discover_declared_transaction_routes(
    query: &str,
    results: &[SearchResultItem],
) -> Vec<TransactionRouteCandidate> {
    const DEADLINE: std::time::Duration = std::time::Duration::from_millis(1_500);
    const ROOT_BODY_BYTES: u64 = 2 * 1024 * 1024;

    let mut roots = Vec::new();
    let mut seen_roots = std::collections::HashSet::new();
    for result in results {
        if !navigation_entity_host_match(query, result) {
            continue;
        }
        if let Some(root) = canonical_root_url(&result.url)
            && seen_roots.insert(root.clone())
        {
            roots.push(root);
        }
        if roots.len() == 2 {
            break;
        }
    }

    let mut declared = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for root in roots {
        let agent = ureq::Agent::new_with_config(
            ureq::config::Config::builder()
                .timeout_global(Some(DEADLINE))
                .timeout_resolve(Some(std::time::Duration::from_millis(750)))
                .timeout_connect(Some(std::time::Duration::from_secs(1)))
                .build(),
        );
        let Ok(mut response) = agent
            .get(&root)
            .header("User-Agent", "Mozilla/5.0 (compatible; OpenSonarX/0.1)")
            .call()
        else {
            continue;
        };
        let Ok(body) = response
            .body_mut()
            .with_config()
            .limit(ROOT_BODY_BYTES)
            .lossy_utf8(true)
            .read_to_string()
        else {
            continue;
        };
        for candidate in declared_transaction_routes_from_html(query, &root, &body) {
            if seen.insert(candidate.url.clone()) {
                declared.push(candidate);
            }
        }
    }
    // Prefer the canonical action URL over plan/campaign variants and probe
    // only one declaration for the same host+path. Completion order must not
    // accidentally route a plain login query into a paid-plan flow.
    declared.sort_by_key(|candidate| {
        url::Url::parse(&candidate.url)
            .ok()
            .map(|url| (url.query().is_some(), candidate.url.len()))
            .unwrap_or((true, candidate.url.len()))
    });
    let mut action_paths = std::collections::HashSet::new();
    declared.retain(|candidate| {
        url::Url::parse(&candidate.url)
            .ok()
            .map(|url| {
                action_paths.insert(format!(
                    "{}://{}{}",
                    url.scheme(),
                    url.host_str().unwrap_or_default(),
                    url.path()
                ))
            })
            .unwrap_or(false)
    });
    declared.truncate(4);
    declared
}

fn declared_transaction_routes_from_html(
    query: &str,
    root: &str,
    body: &str,
) -> Vec<TransactionRouteCandidate> {
    let document = scraper::Html::parse_document(body);
    let Ok(selector) = scraper::Selector::parse("a[href]") else {
        return Vec::new();
    };
    let Ok(base) = url::Url::parse(root) else {
        return Vec::new();
    };
    document
        .select(&selector)
        .filter_map(|anchor| {
            let href = anchor.value().attr("href")?;
            let label = anchor.text().collect::<Vec<_>>().join(" ");
            let action_surface = format!("{label} {href}");
            if !transaction_destination_matches_query(query, &action_surface) {
                return None;
            }
            let url = base.join(href).ok()?.to_string();
            let placeholder = external_navigation_result(url.clone(), label.clone());
            (navigation_entity_host_match(query, &placeholder)
                && !suspicious_transaction_path(query, &url)
                && transaction_destination_matches_query(query, &url))
            .then_some(TransactionRouteCandidate {
                url,
                label,
                declared_by_first_party: true,
            })
        })
        .collect()
}

enum TransactionProbeOutcome {
    Verified(SearchResultItem),
    NotFound,
    Inconclusive,
}

/// Fetch conventional action routes only on an entity-matching first-party
/// origin. A route explicitly linked by that origin or protected by 401/403 is
/// positive destination evidence; an otherwise guessed 2xx route must expose
/// the action in its HTML title, preventing a SPA catch-all page from turning
/// an invented path into a destination.
fn probe_transaction_routes(
    query: &str,
    candidates: &[TransactionRouteCandidate],
) -> (Option<SearchResultItem>, bool) {
    const DEADLINE: std::time::Duration = std::time::Duration::from_millis(1_500);
    const BODY_PREFIX_BYTES: u64 = 64 * 1024;

    let expected = candidates.len();
    let (sender, receiver) = std::sync::mpsc::channel();
    for candidate in candidates.iter().cloned() {
        let sender = sender.clone();
        let query = query.to_string();
        std::thread::spawn(move || {
            let agent = ureq::Agent::new_with_config(
                ureq::config::Config::builder()
                    .timeout_global(Some(DEADLINE))
                    .timeout_resolve(Some(std::time::Duration::from_millis(750)))
                    .timeout_connect(Some(std::time::Duration::from_secs(1)))
                    .http_status_as_error(false)
                    .build(),
            );
            let outcome = agent
                .get(&candidate.url)
                .header("User-Agent", "Mozilla/5.0 (compatible; OpenSonarX/0.1)")
                .call()
                .ok()
                .map(|mut response| {
                    let status = response.status().as_u16();
                    let final_url = response.get_uri().to_string();
                    if matches!(status, 404 | 410) {
                        return TransactionProbeOutcome::NotFound;
                    }
                    let route_matches = {
                        let placeholder = external_navigation_result(
                            final_url.clone(),
                            navigation_entity_key(&query),
                        );
                        navigation_entity_host_match(&query, &placeholder)
                            && !suspicious_transaction_path(&query, &final_url)
                            && transaction_destination_matches_query(&query, &final_url)
                    };
                    tracing::debug!(
                        query,
                        candidate = %candidate.url,
                        status,
                        final_url,
                        route_matches,
                        "transaction-route probe response"
                    );
                    if !route_matches {
                        return TransactionProbeOutcome::Inconclusive;
                    }

                    let protected_route = matches!(status, 401 | 403);
                    let mut title = String::new();
                    if (200..400).contains(&status) {
                        use std::io::Read as _;
                        let mut body = String::new();
                        let _ = response
                            .body_mut()
                            .as_reader()
                            .take(BODY_PREFIX_BYTES)
                            .read_to_string(&mut body);
                        title = html_title(&body).unwrap_or_default();
                    }
                    let titled_route = (200..400).contains(&status)
                        && transaction_destination_matches_query(&query, &title);
                    let declared_route = candidate.declared_by_first_party && status < 500;
                    if declared_route || protected_route || titled_route {
                        let action_label = if candidate.declared_by_first_party
                            && !candidate.label.trim().is_empty()
                        {
                            candidate.label.clone()
                        } else {
                            (!title.is_empty())
                                .then_some(title)
                                .unwrap_or_else(|| candidate.label.clone())
                        };
                        let display_title = format!(
                            "{} — {}",
                            transaction_entity_display_name(&query),
                            action_label
                        );
                        TransactionProbeOutcome::Verified(external_transaction_result(
                            final_url,
                            display_title,
                            status,
                        ))
                    } else {
                        TransactionProbeOutcome::Inconclusive
                    }
                })
                .unwrap_or(TransactionProbeOutcome::Inconclusive);
            let _ = sender.send(outcome);
        });
    }
    drop(sender);

    let deadline = std::time::Instant::now() + DEADLINE;
    let mut received = 0;
    let mut not_found = 0;
    while received < expected {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        let Ok(outcome) = receiver.recv_timeout(remaining) else {
            break;
        };
        received += 1;
        match outcome {
            TransactionProbeOutcome::Verified(result) => {
                tracing::info!(query, url = %result.url, "verified direct transaction route");
                return (Some(result), false);
            }
            TransactionProbeOutcome::NotFound => not_found += 1,
            TransactionProbeOutcome::Inconclusive => {}
        }
    }
    (None, received == expected && not_found == expected)
}

/// Last-resort destination resolution for multi-token brands. This is not a
/// truth or trust signal: it only proves that a likely first-party origin is
/// reachable and redirects to a host within the same conservative one-edit
/// entity boundary used by navigational ranking.
fn resolve_direct_navigation(query: &str) -> Option<SearchResultItem> {
    const DEADLINE: std::time::Duration = std::time::Duration::from_millis(1_500);
    let labels = navigation_domain_guesses(query);
    if labels.is_empty() {
        return None;
    }
    let expected = labels.len();
    let (sender, receiver) = std::sync::mpsc::channel();
    for label in labels {
        let sender = sender.clone();
        std::thread::spawn(move || {
            let candidate = format!("https://{label}.com/");
            let result = crate::engines::http_agent()
                .get(&candidate)
                .header("User-Agent", "Mozilla/5.0 (compatible; OpenSonarX/0.1)")
                .call()
                .ok()
                .and_then(|mut response| {
                    let final_url = response.get_uri().to_string();
                    let body = response.body_mut().read_to_vec().ok()?;
                    let html = String::from_utf8_lossy(&body);
                    let title = html_title(&html).unwrap_or_else(|| label.clone());
                    Some(external_navigation_result(final_url, title))
                });
            let _ = sender.send(result);
        });
    }
    drop(sender);
    let deadline = std::time::Instant::now() + DEADLINE;
    for _ in 0..expected {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        let Ok(result) = receiver.recv_timeout(remaining) else {
            break;
        };
        if let Some(result) = result
            && navigation_entity_host_match(query, &result)
            && navigation_title_supports_entity(query, &result.title)
        {
            tracing::info!(query, url = %result.url, "resolved navigational destination directly");
            return Some(result);
        }
    }
    None
}

fn html_title(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<title")?;
    let content_start = lower[start..].find('>')? + start + 1;
    let content_end = lower[content_start..].find("</title>")? + content_start;
    let title = crate::engines::strip_html_tags(&html[content_start..content_end]);
    (!title.trim().is_empty()).then(|| title.trim().to_string())
}

fn external_navigation_result(url: String, title: String) -> SearchResultItem {
    SearchResultItem {
        result_id: format!("direct-navigation:{url}"),
        rank: 0,
        url,
        title,
        snippet: "Reachable candidate origin; destination signal only.".to_string(),
        content_md: String::new(),
        semantic_score: 0.0,
        bm25_score: 0.0,
        final_score: 0.0,
        stake_truth: 0,
        stake_usd: 0,
        stake_type: "none".to_string(),
        reputation: 0.0,
        content_hash: String::new(),
        extraction_quality: "discovery_metadata".to_string(),
        source: "direct_navigation".to_string(),
        entity: None,
        content_kind: None,
        crawled_at: None,
        published_at: None,
        trust_verdict: "external".to_string(),
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

fn external_transaction_result(url: String, title: String, status: u16) -> SearchResultItem {
    let mut result = external_navigation_result(url.clone(), title);
    result.result_id = format!("verified-transaction:{url}");
    result.snippet = format!(
        "Verified conventional first-party transaction route (HTTP {status}); destination signal only."
    );
    result.extraction_quality = "verified_transaction_route".to_string();
    result.source = "transaction_route".to_string();
    result
}

fn transaction_entity_display_name(query: &str) -> String {
    entity_words(query, QueryIntent::Transactional)
        .into_iter()
        .map(|word| {
            let mut characters = word.chars();
            characters
                .next()
                .map(|first| first.to_uppercase().collect::<String>() + characters.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn canonical_entity_root_from_subdomain(query: &str, url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    if !matches!(scheme, "http" | "https") {
        return None;
    }
    let host = rest
        .split('/')
        .next()?
        .split(':')
        .next()?
        .trim_start_matches("www.")
        .to_ascii_lowercase();
    let labels: Vec<_> = host.split('.').filter(|label| !label.is_empty()).collect();
    if labels.len() < 3 {
        return None;
    }
    let country_second_level = labels.last().is_some_and(|label| label.len() == 2)
        && labels
            .get(labels.len().saturating_sub(2))
            .is_some_and(|label| matches!(*label, "com" | "edu" | "gov" | "net" | "org"));
    // Government namespaces are not ordinary brand origins. Deriving
    // `receitafederal.gov.br` from `servicos.receitafederal.gov.br` can hide
    // the current canonical portal at `gov.br/receitafederal`; let the public
    // authority path matcher and live discovery resolve those destinations.
    if country_second_level
        && labels
            .get(labels.len().saturating_sub(2))
            .is_some_and(|label| *label == "gov")
    {
        return None;
    }
    let registrable_start = labels
        .len()
        .checked_sub(if country_second_level { 3 } else { 2 })?;
    let registrable = labels[registrable_start..].join(".");
    let entity_label: String = labels[registrable_start]
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .collect();
    (entity_label == navigation_entity_key(query)).then(|| format!("{scheme}://{registrable}/"))
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
    fn recommendation_recovery_preserves_product_purpose() {
        assert_eq!(
            recommendation_recovery_queries("melhor notebook para programar em 2026"),
            vec![
                "best laptops for programming".to_string(),
                "notebook programação comprar".to_string(),
            ]
        );
        assert_eq!(
            recommendation_recovery_queries("melhor câmera para viagem 2026"),
            vec!["câmera para viagem comparação review".to_string()]
        );
    }

    #[test]
    fn recommendation_slate_requires_depth_and_source_diversity() {
        let one = vec![coverage_result(
            "https://pcworld.com/laptops",
            "Best laptops",
            "comparison",
        )];
        assert!(!recommendation_slate_is_sufficient(&one, 3));

        let duplicated_family = vec![
            coverage_result("https://pcworld.com/laptops", "Best laptops", "comparison"),
            coverage_result(
                "https://www.pcworld.com/coding",
                "Coding laptops",
                "comparison",
            ),
            coverage_result(
                "https://news.pcworld.com/programming",
                "Programming laptops",
                "comparison",
            ),
        ];
        assert!(!recommendation_slate_is_sufficient(&duplicated_family, 3));

        let diverse = vec![
            one[0].clone(),
            coverage_result("https://mashable.com/laptops", "Laptop lab", "comparison"),
            coverage_result(
                "https://digitaltrends.com/laptops",
                "Laptop testing",
                "comparison",
            ),
        ];
        assert!(recommendation_slate_is_sufficient(&diverse, 3));
    }

    #[test]
    fn equivalent_office_recommendation_and_local_queries_share_canonical_retrieval() {
        assert_eq!(
            canonical_equivalent_query(
                "pesquisa eleitoral atual para presidente do brasil",
                QueryIntent::CurrentEvents
            ),
            None,
            "a poll query must never canonicalize to the incumbent answer"
        );
        for query in [
            "quem é o presidente do Brasil?",
            "quem é o atual presidente do Brasil?",
            "atual presidente do Brasil",
            "who is the current president of Brazil?",
        ] {
            assert_eq!(
                canonical_equivalent_query(query, classify_query(query)),
                Some("quem é o presidente do Brasil?"),
                "{query}"
            );
        }
        for query in [
            "preço do bitcoin hoje",
            "cotação do bitcoin agora",
            "bitcoin price today",
            "valor atual do bitcoin",
        ] {
            assert_eq!(
                canonical_equivalent_query(query, classify_query(query)),
                Some("preço do bitcoin hoje"),
                "{query}"
            );
        }
        assert_eq!(
            canonical_equivalent_query("bitcoin price today in USD", QueryIntent::CurrentEvents),
            None,
            "an explicit currency must preserve its own quote identity"
        );
        for query in [
            "melhor notebook para programar em 2026",
            "notebook para desenvolvimento de software 2026",
            "qual laptop comprar para programação",
            "best laptop for programming 2026",
        ] {
            assert_eq!(
                canonical_equivalent_query(query, classify_query(query)),
                Some("melhor notebook para programar em 2026"),
                "{query}"
            );
        }
        for query in [
            "restaurantes japoneses perto de mim",
            "comida japonesa perto de mim",
            "restaurante japonês próximo",
            "japanese restaurants near me",
        ] {
            assert_eq!(
                canonical_equivalent_query(query, classify_query(query)),
                Some("restaurantes japoneses perto de mim"),
                "{query}"
            );
        }
        for query in [
            "o que é computação quântica",
            "como funciona computação quântica",
            "computação quântica explicada",
            "what is quantum computing",
        ] {
            assert_eq!(
                canonical_equivalent_query(query, classify_query(query)),
                Some("o que é computação quântica"),
                "{query}"
            );
        }
        for query in [
            "sintomas de dengue quando ir ao médico",
            "dengue sinais de alarme",
            "quando procurar médico por dengue",
            "sintomas graves de dengue",
        ] {
            assert_eq!(
                canonical_equivalent_query(query, classify_query(query)),
                Some("sintomas de dengue quando ir ao médico"),
                "{query}"
            );
        }
    }

    #[test]
    fn equivalent_poll_variants_share_one_answer_acquisition_flight() {
        let intent = QueryIntent::CurrentEvents;
        let canonical = answer_acquisition_key("pesquisa eleitoral presidente brasil hoje", intent);
        for variant in [
            "resultado pesquisa eleitoral presidente brasil hoje",
            "pesquisa eleitoral atual para presidente do brasil",
            "última pesquisa para presidente do brasil",
        ] {
            assert_eq!(answer_acquisition_key(variant, intent), canonical);
        }
        assert_ne!(
            answer_acquisition_key("pesquisa eleitoral presidente brasil segundo turno", intent),
            canonical,
            "an explicit round must never share the broad poll decision"
        );
        assert_ne!(
            answer_acquisition_key("quem criou o Pix?", QueryIntent::Factual),
            canonical
        );
        let diligence = answer_acquisition_key("gama fund é legítimo", QueryIntent::DueDiligence);
        for variant in [
            "gama fund é confiável",
            "gama fund é fraude",
            "gama fund é seguro",
        ] {
            assert_eq!(
                answer_acquisition_key(variant, QueryIntent::DueDiligence),
                diligence
            );
        }
        let filters = QueryFilters::default();
        assert_eq!(
            answered_search_cache_key(
                "gama fund é legítimo",
                QueryIntent::DueDiligence,
                5,
                &filters
            ),
            answered_search_cache_key("gama fund é fraude", QueryIntent::DueDiligence, 5, &filters)
        );
        assert_ne!(
            answered_search_cache_key(
                "gama fund é legítimo",
                QueryIntent::DueDiligence,
                5,
                &filters
            ),
            answered_search_cache_key(
                "gama fund é legítimo",
                QueryIntent::DueDiligence,
                10,
                &filters
            )
        );
        let founders = answer_acquisition_key("quem fundou a Anthropic?", QueryIntent::Factual);
        assert_eq!(
            answer_acquisition_key("quem são os fundadores da Anthropic?", QueryIntent::Factual),
            founders
        );
        assert_eq!(
            answer_acquisition_key("who founded Anthropic?", QueryIntent::Factual),
            founders
        );
        assert_ne!(
            answer_acquisition_key("quem criou a Anthropic?", QueryIntent::Factual),
            founders,
            "created and founded are not universally interchangeable"
        );
        assert_eq!(
            answer_acquisition_key(
                "últimas notícias de inteligência artificial hoje",
                QueryIntent::CurrentEvents
            ),
            answer_acquisition_key(
                "Últimas notícias de inteligência artificial hoje!",
                QueryIntent::CurrentEvents
            )
        );
    }

    #[test]
    fn test_shared_lexical_coverage_handles_plural_variants() {
        let query =
            crate::query_intent::relevance_terms("pesquisa eleitoral presidente brasil hoje");
        let candidate = crate::query_intent::relevance_terms(
            "Pesquisas eleitorais para presidente do Brasil saem hoje",
        );
        assert_eq!(super::term_coverage(&query, &candidate), 1.0);

        let unrelated = crate::query_intent::relevance_terms("Receita Federal imposto de renda");
        assert_eq!(super::term_coverage(&query, &unrelated), 0.0);
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

    #[test]
    fn current_answer_hydration_revalidates_stale_retained_families_first() {
        let client = OpenSonarXClient::with_embedder(
            SdkConfig::default(),
            Arc::new(MockEmbedder::default()),
        );
        let mut first = coverage_result(
            "https://news-a.test/poll",
            "Pesquisa eleitoral",
            "Alice 39% e Bruno 33%",
        );
        first.crawled_at = Some(1);
        let mut second = coverage_result(
            "https://news-b.test/poll",
            "Pesquisa eleitoral",
            "Alice 39% e Bruno 33%",
        );
        second.crawled_at = Some(1);
        let mut external = coverage_result(
            "https://news-c.test/poll",
            "Pesquisa eleitoral",
            "Alice 39% e Bruno 33%",
        );
        external.source = "bing".to_string();
        external.content_md.clear();

        let urls = client.answer_urls_to_crawl(
            "pesquisa eleitoral presidente brasil hoje",
            &[first, second, external],
            5,
        );
        assert_eq!(urls.len(), 3);
        assert_eq!(urls[0], "https://news-a.test/poll");
        assert_eq!(urls[1], "https://news-b.test/poll");
        assert_eq!(urls[2], "https://news-c.test/poll");
    }

    #[test]
    fn factual_answer_hydration_reserves_a_slot_for_a_relevant_primary_source() {
        let client = OpenSonarXClient::with_embedder(
            SdkConfig::default(),
            Arc::new(MockEmbedder::default()),
        );
        let mut results = vec![
            coverage_result(
                "https://www.gov.br/receitafederal/pt-br/assuntos",
                "Receita Federal",
                "Informações sobre tributos federais.",
            ),
            coverage_result(
                "https://news-a.test/pix",
                "A história do Pix",
                "O Pix mudou os pagamentos no Brasil.",
            ),
            coverage_result(
                "https://news-b.test/pix",
                "Quem criou o Pix",
                "Uma reportagem sobre a criação do Pix.",
            ),
            coverage_result(
                "https://news-c.test/pix",
                "Origem do Pix",
                "Como nasceu o sistema de pagamento.",
            ),
            coverage_result(
                "https://news-d.test/pix",
                "Pix no Brasil",
                "História do meio de pagamento.",
            ),
            coverage_result(
                "https://www.bcb.gov.br/estabilidadefinanceira/pix",
                "Pix",
                "O Pix foi criado pelo Banco Central do Brasil.",
            ),
        ];
        for result in &mut results {
            result.source = "bing".to_string();
            result.content_md.clear();
        }

        let urls = client.answer_urls_to_crawl("qual instituição criou o Pix?", &results, 5);
        assert_eq!(urls.len(), 5);
        assert_eq!(urls[0], "https://www.bcb.gov.br/estabilidadefinanceira/pix");
        assert!(!urls.contains(&"https://news-d.test/pix".to_string()));
    }

    #[test]
    fn factual_completion_avoids_retained_families_and_caps_the_second_batch() {
        let client = OpenSonarXClient::with_embedder(
            SdkConfig::default(),
            Arc::new(MockEmbedder::default()),
        );
        let retained = vec![coverage_result(
            "https://www.bcb.gov.br/estabilidadefinanceira/pix",
            "Pix",
            "O Pix foi criado pelo Banco Central do Brasil.",
        )];
        let mut discovery = vec![
            coverage_result(
                "https://www.bcb.gov.br/estabilidadefinanceira/pix-sobre",
                "Sobre o Pix",
                "O Pix foi criado pelo Banco Central.",
            ),
            coverage_result(
                "https://news-a.test/pix",
                "Quem criou o Pix?",
                "Uma reportagem sobre a origem do sistema.",
            ),
            coverage_result(
                "https://news-b.test/pix",
                "Pix criado pelo Banco Central",
                "História do pagamento instantâneo.",
            ),
            coverage_result(
                "https://news-c.test/pix",
                "A origem do Pix",
                "Como nasceu o sistema.",
            ),
            coverage_result(
                "https://news-d.test/pix",
                "História do Pix",
                "Panorama do meio de pagamento.",
            ),
        ];
        for result in &mut discovery {
            result.source = "bing".to_string();
            result.content_md.clear();
            result.content_relevant = None;
        }

        let urls =
            client.factual_completion_urls_to_crawl("quem criou o Pix?", &retained, &discovery, 3);
        assert_eq!(urls.len(), 1);
        assert!(!urls.iter().any(|url| url.contains("bcb.gov.br")));
        assert_eq!(urls[0], "https://news-a.test/pix");

        let cold_urls =
            client.factual_completion_urls_to_crawl("quem criou o Pix?", &[], &discovery, 3);
        assert_eq!(cold_urls.len(), 2);
        assert_eq!(
            cold_urls[0],
            "https://www.bcb.gov.br/estabilidadefinanceira/pix-sobre"
        );
        assert_eq!(cold_urls[1], "https://news-a.test/pix");
    }

    #[test]
    fn display_rank_normalization_preserves_stage_one_retrieval_rank() {
        let mut results = vec![
            coverage_result("https://a.test", "A", "Anthropic was founded by A."),
            coverage_result("https://b.test", "B", "Anthropic was founded by B."),
            coverage_result("https://c.test", "C", "Anthropic was founded by C."),
        ];
        crate::evidence::apply_decision_policy("quem fundou a Anthropic?", &mut results);
        results.remove(0);
        let retained_retrieval_ranks = results
            .iter()
            .map(|result| result.decision.as_ref().unwrap().retrieval_rank)
            .collect::<Vec<_>>();

        super::normalize_display_decision_ranks(&mut results);

        assert_eq!(
            results.iter().map(|result| result.rank).collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(
            results
                .iter()
                .map(|result| result.decision.as_ref().unwrap().decision_rank)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(
            results
                .iter()
                .map(|result| result.decision.as_ref().unwrap().retrieval_rank)
                .collect::<Vec<_>>(),
            retained_retrieval_ranks
        );
    }

    #[test]
    fn factual_completion_budget_keeps_headroom_and_never_skips_primary_repair() {
        assert!(!super::factual_completion_hydration_budget_exhausted(
            QueryIntent::Factual,
            true,
            false,
            3_499.9,
        ));
        assert!(super::factual_completion_hydration_budget_exhausted(
            QueryIntent::Factual,
            true,
            false,
            3_500.0,
        ));
        assert!(!super::factual_completion_hydration_budget_exhausted(
            QueryIntent::Factual,
            true,
            true,
            10_000.0,
        ));
    }

    #[test]
    fn only_claim_shapes_supported_by_the_foreground_path_block_on_hydration() {
        assert!(super::answer_requires_synchronous_hydration(
            "quem fundou a Anthropic?",
            QueryIntent::Factual,
        ));
        assert!(super::answer_requires_synchronous_hydration(
            "pesquisa eleitoral presidente brasil hoje",
            QueryIntent::CurrentEvents,
        ));
        assert!(super::answer_requires_synchronous_hydration(
            "quem é o presidente do Brasil?",
            QueryIntent::CurrentEvents,
        ));
        assert!(!super::answer_requires_synchronous_hydration(
            "últimas notícias de inteligência artificial hoje",
            QueryIntent::CurrentEvents,
        ));
    }

    #[test]
    fn factual_reference_query_is_relation_aware_and_entity_bounded() {
        assert_eq!(
            super::factual_reference_query("quem criou a Anthropic?").as_deref(),
            Some("anthropic founders")
        );
        assert!(super::factual_reference_query("quem inventou o Pix?").is_none());
        assert!(super::factual_reference_query("Anthropic").is_none());
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
    fn office_hydration_keeps_same_family_failure_margin_without_claiming_independence() {
        let client = OpenSonarXClient::with_embedder(
            SdkConfig::default(),
            Arc::new(MockEmbedder::default()),
        );
        let mut g1_first = coverage_result(
            "https://g1.globo.com/politica/lula-presidente-1.ghtml",
            "Lula presidente do Brasil",
            "",
        );
        g1_first.source = "bing".to_string();
        let mut g1_second = coverage_result(
            "https://g1.globo.com/politica/lula-presidente-2.ghtml",
            "Lula atual presidente",
            "",
        );
        g1_second.source = "bing".to_string();
        let mut folha = coverage_result(
            "https://folha.test/politica/lula-presidente",
            "Atual presidente do Brasil",
            "",
        );
        folha.source = "bing".to_string();

        let urls = client.office_urls_to_crawl(&[g1_first, g1_second, folha], 3);
        assert_eq!(urls.len(), 3);
        assert!(urls.iter().any(|url| url.contains("folha.test")));
        assert_eq!(
            urls.iter()
                .filter(|url| url.contains("g1.globo.com"))
                .count(),
            2
        );
    }

    #[test]
    fn factual_abstention_can_restore_discovery_without_changing_evidence_order() {
        let evidence = coverage_result(
            "https://evidence.test/anthropic-founder",
            "One Anthropic founder",
            "Dario Amodei founded Anthropic.",
        );
        let duplicate = evidence.clone();
        let second = coverage_result(
            "https://reference.test/anthropic",
            "Anthropic overview",
            "Background about Anthropic.",
        );
        let third = coverage_result(
            "https://profile.test/dario-amodei",
            "Dario Amodei profile",
            "Profile and company history.",
        );
        let mut answer_results = vec![evidence];
        restore_display_fallback(&mut answer_results, vec![duplicate, second, third], 3);
        assert_eq!(answer_results.len(), 3);
        assert_eq!(
            answer_results[0].url,
            "https://evidence.test/anthropic-founder"
        );
        assert_eq!(answer_results[1].url, "https://reference.test/anthropic");
        assert_eq!(answer_results[2].rank, 3);
    }

    #[test]
    fn exact_quoted_poll_pair_gets_the_first_verification_slot() {
        let queries = vec![
            "pesquisa eleitoral presidente resultado".to_string(),
            "\"39%\" \"33%\" lula flavio pesquisa eleitoral".to_string(),
            "(\"BR-04496/2026\" OR \"BR-00109/2026\") pesquisa resultado".to_string(),
        ];
        let exact = coverage_result(
            "https://news.test/datafolha",
            "Datafolha: Lula tem 39% e Flávio 33%",
            "",
        );
        let partial = coverage_result(
            "https://other.test/datafolha",
            "Datafolha: Lula lidera com 39%",
            "",
        );
        let generic = coverage_result(
            "https://generic.test/pesquisa",
            "Pesquisa eleitoral para presidente",
            "",
        );
        assert!(
            super::quoted_discovery_surface_score(&queries, &exact)
                > super::quoted_discovery_surface_score(&queries, &partial)
        );
        assert!(
            super::quoted_discovery_surface_score(&queries, &partial)
                > super::quoted_discovery_surface_score(&queries, &generic)
        );
    }

    #[test]
    fn broad_poll_crawl_slate_covers_two_families_per_round() {
        let mut generic = coverage_result(
            "https://generic.test/poll",
            "Pesquisa eleitoral para presidente",
            "",
        );
        generic.source = "bing".to_string();
        let mut first_a = coverage_result(
            "https://first-a.test/poll",
            "Pesquisa: 39% a 33% no primeiro turno",
            "",
        );
        first_a.source = "bing".to_string();
        let mut second_a = coverage_result(
            "https://second-a.test/poll",
            "Pesquisa: 47% a 43% no segundo turno",
            "",
        );
        second_a.source = "bing".to_string();
        let mut first_b = coverage_result(
            "https://first-b.test/poll",
            "Datafolha mostra cenário de 1º turno",
            "",
        );
        first_b.source = "bing".to_string();
        let mut second_b = coverage_result(
            "https://second-b.test/poll",
            "Datafolha mostra cenário de 2º turno",
            "",
        );
        second_b.source = "bing".to_string();

        let urls = super::poll_urls_to_crawl(&[generic, first_a, second_a, first_b, second_b], 5);
        assert_eq!(urls.len(), 5);
        assert!(urls[0].contains("first-a"));
        assert!(urls[1].contains("first-b"));
        assert!(urls[2].contains("second-a"));
        assert!(urls[3].contains("second-b"));
        assert!(urls[4].contains("generic"));
    }

    #[test]
    fn poll_display_interleaves_claims_and_moves_derivative_families_to_the_tail() {
        let citation = |url: &str, rank: u32| crate::answer::AnswerCitation {
            evidence_role: "claim_text".to_string(),
            decision_rank: rank,
            title: String::new(),
            url: url.to_string(),
            source: "local".to_string(),
            evidence_excerpt: None,
        };
        let mut results = vec![
            coverage_result("https://g1.globo.com/first", "Primeiro turno", ""),
            coverage_result("https://brasil247.com/second", "Segundo turno", ""),
            coverage_result("https://valor.globo.com/duplicate", "Derivada", ""),
            coverage_result("https://dcm.test/second", "Segundo turno", ""),
            coverage_result("https://r7.test/first", "Primeiro turno", ""),
        ];
        crate::evidence::apply_decision_policy(
            "pesquisa eleitoral presidente brasil hoje",
            &mut results,
        );
        let representative = results
            .iter_mut()
            .find(|result| result.url == "https://r7.test/first")
            .unwrap();
        representative.decision.as_mut().unwrap().evidence_lineage =
            Some(oi_types::EvidenceLineageMetadata {
                unit_kind: "poll_measurement".to_string(),
                unit_ref: "BR-TEST/2026".to_string(),
                relation: "measurement_representative".to_string(),
                cluster_size: 5,
                representative: true,
                registry_verified: true,
            });
        let answer = crate::answer::SearchAnswer {
            status: "extractive".to_string(),
            intent: "current_events".to_string(),
            summary: String::new(),
            claims: vec![
                crate::answer::AnswerClaim {
                    text: "primeiro turno".to_string(),
                    citations: vec![
                        citation("https://g1.globo.com/first", 1),
                        citation("https://r7.test/first", 5),
                    ],
                    evidence_lineage: None,
                },
                crate::answer::AnswerClaim {
                    text: "segundo turno".to_string(),
                    citations: vec![
                        citation("https://brasil247.com/second", 2),
                        citation("https://dcm.test/second", 4),
                    ],
                    evidence_lineage: None,
                },
            ],
            uncertainty_mode: "evidential_only".to_string(),
            average_ignorance: Some(0.5),
            warning: None,
        };

        super::prefer_claim_citation_diversity(&mut results, &answer);
        assert_eq!(results[0].url, "https://r7.test/first");
        assert_eq!(results[1].url, "https://g1.globo.com/first");
        assert_eq!(results[2].url, "https://brasil247.com/second");
        assert_eq!(results[3].url, "https://dcm.test/second");
        assert_eq!(results[4].url, "https://valor.globo.com/duplicate");
    }

    #[test]
    fn test_factual_local_coverage_rejects_entity_pages_without_answer_evidence() {
        let results = vec![
            coverage_result(
                "https://claude.com/",
                "Claude by Anthropic",
                "Claude is an AI assistant created by Anthropic.",
            ),
            coverage_result(
                "https://claude.com/product",
                "Claude product",
                "Learn about Claude products from Anthropic.",
            ),
        ];

        assert!(!super::factual_local_coverage(
            "quem fundou a Anthropic?",
            &results
        ));
    }

    #[test]
    fn test_factual_local_coverage_accepts_retained_independent_evidence() {
        let results = vec![
            coverage_result(
                "https://example-news.test/fundadores-anthropic",
                "Quem são os fundadores da Anthropic",
                "A Anthropic foi fundada por Dario Amodei e Daniela Amodei.",
            ),
            coverage_result(
                "https://example-reference.test/anthropic",
                "Anthropic: história e fundadores",
                "Dario e Daniela Amodei fundaram a empresa de inteligência artificial.",
            ),
        ];

        let answer = crate::answer::construct_answer(
            "quem fundou a Anthropic?",
            QueryIntent::Factual,
            &results,
        );
        assert_eq!(answer.status, "extractive", "{}", answer.summary);
        assert!(super::lexical_relevance("quem fundou a Anthropic?", &results[0]) >= 0.60);

        assert!(super::factual_local_coverage(
            "quem fundou a Anthropic?",
            &results
        ));
    }

    #[test]
    fn freshly_hydrated_primary_evidence_can_be_retrieved_by_exact_site() {
        let embedder = Arc::new(MockEmbedder::new(16));
        let client = OpenSonarXClient::with_embedder(SdkConfig::default(), embedder.clone());
        let content = "O Pix é o meio de pagamento instantâneo criado pelo Banco Central.";
        client
            .ingest(&[ContentChunk {
                chunk_id: "bcb-pix".to_string(),
                content: content.to_string(),
                embedding: Some(embedder.embed_text(content).unwrap()),
                seq: 0,
                meta: Some(ChunkMeta {
                    url: "https://www.bcb.gov.br/estabilidadefinanceira/pix".to_string(),
                    title: "Pix".to_string(),
                    domain: "www.bcb.gov.br".to_string(),
                    content_hash: "bcb-pix-hash".to_string(),
                    ..Default::default()
                }),
            }])
            .unwrap();

        let evidence = client
            .local_evidence_for_urls(
                "who created Pix?",
                &["https://www.bcb.gov.br/estabilidadefinanceira/pix".to_string()],
                10,
            )
            .unwrap();
        assert_eq!(evidence.len(), 1);
        assert!(evidence[0].url.contains("bcb.gov.br/"));
    }

    #[test]
    fn registered_authority_lookup_is_exact_within_a_shared_government_domain() {
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

    #[test]
    fn claim_focused_chunk_replaces_broad_chunk_for_the_same_url() {
        let url = "https://news.example/poll";
        let mut broad = vec![coverage_result(url, "Poll", "generic navigation chunk")];
        let exact = vec![coverage_result(
            &format!("{url}/"),
            "Poll result",
            "Lula has 39% and Flavio has 33% in the first round.",
        )];

        prefer_exact_evidence_for_same_urls(&mut broad, &exact);

        assert!(broad.is_empty());
    }

    #[test]
    fn resident_authority_requires_one_recent_retained_registered_entrypoint() {
        let now = 10_000;
        let first_url = "https://authority.example/guidance";
        let second_url = "https://authority.example/method";
        let mut first = coverage_result(first_url, "Guidance", "retained official text");
        first.crawled_at = Some(now - 60);
        assert!(retained_exact_url_is_fresh(
            &[first.clone()],
            &[first_url.to_string()],
            now,
            900
        ));
        assert!(retained_exact_url_is_fresh(
            &[first.clone()],
            &[first_url.to_string(), second_url.to_string()],
            now,
            900
        ));

        first.crawled_at = Some(now - 901);
        assert!(!retained_exact_url_is_fresh(
            &[first],
            &[first_url.to_string()],
            now,
            900
        ));
    }

    #[test]
    fn canonical_root_keeps_scheme_and_host_only() {
        assert_eq!(
            super::canonical_root_url("https://chatgpt.com/features?ref=search"),
            Some("https://chatgpt.com/".to_string())
        );
        assert_eq!(super::canonical_root_url("mailto:test@example.com"), None);
    }

    #[test]
    fn direct_navigation_title_extraction_handles_attributes_and_entities() {
        assert_eq!(
            super::html_title(
                "<html><head><title data-app=\"brand\">Gama Fund &amp; Google</title></head></html>"
            ),
            Some("Gama Fund & Google".to_string())
        );
        assert_eq!(super::html_title("<html></html>"), None);
    }

    #[test]
    fn first_party_action_links_resolve_transaction_routes_without_cross_entity_leakage() {
        let html = r#"
            <a href="https://claude.ai/login?plan=pro">Get Pro</a>
            <a href="https://claude.ai/login">Log in</a>
            <a href="https://evil.example/login">Log in elsewhere</a>
            <a href="https://claude.ai/login-Claude">Suspicious login</a>
        "#;
        let routes = super::declared_transaction_routes_from_html(
            "claude login",
            "https://claude.com/",
            html,
        );
        assert_eq!(routes.len(), 2);
        assert!(routes.iter().all(|route| route.declared_by_first_party));
        assert!(
            routes
                .iter()
                .any(|route| route.url == "https://claude.ai/login" && route.label == "Log in")
        );
        assert!(
            !routes
                .iter()
                .any(|route| route.url.contains("evil.example"))
        );
        assert!(
            !routes
                .iter()
                .any(|route| route.url.contains("login-Claude"))
        );
        assert_eq!(
            super::transaction_entity_display_name("entrar claude"),
            "Claude"
        );
    }

    #[test]
    fn canonical_navigation_root_can_be_derived_from_matching_first_party_subdomain() {
        assert_eq!(
            super::canonical_entity_root_from_subdomain(
                "perplexity",
                "https://social.perplexity.ai/feed"
            ),
            Some("https://perplexity.ai/".to_string())
        );
        assert_eq!(
            super::canonical_entity_root_from_subdomain(
                "nubank",
                "https://blog.nubank.com.br/conta"
            ),
            Some("https://nubank.com.br/".to_string())
        );
        assert_eq!(
            super::canonical_entity_root_from_subdomain(
                "python",
                "https://learnpython.org/tutorial"
            ),
            None
        );
        assert_eq!(
            super::canonical_entity_root_from_subdomain("java", "https://docs.oracle.com/java"),
            None
        );
        assert_eq!(
            super::canonical_entity_root_from_subdomain(
                "receita federal",
                "http://servicos.receitafederal.gov.br/"
            ),
            None
        );
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
    fn test_answered_search_with_zero_limit_does_no_search_work() {
        let client = OpenSonarXClient::new(SdkConfig::default());
        let response = client
            .search_answered("pesquisa eleitoral hoje", 0, &QueryFilters::default())
            .unwrap();
        assert!(response.results.is_empty());
        assert!(response.sources_used.is_empty());
        assert_eq!(response.search_passes, 0);
        assert_eq!(response.hydrated_pages, 0);
        assert_eq!(response.answer.status, "insufficient_evidence");
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
    fn background_flywheel_skips_a_recently_retained_exact_url() {
        let client = OpenSonarXClient::new(SdkConfig::default());
        let embedding = client
            .embedder()
            .embed_text("receita federal imposto de renda")
            .unwrap();
        client
            .ingest(&[ContentChunk {
                chunk_id: "recent-background-url".to_string(),
                content: "Receita Federal: orientações sobre imposto de renda".to_string(),
                embedding: Some(embedding),
                seq: 0,
                meta: Some(ChunkMeta {
                    url: "https://www.gov.br/receitafederal/".to_string(),
                    title: "Receita Federal".to_string(),
                    domain: "gov.br".to_string(),
                    content_hash: "recent-background-url-hash".to_string(),
                    crawled_at: Some(super::unix_time_secs()),
                    content_type: None,
                    entity: None,
                    content_kind: None,
                    ..Default::default()
                }),
            }])
            .unwrap();

        let pending = client.exclude_recently_retained_background_urls(
            "receita federal",
            vec![
                "https://www.gov.br/receitafederal".to_string(),
                "https://example.test/new".to_string(),
            ],
        );
        assert_eq!(pending, vec!["https://example.test/new"]);
        let urls = vec!["https://www.gov.br/receitafederal/".to_string()];
        let factual_query = "who founded Receita Federal";
        assert_eq!(
            client
                .local_evidence_for_exact_urls(factual_query, &urls, 10)
                .unwrap()
                .len(),
            1
        );
        assert!(
            client
                .factual_evidence_for_exact_urls(factual_query, &urls, 10)
                .unwrap()
                .is_empty()
        );
        assert!(
            client
                .exclude_recently_retained_background_urls(factual_query, urls)
                .is_empty(),
            "freshness must not depend on whether the origin parser finds an answer"
        );
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
