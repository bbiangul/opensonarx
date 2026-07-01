use crate::{
    ContentChunk, CrawlOutput, DisputeInfo, ProposalInfo, QueryFilters, Result, SearchResultItem,
    SignedTicket, StakeInfo,
};
use std::collections::HashMap;

/// E1: Stake Registry — owned by oi-staking, consumed by oi-index/oi-network/oi-crawl
pub trait StakeRegistry: Send + Sync {
    /// Get stake information for a domain
    fn get_stake(&self, domain: &str) -> Result<Option<StakeInfo>>;

    /// List all active (non-cooldown) stakes
    fn list_active_stakes(&self) -> Result<Vec<StakeInfo>>;

    /// Check if a domain has an active stake
    fn is_staked(&self, domain: &str) -> Result<bool>;

    /// Batch lookup: get stake info for multiple domains in one call.
    /// Default falls back to individual lookups; implementors can override
    /// for single-lock or batched I/O.
    fn get_stakes_batch(&self, domains: &[&str]) -> Result<HashMap<String, StakeInfo>> {
        let mut map = HashMap::new();
        for d in domains {
            if let Some(info) = self.get_stake(d)? {
                map.insert(d.to_string(), info);
            }
        }
        Ok(map)
    }

    /// Look up stake for an entity, falling back to its parent domain.
    /// Returns entity-level stake if found, otherwise domain-level stake.
    /// For UGC platforms, does NOT fall back to domain — domain-level staking
    /// provides zero passthrough on UGC.
    fn get_entity_stake(&self, entity: &str, domain: &str) -> Result<Option<StakeInfo>> {
        if let Some(info) = self.get_stake(entity)? {
            return Ok(Some(info));
        }
        // Don't fall back to domain-level stake for UGC platforms
        if crate::entity::EntityRef::parse(domain)
            .map(|e| e.is_ugc_platform())
            .unwrap_or(false)
        {
            return Ok(None);
        }
        self.get_stake(domain)
    }
}

/// E2: Search Index — owned by oi-index, consumed by oi-network/oi-sdk
pub trait SearchIndex: Send + Sync {
    /// Query the hybrid index with embedding + text + filters
    fn query(
        &self,
        embedding: &[f32],
        text: &str,
        filters: &QueryFilters,
        top_k: usize,
    ) -> Result<Vec<SearchResultItem>>;

    /// Ingest content chunks into the index
    fn ingest(&self, chunks: &[ContentChunk]) -> Result<()>;

    /// Remove all chunks associated with a URL. Returns the count of removed chunks.
    fn remove_by_url(&self, url: &str) -> Result<usize>;
}

/// E1: State Channel — owned by oi-staking, consumed by oi-network/oi-sdk
pub trait StateChannel: Send + Sync {
    /// Create a signed payment ticket for a payee node
    fn create_ticket(&self, amount_lamports: u64, payee_pubkey: &str) -> Result<SignedTicket>;

    /// Settle a batch of tickets on-chain, returns tx hash
    fn settle_batch(&self, tickets: &[SignedTicket]) -> Result<String>;
}

/// E4: Crawl Pipeline — owned by oi-crawl, consumed by oi-index
pub trait CrawlPipeline: Send + Sync {
    /// Crawl a URL and produce structured output
    fn crawl(&self, url: &str) -> impl std::future::Future<Output = Result<CrawlOutput>> + Send;
}

/// Dispute Registry — query on-chain dispute state
pub trait DisputeRegistry: Send + Sync {
    /// Get dispute info for a domain by nonce
    fn get_dispute(&self, domain: &str, nonce: u64) -> Result<Option<DisputeInfo>>;

    /// List active disputes
    fn list_active_disputes(&self) -> Result<Vec<DisputeInfo>>;
}

/// Governance Registry — query on-chain governance proposals
pub trait GovernanceRegistry: Send + Sync {
    /// Get proposal info by nonce
    fn get_proposal(&self, nonce: u64) -> Result<Option<ProposalInfo>>;

