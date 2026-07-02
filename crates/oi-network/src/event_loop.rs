use crate::behaviour::{OpenSonarXBehaviour, OpenSonarXBehaviourEvent};
use crate::fanout::{self, GossipCommand, PendingFanout, QueryCommand};
use crate::gossip;
use crate::routing::{self, RoutingTable};
use futures::StreamExt;
use libp2p::gossipsub;
use libp2p::mdns;
use libp2p::request_response::{self, OutboundRequestId};
use libp2p::swarm::SwarmEvent;
use libp2p::Swarm;
use oi_index::HybridIndex;
use oi_types::proto::opensonarx::{
    gossip_message, DocumentChunk, ErrorInfo, QueryRequest, QueryResponse, ReplicationReq,
    ReplicationResp, ResponseMeta, SearchResult, Scores, FreshnessInfo,
};
use oi_types::traits::SearchIndex;
use oi_types::{EarningsTracker, SignedTicket, TopicRouter};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, info, warn};

/// Action to be taken after processing a swarm event.
/// We collect these during the `select!` block and apply them after,
/// once the swarm borrow is released.
enum EventAction {
    None,
    SendResponse(request_response::ResponseChannel<QueryResponse>, QueryResponse),
    FanoutResponse {
        request_id: String,
        response: QueryResponse,
    },
    SendReplicationResponse(request_response::ResponseChannel<ReplicationResp>, ReplicationResp),
    IngestReplicatedChunks {
        peer: libp2p::PeerId,
        response: ReplicationResp,
    },
    SendReplicationRequest {
        peer: libp2p::PeerId,
        shard_id: String,
        max_chunks: u32,
    },
    AddKademliaPeers(Vec<(libp2p::PeerId, libp2p::Multiaddr)>),
    /// A peer joined the gossip topic — (re-)announce our shards so its
    /// routing table learns about us. The startup-only announcement raced
    /// mesh formation (published before any peer subscribed), leaving plain
    /// nodes permanently absent from every routing table.
    AnnounceShards,
}

/// Returns the gossipsub topic used for OpenSonarX gossip messages.
fn get_gossip_topic() -> gossipsub::IdentTopic {
    gossipsub::IdentTopic::new("opensonarx/gossip/v1")
}

