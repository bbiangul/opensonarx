use serde::{Deserialize, Serialize};
use std::fmt;

/// Configuration for a single search engine adapter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchEngineConfig {
    /// Engine name: "duckduckgo", "brave", "brave-html", "mojeek", "searxng", "google", "bing", "qwant", "yep", "stract"
    pub name: String,
    /// API key (or instance URL for SearXNG)
    pub api_key: Option<String>,
    /// Whether this engine is enabled
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Priority order (lower = higher priority, queried first)
    #[serde(default = "default_priority")]
    pub priority: u32,
}

fn default_true() -> bool {
    true
}

fn default_priority() -> u32 {
    100
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SdkConfig {
    pub bootstrap_peers: Vec<String>,
    pub wallet_path: Option<String>,
    /// Path to local HNSW/BM25 index storage
    pub local_index_path: Option<String>,
    /// Network environment: "mainnet", "devnet", or "localnet"
    pub network: String,
    /// Brave Search API key for external search fallback
    pub brave_api_key: Option<String>,
    /// Serper (Google results) API key for external search fallback.
    /// When set, Serper is the highest-priority external engine.
    pub serper_api_key: Option<String>,
    /// Configured search engines (overrides legacy API-key fields when set)
    pub search_engines: Option<Vec<SearchEngineConfig>>,
    /// Where to persist the stake registry (off-chain mirror). None (default)
    /// keeps stakes in-memory only — no hidden home-dir writes for library
    /// consumers; the CLI and node set this to ~/.opensonarx/stakes.json.
    #[serde(default)]
    pub stakes_path: Option<String>,
    /// Where to persist the creator usage ledger (agent-usage attribution →
    /// creator credits). None (default) = in-memory only.
    #[serde(default)]
    pub creator_ledger_path: Option<String>,
    /// Enable background crawl-on-query for external URLs (default false)
    #[serde(default)]
    pub crawl_on_query: bool,
    /// Max URLs to crawl per query when crawl-on-query is enabled (default 3)
    #[serde(default = "default_crawl_max")]
    pub crawl_on_query_max_urls: usize,
    /// Model that reads retained passages for `search_answered`. None
    /// (default) = no model: the answered path returns the slate unanswered.
    #[serde(default)]
    pub answer_model: Option<crate::chat_model::AnswerModelConfig>,
    /// Path to an answer calibration table (`crate::calibration`). None
    /// (default) = complete answers carry no probability.
    #[serde(default)]
    pub answer_calibration_path: Option<String>,
}

fn default_crawl_max() -> usize {
    3
}

impl fmt::Debug for SdkConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SdkConfig")
            .field("bootstrap_peers", &self.bootstrap_peers)
            .field("wallet_path", &self.wallet_path)
            .field("local_index_path", &self.local_index_path)
            .field("network", &self.network)
            .field(
                "brave_api_key",
                &self.brave_api_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "serper_api_key",
                &self.serper_api_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field("search_engines", &self.search_engines)
            .field("answer_model", &self.answer_model)
            .finish()
    }
}

impl Default for SdkConfig {
    fn default() -> Self {
        Self {
            bootstrap_peers: Vec::new(),
            wallet_path: None,
            local_index_path: None,
            network: "mainnet".to_string(),
            brave_api_key: None,
            serper_api_key: None,
            search_engines: None,
            stakes_path: None,
            creator_ledger_path: None,
            crawl_on_query: false,
            crawl_on_query_max_urls: 3,
            answer_model: None,
            answer_calibration_path: None,
        }
    }
}