    /// List active proposals
    fn list_active_proposals(&self) -> Result<Vec<ProposalInfo>>;
}

/// Off-chain economics engine — compute emissions, burns, revenue shares.
///
/// Methods accept governable parameters so the off-chain engine mirrors
/// on-chain behavior when DAO governance changes economic constants.
pub trait EconomicsEngine: Send + Sync {
    /// Compute token emission for an epoch given total staked
    fn compute_emission(&self, epoch: u64, total_staked: u64) -> u64;

    /// Compute burn amount from query fees and dispute slashes.
    /// `burn_pct` — governable burn percentage (0-100, default 10).
    /// Dispute slashes are not burned at settlement (handled lazily on-chain).
    fn compute_burn(&self, query_fees: u64, dispute_slashes: u64, burn_pct: u8) -> u64;

    /// Compute channel settlement shares: (publisher, curator, sentinel, burned).
    /// Mirrors on-chain SettleChannel: `burn_pct`% burned, `sentinel_fee_pct`%
    /// to sentinels, remainder to the payee (publisher). Curators earn via
    /// emissions, not channel passthrough, so the curator share is always 0.
    /// `burn_pct` default 10, `sentinel_fee_pct` default 5.
    fn compute_revenue_shares(
        &self,
        query_revenue: u64,
        burn_pct: u8,
        sentinel_fee_pct: u8,
    ) -> (u64, u64, u64, u64);

    /// Compute emission split: (staker_portion, sentinel_portion).
    /// `staker_pct` — governable staker emission share (0-100, default 70).
    fn compute_emission_shares(&self, emission_amount: u64, staker_pct: u8) -> (u64, u64);

    /// Compute curator yield cap.
    /// `cap_bps` — governable yield cap in basis points (default 1500 = 15.00% APY).
    fn compute_curator_yield_cap(&self, stake_amount: u64, duration_secs: u64, cap_bps: u64) -> u64;

    /// Compute escrow yield for a state channel
    fn compute_escrow_yield(&self, channel_amount: u64, duration_secs: u64) -> u64;
}

/// Reranker — cross-encoder that scores (query, document) pairs for second-stage ranking.
///
/// Owned by oi-embed, consumed by oi-index. The reranker takes a query and a batch of
/// document texts and returns a relevance score for each pair. Higher scores = more relevant.
pub trait Reranker: Send + Sync {
    /// Score (query, document) pairs. Returns one f32 per document.
    /// Scores are raw logits — higher means more relevant.
    fn rerank(&self, query: &str, documents: &[&str]) -> Result<Vec<f32>>;

    /// Human-readable reranker name.
    fn name(&self) -> &str;
}

/// Embedding pipeline — owned by oi-embed, consumed by oi-sdk/oi-crawl/oi-index
pub trait EmbeddingPipeline: Send + Sync {
    /// Embed a single text string (document) into a vector.
    fn embed_text(&self, text: &str) -> Result<Vec<f32>>;

    /// Embed a search query. Retrieval models often need a special prefix
    /// for queries (e.g., "Represent this sentence for searching relevant passages: ").
    /// Default: same as `embed_text`.
    fn embed_query(&self, query: &str) -> Result<Vec<f32>> {
        self.embed_text(query)
    }

    /// Embed a batch of texts (default: sequential calls to embed_text)
    fn embed_text_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        texts.iter().map(|t| self.embed_text(t)).collect()
    }

    /// Output embedding dimension
    fn dimension(&self) -> usize;

    /// Human-readable pipeline name
    fn name(&self) -> &str;

    /// Whether this pipeline supports image embeddings
    fn supports_images(&self) -> bool {
        false
    }

    /// Embed raw image bytes into a vector
    fn embed_image(&self, _image_bytes: &[u8]) -> Result<Vec<f32>> {
        Err(crate::Error::Internal(
            "image embeddings not supported".into(),
        ))
    }
}

