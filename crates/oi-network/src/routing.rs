use libp2p::PeerId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::time::Instant;

/// Maximum number of peers tracked in the routing table.
const MAX_PEERS: usize = 10_000;

/// Serializable subset of PeerInfo for persistence.
#[derive(Serialize, Deserialize)]
struct PersistedPeerInfo {
    node_id: String,
    peer_id: Option<String>,
    shard_ids: Vec<String>,
    vectors_stored: u64,
    last_seen_timestamp: u64,
    queries_answered_for_us: u64,
    queries_we_answered: u64,
    chunks_received: u64,
    chunks_sent: u64,
    /// Per-shard replication watermarks (shard_id -> last received seq).
    replication_watermarks: HashMap<String, u64>,
}

/// Tracks which peers serve which shards, and their liveness.
pub struct RoutingTable {
    peers: HashMap<String, PeerInfo>,
}

struct PeerInfo {
    peer_id: Option<PeerId>,
    shard_ids: Vec<String>,
    last_heartbeat: Instant,
    vectors_stored: u64,
    /// Times this peer answered our queries.
    queries_answered_for_us: u64,
    /// Times we answered this peer's queries.
    queries_we_answered: u64,
    /// Chunks we received from this peer (replication).
    chunks_received: u64,
    /// Chunks we sent to this peer (replication).
    chunks_sent: u64,
    /// Per-shard replication watermarks (shard_id -> last received seq).
    replication_watermarks: HashMap<String, u64>,
}

impl RoutingTable {
    pub fn new() -> Self {
        Self {
            peers: HashMap::new(),
        }
    }

    pub fn update_peer(&mut self, node_id: &str, shard_ids: Vec<String>, vectors_stored: u64) {
        self.update_peer_with_id(node_id, shard_ids, vectors_stored, None);
    }

    pub fn update_peer_with_id(
        &mut self,
        node_id: &str,
        shard_ids: Vec<String>,
        vectors_stored: u64,
        peer_id: Option<PeerId>,
    ) {
        // If at capacity and this is a new peer, skip the insert
        if self.peers.len() >= MAX_PEERS && !self.peers.contains_key(node_id) {
            return;
        }
        // Preserve existing peer_id, contribution counters, and watermarks if updating
        let existing = self.peers.get(node_id);
        let existing_peer_id = existing.and_then(|p| p.peer_id);
        let (qa, qw, cr, cs) = existing
            .map(|p| (p.queries_answered_for_us, p.queries_we_answered, p.chunks_received, p.chunks_sent))
            .unwrap_or((0, 0, 0, 0));
        let watermarks = existing
            .map(|p| p.replication_watermarks.clone())
            .unwrap_or_default();
        self.peers.insert(
            node_id.to_string(),
            PeerInfo {
                peer_id: peer_id.or(existing_peer_id),
                shard_ids,
                last_heartbeat: Instant::now(),
                vectors_stored,
                queries_answered_for_us: qa,
                queries_we_answered: qw,
                chunks_received: cr,
                chunks_sent: cs,
                replication_watermarks: watermarks,
            },
        );
    }

    /// Set the PeerId for an existing peer (e.g. when we learn the mapping from gossip).
    pub fn set_peer_id(&mut self, node_id: &str, peer_id: PeerId) {
        if let Some(info) = self.peers.get_mut(node_id) {
            info.peer_id = Some(peer_id);
        }
    }

    /// Find peers that serve a given shard.
    /// Supports sub-shard prefix matching: querying `"topic-technology"` will match
    /// peers serving `"topic-technology/0"`, `"topic-technology/1"`, etc.
    pub fn peers_for_shard(&self, shard_id: &str) -> Vec<String> {
        self.peers
            .iter()
            .filter(|(_, info)| {
                info.shard_ids.iter().any(|s| {
                    s == shard_id || oi_types::shard::is_sub_shard_of(s, shard_id)
                })
            })
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Remove peers that haven't sent a heartbeat within the timeout
    pub fn evict_stale(&mut self, timeout: std::time::Duration) -> Vec<String> {
        let now = Instant::now();
        let stale: Vec<String> = self
            .peers
            .iter()
            .filter(|(_, info)| now.duration_since(info.last_heartbeat) > timeout)
            .map(|(id, _)| id.clone())
            .collect();

        for id in &stale {
            self.peers.remove(id);
        }
        stale
    }

    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }

