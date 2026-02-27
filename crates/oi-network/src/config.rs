use serde::{Deserialize, Serialize};

/// Default bootstrap peers for the OpenSonarX network.
/// Empty until official seed nodes are deployed. Nodes start in local
/// discovery mode (mDNS) and can join the network with `--bootstrap`.
pub const DEFAULT_BOOTSTRAP_PEERS: &[&str] = &[];

/// Default DNS hostname for bootstrap peer resolution.
/// Empty until official DNS bootstrap is deployed.
pub const DEFAULT_DNS_BOOTSTRAP: &str = "";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    pub listen_port: u16,
    pub bootstrap_peers: Vec<String>,
    pub heartbeat_interval_secs: u64,
    pub heartbeat_timeout_secs: u64,
    /// Timeout for query request-response exchanges (seconds).
    pub query_timeout_secs: u64,
    /// Maximum number of peers to fan out a query to.
    pub max_query_fanout: usize,
    /// Duration (seconds) after which a peer with no heartbeat is evicted.
    pub stale_peer_timeout_secs: u64,
    /// Minimum documents a peer must have to be prioritized for queries.
    /// Peers with fewer docs are deprioritized (ranked last) in peer selection.
    pub min_docs_for_queries: u64,
    /// Target coverage ratio (0.0-1.0) for adaptive fanout.
    /// If estimated coverage falls below this, fanout is increased.
    pub target_coverage: f64,
    /// Enable mDNS for local peer discovery.
    pub enable_mdns: bool,
    /// Optional DNS hostname for bootstrap peer resolution (e.g. "bootstrap.opensonarx.com").
    pub dns_bootstrap: Option<String>,
    /// Minimum number of replicas per shard before proactive replication kicks in.
    pub min_replicas_per_shard: usize,
    /// How often to check for under-replicated shards (seconds).
    pub replication_check_interval_secs: u64,
    /// Minimum contribution ratio before throttling a peer. Default 0.1 (very lenient).
    pub min_contribution_ratio: f64,
    /// Enable tit-for-tat throttling. When false, contributions are tracked but not enforced.
    pub enable_tit_for_tat: bool,
    /// Path to persist the routing table (JSON). None = no persistence.
    /// Env: OI_ROUTING_TABLE_PATH
    pub routing_table_path: Option<String>,
    /// Maximum age (seconds) for persisted routing table entries. Default 86400 (24h).
    pub routing_table_max_age_secs: u64,
    /// Enable QUIC transport (in addition to TCP). Default true.
    pub enable_quic: bool,
    /// Enable NAT traversal (AutoNAT, relay, DCUtR, UPnP). Default true.
    pub enable_nat_traversal: bool,
    /// Known relay server multiaddrs for fallback connectivity.
    pub relay_servers: Vec<String>,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            listen_port: 4001,
            bootstrap_peers: Vec::new(),
            heartbeat_interval_secs: 30,
            heartbeat_timeout_secs: 180, // 3 minutes
            query_timeout_secs: 10,
            max_query_fanout: 3,
            stale_peer_timeout_secs: 300, // 5 minutes
            min_docs_for_queries: 1000,
            target_coverage: 0.8,
            enable_mdns: true,
            dns_bootstrap: None,
            min_replicas_per_shard: 3,
            replication_check_interval_secs: 60,
            min_contribution_ratio: 0.1,
            enable_tit_for_tat: false,
            routing_table_path: None,
            routing_table_max_age_secs: 86400,
            enable_quic: true,
            enable_nat_traversal: true,
            relay_servers: Vec::new(),
        }
    }
}