/// Parameters for locale/region-aware and site-scoped search.
#[derive(Debug, Clone, Default)]
pub struct SearchParams {
    /// Locale hint for external engines (e.g., "en", "pt-BR")
    pub locale: Option<String>,
    /// Country/region code (e.g., "us", "br")
    pub gl: Option<String>,
    /// Domain scope filter (e.g., "nytimes.com")
    pub site: Option<String>,
}

/// Search engine adapter — owned by oi-sdk engine modules, consumed by OpenSonarXClient.
///
/// Each implementation wraps a single external search backend (DuckDuckGo, Brave, etc.).
/// Engines are queried sequentially in priority order; results are deduped by URL.
pub trait SearchEngine: Send + Sync {
    /// Human-readable engine name (e.g., "duckduckgo", "brave").
    fn name(&self) -> &str;

    /// Execute a search query and return up to `limit` results.
    fn search(&self, query: &str, limit: usize) -> Vec<SearchResultItem>;

    /// Execute a search query with locale/region/site parameters.
    ///
    /// Default: prepends `site:` to query if set, delegates to `search()`.
    /// Override in engines with native locale support (Serper, Brave, DDG).
    fn search_with_params(&self, query: &str, limit: usize, params: &SearchParams) -> Vec<SearchResultItem> {
        let q = if let Some(ref site) = params.site {
            format!("site:{site} {query}")
        } else {
            query.to_string()
        };
        self.search(&q, limit)
    }

    /// Whether this engine requires an API key to function.
    fn requires_api_key(&self) -> bool {
        false
    }

    /// Whether this engine is currently available (has required keys, reachable, etc.).
    fn is_available(&self) -> bool {
        true
    }
}

/// Query leaderboard — tracks which domains/entities are most searched.
///
/// Used to surface staking opportunities: "these domains get queried a lot but have no stake."
pub trait QueryLeaderboard: Send + Sync {
    /// Record that domains/entities from search results were queried.
    fn record_query(&self, identifiers: &[&str]) -> Result<()>;

    /// Get top queried identifiers with their counts.
    fn top_queried(&self, limit: usize) -> Result<Vec<crate::LeaderboardEntry>>;
}

/// Reputation provider — allows the core index to fetch domain reputation scores.
///
/// Implementations may be backed by SQLite (gateway), in-memory maps (tests),
/// or remote services. The trait lives in oi-types so oi-index can depend on it
/// without pulling in gateway dependencies.
pub trait ReputationProvider: Send + Sync {
    /// Get the reputation multiplier for a single domain.
    /// Returns None if no reputation data exists (caller should default to 1.0).
    fn get_reputation(&self, domain: &str) -> Option<f64>;

    /// Get reputation score and vote count for a single domain.
    /// Returns None if no reputation data exists.
    fn get_reputation_with_votes(&self, domain: &str) -> Option<(f64, u32)> {
        self.get_reputation(domain).map(|r| (r, 0))
    }

    /// Batch-fetch reputation scores with vote counts for multiple domains.
    /// Returns (score, vote_count) per domain.
    /// Default implementation falls back to individual lookups.
    fn get_reputations_batch(&self, domains: &[&str]) -> HashMap<String, (f64, u32)> {
        let mut map = HashMap::new();
        for d in domains {
            if let Some(rep) = self.get_reputation_with_votes(d) {
                map.insert(d.to_string(), rep);
            }
        }
        map
    }
}

/// Fact extraction pipeline — owned by oi-facts, consumed by oi-crawl.
///
/// Implementations extract named entities and structured fact triplets from text.
/// The trait is zero-shot: entity/relation types are configurable at runtime.
pub trait FactExtractionPipeline: Send + Sync {
    /// Extract entities and facts from a single text.
    fn extract(&self, text: &str) -> Result<crate::facts::FactExtractionResult>;