/// Run the P2P event loop.
///
/// Handles: Kademlia discovery, Gossipsub messages, RequestResponse queries,
/// periodic heartbeats, earnings snapshots, stale peer eviction, and query fanout.
#[allow(clippy::too_many_arguments)]
pub async fn run_event_loop(
    mut swarm: Swarm<OpenSonarXBehaviour>,
    routing_table: Arc<RwLock<RoutingTable>>,
    node_id: String,
    index: Option<Arc<HybridIndex>>,
    earnings: Arc<EarningsTracker>,
    shard_ids: Arc<RwLock<Vec<String>>>,
    mut query_rx: mpsc::Receiver<QueryCommand>,
    mut gossip_rx: mpsc::Receiver<GossipCommand>,
    max_query_fanout: usize,
    query_timeout: Duration,
    target_coverage: f64,
    topic_router: Option<Arc<TopicRouter>>,
    min_replicas_per_shard: usize,
    replication_check_interval_secs: u64,
    enable_tit_for_tat: bool,
    min_contribution_ratio: f64,
    routing_table_path: Option<String>,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) {
    // Subscribe to the gossip topic
    let topic = get_gossip_topic();
    if let Err(e) = swarm.behaviour_mut().gossipsub.subscribe(&topic) {
        warn!("Failed to subscribe to gossip topic: {e}");
    }

    // Publish shard announcement on startup if we have shards
    {
        let shard_snapshot = shard_ids.read().await.clone();
        if !shard_snapshot.is_empty() {
            let vector_count = index
                .as_ref()
                .map(|idx| idx.doc_count() as u64)
                .unwrap_or(0);
            let data = gossip::encode_shard_announce(&node_id, &shard_snapshot, vector_count, 0);
            let topic = get_gossip_topic();
            match swarm.behaviour_mut().gossipsub.publish(topic, data) {
                Ok(_) => info!("Published shard announcement: {:?}", shard_snapshot),
                Err(e) => debug!("Shard announcement publish failed (expected if no peers): {e}"),
            }
        }
    }

    let mut heartbeat_interval = tokio::time::interval(Duration::from_secs(30));
    let mut eviction_interval = tokio::time::interval(Duration::from_secs(300));
    let mut earnings_save_interval = tokio::time::interval(Duration::from_secs(60));
    let mut fanout_check_interval = tokio::time::interval(Duration::from_secs(1));
    let mut replication_check_interval = tokio::time::interval(Duration::from_secs(replication_check_interval_secs));
    let mut routing_save_interval = tokio::time::interval(Duration::from_secs(300)); // every 5 min

    // In-flight fanout queries, keyed by request_id.
    let mut pending_fanouts: HashMap<String, PendingFanout> = HashMap::new();
    // Maps outbound request IDs to fanout request_ids for response correlation.
    let mut outbound_to_fanout: HashMap<OutboundRequestId, String> = HashMap::new();

    loop {
        let mut action = EventAction::None;

        tokio::select! {
            event = swarm.select_next_some() => {
                let shard_snapshot = shard_ids.read().await.clone();
                action = handle_swarm_event(
                    event,
                    &routing_table,
                    index.as_ref(),
                    &earnings,
                    &node_id,
                    &shard_snapshot,
                    &mut pending_fanouts,
                    &mut outbound_to_fanout,
                    enable_tit_for_tat,
                    min_contribution_ratio,
                ).await;
            }

            Some(cmd) = query_rx.recv() => {
                let shard_snapshot = shard_ids.read().await.clone();
                handle_query_command(
                    cmd,
                    &mut swarm,
                    &routing_table,
                    index.as_ref(),
                    &earnings,
                    &node_id,
                    &shard_snapshot,
                    max_query_fanout,
                    target_coverage,
                    &mut pending_fanouts,
                    &mut outbound_to_fanout,
                    topic_router.as_ref(),
                ).await;
            }

            Some(cmd) = gossip_rx.recv() => {
                match cmd {
                    GossipCommand::ContentAnnouncement {
                        sentinel_id, url, domain, content_hash,
                        chunk_count, shard_id, is_update, entity,
                    } => {
                        let data = gossip::encode_content_announcement(
                            &sentinel_id, &url, &domain, &content_hash,
                            chunk_count, &shard_id, is_update, &entity,
                        );
                        let topic = get_gossip_topic();
                        match swarm.behaviour_mut().gossipsub.publish(topic, data) {
                            Ok(_) => info!("Published content announcement: {url}"),
                            Err(e) => debug!("Content announcement publish failed: {e}"),
                        }
                    }
                    GossipCommand::ReplicationRequest { peer_id, shard_id, max_chunks } => {
                        // Use watermark for incremental replication
                        let from_seq = {
                            let rt = routing_table.read().await;
                            if let Some(nid) = rt.node_id_for_peer(&peer_id) {
                                rt.get_watermark(&nid, &shard_id)
                            } else {
                                0
                            }
                        };
                        let req = ReplicationReq {
                            shard_id: shard_id.clone(),
                            from_seq,
                            max_chunks,
                        };
                        let _outbound_id = swarm
                            .behaviour_mut()
                            .replication
                            .send_request(&peer_id, req);
                        info!("Sent CLI replication request to {peer_id} for shard {shard_id}");
                    }
                    GossipCommand::ShardAnnounce { shard_ids: new_shards } => {
                        // Update the shared shard_ids
                        *shard_ids.write().await = new_shards.clone();
                        let vector_count = index
                            .as_ref()
                            .map(|idx| idx.doc_count() as u64)
                            .unwrap_or(0);
                        // Compute min replica count across announced shards
                        let replica_count = {
                            let rt = routing_table.read().await;
                            new_shards.iter()
                                .map(|s| rt.replica_count(s))
                                .min()
                                .unwrap_or(0) as u32
                        };
                        let data = gossip::encode_shard_announce(&node_id, &new_shards, vector_count, replica_count);
                        let topic = get_gossip_topic();
                        match swarm.behaviour_mut().gossipsub.publish(topic, data) {
                            Ok(_) => info!("Published shard re-announcement: {:?}", new_shards),
                            Err(e) => debug!("Shard re-announcement publish failed: {e}"),
                        }
                    }
                }
            }

            _ = fanout_check_interval.tick() => {
                check_fanout_timeouts(&mut pending_fanouts, query_timeout);
            }

            _ = heartbeat_interval.tick() => {
                let shard_count = shard_ids.read().await.len() as u32;
                let vectors_stored = index.as_ref().map(|idx| idx.doc_count() as u64).unwrap_or(0);
                let queries_served = earnings.queries_served() as u32;
                let data = gossip::encode_heartbeat(&node_id, shard_count, vectors_stored, queries_served);
                let topic = get_gossip_topic();
                match swarm.behaviour_mut().gossipsub.publish(topic, data) {
                    Ok(_) => debug!("Heartbeat published"),
                    Err(e) => debug!("Heartbeat publish failed (expected if no peers): {e}"),
                }
            }

            _ = eviction_interval.tick() => {
                let mut rt = routing_table.write().await;
                let evicted = rt.evict_stale(Duration::from_secs(300));
                if !evicted.is_empty() {
                    info!("Evicted {} stale peers", evicted.len());
                }
            }

            _ = earnings_save_interval.tick() => {
                save_earnings_snapshot(&earnings);
            }

            _ = routing_save_interval.tick() => {
                if let Some(ref path) = routing_table_path {
                    let rt = routing_table.read().await;
                    if let Err(e) = rt.save_to_file(std::path::Path::new(path)) {
                        warn!("Failed to save routing table: {e}");
                    }
                }
            }

            _ = replication_check_interval.tick() => {
                if min_replicas_per_shard > 0 {
                    let local_shards = shard_ids.read().await.clone();
                    let rt = routing_table.read().await;
                    let under = rt.under_replicated_shards(min_replicas_per_shard);
                    drop(rt);

                    for (shard_id, count, sources) in under {
                        // Only replicate shards we don't already serve
                        if local_shards.contains(&shard_id) {
                            continue;
                        }
                        if let Some((_node_id, peer_id)) = sources.first() {
                            info!(
                                shard = %shard_id,
                                replicas = count,
                                min = min_replicas_per_shard,
                                "under-replicated shard, requesting replication"
                            );
                            action = EventAction::SendReplicationRequest {
                                peer: *peer_id,
                                shard_id,
                                max_chunks: 1000,
                            };
                            break; // One replication per tick to avoid flooding
                        }
                    }
                }
            }

            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    info!("Shutdown signal received, exiting event loop");
                    break;
                }
            }
        }

        // Apply deferred actions (swarm is no longer borrowed in the select!)
        match action {
            EventAction::None => {}
            EventAction::SendResponse(channel, response) => {
                if let Err(resp) = swarm
                    .behaviour_mut()
                    .request_response
                    .send_response(channel, response)
                {
                    warn!("Failed to send response: {}", resp.request_id);
                }
                // Contribution tracking is done at the peer_id level in behaviour events
            }
            EventAction::FanoutResponse {
                request_id,
                response,
            } => {
                if let Some(fanout) = pending_fanouts.get_mut(&request_id) {
                    fanout.remote_results.extend(response.results);
                    fanout.received_remotes += 1;
                    if fanout::maybe_complete_fanout(fanout, false) {
                        pending_fanouts.remove(&request_id);
                    }
                }
            }
            EventAction::SendReplicationResponse(channel, response) => {
                if swarm
                    .behaviour_mut()
                    .replication
                    .send_response(channel, response)
                    .is_err()
                {
                    warn!("Failed to send replication response");
                }
            }
            EventAction::IngestReplicatedChunks { peer, response } => {
                let chunk_count = response.chunks.len() as u64;
                let latest_seq = response.latest_seq;
                if let Some(idx) = &index {
                    let chunks = convert_proto_chunks_to_content_chunks(&response.chunks);
                    // Compute max seq from received chunks
                    let max_seq = chunks.iter().map(|c| c.seq).max().unwrap_or(latest_seq);
                    match idx.ingest(&chunks) {
                        Ok(()) => {
                            info!(
                                "Replicated {} chunks from {peer} (latest_seq={max_seq})",
                                chunk_count
                            );
                            // Record replication received (tit-for-tat) + update watermark.
                            // Untagged batches (sender ingested outside the topic
                            // router) count toward our own primary shard.
                            let shard_of_batch = chunks.first()
                                .and_then(|c| c.meta.as_ref())
                                .and_then(|m| m.shard_id.clone());
                            let shard_of_batch = match shard_of_batch {
                                Some(s) => Some(s),
                                None => shard_ids.read().await.first().cloned(),
                            };
                            let mut rt = routing_table.write().await;
                            if let Some(nid) = rt.node_id_for_peer(&peer) {
                                rt.record_replication_received(&nid, chunk_count);
                                if let Some(ref shard_id) = shard_of_batch
                                    && max_seq > 0
                                {
                                    rt.set_watermark(&nid, shard_id, max_seq);
                                }
                            }
                            drop(rt);

                            // Continuation: the peer holds more beyond this
                            // batch — keep pulling from the new watermark until
                            // caught up (one batch per response, no flooding).
                            if chunk_count > 0 && latest_seq > max_seq
                                && let Some(shard_id) = shard_of_batch {
                                    info!(
                                        "Replication catch-up continues: {} of latest_seq {latest_seq} synced",
                                        max_seq
                                    );
                                    let req = ReplicationReq {
                                        shard_id,
                                        from_seq: max_seq,
                                        max_chunks: 1000,
                                    };
                                    let _ = swarm.behaviour_mut().replication.send_request(&peer, req);
                                }
                        }
                        Err(e) => {
                            warn!("Failed to ingest replicated chunks from {peer}: {e}");
                        }
                    }
                }
            }
            EventAction::SendReplicationRequest {
                peer,
                shard_id,
                max_chunks,
            } => {
                // Use watermark for incremental replication
                let from_seq = {
                    let rt = routing_table.read().await;
                    if let Some(nid) = rt.node_id_for_peer(&peer) {
                        rt.get_watermark(&nid, &shard_id)
                    } else {
                        0
                    }
                };
                let req = ReplicationReq {
                    shard_id: shard_id.clone(),
                    from_seq,
                    max_chunks,
                };
                let _outbound_id = swarm
                    .behaviour_mut()
                    .replication
                    .send_request(&peer, req);
                info!("Sent replication request to {peer} for shard {shard_id} (from_seq={from_seq})");
            }
            EventAction::AddKademliaPeers(peers) => {
                for (peer_id, addr) in peers {
                    swarm.behaviour_mut().kademlia.add_address(&peer_id, addr);
                }
            }
            EventAction::AnnounceShards => {
                let shard_snapshot = shard_ids.read().await.clone();
                if !shard_snapshot.is_empty() {
                    let vector_count = index
                        .as_ref()
                        .map(|idx| idx.doc_count() as u64)
                        .unwrap_or(0);
                    let data =
                        gossip::encode_shard_announce(&node_id, &shard_snapshot, vector_count, 0);
                    match swarm.behaviour_mut().gossipsub.publish(get_gossip_topic(), data) {
                        Ok(_) => info!(
                            "Announced shards to newly subscribed peer: {:?}",
                            shard_snapshot
                        ),
                        Err(e) => debug!("Shard announcement on subscribe failed: {e}"),
                    }
                }
            }
        }
    }

    // Post-loop cleanup: flush routing table and earnings before exiting
    if let Some(ref path) = routing_table_path {
        let rt = routing_table.read().await;
        match rt.save_to_file(std::path::Path::new(path)) {
            Ok(()) => info!("Final routing table save completed"),
            Err(e) => warn!("Final routing table save failed: {e}"),
        }
    }
    save_earnings_snapshot(&earnings);
    info!("Event loop shut down cleanly");
}

