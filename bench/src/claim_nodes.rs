//! The ingestion-time corroboration signal served by real network nodes.
//!
//! Stored claims are split across N nodes by a hash of their URL. Each node is
//! an `oi_network::Node` over localhost TCP holding its share in an ordinary
//! `HybridIndex`, announced as one shard. A query goes to node 0 only: the
//! node searches its own shard, fans the query out to the peers that serve
//! the others, and merges what comes back, all through the network layer's
//! existing query path. The caller then counts the source families that state
//! the head claim, with the SDK's `supporting_families`.
//!
//! Run with one node and with three, the two outputs show whether splitting
//! the claims across nodes changes the signal and what it costs in time.
//!
//! Differences from `claim-signal`, which reads a single in-process vector
//! index: the node's search is the hybrid one (vector plus keywords, best
//! result per URL), and the caller has to embed the returned claims again to
//! compare them with the head, because vectors do not travel in a response.

use oi_index::{HybridConfig, HybridIndex};
use oi_network::config::NetworkConfig;
use oi_network::{Node, NodeHandle};
use oi_sdk::claims::{supporting_families, ClaimPolicy, ClaimRecord, MatchedClaim};
use oi_types::proto::opensonarx::{Filters, QueryRequest};
use oi_types::traits::SearchIndex;
use oi_types::{ChunkMeta, ContentChunk};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Separates the claim from its quote inside a stored chunk.
const QUOTE_MARK: &str = "\n> ";
const CLAIM_KIND: &str = "claim";

fn shard_of(url: &str, shards: usize) -> usize {
    let digest = Sha256::digest(url.as_bytes());
    u64::from_le_bytes(digest[..8].try_into().unwrap_or([0; 8])) as usize % shards.max(1)
}