    /// Return all unique shard IDs known to the routing table.
    pub fn all_shards(&self) -> Vec<String> {
        let mut shards: std::collections::HashSet<String> = std::collections::HashSet::new();
        for info in self.peers.values() {
            for s in &info.shard_ids {
                shards.insert(s.clone());
            }
        }
        let mut result: Vec<String> = shards.into_iter().collect();
        result.sort();
        result
    }

    /// Select peers for a shard, ranked by doc count (richest first).
    /// Only includes peers with a known PeerId.
    /// Returns (node_id, PeerId, vectors_stored).
    pub fn peers_for_shard_ranked(
        &self,
        shard_id: &str,
        max_peers: usize,
    ) -> Vec<(String, PeerId, u64)> {
        let mut peers: Vec<(String, PeerId, u64)> = self
            .peers
            .iter()
            .filter(|(_, info)| {
                info.peer_id.is_some()
                    && info.shard_ids.iter().any(|s| {
                        s == shard_id || oi_types::shard::is_sub_shard_of(s, shard_id)
                    })
            })
            .map(|(id, info)| (id.clone(), info.peer_id.unwrap(), info.vectors_stored))
            .collect();

        // Sort by vectors_stored descending — richest nodes first
        peers.sort_by(|a, b| b.2.cmp(&a.2));
        peers.truncate(max_peers);
        peers
    }

    /// Estimate total documents across all peers serving a shard.
    /// Supports sub-shard prefix matching.
    pub fn total_docs_for_shard(&self, shard_id: &str) -> u64 {
        self.peers
            .values()
            .filter(|info| info.shard_ids.iter().any(|s| {
                s == shard_id || oi_types::shard::is_sub_shard_of(s, shard_id)
            }))
            .map(|info| info.vectors_stored)
            .sum()
    }

    /// Count the number of peers that serve a given shard.
    /// Supports sub-shard prefix matching.
    pub fn replica_count(&self, shard_id: &str) -> usize {
        self.peers
            .values()
            .filter(|info| info.shard_ids.iter().any(|s| {
                s == shard_id || oi_types::shard::is_sub_shard_of(s, shard_id)
            }))
            .count()
    }

    /// Find shards with fewer replicas than `min_replicas`.
    /// Returns `(shard_id, current_count, source_peers)` where source_peers
    /// are peers with a known PeerId that serve the shard (candidates for replication pull).
    #[allow(clippy::type_complexity)]
    pub fn under_replicated_shards(
        &self,
        min_replicas: usize,
    ) -> Vec<(String, usize, Vec<(String, PeerId)>)> {
        let all = self.all_shards();
        let mut result = Vec::new();
        for shard_id in all {
            let count = self.replica_count(&shard_id);
            if count < min_replicas {
                let sources: Vec<(String, PeerId)> = self
                    .peers
                    .iter()
                    .filter(|(_, info)| {
                        info.peer_id.is_some()
                            && info.shard_ids.iter().any(|s| {
                                s == &shard_id || oi_types::shard::is_sub_shard_of(s, &shard_id)
                            })
                    })
                    .map(|(id, info)| (id.clone(), info.peer_id.unwrap()))
                    .collect();
                result.push((shard_id, count, sources));
            }
        }
        result
    }

    // ── Contribution tracking (tit-for-tat) ───────────────────────────────

    /// Look up the node_id for a given PeerId.
    /// Returns `None` if no peer with that PeerId is known.
    pub fn node_id_for_peer(&self, peer_id: &PeerId) -> Option<String> {
        self.peers
            .iter()
            .find(|(_, info)| info.peer_id == Some(*peer_id))
            .map(|(id, _)| id.clone())
    }