/// Handle a QueryCommand from the NodeHandle — search locally and fan out to peers.
#[allow(clippy::too_many_arguments)]
async fn handle_query_command(
    cmd: QueryCommand,
    swarm: &mut Swarm<OpenSonarXBehaviour>,
    routing_table: &Arc<RwLock<RoutingTable>>,
    index: Option<&Arc<HybridIndex>>,
    earnings: &Arc<EarningsTracker>,
    node_id: &str,
    shard_ids: &[String],
    max_query_fanout: usize,
    target_coverage: f64,
    pending_fanouts: &mut HashMap<String, PendingFanout>,
    outbound_to_fanout: &mut HashMap<OutboundRequestId, String>,
    topic_router: Option<&Arc<TopicRouter>>,
) {
    let request_id = cmd.request.request_id.clone();

    // Search local index
    let local_results = if let Some(idx) = index {
        let response = handle_query_request(&cmd.request, idx, earnings, node_id, shard_ids);
        response.results
    } else {
        vec![]
    };

    // Read routing table and find peers for shards we don't serve locally
    let rt = routing_table.read().await;
    let shard_peers = rt.one_peer_per_shard();
    drop(rt);

    // Determine which remote shards to target using 3-tier priority:
    // 1. Explicit shard filters from request
    // 2. TopicRouter ranking (if available and query has an embedding)
    // 3. Fallback: all remote shards
    let explicit_shards: Vec<String> = cmd
        .request
        .filters
        .as_ref()
        .map(|f| f.shards.clone())
        .unwrap_or_default();

    let selected_remote_shards: Vec<&String> = if !explicit_shards.is_empty() {
        // Tier 1: Only fan out to explicitly requested shards
        shard_peers
            .keys()
            .filter(|sid| explicit_shards.contains(sid) && !shard_ids.contains(sid))
            .collect()
    } else if let Some(router) = topic_router {
        // Tier 2: Rank shards by topic similarity and take top-N
        let embedding: Vec<f32> = cmd
            .request
            .query_embedding
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();

        if !embedding.is_empty() {
            let ranked = router.rank_shards(&embedding);
            let ranked_ids: Vec<&str> = ranked.iter().map(|(id, _)| *id).collect();
            shard_peers
                .keys()
                .filter(|sid| !shard_ids.contains(sid))
                .filter(|sid| ranked_ids.contains(&sid.as_str()))
                .collect::<Vec<_>>()
                .into_iter()
                .take(max_query_fanout)
                .collect()
        } else {
            // No embedding — fall through to all remote shards
            shard_peers
                .keys()
                .filter(|sid| !shard_ids.contains(sid))
                .collect()
        }
    } else {
        // Tier 3: All remote shards (old behavior)
        shard_peers
            .keys()
            .filter(|sid| !shard_ids.contains(sid))
            .collect()
    };

    // Map selected shards → peers, deduplicating by PeerId
    let mut target_peers: HashMap<String, libp2p::PeerId> = HashMap::new();
    for shard_id in &selected_remote_shards {
        if let Some((_peer_node_id, peer_id)) = shard_peers.get(*shard_id) {
            target_peers
                .entry(peer_id.to_string())
                .or_insert(*peer_id);
        }
    }

    // Compute adaptive fanout based on shard coverage estimation.
    // If coverage is low (peers hold a small fraction of total shard docs),
    // increase fanout beyond the base to improve result quality.
    let adaptive_fanout = {
        let rt = routing_table.read().await;
        let max_adaptive = (max_query_fanout * 3).min(10);
        routing::compute_adaptive_fanout(
            &rt,
            &selected_remote_shards,
            max_query_fanout,
            target_coverage,
            max_adaptive,
        )
    };

    let remote_count = target_peers.len().min(adaptive_fanout);

    if remote_count == 0 {
        // No remote peers needed — immediately reply with local results
        let limit = if cmd.request.limit > 0 {
            cmd.request.limit as usize
        } else {
            5
        };
        let response =
            fanout::build_merged_response(&request_id, local_results, vec![], limit);
        let _ = cmd.reply_tx.send(response);
        return;
    }

    // Strip ticket from fanout requests — entry node keeps the ticket
    // for on-chain settlement. Remote peers get ticketless requests and
    // earn emission credit via queries_served instead.
    let mut remote_request = cmd.request.clone();
    remote_request.signed_ticket = vec![];

    // Send query to each remote peer, capped by adaptive fanout
    for peer_id in target_peers.values().take(adaptive_fanout) {
        let outbound_id = swarm
            .behaviour_mut()
            .request_response
            .send_request(peer_id, remote_request.clone());
        outbound_to_fanout.insert(outbound_id, request_id.clone());
        debug!("Sent fanout query to {peer_id} (outbound_id={outbound_id})");
    }

    // Track the pending fanout
    pending_fanouts.insert(
        request_id,
        PendingFanout {
            reply_tx: Some(cmd.reply_tx),
            request: cmd.request,
            local_results,
            remote_results: vec![],
            expected_remotes: remote_count,
            received_remotes: 0,
            created_at: Instant::now(),
        },
    );
}

