use libp2p::PeerId;
use oi_types::proto::opensonarx::{QueryRequest, QueryResponse, SearchResult};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{mpsc, oneshot};

/// Command sent from NodeHandle to the event loop to initiate a query.
pub struct QueryCommand {
    pub request: QueryRequest,
    pub reply_tx: oneshot::Sender<QueryResponse>,
}

/// Command sent from NodeHandle to the event loop to publish a gossip message or trigger replication.
pub enum GossipCommand {
    ContentAnnouncement {
        sentinel_id: String,
        url: String,
        domain: String,
        content_hash: String,
        chunk_count: u32,
        shard_id: String,
        is_update: bool,
        entity: String,
    },
    ShardAnnounce {
        shard_ids: Vec<String>,
    },
    ReplicationRequest {
        peer_id: PeerId,
        shard_id: String,
        max_chunks: u32,
    },
}

/// Tracks an in-flight fanout query.
pub struct PendingFanout {
    pub reply_tx: Option<oneshot::Sender<QueryResponse>>,
    pub request: QueryRequest,
    pub local_results: Vec<SearchResult>,
    pub remote_results: Vec<SearchResult>,
    pub expected_remotes: usize,
    pub received_remotes: usize,
    pub created_at: Instant,
}

/// Handle returned by `Node::start()` — lets callers submit queries to the running node.
#[derive(Clone)]
pub struct NodeHandle {
    pub peer_id: PeerId,
    query_tx: mpsc::Sender<QueryCommand>,
    gossip_tx: mpsc::Sender<GossipCommand>,
    shutdown_tx: Arc<tokio::sync::watch::Sender<bool>>,
}

impl NodeHandle {
    pub fn new(
        peer_id: PeerId,
        query_tx: mpsc::Sender<QueryCommand>,
        gossip_tx: mpsc::Sender<GossipCommand>,
        shutdown_tx: tokio::sync::watch::Sender<bool>,
    ) -> Self {
        Self {
            peer_id,
            query_tx,
            gossip_tx,
            shutdown_tx: Arc::new(shutdown_tx),
        }
    }

    /// Signal the event loop to shut down gracefully (flush routing table + earnings).
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
    }

    /// Submit a query and wait for the merged fanout response.
    pub async fn query(
        &self,
        request: QueryRequest,
    ) -> Result<QueryResponse, Box<dyn std::error::Error + Send + Sync>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.query_tx
            .send(QueryCommand { request, reply_tx })
            .await
            .map_err(|_| "event loop shut down")?;
        Ok(reply_rx.await.map_err(|_| "fanout reply dropped")?)
    }

    /// Announce updated shard assignments to the gossip network (fire-and-forget).
    pub fn announce_shards(&self, shard_ids: Vec<String>) {
        let _ = self.gossip_tx.try_send(GossipCommand::ShardAnnounce { shard_ids });
    }

    /// Request replication of a shard from a specific peer (fire-and-forget).
    pub fn request_replication(&self, peer_id: PeerId, shard_id: String, max_chunks: u32) {
        let _ = self.gossip_tx.try_send(GossipCommand::ReplicationRequest {
            peer_id,
            shard_id,
            max_chunks,
        });
    }

    /// Announce content to the gossip network (fire-and-forget).
    #[allow(clippy::too_many_arguments)]
    pub fn announce_content(
        &self,
        sentinel_id: String,
        url: String,
        domain: String,
        content_hash: String,
        chunk_count: u32,
        shard_id: String,
        is_update: bool,
        entity: String,
    ) {
        let _ = self.gossip_tx.try_send(GossipCommand::ContentAnnouncement {
            sentinel_id,
            url,
            domain,
            content_hash,
            chunk_count,
            shard_id,
            is_update,
            entity,
        });
    }
}

