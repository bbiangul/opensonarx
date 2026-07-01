use serde::{Deserialize, Serialize};

/// Information about a domain's stake
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StakeInfo {
    pub domain: String,
    pub amount_lamports: u64,
    pub amount_usd: f64,
    pub staked_at: i64,
    pub cooldown_until: Option<i64>,
    pub wallet: String,
    pub dns_verified: bool,
    pub reputation: f64,
    /// Full entity ref for entity-level stakes (None for domain-only stakes)
    pub entity: Option<String>,
    /// Whether this stake has an active dispute
    #[serde(default)]
    pub disputed: bool,
}

/// A search result returned from the index
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResultItem {
    pub result_id: String,
    pub rank: u32,
    pub url: String,
    pub title: String,
    pub snippet: String,
    pub content_md: String,
    pub semantic_score: f64,
    pub bm25_score: f64,
    pub final_score: f64,
    pub stake_truth: u64,
    pub stake_usd: u64,
    pub stake_type: String,
    pub reputation: f64,
    pub content_hash: String,
    pub extraction_quality: String,
    pub source: String,
    /// Entity ref for entity-level content
    pub entity: Option<String>,
    /// Content kind: "video", "post", etc.
    pub content_kind: Option<String>,
    /// When OpenSonarX last crawled this content (Unix seconds)
    #[serde(default)]
    pub crawled_at: Option<u64>,
    /// When the content was originally published (Unix seconds)
    #[serde(default)]
    pub published_at: Option<i64>,
    /// Trust verdict: "highly_trusted", "trusted", "neutral", "unverified", "disputed"
    #[serde(default)]
    pub trust_verdict: String,
    /// Number of reputation feedback votes for this domain
    #[serde(default)]
    pub reputation_votes: u32,
    /// Whether this domain has an active stake dispute
    #[serde(default)]
    pub disputed: bool,
    /// Extracted facts and entities (populated by oi-facts at crawl time)
    #[serde(default)]
    pub facts: Option<crate::facts::FactExtractionResult>,
    /// Condensed content_md: only sentences relevant to the query.
    /// Uses entity offsets + keyword matching. None for external results or when no signal found.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_relevant: Option<String>,

    // --- Domain-level metadata (helps agents reason about source trustworthiness) ---

    /// When OpenSonarX first observed this domain (Unix seconds). None for external-only results.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain_first_seen_at: Option<i64>,
    /// How many URLs from this domain are in our index. None for external-only results.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain_url_count: Option<i64>,
    /// Domain-level reputation score (Wilson lower bound from user feedback). None if no votes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain_reputation_score: Option<f64>,
    /// Total feedback votes for this domain across all epochs. None if no votes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain_total_votes: Option<u64>,
}

/// Entry in the most-queried-domains leaderboard.
///
/// Unified: publishers appear as `"nytimes.com"`, UGC creators as `"youtube.com/c/3blue1brown"`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaderboardEntry {
    /// Domain or entity identifier (e.g., "nytimes.com" or "youtube.com/c/3blue1brown")
    pub identifier: String,
    /// Total number of queries that returned results from this domain/entity
    pub query_count: u64,
    /// Current stake in $TRUTH lamports (enriched at read time)
    pub stake_truth: u64,
    /// Current stake in USD (enriched at read time)
    pub stake_usd: f64,
    /// Whether this is an entity-level entry (UGC creator) vs domain
    pub is_entity: bool,
    /// Content kind for entities: "video", "post", etc.
    pub content_kind: Option<String>,
}

/// Query filters matching proto Filters message
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QueryFilters {
    pub min_stake_usd: Option<u32>,
    pub freshness: Option<String>,
    pub types: Vec<String>,
    pub min_reputation: Option<f32>,
    /// Minimum trust verdict to include, e.g. "neutral", "trusted", "highly_trusted".
    /// Results whose `compute_verdict` rank is below this are dropped. Stake/reputation
    /// inform this label but never the ranking score (capital must not buy rank).
    #[serde(default)]
    pub min_trust: Option<String>,
    pub shards: Vec<String>,
    /// Filter by entity refs (e.g., "youtube.com/c/3blue1brown")
    pub entities: Vec<String>,
    /// Filter by content kinds (e.g., "video", "post")
    pub content_kinds: Vec<String>,
    /// Locale hint for external engines (e.g., "en", "pt-BR")
    #[serde(default)]
    pub locale: Option<String>,
    /// Country/region code for geo-scoped search (e.g., "us", "br")
    #[serde(default)]
    pub gl: Option<String>,
    /// Domain scope filter (e.g., "nytimes.com")
    #[serde(default)]
    pub site: Option<String>,
}

impl QueryFilters {
    /// Convert from protobuf Filters message.
    pub fn from_proto(f: &crate::proto::opensonarx::Filters) -> Self {
        Self {
            min_stake_usd: if f.min_stake_usd > 0 { Some(f.min_stake_usd) } else { None },
            freshness: if f.freshness.is_empty() { None } else { Some(f.freshness.clone()) },
            types: f.types.clone(),
            min_reputation: if f.min_reputation > 0.0 { Some(f.min_reputation) } else { None },
            min_trust: if f.min_trust.is_empty() { None } else { Some(f.min_trust.clone()) },
            shards: f.shards.clone(),
            entities: f.entities.clone(),
            content_kinds: f.content_kinds.clone(),
            locale: if f.locale.is_empty() { None } else { Some(f.locale.clone()) },
            gl: if f.gl.is_empty() { None } else { Some(f.gl.clone()) },
            site: if f.site.is_empty() { None } else { Some(f.site.clone()) },
        }
    }