/// Check for timed-out fanouts and complete them with whatever results are available.
fn check_fanout_timeouts(
    pending_fanouts: &mut HashMap<String, PendingFanout>,
    query_timeout: Duration,
) {
    let now = Instant::now();
    let timed_out: Vec<String> = pending_fanouts
        .iter()
        .filter(|(_, f)| now.duration_since(f.created_at) > query_timeout)
        .map(|(id, _)| id.clone())
        .collect();

    for request_id in timed_out {
        if let Some(fanout) = pending_fanouts.get_mut(&request_id) {
            warn!(
                "Fanout query {request_id} timed out ({}/{} remotes received)",
                fanout.received_remotes, fanout.expected_remotes
            );
            fanout::maybe_complete_fanout(fanout, true);
        }
        pending_fanouts.remove(&request_id);
    }
}

/// Save earnings snapshot to ~/.opensonarx/earnings.json
fn save_earnings_snapshot(earnings: &EarningsTracker) {
    let snap = earnings.snapshot();
    if snap.queries_served == 0 {
        return;
    }
    if let Some(home) = dirs::home_dir() {
        let dir = home.join(".opensonarx");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("earnings.json");
        if let Ok(json) = serde_json::to_string_pretty(&snap)
            && let Err(e) = std::fs::write(&path, json)
        {
            warn!("Failed to save earnings snapshot: {e}");
        }
    }
}

/// Convert `ContentChunk` (oi-types) to proto `DocumentChunk` for replication responses.
fn convert_content_chunks_to_proto(chunks: &[oi_types::ContentChunk]) -> Vec<DocumentChunk> {
    chunks
        .iter()
        .map(|c| {
            let embedding_bytes = c
                .embedding
                .as_ref()
                .map(|e| e.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>())
                .unwrap_or_default();

            let mut metadata = std::collections::HashMap::new();
            if let Some(meta) = &c.meta {
                metadata.insert("title".to_string(), meta.title.clone());
                metadata.insert("domain".to_string(), meta.domain.clone());
                if let Some(ct) = &meta.content_type {
                    metadata.insert("content_type".to_string(), ct.clone());
                }
                if let Some(entity) = &meta.entity {
                    metadata.insert("entity".to_string(), entity.clone());
                }
                if let Some(kind) = &meta.content_kind {
                    metadata.insert("content_kind".to_string(), kind.clone());
                }
                if let Some(sid) = &meta.shard_id {
                    metadata.insert("shard_id".to_string(), sid.clone());
                }
                if let Some(crawled) = meta.crawled_at {
                    metadata.insert("crawled_at".to_string(), crawled.to_string());
                }
            }

            DocumentChunk {
                chunk_id: c.chunk_id.clone(),
                url: c.meta.as_ref().map(|m| m.url.clone()).unwrap_or_default(),
                content: c.content.clone(),
                embedding: embedding_bytes,
                metadata,
                content_hash: c.meta.as_ref().map(|m| m.content_hash.clone()).unwrap_or_default(),
                seq: c.seq,
            }
        })
        .collect()
}