/// Merge local and remote results into a single `QueryResponse`.
///
/// - Combines all `SearchResult` items
/// - Deduplicates by URL (keeps highest `final` score)
/// - Sorts by `final` score descending
/// - Truncates to `limit`
/// - Renumbers ranks 1..N
pub fn build_merged_response(
    request_id: &str,
    local_results: Vec<SearchResult>,
    remote_results: Vec<SearchResult>,
    limit: usize,
) -> QueryResponse {
    // Deduplicate by URL, keeping the highest final score
    let mut by_url: HashMap<String, SearchResult> = HashMap::new();
    for result in local_results.into_iter().chain(remote_results) {
        let final_score = result.scores.as_ref().map(|s| s.r#final).unwrap_or(0.0);
        by_url
            .entry(result.url.clone())
            .and_modify(|existing| {
                let existing_score = existing
                    .scores
                    .as_ref()
                    .map(|s| s.r#final)
                    .unwrap_or(0.0);
                if final_score > existing_score {
                    *existing = result.clone();
                }
            })
            .or_insert(result);
    }

    // Sort by final score descending
    let mut merged: Vec<SearchResult> = by_url.into_values().collect();
    merged.sort_by(|a, b| {
        let sa = a.scores.as_ref().map(|s| s.r#final).unwrap_or(0.0);
        let sb = b.scores.as_ref().map(|s| s.r#final).unwrap_or(0.0);
        sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
    });

    // Truncate and renumber
    merged.truncate(limit);
    for (i, result) in merged.iter_mut().enumerate() {
        result.rank = (i + 1) as u32;
    }

    QueryResponse {
        request_id: request_id.to_string(),
        meta: None,
        results: merged,
        error: None,
    }
}

/// Check if a PendingFanout is done (all remotes received OR timed out).
/// If done, build the merged response and send it via `reply_tx`.
pub fn maybe_complete_fanout(fanout: &mut PendingFanout, timed_out: bool) -> bool {
    let done = timed_out || fanout.received_remotes >= fanout.expected_remotes;
    if !done {
        return false;
    }

    let limit = if fanout.request.limit > 0 {
        fanout.request.limit as usize
    } else {
        5
    };

    let response = build_merged_response(
        &fanout.request.request_id,
        std::mem::take(&mut fanout.local_results),
        std::mem::take(&mut fanout.remote_results),
        limit,
    );

    if let Some(tx) = fanout.reply_tx.take() {
        let _ = tx.send(response);
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use oi_types::proto::opensonarx::Scores;

    fn make_result(url: &str, score: f32, rank: u32) -> SearchResult {
        SearchResult {
            result_id: format!("r-{url}"),
            rank,
            title: format!("Title {url}"),
            url: url.to_string(),
            snippet: String::new(),
            content_md: String::new(),
            scores: Some(Scores {
                semantic: score * 0.6,
                bm25: score * 0.4,
                r#final: score,
            }),
            stake: None,
            utility: None,
            freshness: None,
            extraction_quality: String::new(),
            source: "test".to_string(),
            entity: String::new(),
            content_kind: String::new(),
        }
    }

    #[test]
    fn test_build_merged_response_deduplicates_by_url() {
        let local = vec![make_result("https://a.com", 0.9, 1)];
        let remote = vec![
            make_result("https://a.com", 0.7, 1), // dup, lower score
            make_result("https://b.com", 0.8, 2),
        ];

        let resp = build_merged_response("req-1", local, remote, 10);
        assert_eq!(resp.results.len(), 2);
        // a.com should keep the 0.9 score
        let a = resp.results.iter().find(|r| r.url == "https://a.com").unwrap();
        assert!((a.scores.as_ref().unwrap().r#final - 0.9).abs() < 0.001);
    }

    #[test]
    fn test_build_merged_response_sorts_by_score() {
        let local = vec![make_result("https://a.com", 0.5, 1)];
        let remote = vec![
            make_result("https://b.com", 0.9, 1),
            make_result("https://c.com", 0.7, 2),
        ];

        let resp = build_merged_response("req-2", local, remote, 10);
        assert_eq!(resp.results.len(), 3);
        assert_eq!(resp.results[0].url, "https://b.com");
        assert_eq!(resp.results[1].url, "https://c.com");
        assert_eq!(resp.results[2].url, "https://a.com");
    }

    #[test]
    fn test_build_merged_response_truncates_to_limit() {
        let local = vec![
            make_result("https://a.com", 0.9, 1),
            make_result("https://b.com", 0.8, 2),
        ];
        let remote = vec![make_result("https://c.com", 0.7, 1)];

        let resp = build_merged_response("req-3", local, remote, 2);
        assert_eq!(resp.results.len(), 2);
        assert_eq!(resp.results[0].url, "https://a.com");
        assert_eq!(resp.results[1].url, "https://b.com");
    }

    #[test]
    fn test_build_merged_response_renumbers_ranks() {
        let local = vec![make_result("https://a.com", 0.5, 99)];
        let remote = vec![make_result("https://b.com", 0.9, 99)];

        let resp = build_merged_response("req-4", local, remote, 10);
        assert_eq!(resp.results[0].rank, 1);
        assert_eq!(resp.results[1].rank, 2);
    }

    #[test]
    fn test_maybe_complete_fanout_all_received() {
        let (tx, mut rx) = oneshot::channel();
        let mut fanout = PendingFanout {
            reply_tx: Some(tx),
            request: QueryRequest {
                request_id: "req-1".to_string(),
                limit: 5,
                ..Default::default()
            },
            local_results: vec![make_result("https://a.com", 0.9, 1)],
            remote_results: vec![make_result("https://b.com", 0.8, 1)],
            expected_remotes: 1,
            received_remotes: 1,
            created_at: Instant::now(),
        };

        assert!(maybe_complete_fanout(&mut fanout, false));
        let resp = rx.try_recv().unwrap();
        assert_eq!(resp.results.len(), 2);
    }

    #[test]
    fn test_maybe_complete_fanout_not_done() {
        let (tx, _rx) = oneshot::channel();
        let mut fanout = PendingFanout {
            reply_tx: Some(tx),
            request: QueryRequest {
                request_id: "req-1".to_string(),
                limit: 5,
                ..Default::default()
            },
            local_results: vec![],
            remote_results: vec![],
            expected_remotes: 2,
            received_remotes: 1,
            created_at: Instant::now(),
        };

        assert!(!maybe_complete_fanout(&mut fanout, false));
    }

    #[test]
    fn test_maybe_complete_fanout_timeout_with_partial_results() {
        let (tx, mut rx) = oneshot::channel();
        let mut fanout = PendingFanout {
            reply_tx: Some(tx),
            request: QueryRequest {
                request_id: "req-1".to_string(),
                limit: 5,
                ..Default::default()
            },
            local_results: vec![make_result("https://a.com", 0.9, 1)],
            remote_results: vec![], // no remotes yet
            expected_remotes: 3,
            received_remotes: 0,
            created_at: Instant::now(),
        };

        // Should complete because of timeout
        assert!(maybe_complete_fanout(&mut fanout, true));
        let resp = rx.try_recv().unwrap();
        assert_eq!(resp.results.len(), 1);
        assert_eq!(resp.results[0].url, "https://a.com");
    }

    #[tokio::test]
    async fn test_node_handle_query_sends_and_receives() {
        let peer_id = PeerId::random();
        let (query_tx, mut query_rx) = mpsc::channel(16);
        let (gossip_tx, _gossip_rx) = mpsc::channel(16);
        let (shutdown_tx, _shutdown_rx) = tokio::sync::watch::channel(false);
        let handle = NodeHandle::new(peer_id, query_tx, gossip_tx, shutdown_tx);

        // Spawn a mock event loop that immediately replies
        tokio::spawn(async move {
            if let Some(cmd) = query_rx.recv().await {
                let response = QueryResponse {
                    request_id: cmd.request.request_id.clone(),
                    meta: None,
                    results: vec![],
                    error: None,
                };
                let _ = cmd.reply_tx.send(response);
            }
        });

        let request = QueryRequest {
            request_id: "test-req".to_string(),
            ..Default::default()
        };
        let response = handle.query(request).await.unwrap();
        assert_eq!(response.request_id, "test-req");
    }

    #[tokio::test]
    async fn test_node_handle_announce_content() {
        let peer_id = PeerId::random();
        let (query_tx, _query_rx) = mpsc::channel(16);
        let (gossip_tx, mut gossip_rx) = mpsc::channel(16);
        let (shutdown_tx, _shutdown_rx) = tokio::sync::watch::channel(false);
        let handle = NodeHandle::new(peer_id, query_tx, gossip_tx, shutdown_tx);

        handle.announce_content(
            "sentinel-1".into(),
            "https://example.com/page".into(),
            "example.com".into(),
            "hash123".into(),
            3,
            "shard-0".into(),
            false,
            "".into(),
        );

        let cmd = gossip_rx.try_recv().unwrap();
        match cmd {
            GossipCommand::ContentAnnouncement {
                sentinel_id,
                url,
                domain,
                content_hash,
                chunk_count,
                shard_id,
                is_update,
                entity,
            } => {
                assert_eq!(sentinel_id, "sentinel-1");
                assert_eq!(url, "https://example.com/page");
                assert_eq!(domain, "example.com");
                assert_eq!(content_hash, "hash123");
                assert_eq!(chunk_count, 3);
                assert_eq!(shard_id, "shard-0");
                assert!(!is_update);
                assert!(entity.is_empty());
            }
            _ => panic!("expected ContentAnnouncement"),
        }
    }

    #[tokio::test]
    async fn test_node_handle_request_replication() {
        let peer_id = PeerId::random();
        let target_peer = PeerId::random();
        let (query_tx, _query_rx) = mpsc::channel(16);
        let (gossip_tx, mut gossip_rx) = mpsc::channel(16);
        let (shutdown_tx, _shutdown_rx) = tokio::sync::watch::channel(false);
        let handle = NodeHandle::new(peer_id, query_tx, gossip_tx, shutdown_tx);

        handle.request_replication(target_peer, "topic-technology".into(), 50);

        let cmd = gossip_rx.try_recv().unwrap();
        match cmd {
            GossipCommand::ReplicationRequest {
                peer_id: pid,
                shard_id,
                max_chunks,
            } => {
                assert_eq!(pid, target_peer);
                assert_eq!(shard_id, "topic-technology");
                assert_eq!(max_chunks, 50);
            }
            _ => panic!("expected ReplicationRequest"),
        }
    }

    #[tokio::test]
    async fn test_node_handle_announce_shards() {
        let peer_id = PeerId::random();
        let (query_tx, _query_rx) = mpsc::channel(16);
        let (gossip_tx, mut gossip_rx) = mpsc::channel(16);
        let (shutdown_tx, _shutdown_rx) = tokio::sync::watch::channel(false);
        let handle = NodeHandle::new(peer_id, query_tx, gossip_tx, shutdown_tx);

        handle.announce_shards(vec![
            "topic-technology".into(),
            "topic-science".into(),
        ]);

        let cmd = gossip_rx.try_recv().unwrap();
        match cmd {
            GossipCommand::ShardAnnounce { shard_ids } => {
                assert_eq!(shard_ids.len(), 2);
                assert_eq!(shard_ids[0], "topic-technology");
                assert_eq!(shard_ids[1], "topic-science");
            }
            _ => panic!("expected ShardAnnounce"),
        }
    }
}