fn chunk(index: usize, record: &ClaimRecord, vector: Vec<f32>, shard_id: &str) -> ContentChunk {
    let content = format!("{}{QUOTE_MARK}{}", record.claim, record.quote);
    let content_hash: String = Sha256::digest(format!("{}\n{content}", record.url).as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    ContentChunk {
        chunk_id: format!("claim-{index}"),
        content,
        embedding: Some(vector),
        seq: index as u64,
        meta: Some(ChunkMeta {
            url: record.url.clone(),
            title: String::new(),
            domain: record.source_family.clone(),
            content_hash,
            crawled_at: None,
            content_type: None,
            entity: None,
            content_kind: Some(CLAIM_KIND.to_string()),
            quality_score: None,
            spam_score: None,
            slop_score: None,
            published_at: None,
            shard_id: Some(shard_id.to_string()),
            facts: None,
        }),
    }
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm = |v: &[f32]| v.iter().map(|x| x * x).sum::<f32>().sqrt();
    let denominator = norm(a) * norm(b);
    if denominator > 0.0 {
        dot / denominator
    } else {
        0.0
    }
}

fn percentile(values: &[f64], quantile: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let index = ((sorted.len() as f64 - 1.0) * quantile).round() as usize;
    Some(sorted[index.min(sorted.len() - 1)])
}

fn latency(values: &[f64]) -> Value {
    json!({
        "n": values.len(),
        "p50_ms": percentile(values, 0.50),
        "p95_ms": percentile(values, 0.95),
        "max_ms": percentile(values, 1.0),
    })
}

fn read_records(claims_path: &str) -> Result<Vec<ClaimRecord>, String> {
    let raw = std::fs::read_to_string(claims_path)
        .map_err(|error| format!("cannot read {claims_path}: {error}"))?;
    Ok(raw
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .flat_map(|line| {
            line["records"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter_map(|record| serde_json::from_value(record).ok())
        })
        .collect())
}

async fn start_nodes(
    indexes: Vec<Arc<HybridIndex>>,
    base_port: u16,
) -> Result<Vec<NodeHandle>, String> {
    let mut handles = Vec::new();
    let expected_remote_shards = indexes.len() - 1;
    let mut entry_routing = None;
    for (position, index) in indexes.into_iter().enumerate() {
        let config = NetworkConfig {
            listen_port: base_port + position as u16,
            bootstrap_peers: if position == 0 {
                Vec::new()
            } else {
                vec![format!("/ip4/127.0.0.1/tcp/{base_port}")]
            },
            heartbeat_interval_secs: 1,
            enable_mdns: false,
            // Each shard lives on exactly one node here; replication would
            // copy shards between nodes while the measurement runs.
            replication_check_interval_secs: 86_400,
            routing_table_path: None,
            ..NetworkConfig::default()
        };
        let node = Node::new(config).with_index(index, vec![format!("claims-{position}")]);
        if position == 0 {
            entry_routing = Some(node.routing_table());
        }
        handles.push(
            node.start()
                .await
                .map_err(|error| format!("node {position} did not start: {error}"))?,
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    // The entry node can only fan out to shards it has heard announced.
    let routing = entry_routing.ok_or("no entry node")?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let known = routing.read().await.one_peer_per_shard().len();
        if known >= expected_remote_shards {
            break;
        }
        if Instant::now() > deadline {
            return Err(format!(
                "entry node knows {known} of {expected_remote_shards} remote shards after 30 s"
            ));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Ok(handles)
}

#[allow(clippy::too_many_arguments)]
pub async fn run(
    claims_path: &str,
    reports: &[String],
    nodes: usize,
    policy: ClaimPolicy,
    base_port: u16,
    fetch_limit: Option<usize>,
    output: &str,
) -> Result<(), String> {
    let nodes = nodes.max(1);
    let records = read_records(claims_path)?;
    if records.is_empty() {
        return Err(format!("{claims_path} has no claim records"));
    }
    let embedder = oi_sdk::OpenSonarXClient::default_embedder();
    let dimension = embedder.dimension();

    // A claims shard is not a page index: the same statement from two sites
    // must both be kept, and every claim of a page is kept.
    let config = HybridConfig {
        skip_dedup: true,
        max_chunks_per_url: 0,
        max_per_domain: 0,
        embedder_name: embedder.name().to_string(),
        ..HybridConfig::default()
    };
    let indexes: Vec<Arc<HybridIndex>> = (0..nodes)
        .map(|_| Arc::new(HybridIndex::with_config(dimension, config.clone())))
        .collect();
    let mut per_shard = vec![0usize; nodes];
    let started = Instant::now();
    for (offset, batch) in records.chunks(64).enumerate() {
        let texts: Vec<&str> = batch.iter().map(|record| record.claim.as_str()).collect();
        let vectors = embedder
            .embed_text_batch(&texts)
            .map_err(|error| format!("embedding claims: {error}"))?;
        for (position, (record, vector)) in batch.iter().zip(vectors).enumerate() {
            let shard = shard_of(&record.url, nodes);
            per_shard[shard] += 1;
            indexes[shard]
                .ingest(&[chunk(
                    offset * 64 + position,
                    record,
                    vector,
                    &format!("claims-{shard}"),
                )])
                .map_err(|error| format!("ingest: {error}"))?;
        }
    }
    eprintln!(
        "{} claims on {nodes} node(s) {per_shard:?}, embedded and ingested in {:.0} s",
        records.len(),
        started.elapsed().as_secs_f64()
    );

    let handles = start_nodes(indexes, base_port).await?;
    let entry = &handles[0];
    eprintln!("{nodes} node(s) up; querying node 0");

    let mut results = Vec::new();
    let (mut embed_times, mut network_times, mut count_times, mut total_times) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut sequence = 0u64;
    for path in reports {
        let raw = std::fs::read_to_string(path)
            .map_err(|error| format!("cannot read {path}: {error}"))?;
        let report: Value =
            serde_json::from_str(&raw).map_err(|error| format!("{path}: {error}"))?;
        let report_stem = std::path::Path::new(path)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or(path)
            .to_string();
        for result in report["results"].as_array().into_iter().flatten() {
            let query = result["query"].as_str().unwrap_or_default();
            sequence += 1;

            let started = Instant::now();
            let vector = embedder
                .embed_query(query)
                .map_err(|error| format!("embedding query: {error}"))?;
            let embed_ms = started.elapsed().as_secs_f64() * 1_000.0;

            let request = QueryRequest {
                request_id: format!("claims-{sequence}"),
                query_embedding: vector
                    .iter()
                    .flat_map(|value| value.to_le_bytes())
                    .collect(),
                query_text: query.to_string(),
                filters: Some(Filters {
                    content_kinds: vec![CLAIM_KIND.to_string()],
                    ..Filters::default()
                }),
                limit: fetch_limit.unwrap_or(policy.top_k) as u32,
                ..QueryRequest::default()
            };
            let started = Instant::now();
            let response = entry
                .query(request)
                .await
                .map_err(|error| format!("node query: {error}"))?;
            let network_ms = started.elapsed().as_secs_f64() * 1_000.0;

            // With a wider fetch the caller re-ranks what came back by the
            // absolute similarity to the request, which means the same thing
            // on every node, and keeps the closest `top_k`. A node's own
            // ordering uses a score that is relative to that node's results.
            let started = Instant::now();
            let mut returned: Vec<_> = response.results.iter().collect();
            if fetch_limit.is_some() {
                returned.sort_by(|a, b| {
                    let similarity = |item: &&oi_types::proto::opensonarx::SearchResult| {
                        item.scores.as_ref().map_or(0.0, |scores| scores.semantic)
                    };
                    similarity(b).total_cmp(&similarity(a))
                });
                returned.truncate(policy.top_k);
            }
            // Vectors do not travel in a response, so the returned claims are
            // embedded again to compare each with the head.
            let claims: Vec<(String, String)> = returned
                .iter()
                .map(|item| match item.content_md.split_once(QUOTE_MARK) {
                    Some((claim, quote)) => (claim.to_string(), quote.to_string()),
                    None => (item.content_md.clone(), String::new()),
                })
                .collect();
            let texts: Vec<&str> = claims.iter().map(|(claim, _)| claim.as_str()).collect();
            let vectors = if texts.is_empty() {
                Vec::new()
            } else {
                embedder
                    .embed_text_batch(&texts)
                    .map_err(|error| format!("embedding returned claims: {error}"))?
            };
            let matches: Vec<MatchedClaim> = returned
                .iter()
                .zip(&claims)
                .zip(&vectors)
                .map(|((item, (claim, quote)), vector)| MatchedClaim {
                    claim: claim.clone(),
                    quote: quote.clone(),
                    url: item.url.clone(),
                    source_family: oi_sdk::source::source_family(&item.url),
                    query_similarity: item.scores.as_ref().map_or(0.0, |scores| scores.semantic),
                    head_similarity: cosine(&vectors[0], vector),
                })
                .collect();
            let families = supporting_families(&matches, &policy);
            let count_ms = started.elapsed().as_secs_f64() * 1_000.0;

            embed_times.push(embed_ms);
            network_times.push(network_ms);
            count_times.push(count_ms);
            total_times.push(embed_ms + network_ms + count_ms);
            results.push(json!({
                "report": report_stem,
                "id": result["id"],
                "query": query,
                "supporting_families": families,
                "corroborated": families >= policy.min_source_families,
                "embed_ms": embed_ms,
                "node_query_ms": network_ms,
                "count_ms": count_ms,
                "matches": matches,
            }));
        }
    }
    for handle in &handles {
        handle.shutdown();
    }

    let corroborated = results
        .iter()
        .filter(|result| result["corroborated"] == true)
        .count();
    let summary = json!({
        "nodes": nodes,
        "fetch_limit": fetch_limit,
        "claims": records.len(),
        "claims_per_node": per_shard,
        "embedder": embedder.name(),
        "queries": results.len(),
        "corroborated": corroborated,
        "query_embed": latency(&embed_times),
        // The node's own search plus, with more than one node, the fan-out
        // to its peers and the merge.
        "node_query": latency(&network_times),
        "claim_lookup": latency(&count_times),
        "query_total": latency(&total_times),
    });
    let report = json!({
        "schema_version": 1,
        "claims_path": claims_path,
        "policy": policy,
        "summary": summary,
        "limitations": [
            "All nodes run in one process on one machine and talk over localhost TCP; there is no real network latency.",
            "Every query is covered: the claim shards hold the passages these same queries retrieved.",
            "The node's search is the hybrid one and returns the best claim per URL; it is not the single vector index of claim-signal.",
            "One query at a time."
        ],
        "results": results,
    });
    let body = serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?;
    std::fs::write(output, body + "\n")
        .map_err(|error| format!("cannot write {output}: {error}"))?;
    println!(
        "claim-nodes → {output}\n{}",
        serde_json::to_string_pretty(&report["summary"]).unwrap_or_default()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_url_always_lands_on_the_same_shard() {
        let url = "https://example.org/page";
        assert_eq!(shard_of(url, 3), shard_of(url, 3));
        assert!(shard_of(url, 3) < 3);
        assert_eq!(shard_of(url, 1), 0);
    }

    #[test]
    fn a_stored_claim_carries_its_quote_and_kind() {
        let record = ClaimRecord {
            claim: "Antarctica is the largest desert.".to_string(),
            quote: "the largest desert in the world is Antarctica".to_string(),
            url: "https://www.example.org/deserts".to_string(),
            source_family: "example.org".to_string(),
            extractor: "scripted".to_string(),
        };
        let stored = chunk(7, &record, vec![0.0; 3], "claims-1");
        let (claim, quote) = stored.content.split_once(QUOTE_MARK).unwrap();
        assert_eq!(claim, record.claim);
        assert_eq!(quote, record.quote);
        let meta = stored.meta.unwrap();
        assert_eq!(meta.content_kind.as_deref(), Some(CLAIM_KIND));
        assert_eq!(meta.domain, "example.org");
        assert_eq!(meta.shard_id.as_deref(), Some("claims-1"));
    }
}