    /// Convert to protobuf Filters message (inverse of `from_proto`).
    pub fn to_proto(&self) -> crate::proto::opensonarx::Filters {
        crate::proto::opensonarx::Filters {
            min_stake_usd: self.min_stake_usd.unwrap_or(0),
            freshness: self.freshness.clone().unwrap_or_default(),
            types: self.types.clone(),
            min_reputation: self.min_reputation.unwrap_or(0.0),
            shards: self.shards.clone(),
            entities: self.entities.clone(),
            content_kinds: self.content_kinds.clone(),
            locale: self.locale.clone().unwrap_or_default(),
            gl: self.gl.clone().unwrap_or_default(),
            site: self.site.clone().unwrap_or_default(),
            min_trust: self.min_trust.clone().unwrap_or_default(),
        }
    }
}

/// Output from the crawl/distiller pipeline
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawlOutput {
    pub url: String,
    pub markdown: String,
    pub chunks: Vec<ContentChunk>,
    pub metadata: CrawlMetadata,
    pub content_hash: String,
    pub extraction_quality: f64,
}

/// Metadata attached to a content chunk for search result population
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChunkMeta {
    pub url: String,
    pub title: String,
    pub domain: String,
    pub content_hash: String,
    pub crawled_at: Option<u64>,
    pub content_type: Option<String>,
    /// Entity ref for entity-level content (e.g., "youtube.com/c/3blue1brown")
    pub entity: Option<String>,
    /// Content kind: "video", "post", "comment", etc. (None for regular web content)
    pub content_kind: Option<String>,
    /// Quality score 0.0-1.0 (word count / 500, capped)
    pub quality_score: Option<f64>,
    /// Spam score 0.0-1.0 (keyword density, trigrams, caps)
    pub spam_score: Option<f64>,
    /// AI slop score 0.0-1.0 (formulaic patterns, low diversity)
    pub slop_score: Option<f64>,
    /// Content publish date (Unix seconds), if extracted
    pub published_at: Option<i64>,
    /// Topic-based shard ID assigned by the TopicRouter (e.g., "topic-technology")
    #[serde(default)]
    pub shard_id: Option<String>,
    /// Extracted facts and entities (populated by oi-facts pipeline)
    #[serde(default)]
    pub facts: Option<crate::facts::FactExtractionResult>,
}

/// A content chunk from the crawl pipeline
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentChunk {
    pub chunk_id: String,
    pub content: String,
    pub embedding: Option<Vec<f32>>,
    pub seq: u64,
    /// Optional metadata for search result population and filtering
    pub meta: Option<ChunkMeta>,
}

/// Metadata from the crawl pipeline
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawlMetadata {
    pub title: Option<String>,
    pub description: Option<String>,
    pub language: Option<String>,
    pub published_at: Option<i64>,
    pub content_type: String,
    pub fetch_time_ms: u64,
}

/// A signed state channel ticket (matches proto StateChannelTicket)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedTicket {
    pub payer_pubkey: String,
    pub payee_pubkey: String,
    pub amount_lamports: u64,
    pub nonce: u64,
    pub signature: Vec<u8>,
    pub expiry_slot: u64,
}

// ── Protocol Economics Types ──

/// Information about an active dispute
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisputeInfo {
    pub domain: String,
    pub reporter: String,
    pub bond: u64,
    pub reason_hash: [u8; 32],
    pub status: String, // "active", "resolved_guilty", "resolved_innocent"
    pub votes_guilty: u64,
    pub votes_innocent: u64,
    pub jury_count: u32,
    pub created_at: i64,
    pub deadline: i64,
    pub nonce: u64,
}

/// Information about a governance proposal
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProposalInfo {
    pub proposer: String,
    pub description_hash: [u8; 32],
    pub param_key: String,
    pub param_value: u64,
    pub votes_for: u64,
    pub votes_against: u64,
    pub status: String, // "active", "executed", "failed"
    pub created_at: i64,
    pub voting_ends: i64,
    pub nonce: u64,
}

/// Reward information for a staker
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RewardInfo {
    pub domain: String,
    pub staker: String,
    pub accrued: u64,
    pub claimed: u64,
    pub reward_debt: u128,
}

// ── P2P & Network Types ──

/// Information about a connected peer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInfo {
    pub peer_id: String,
    pub addr: String,
    pub shard_ids: Vec<String>,
    pub vectors_stored: u64,
    pub last_heartbeat_ms: u64,
    pub uptime_pct: f64,
}

/// Shard assignment for a node
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShardAssignment {
    pub shard_id: String,
    pub node_id: String,
    pub vector_count: u64,
    pub last_sync_block: u64,
}

/// Health status of a node or service
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthStatus {
    pub healthy: bool,
    pub uptime_secs: u64,
    pub doc_count: u64,
    pub query_count: u64,
    pub avg_latency_ms: f64,
    pub version: String,
}

/// Snapshot of metrics for Prometheus export
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MetricsSnapshot {
    pub queries_total: u64,
    pub queries_latency_sum_ms: f64,
    pub ingest_total: u64,
    pub docs_stored: u64,
    pub peers_connected: u64,
    pub errors_total: u64,
}