/// Convert proto `DocumentChunk` to `ContentChunk` (oi-types) for ingestion.
fn convert_proto_chunks_to_content_chunks(proto_chunks: &[DocumentChunk]) -> Vec<oi_types::ContentChunk> {
    proto_chunks
        .iter()
        .map(|pc| {
            let embedding = if pc.embedding.len() >= 4 {
                Some(
                    pc.embedding
                        .chunks_exact(4)
                        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                        .collect::<Vec<f32>>(),
                )
            } else {
                None
            };

            let crawled_at = pc
                .metadata
                .get("crawled_at")
                .and_then(|s| s.parse::<u64>().ok());

            let meta = oi_types::ChunkMeta {
                url: pc.url.clone(),
                title: pc.metadata.get("title").cloned().unwrap_or_default(),
                domain: pc.metadata.get("domain").cloned().unwrap_or_default(),
                content_hash: pc.content_hash.clone(),
                crawled_at,
                content_type: pc.metadata.get("content_type").cloned(),
                entity: pc.metadata.get("entity").cloned(),
                content_kind: pc.metadata.get("content_kind").cloned(),
                shard_id: pc.metadata.get("shard_id").cloned(),
                ..Default::default()
            };

            oi_types::ContentChunk {
                chunk_id: pc.chunk_id.clone(),
                content: pc.content.clone(),
                embedding,
                seq: pc.seq,
                meta: Some(meta),
            }
        })
        .collect()
}

/// Validate a signed ticket: check payee, amounts, expiry, replay, and Ed25519 signature.
fn validate_ticket(ticket: &SignedTicket, our_pubkey: &str, earnings: &EarningsTracker) -> bool {
    // Check payee matches us
    if ticket.payee_pubkey != our_pubkey {
        warn!(
            "Ticket payee mismatch: expected {our_pubkey}, got {}",
            ticket.payee_pubkey
        );
        return false;
    }

    if ticket.amount_lamports == 0 {
        warn!("Ticket has zero amount");
        return false;
    }

    if ticket.expiry_slot == 0 {
        warn!("Ticket has zero expiry");
        return false;
    }

    // Check expiry: treat expiry_slot as a Unix timestamp.
    // u64::MAX means "never expires" (used by mock channels).
    if ticket.expiry_slot != u64::MAX {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if ticket.expiry_slot < now {
            warn!(
                "Ticket expired: expiry_slot={} < now={now}",
                ticket.expiry_slot
            );
            return false;
        }
    }

    // Replay protection: reject if we've already seen this (payer, nonce)
    if !earnings.check_and_record_nonce(&ticket.payer_pubkey, ticket.nonce) {
        warn!(
            "Ticket replay detected: payer={} nonce={}",
            ticket.payer_pubkey, ticket.nonce
        );
        return false;
    }

    // Decode payer pubkey from hex → Ed25519 VerifyingKey
    let pubkey_bytes: Vec<u8> = match (0..ticket.payer_pubkey.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&ticket.payer_pubkey[i..i + 2], 16))
        .collect::<Result<Vec<u8>, _>>()
    {
        Ok(bytes) => bytes,
        Err(_) => {
            warn!("Ticket payer_pubkey is not valid hex: {}", ticket.payer_pubkey);
            return false;
        }
    };

    let pubkey_array: [u8; 32] = match pubkey_bytes.try_into() {
        Ok(arr) => arr,
        Err(_) => {
            warn!("Ticket payer_pubkey wrong length (expected 32 bytes)");
            return false;
        }
    };

    let verifying_key = match ed25519_dalek::VerifyingKey::from_bytes(&pubkey_array) {
        Ok(vk) => vk,
        Err(_) => {
            warn!("Ticket payer_pubkey is not a valid Ed25519 key");
            return false;
        }
    };

    // Reconstruct canonical payload: {payer}|{payee}|{amount}|{nonce}|{expiry}
    let payload = format!(
        "{}|{}|{}|{}|{}",
        ticket.payer_pubkey,
        ticket.payee_pubkey,
        ticket.amount_lamports,
        ticket.nonce,
        ticket.expiry_slot
    );

    // Verify Ed25519 signature
    let signature = match ed25519_dalek::Signature::from_slice(&ticket.signature) {
        Ok(sig) => sig,
        Err(_) => {
            warn!("Ticket signature is not valid Ed25519 (expected 64 bytes, got {})", ticket.signature.len());
            return false;
        }
    };

    use ed25519_dalek::Verifier;
    if verifying_key.verify(payload.as_bytes(), &signature).is_err() {
        warn!("Ticket Ed25519 signature verification failed");
        return false;
    }

    true
}