    /// Record that a peer answered one of our queries.
    pub fn record_query_answered(&mut self, node_id: &str) {
        if let Some(info) = self.peers.get_mut(node_id) {
            info.queries_answered_for_us += 1;
        }
    }

    /// Record that we answered a query for a peer.
    pub fn record_query_served(&mut self, node_id: &str) {
        if let Some(info) = self.peers.get_mut(node_id) {
            info.queries_we_answered += 1;
        }
    }

    /// Record chunks received from a peer (replication).
    pub fn record_replication_received(&mut self, node_id: &str, chunks: u64) {
        if let Some(info) = self.peers.get_mut(node_id) {
            info.chunks_received += chunks;
        }
    }

    /// Record chunks sent to a peer (replication).
    pub fn record_replication_sent(&mut self, node_id: &str, chunks: u64) {
        if let Some(info) = self.peers.get_mut(node_id) {
            info.chunks_sent += chunks;
        }
    }

    /// Contribution ratio for a peer: what they gave us / what we gave them.
    /// Returns 1.0 for new peers, >1.0 for generous peers, <1.0 for leechers.
    pub fn contribution_ratio(&self, node_id: &str) -> f64 {
        match self.peers.get(node_id) {
            Some(info) => {
                let given = (info.queries_answered_for_us + info.chunks_received) as f64;
                let received = (info.queries_we_answered + info.chunks_sent) as f64;
                if received == 0.0 {
                    return 1.0; // No interactions yet — neutral
                }
                given / received
            }
            None => 1.0,
        }
    }

    /// Check if a peer should be throttled based on their contribution ratio.
    pub fn should_throttle(&self, node_id: &str, min_ratio: f64) -> bool {
        let ratio = self.contribution_ratio(node_id);
        // Only throttle if we've had enough interactions to judge (at least 10 total)
        let total = match self.peers.get(node_id) {
            Some(info) => {
                info.queries_answered_for_us + info.queries_we_answered
                    + info.chunks_received + info.chunks_sent
            }
            None => return false,
        };
        total >= 10 && ratio < min_ratio
    }

    /// Pick one peer per shard (for fan-out queries).
    /// Returns a map of shard_id -> (node_id, PeerId), choosing the first peer found
    /// for each shard. Only includes peers with a known PeerId.
    pub fn one_peer_per_shard(&self) -> HashMap<String, (String, PeerId)> {
        let mut result: HashMap<String, (String, PeerId)> = HashMap::new();
        for (node_id, info) in &self.peers {
            if let Some(pid) = info.peer_id {
                for shard_id in &info.shard_ids {
                    result
                        .entry(shard_id.clone())
                        .or_insert_with(|| (node_id.clone(), pid));
                }
            }
        }
        result
    }

    // ── Replication watermarks ─────────────────────────────────────────

    /// Get the replication watermark for a peer's shard.
    /// Returns 0 if no watermark has been recorded yet.
    pub fn get_watermark(&self, node_id: &str, shard_id: &str) -> u64 {
        self.peers
            .get(node_id)
            .and_then(|p| p.replication_watermarks.get(shard_id).copied())
            .unwrap_or(0)
    }

    /// Set the replication watermark for a peer's shard.
    pub fn set_watermark(&mut self, node_id: &str, shard_id: &str, seq: u64) {
        if let Some(info) = self.peers.get_mut(node_id) {
            info.replication_watermarks.insert(shard_id.to_string(), seq);
        }
    }

    // ── Persistence ─────────────────────────────────────────────────────