    /// Extract from a batch of texts. Default: sequential calls to `extract`.
    fn extract_batch(&self, texts: &[&str]) -> Result<Vec<crate::facts::FactExtractionResult>> {
        texts.iter().map(|t| self.extract(t)).collect()
    }

    /// Entity types this pipeline can detect.
    fn supported_entity_types(&self) -> &[&str];

    /// Relation types this pipeline can detect.
    fn supported_relation_types(&self) -> &[&str];

    /// Human-readable pipeline name.
    fn name(&self) -> &str;
}

/// Metrics collector for Prometheus-style instrumentation
pub trait MetricsCollector: Send + Sync {
    /// Record a query with its latency
    fn record_query(&self, latency_ms: f64);

    /// Record a document ingestion
    fn record_ingest(&self, doc_count: u64);

    /// Record an error
    fn record_error(&self);

    /// Get current metrics snapshot
    fn snapshot(&self) -> crate::MetricsSnapshot;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct MockRegistry {
        stakes: HashMap<String, StakeInfo>,
    }

    impl StakeRegistry for MockRegistry {
        fn get_stake(&self, domain: &str) -> Result<Option<StakeInfo>> {
            Ok(self.stakes.get(domain).cloned())
        }
        fn list_active_stakes(&self) -> Result<Vec<StakeInfo>> {
            Ok(self.stakes.values().cloned().collect())
        }
        fn is_staked(&self, domain: &str) -> Result<bool> {
            Ok(self.stakes.contains_key(domain))
        }
    }

    fn make_info(domain: &str) -> StakeInfo {
        StakeInfo {
            domain: domain.to_string(),
            amount_lamports: 1_000_000,
            amount_usd: 100.0,
            staked_at: 0,
            cooldown_until: None,
            wallet: "test".to_string(),
            dns_verified: false,
            reputation: 1.0,
            entity: None,
            disputed: false,
        }
    }

    #[test]
    fn test_entity_stake_found_returns_entity() {
        let mut stakes = HashMap::new();
        stakes.insert("youtube.com/c/test".to_string(), make_info("youtube.com/c/test"));
        stakes.insert("youtube.com".to_string(), make_info("youtube.com"));
        let reg = MockRegistry { stakes };

        let result = reg.get_entity_stake("youtube.com/c/test", "youtube.com").unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().domain, "youtube.com/c/test");
    }

    #[test]
    fn test_entity_not_found_non_ugc_falls_back_to_domain() {
        let mut stakes = HashMap::new();
        stakes.insert("nytimes.com".to_string(), make_info("nytimes.com"));
        let reg = MockRegistry { stakes };

        // Entity not found, domain is NOT UGC → falls back to domain
        let result = reg.get_entity_stake("nytimes.com/2024/article", "nytimes.com").unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().domain, "nytimes.com");
    }

    #[test]
    fn test_entity_not_found_ugc_no_fallback() {
        let mut stakes = HashMap::new();
        stakes.insert("youtube.com".to_string(), make_info("youtube.com"));
        let reg = MockRegistry { stakes };

        // Entity not found, domain IS UGC → does NOT fall back
        let result = reg.get_entity_stake("youtube.com/c/unknown", "youtube.com").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_neither_entity_nor_domain_returns_none() {
        let reg = MockRegistry { stakes: HashMap::new() };
        let result = reg.get_entity_stake("example.com/path", "example.com").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_get_stakes_batch_default() {
        let mut stakes = HashMap::new();
        stakes.insert("a.com".to_string(), make_info("a.com"));
        stakes.insert("b.com".to_string(), make_info("b.com"));
        let reg = MockRegistry { stakes };

        let batch = reg.get_stakes_batch(&["a.com", "b.com", "c.com"]).unwrap();
        assert_eq!(batch.len(), 2);
        assert!(batch.contains_key("a.com"));
        assert!(batch.contains_key("b.com"));
        assert!(!batch.contains_key("c.com"));
    }
}