/// Handle a query request: decode embedding, search index, build proto response.
fn handle_query_request(
    request: &QueryRequest,
    index: &HybridIndex,
    earnings: &EarningsTracker,
    node_id: &str,
    shard_ids: &[String],
) -> QueryResponse {
    let start = std::time::Instant::now();

    // Decode the ticket from the request
    if !request.signed_ticket.is_empty()
        && let Ok(ticket) = bincode::deserialize::<SignedTicket>(&request.signed_ticket)
        && validate_ticket(&ticket, node_id, earnings)
    {
        earnings.record_ticket(ticket);
    } else {
        // Ticketless query (e.g., fanout from entry node) — record for emission credit
        earnings.record_query_served();
    }

    // Decode embedding from bytes (f32 LE)
    let embedding: Vec<f32> = request
        .query_embedding
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();

    if embedding.is_empty() {
        return QueryResponse {
            request_id: request.request_id.clone(),
            meta: None,
            results: vec![],
            error: Some(ErrorInfo {
                code: "E1004".to_string(),
                name: "INVALID_FILTER".to_string(),
                message: "Query embedding is empty or has invalid length".to_string(),
                retryable: false,
                retry_after_ms: 0,
            }),
        };
    }

    // Build filters
    let filters = request
        .filters
        .as_ref()
        .map(oi_types::QueryFilters::from_proto)
        .unwrap_or_default();

    let limit = if request.limit > 0 {
        request.limit as usize
    } else {
        5
    };

    // Search the index
    match index.query(&embedding, &request.query_text, &filters, limit) {
        Ok(results) => {
            let latency_ms = start.elapsed().as_millis() as u32;
            let shard_id = shard_ids.first().cloned().unwrap_or_default();

            let proto_results: Vec<SearchResult> = results
                .iter()
                .map(|r| SearchResult {
                    result_id: r.result_id.clone(),
                    rank: r.rank,
                    title: r.title.clone(),
                    url: r.url.clone(),
                    snippet: r.snippet.clone(),
                    content_md: r.content_md.clone(),
                    scores: Some(Scores {
                        semantic: r.semantic_score as f32,
                        bm25: r.bm25_score as f32,
                        r#final: r.final_score as f32,
                    }),
                    stake: None,
                    utility: None,
                    freshness: Some(FreshnessInfo {
                        crawled_at: 0,
                        published_at: 0,
                        content_hash: r.content_hash.clone(),
                    }),
                    extraction_quality: r.extraction_quality.clone(),
                    source: r.source.clone(),
                    entity: r.entity.clone().unwrap_or_default(),
                    content_kind: r.content_kind.clone().unwrap_or_default(),
                })
                .collect();

            QueryResponse {
                request_id: request.request_id.clone(),
                meta: Some(ResponseMeta {
                    latency_ms,
                    cost_micro_usd: 1500,
                    shard_id,
                    node_id: node_id.to_string(),
                    model_version: "local".to_string(),
                }),
                results: proto_results,
                error: None,
            }
        }
        Err(e) => QueryResponse {
            request_id: request.request_id.clone(),
            meta: None,
            results: vec![],
            error: Some(ErrorInfo {
                code: "E1002".to_string(),
                name: "QUERY_FAILED".to_string(),
                message: format!("Index query failed: {e}"),
                retryable: true,
                retry_after_ms: 1000,
            }),
        },
    }
}

/// Handle a single swarm event by dispatching to the appropriate handler.
#[allow(clippy::too_many_arguments)]
async fn handle_swarm_event(
    event: SwarmEvent<OpenSonarXBehaviourEvent>,
    routing_table: &Arc<RwLock<RoutingTable>>,
    index: Option<&Arc<HybridIndex>>,
    earnings: &Arc<EarningsTracker>,
    node_id: &str,
    shard_ids: &[String],
    pending_fanouts: &mut HashMap<String, PendingFanout>,
    outbound_to_fanout: &mut HashMap<OutboundRequestId, String>,
    enable_tit_for_tat: bool,
    min_contribution_ratio: f64,
) -> EventAction {
    match event {
        SwarmEvent::Behaviour(behaviour_event) => {
            handle_behaviour_event(
                behaviour_event,
                routing_table,
                index,
                earnings,
                node_id,
                shard_ids,
                pending_fanouts,
                outbound_to_fanout,
                enable_tit_for_tat,
                min_contribution_ratio,
            )
            .await
        }
        SwarmEvent::NewListenAddr { address, .. } => {
            info!("Listening on {address}");
            EventAction::None
        }
        SwarmEvent::ConnectionEstablished {
            peer_id, endpoint, ..
        } => {
            info!("Connection established with {peer_id} via {endpoint:?}");
            EventAction::None
        }
        SwarmEvent::ConnectionClosed { peer_id, cause, .. } => {
            info!("Connection closed with {peer_id}: {cause:?}");
            EventAction::None
        }
        SwarmEvent::IncomingConnection { local_addr, .. } => {
            debug!("Incoming connection on {local_addr}");
            EventAction::None
        }
        _ => {
            debug!("Swarm event");
            EventAction::None
        }
    }
}