    /// Save the routing table to a JSON file.
    pub fn save_to_file(&self, path: &Path) -> io::Result<()> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let persisted: Vec<PersistedPeerInfo> = self
            .peers
            .iter()
            .map(|(node_id, info)| PersistedPeerInfo {
                node_id: node_id.clone(),
                peer_id: info.peer_id.map(|p| p.to_string()),
                shard_ids: info.shard_ids.clone(),
                vectors_stored: info.vectors_stored,
                last_seen_timestamp: now,
                queries_answered_for_us: info.queries_answered_for_us,
                queries_we_answered: info.queries_we_answered,
                chunks_received: info.chunks_received,
                chunks_sent: info.chunks_sent,
                replication_watermarks: info.replication_watermarks.clone(),
            })
            .collect();

        let json = serde_json::to_string_pretty(&persisted)
            .map_err(io::Error::other)?;

        // Write to temp file then rename for atomicity
        let tmp_path = path.with_extension("json.tmp");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&tmp_path, json)?;
        std::fs::rename(&tmp_path, path)?;

        Ok(())
    }

    /// Load routing table from a JSON file, pruning entries older than `max_age_secs`.
    /// Returns an empty table if the file doesn't exist or is corrupt.
    pub fn load_from_file(path: &Path, max_age_secs: u64) -> io::Result<Self> {
        let data = match std::fs::read_to_string(path) {
            Ok(d) => d,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Ok(Self::new());
            }
            Err(e) => return Err(e),
        };

        let persisted: Vec<PersistedPeerInfo> = match serde_json::from_str(&data) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("Corrupt routing table file, starting fresh: {e}");
                return Ok(Self::new());
            }
        };

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let mut table = Self::new();
        for entry in persisted {
            // Prune entries older than max_age
            if now.saturating_sub(entry.last_seen_timestamp) > max_age_secs {
                continue;
            }

            let peer_id = entry
                .peer_id
                .and_then(|s| s.parse::<PeerId>().ok());

            table.peers.insert(
                entry.node_id,
                PeerInfo {
                    peer_id,
                    shard_ids: entry.shard_ids,
                    last_heartbeat: Instant::now(),
                    vectors_stored: entry.vectors_stored,
                    queries_answered_for_us: entry.queries_answered_for_us,
                    queries_we_answered: entry.queries_we_answered,
                    chunks_received: entry.chunks_received,
                    chunks_sent: entry.chunks_sent,
                    replication_watermarks: entry.replication_watermarks,
                },
            );
        }

        Ok(table)
    }

    /// Return multiaddrs for all peers with known PeerIds (for re-dialing on startup).
    pub fn peer_ids(&self) -> Vec<PeerId> {
        self.peers
            .values()
            .filter_map(|info| info.peer_id)
            .collect()
    }
}

impl Default for RoutingTable {
    fn default() -> Self {
        Self::new()
    }
}