/// Handle a behaviour-level event.
#[allow(clippy::too_many_arguments)]
async fn handle_behaviour_event(
    event: OpenSonarXBehaviourEvent,
    routing_table: &Arc<RwLock<RoutingTable>>,
    index: Option<&Arc<HybridIndex>>,
    earnings: &Arc<EarningsTracker>,
    node_id: &str,
    shard_ids: &[String],
    pending_fanouts: &mut HashMap<String, PendingFanout>,
    outbound_to_fanout: &mut HashMap<OutboundRequestId, String>,
    enable_tit_for_tat: bool,
    min_contribution_ratio: f64,
) -> EventAction {
    match event {
        OpenSonarXBehaviourEvent::Gossipsub(gossipsub::Event::Message {
            message,
            propagation_source,
            ..
        }) => {
            debug!("Gossip message from {propagation_source}");
            if let Some(gossip_msg) = gossip::decode_gossip(&message.data) {
                match gossip_msg.payload {
                    Some(gossip_message::Payload::Heartbeat(hb)) => {
                        debug!(
                            "Heartbeat from {}: shards={}, vectors={}",
                            hb.node_id, hb.shard_count, hb.vectors_stored
                        );
                        let mut rt = routing_table.write().await;
                        rt.update_peer(&hb.node_id, vec![], hb.vectors_stored);
                        // Map the gossip source PeerId to this node_id
                        rt.set_peer_id(&hb.node_id, propagation_source);
                    }
                    Some(gossip_message::Payload::ShardAnnounce(sa)) => {
                        info!(
                            "Shard announce from {}: {:?}",
                            sa.node_id, sa.shard_ids
                        );
                        let mut rt = routing_table.write().await;
                        rt.update_peer_with_id(
                            &sa.node_id,
                            sa.shard_ids.clone(),
                            sa.vector_count,
                            Some(propagation_source),
                        );
                        drop(rt);

                        // Catch-up sync: a peer serving one of OUR shards with
                        // substantially more content means we're behind (e.g.
                        // we just joined). Pull incrementally from our
                        // watermark. Without this, same-shard peers never
                        // exchange backlog — queries don't fan out to them
                        // (replicas are assumed in sync) and the
                        // content-announcement path only covers NEW crawls.
                        if let Some(idx) = &index {
                            let our_docs = idx.doc_count() as u64;
                            if sa.vector_count > our_docs.saturating_mul(2).max(100)
                                && let Some(shard_id) = sa
                                    .shard_ids
                                    .iter()
                                    .find(|sid| shard_ids.contains(*sid))
                                {
                                    info!(
                                        "Peer {} has {} vectors vs our {} in shard {shard_id} — catch-up replication",
                                        sa.node_id, sa.vector_count, our_docs
                                    );
                                    return EventAction::SendReplicationRequest {
                                        peer: propagation_source,
                                        shard_id: shard_id.clone(),
                                        max_chunks: 1000,
                                    };
                                }
                        }
                    }
                    Some(gossip_message::Payload::StakeEvent(se)) => {
                        info!(
                            "Stake event: domain={}, amount={}, action={:?}",
                            se.domain, se.amount, se.action
                        );
                    }
                    Some(gossip_message::Payload::ContentAnnouncement(ca)) => {
                        let action_str = if ca.is_update { "updated" } else { "discovered" };
                        info!(
                            "Content {}: url={} domain={} hash={} chunks={} by sentinel={}",
                            action_str, ca.url, ca.domain, ca.content_hash, ca.chunk_count,
                            ca.sentinel_id
                        );
                        // Trigger replication pull if we serve this shard and have an index
                        if index.is_some() && !ca.shard_id.is_empty()
                            && shard_ids.contains(&ca.shard_id)
                        {
                            info!(
                                "Triggering replication pull from {} for shard {}",
                                propagation_source, ca.shard_id
                            );
                            return EventAction::SendReplicationRequest {
                                peer: propagation_source,
                                shard_id: ca.shard_id,
                                max_chunks: ca.chunk_count.max(100),
                            };
                        }
                    }
                    Some(gossip_message::Payload::Replication(_)) => {
                        debug!("Replication request received via gossip (ignored)");
                    }
                    None => {
                        warn!("Gossip message with empty payload");
                    }
                }
            }
            EventAction::None
        }
        OpenSonarXBehaviourEvent::Gossipsub(gossipsub::Event::Subscribed { peer_id, topic }) => {
            debug!("Peer {peer_id} subscribed to {topic}");
            EventAction::AnnounceShards
        }
        OpenSonarXBehaviourEvent::Gossipsub(gossipsub::Event::Unsubscribed { peer_id, topic }) => {
            debug!("Peer {peer_id} unsubscribed from {topic}");
            EventAction::None
        }
        OpenSonarXBehaviourEvent::Gossipsub(_) => {
            debug!("Other gossipsub event");
            EventAction::None
        }
        OpenSonarXBehaviourEvent::RequestResponse(request_response::Event::Message {
            peer,
            message,
        }) => match message {
            request_response::Message::Request {
                request,
                channel,
                request_id,
            } => {
                info!(
                    "Query request from {peer} (id={request_id}): request_id={}",
                    request.request_id
                );
                // Record that we served a query for this peer (tit-for-tat)
                // and optionally throttle leechers
                {
                    let mut rt = routing_table.write().await;
                    if let Some(nid) = rt.node_id_for_peer(&peer) {
                        // Check throttle before serving
                        if enable_tit_for_tat && rt.should_throttle(&nid, min_contribution_ratio) {
                            warn!("Throttling leecher peer {peer} (node_id={nid})");
                            let response = QueryResponse {
                                request_id: request.request_id,
                                meta: None,
                                results: vec![],
                                error: Some(ErrorInfo {
                                    code: "E2002".to_string(),
                                    name: "THROTTLED".to_string(),
                                    message: "Peer throttled due to low contribution ratio".to_string(),
                                    retryable: true,
                                    retry_after_ms: 30_000,
                                }),
                            };
                            return EventAction::SendResponse(channel, response);
                        }
                        rt.record_query_served(&nid);
                    }
                }
                let response = if let Some(idx) = index {
                    handle_query_request(&request, idx, earnings, node_id, shard_ids)
                } else {
                    QueryResponse {
                        request_id: request.request_id,
                        meta: None,
                        results: vec![],
                        error: Some(ErrorInfo {
                            code: "E2001".to_string(),
                            name: "NO_INDEX".to_string(),
                            message: "This node has no search index attached".to_string(),
                            retryable: false,
                            retry_after_ms: 0,
                        }),
                    }
                };
                EventAction::SendResponse(channel, response)
            }
            request_response::Message::Response {
                request_id,
                response,
            } => {
                // Record that this peer answered our query (tit-for-tat)
                {
                    let mut rt = routing_table.write().await;
                    if let Some(nid) = rt.node_id_for_peer(&peer) {
                        rt.record_query_answered(&nid);
                    }
                }
                // Correlate this response with a pending fanout
                if let Some(fanout_request_id) = outbound_to_fanout.remove(&request_id) {
                    debug!(
                        "Fanout response from {peer} for {fanout_request_id} ({} results)",
                        response.results.len()
                    );
                    EventAction::FanoutResponse {
                        request_id: fanout_request_id,
                        response,
                    }
                } else {
                    info!(
                        "Query response for request_id={request_id}: {}",
                        response.request_id
                    );
                    EventAction::None
                }
            }
        },
        OpenSonarXBehaviourEvent::RequestResponse(
            request_response::Event::OutboundFailure {
                peer,
                request_id,
                error,
            },
        ) => {
            warn!("Outbound query to {peer} failed (id={request_id}): {error}");
            // Count this as a received (failed) remote for fanout purposes
            if let Some(fanout_request_id) = outbound_to_fanout.remove(&request_id)
                && let Some(fanout) = pending_fanouts.get_mut(&fanout_request_id)
            {
                fanout.received_remotes += 1;
                if fanout::maybe_complete_fanout(fanout, false) {
                    pending_fanouts.remove(&fanout_request_id);
                }
            }
            EventAction::None
        }
        OpenSonarXBehaviourEvent::RequestResponse(
            request_response::Event::InboundFailure {
                peer,
                request_id,
                error,
            },
        ) => {
            warn!("Inbound query from {peer} failed (id={request_id}): {error}");
            EventAction::None
        }
        OpenSonarXBehaviourEvent::RequestResponse(request_response::Event::ResponseSent {
            peer,
            request_id,
        }) => {
            debug!("Response sent to {peer} for request {request_id}");
            EventAction::None
        }
        OpenSonarXBehaviourEvent::Kademlia(event) => {
            debug!("Kademlia event: {event:?}");
            EventAction::None
        }
        OpenSonarXBehaviourEvent::Identify(event) => {
            debug!("Identify event: {event:?}");
            EventAction::None
        }
        OpenSonarXBehaviourEvent::Replication(request_response::Event::Message {
            peer,
            message,
        }) => match message {
            request_response::Message::Request {
                request,
                channel,
                request_id,
            } => {
                info!(
                    "Replication request from {peer} (id={request_id}): shard={} max_chunks={}",
                    request.shard_id, request.max_chunks
                );
                if let Some(idx) = index {
                    // If the requested shard is one WE announce, serve our whole
                    // index: chunks ingested outside the topic router carry no
                    // shard tag (shard_id None), but from the network's view
                    // they ARE this node's announced shard — a strict tag
                    // filter returned 0 chunks and silently broke catch-up
                    // replication between same-shard peers.
                    let shard_filter = if request.shard_id.is_empty()
                        || shard_ids.contains(&request.shard_id)
                    {
                        None
                    } else {
                        Some(request.shard_id.as_str())
                    };
                    let max = if request.max_chunks == 0 { 1000 } else { request.max_chunks as usize };
                    // Use incremental replication: only return chunks after from_seq
                    let from_seq = request.from_seq;
                    match idx.chunks_since(shard_filter, from_seq, max) {
                        Ok(chunks) => {
                            let latest_seq = idx.current_seq();
                            let proto_chunks = convert_content_chunks_to_proto(&chunks);
                            let chunk_count = proto_chunks.len() as u64;
                            let resp = ReplicationResp {
                                chunks: proto_chunks,
                                latest_seq,
                            };
                            // Record replication sent (tit-for-tat)
                            if chunk_count > 0 {
                                let mut rt = routing_table.write().await;
                                if let Some(nid) = rt.node_id_for_peer(&peer) {
                                    rt.record_replication_sent(&nid, chunk_count);
                                }
                            }
                            EventAction::SendReplicationResponse(channel, resp)
                        }
                        Err(e) => {
                            warn!("Failed to get chunks for replication: {e}");
                            let resp = ReplicationResp {
                                chunks: vec![],
                                latest_seq: 0,
                            };
                            EventAction::SendReplicationResponse(channel, resp)
                        }
                    }
                } else {
                    let resp = ReplicationResp {
                        chunks: vec![],
                        latest_seq: 0,
                    };
                    EventAction::SendReplicationResponse(channel, resp)
                }
            }
            request_response::Message::Response {
                response,
                ..
            } => {
                info!(
                    "Replication response from {peer}: {} chunks",
                    response.chunks.len()
                );
                if response.chunks.is_empty() {
                    EventAction::None
                } else {
                    EventAction::IngestReplicatedChunks {
                        peer,
                        response,
                    }
                }
            }
        },
        OpenSonarXBehaviourEvent::Replication(request_response::Event::OutboundFailure {
            peer,
            error,
            ..
        }) => {
            warn!("Replication request to {peer} failed: {error}");
            EventAction::None
        }
        OpenSonarXBehaviourEvent::Replication(request_response::Event::InboundFailure {
            peer,
            error,
            ..
        }) => {
            warn!("Inbound replication from {peer} failed: {error}");
            EventAction::None
        }
        OpenSonarXBehaviourEvent::Replication(request_response::Event::ResponseSent {
            peer,
            request_id,
        }) => {
            debug!("Replication response sent to {peer} for request {request_id}");
            EventAction::None
        }
        OpenSonarXBehaviourEvent::Mdns(event) => {
            match event {
                mdns::Event::Discovered(peers) => {
                    if !peers.is_empty() {
                        info!("mDNS discovered {} peers", peers.len());
                    }
                    EventAction::AddKademliaPeers(peers)
                }
                mdns::Event::Expired(_) => EventAction::None,
            }
        }
        OpenSonarXBehaviourEvent::Autonat(event) => {
            match event {
                libp2p::autonat::Event::StatusChanged { old, new } => {
                    info!("AutoNAT status changed: {old:?} → {new:?}");
                }
                _ => {
                    debug!("AutoNAT event");
                }
            }
            EventAction::None
        }
        OpenSonarXBehaviourEvent::RelayClient(event) => {
            debug!("Relay client event: {event:?}");
            EventAction::None
        }
        OpenSonarXBehaviourEvent::Dcutr(event) => {
            info!("DCUtR event: {event:?}");
            EventAction::None
        }
        OpenSonarXBehaviourEvent::Upnp(event) => {
            match event {
                libp2p::upnp::Event::NewExternalAddr(addr) => {
                    info!("UPnP: mapped external address {addr}");
                }
                libp2p::upnp::Event::ExpiredExternalAddr(addr) => {
                    info!("UPnP: expired external address {addr}");
                }
                libp2p::upnp::Event::GatewayNotFound => {
                    debug!("UPnP: no gateway found");
                }
                libp2p::upnp::Event::NonRoutableGateway => {
                    debug!("UPnP: gateway is not routable");
                }
            }
            EventAction::None
        }
    }
}