/// Compute adaptive fanout based on estimated shard coverage.
///
/// For each shard, estimates what fraction of its total docs will be searched
/// by the top `base_fanout` peers. If coverage is too low, increases fanout.
pub fn compute_adaptive_fanout(
    routing_table: &RoutingTable,
    selected_shards: &[&String],
    base_fanout: usize,
    target_coverage: f64,
    max_fanout: usize,
) -> usize {
    if selected_shards.is_empty() {
        return base_fanout;
    }

    let mut min_coverage = 1.0f64;

    for shard_id in selected_shards {
        let total_docs = routing_table.total_docs_for_shard(shard_id);
        if total_docs == 0 {
            continue;
        }

        let ranked = routing_table.peers_for_shard_ranked(shard_id, base_fanout);
        let covered_docs: u64 = ranked.iter().map(|(_, _, docs)| docs).sum();
        let coverage = covered_docs as f64 / total_docs as f64;

        if coverage < min_coverage {
            min_coverage = coverage;
        }
    }

    // Adaptive scaling
    if min_coverage < 0.5 {
        // Very low coverage — double fanout
        (base_fanout * 2).min(max_fanout)
    } else if min_coverage < target_coverage {
        // Below target — increase by 1
        (base_fanout + 1).min(max_fanout)
    } else {
        base_fanout
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_update_and_query() {
        let mut rt = RoutingTable::new();
        rt.update_peer("peer1", vec!["tech-v1".into(), "science-v1".into()], 1000);
        rt.update_peer("peer2", vec!["science-v1".into(), "news-v1".into()], 2000);

        assert_eq!(rt.peers_for_shard("tech-v1"), vec!["peer1"]);
        let science_peers = rt.peers_for_shard("science-v1");
        assert_eq!(science_peers.len(), 2);
        assert_eq!(rt.peers_for_shard("news-v1"), vec!["peer2"]);
        assert!(rt.peers_for_shard("unknown").is_empty());
    }

    #[test]
    fn test_evict_stale() {
        let mut rt = RoutingTable::new();
        rt.update_peer("peer1", vec!["shard-v1".into()], 100);
        assert_eq!(rt.peer_count(), 1);

        // Nothing stale yet (timeout far in future won't trigger)
        let evicted = rt.evict_stale(Duration::from_secs(3600));
        assert!(evicted.is_empty());
        assert_eq!(rt.peer_count(), 1);
    }

    #[test]
    fn test_all_shards() {
        let mut rt = RoutingTable::new();
        rt.update_peer("peer1", vec!["shard-0".into(), "shard-1".into()], 100);
        rt.update_peer("peer2", vec!["shard-1".into(), "shard-2".into()], 200);

        let shards = rt.all_shards();
        assert_eq!(shards, vec!["shard-0", "shard-1", "shard-2"]);
    }

    #[test]
    fn test_one_peer_per_shard() {
        let pid1 = PeerId::random();
        let pid2 = PeerId::random();
        let mut rt = RoutingTable::new();
        rt.update_peer_with_id("peer1", vec!["shard-0".into(), "shard-1".into()], 100, Some(pid1));
        rt.update_peer_with_id("peer2", vec!["shard-1".into(), "shard-2".into()], 200, Some(pid2));

        let mapping = rt.one_peer_per_shard();
        assert_eq!(mapping.len(), 3);
        assert!(mapping.contains_key("shard-0"));
        assert!(mapping.contains_key("shard-1"));
        assert!(mapping.contains_key("shard-2"));
        // shard-0 should map to peer1
        assert_eq!(mapping["shard-0"].0, "peer1");
        assert_eq!(mapping["shard-0"].1, pid1);
        // shard-2 should map to peer2
        assert_eq!(mapping["shard-2"].0, "peer2");
        assert_eq!(mapping["shard-2"].1, pid2);
    }

    #[test]
    fn test_one_peer_per_shard_excludes_unknown_peer_ids() {
        let pid1 = PeerId::random();
        let mut rt = RoutingTable::new();
        rt.update_peer_with_id("peer1", vec!["shard-0".into()], 100, Some(pid1));
        // peer2 has no PeerId — should be excluded
        rt.update_peer("peer2", vec!["shard-1".into()], 200);

        let mapping = rt.one_peer_per_shard();
        assert_eq!(mapping.len(), 1);
        assert!(mapping.contains_key("shard-0"));
        assert!(!mapping.contains_key("shard-1"));
    }

    #[test]
    fn test_set_peer_id() {
        let pid = PeerId::random();
        let mut rt = RoutingTable::new();
        rt.update_peer("peer1", vec!["shard-0".into()], 100);
        // Initially no PeerId
        assert!(rt.one_peer_per_shard().is_empty());

        rt.set_peer_id("peer1", pid);
        let mapping = rt.one_peer_per_shard();
        assert_eq!(mapping.len(), 1);
        assert_eq!(mapping["shard-0"].1, pid);
    }

    #[test]
    fn test_all_shards_empty() {
        let rt = RoutingTable::new();
        assert!(rt.all_shards().is_empty());
        assert!(rt.one_peer_per_shard().is_empty());
    }

    #[test]
    fn test_peer_count() {
        let mut rt = RoutingTable::new();
        assert_eq!(rt.peer_count(), 0);
        rt.update_peer("a", vec!["s1".into()], 0);
        rt.update_peer("b", vec!["s1".into()], 0);
        assert_eq!(rt.peer_count(), 2);
    }

    #[test]
    fn test_peers_for_shard_ranked() {
        let pid1 = PeerId::random();
        let pid2 = PeerId::random();
        let pid3 = PeerId::random();
        let mut rt = RoutingTable::new();
        rt.update_peer_with_id("peer1", vec!["shard-0".into()], 1000, Some(pid1));
        rt.update_peer_with_id("peer2", vec!["shard-0".into()], 5000, Some(pid2));
        rt.update_peer_with_id("peer3", vec!["shard-0".into()], 3000, Some(pid3));

        let ranked = rt.peers_for_shard_ranked("shard-0", 10);
        assert_eq!(ranked.len(), 3);
        // Sorted by vectors_stored descending
        assert_eq!(ranked[0].0, "peer2");
        assert_eq!(ranked[0].2, 5000);
        assert_eq!(ranked[1].0, "peer3");
        assert_eq!(ranked[1].2, 3000);
        assert_eq!(ranked[2].0, "peer1");
        assert_eq!(ranked[2].2, 1000);

        // With limit
        let ranked = rt.peers_for_shard_ranked("shard-0", 2);
        assert_eq!(ranked.len(), 2);
    }

    #[test]
    fn test_total_docs_for_shard() {
        let mut rt = RoutingTable::new();
        rt.update_peer("peer1", vec!["shard-0".into()], 1000);
        rt.update_peer("peer2", vec!["shard-0".into()], 5000);
        rt.update_peer("peer3", vec!["shard-1".into()], 3000);

        assert_eq!(rt.total_docs_for_shard("shard-0"), 6000);
        assert_eq!(rt.total_docs_for_shard("shard-1"), 3000);
        assert_eq!(rt.total_docs_for_shard("shard-2"), 0);
    }

    #[test]
    fn test_replica_count() {
        let mut rt = RoutingTable::new();
        rt.update_peer("peer1", vec!["shard-0".into(), "shard-1".into()], 100);
        rt.update_peer("peer2", vec!["shard-0".into()], 200);
        rt.update_peer("peer3", vec!["shard-1".into(), "shard-2".into()], 300);

        assert_eq!(rt.replica_count("shard-0"), 2);
        assert_eq!(rt.replica_count("shard-1"), 2);
        assert_eq!(rt.replica_count("shard-2"), 1);
        assert_eq!(rt.replica_count("shard-9"), 0);
    }

    #[test]
    fn test_under_replicated_shards() {
        let pid1 = PeerId::random();
        let pid2 = PeerId::random();
        let mut rt = RoutingTable::new();
        rt.update_peer_with_id("peer1", vec!["shard-0".into(), "shard-1".into()], 100, Some(pid1));
        rt.update_peer_with_id("peer2", vec!["shard-0".into()], 200, Some(pid2));

        // shard-0 has 2 replicas, shard-1 has 1 — with min=2, only shard-1 is under-replicated
        let under = rt.under_replicated_shards(2);
        assert_eq!(under.len(), 1);
        assert_eq!(under[0].0, "shard-1");
        assert_eq!(under[0].1, 1);
        assert_eq!(under[0].2.len(), 1); // peer1 serves shard-1

        // With min=3, both are under-replicated
        let under = rt.under_replicated_shards(3);
        assert_eq!(under.len(), 2);
    }

    #[test]
    fn test_under_replicated_shards_all_sufficient() {
        let pid1 = PeerId::random();
        let pid2 = PeerId::random();
        let pid3 = PeerId::random();
        let mut rt = RoutingTable::new();
        rt.update_peer_with_id("peer1", vec!["shard-0".into()], 100, Some(pid1));
        rt.update_peer_with_id("peer2", vec!["shard-0".into()], 200, Some(pid2));
        rt.update_peer_with_id("peer3", vec!["shard-0".into()], 300, Some(pid3));

        let under = rt.under_replicated_shards(3);
        assert!(under.is_empty());
    }

    #[test]
    fn test_adaptive_fanout_full_coverage() {
        let pid1 = PeerId::random();
        let mut rt = RoutingTable::new();
        // Single peer has all docs for shard
        rt.update_peer_with_id("peer1", vec!["shard-0".into()], 10000, Some(pid1));

        let shard_0 = "shard-0".to_string();
        let shards = vec![&shard_0];
        let fanout = compute_adaptive_fanout(&rt, &shards, 3, 0.8, 10);
        assert_eq!(fanout, 3, "Full coverage should not increase fanout");
    }

    #[test]
    fn test_node_id_for_peer() {
        let pid1 = PeerId::random();
        let pid2 = PeerId::random();
        let mut rt = RoutingTable::new();
        rt.update_peer_with_id("node-A", vec!["shard-0".into()], 100, Some(pid1));
        rt.update_peer_with_id("node-B", vec!["shard-1".into()], 200, Some(pid2));
        rt.update_peer("node-C", vec!["shard-2".into()], 300); // no PeerId

        assert_eq!(rt.node_id_for_peer(&pid1), Some("node-A".to_string()));
        assert_eq!(rt.node_id_for_peer(&pid2), Some("node-B".to_string()));
        assert_eq!(rt.node_id_for_peer(&PeerId::random()), None);
    }

    #[test]
    fn test_contribution_ratio_balanced() {
        let mut rt = RoutingTable::new();
        rt.update_peer("peer1", vec!["shard-0".into()], 100);

        // Start neutral
        assert_eq!(rt.contribution_ratio("peer1"), 1.0);

        // Peer answers 5 queries, we answer 5 → balanced
        for _ in 0..5 {
            rt.record_query_answered("peer1");
            rt.record_query_served("peer1");
        }
        assert!((rt.contribution_ratio("peer1") - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_contribution_ratio_leecher() {
        let mut rt = RoutingTable::new();
        rt.update_peer("peer1", vec!["shard-0".into()], 100);

        // We answer 10 queries, peer answers 1 → leecher
        for _ in 0..10 {
            rt.record_query_served("peer1");
        }
        rt.record_query_answered("peer1");

        let ratio = rt.contribution_ratio("peer1");
        assert!(ratio < 0.2, "expected leecher ratio, got {ratio}");
    }

    #[test]
    fn test_should_throttle_respects_minimum_interactions() {
        let mut rt = RoutingTable::new();
        rt.update_peer("peer1", vec!["shard-0".into()], 100);

        // Only 2 interactions — too few to judge
        rt.record_query_served("peer1");
        rt.record_query_served("peer1");
        assert!(!rt.should_throttle("peer1", 0.5), "too few interactions to throttle");

        // Add more to reach threshold (10 total)
        for _ in 0..8 {
            rt.record_query_served("peer1");
        }
        // 10 served, 0 answered → ratio = 0.0 < 0.5
        assert!(rt.should_throttle("peer1", 0.5));
    }

    #[test]
    fn test_should_throttle_disabled_returns_false() {
        let rt = RoutingTable::new();
        // Unknown peer → false
        assert!(!rt.should_throttle("nonexistent", 0.5));
    }

    #[test]
    fn test_contribution_replication_tracking() {
        let mut rt = RoutingTable::new();
        rt.update_peer("peer1", vec!["shard-0".into()], 100);

        rt.record_replication_received("peer1", 50);
        rt.record_replication_sent("peer1", 50);

        // 50 received / 50 sent = 1.0
        assert!((rt.contribution_ratio("peer1") - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_adaptive_fanout_low_coverage() {
        let mut rt = RoutingTable::new();
        // 20 peers, each with 500 docs (total 10K). Fanout=3 covers 1500/10000 = 15%
        for i in 0..20 {
            let pid = PeerId::random();
            rt.update_peer_with_id(
                &format!("peer{i}"),
                vec!["shard-0".into()],
                500,
                Some(pid),
            );
        }

        let shard_0 = "shard-0".to_string();
        let shards = vec![&shard_0];
        let fanout = compute_adaptive_fanout(&rt, &shards, 3, 0.8, 10);
        // 15% coverage < 50% → double to 6
        assert_eq!(fanout, 6);
    }

    #[test]
    fn test_save_load_roundtrip() {
        let pid = PeerId::random();
        let mut rt = RoutingTable::new();
        rt.update_peer_with_id("peer1", vec!["shard-0".into(), "shard-1".into()], 1000, Some(pid));
        rt.update_peer("peer2", vec!["shard-2".into()], 500);
        rt.record_query_answered("peer1");
        rt.record_query_served("peer1");
        rt.record_replication_received("peer1", 42);
        rt.set_watermark("peer1", "shard-0", 99);

        let dir = std::env::temp_dir().join("oi_routing_test_roundtrip");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("routing_table.json");

        rt.save_to_file(&path).unwrap();
        let loaded = RoutingTable::load_from_file(&path, 86400).unwrap();

        assert_eq!(loaded.peer_count(), 2);
        assert_eq!(loaded.peers_for_shard("shard-0"), vec!["peer1"]);
        assert_eq!(loaded.peers_for_shard("shard-2"), vec!["peer2"]);
        assert_eq!(loaded.get_watermark("peer1", "shard-0"), 99);
        // contribution counters survive: given=1+42=43, received=1+0=1 → ratio=43.0
        assert!(loaded.contribution_ratio("peer1") > 1.0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_prunes_old_entries() {
        let dir = std::env::temp_dir().join("oi_routing_test_prune");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("routing_table.json");

        // Write a routing table with an old timestamp (1 hour ago)
        let old_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() - 3600;
        let json = serde_json::to_string(&vec![
            serde_json::json!({
                "node_id": "peer1",
                "peer_id": null,
                "shard_ids": ["shard-0"],
                "vectors_stored": 100,
                "last_seen_timestamp": old_ts,
                "queries_answered_for_us": 0,
                "queries_we_answered": 0,
                "chunks_received": 0,
                "chunks_sent": 0,
                "replication_watermarks": {}
            })
        ]).unwrap();
        std::fs::write(&path, json).unwrap();

        // max_age=1800 (30min) should prune the 1h-old entry
        let loaded = RoutingTable::load_from_file(&path, 1800).unwrap();
        assert_eq!(loaded.peer_count(), 0);

        // max_age=7200 (2h) should keep it
        let loaded = RoutingTable::load_from_file(&path, 7200).unwrap();
        assert_eq!(loaded.peer_count(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_missing_file_returns_empty() {
        let loaded = RoutingTable::load_from_file(
            std::path::Path::new("/tmp/nonexistent_routing_table_12345.json"),
            86400,
        )
        .unwrap();
        assert_eq!(loaded.peer_count(), 0);
    }

    #[test]
    fn test_load_corrupt_file_returns_empty() {
        let dir = std::env::temp_dir().join("oi_routing_test_corrupt");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("routing_table.json");
        std::fs::write(&path, "not valid json!!{{{").unwrap();

        let loaded = RoutingTable::load_from_file(&path, 86400).unwrap();
        assert_eq!(loaded.peer_count(), 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_watermark_get_set() {
        let mut rt = RoutingTable::new();
        rt.update_peer("peer1", vec!["shard-0".into()], 100);

        assert_eq!(rt.get_watermark("peer1", "shard-0"), 0);
        rt.set_watermark("peer1", "shard-0", 42);
        assert_eq!(rt.get_watermark("peer1", "shard-0"), 42);
        rt.set_watermark("peer1", "shard-0", 100);
        assert_eq!(rt.get_watermark("peer1", "shard-0"), 100);

        // Unknown peer returns 0
        assert_eq!(rt.get_watermark("unknown", "shard-0"), 0);
    }
}
