use axum::{Json, Router, extract::State, routing::{get, post}};
use oi_network::NodeHandle;
use oi_types::proto::opensonarx as proto;
use oi_types::{QueryFilters, SearchResultItem};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Shared state for the node HTTP API.
struct ApiState {
    handle: NodeHandle,
    /// Blended-search client over the node's SHARED index: external engines,
    /// crawl-on-query learning, and spell correction on the warm path.
    client: Arc<oi_sdk::OpenSonarXClient>,
    /// The shared index + its on-disk location, for persisting learned pages.
    index: Arc<oi_index::HybridIndex>,
    index_path: Option<std::path::PathBuf>,
}

#[derive(Deserialize)]
struct SearchRequest {
    query: String,
    #[serde(default = "default_limit")]
    limit: u32,
    #[serde(default)]
    filters: QueryFilters,
}

fn default_limit() -> u32 {
    5
}

#[derive(Serialize)]
struct HealthResponse {
    status: String,
    peer_id: String,
}

async fn health_handler(State(state): State<Arc<ApiState>>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        peer_id: state.handle.peer_id.to_string(),
    })
}

async fn search_handler(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<SearchRequest>,
) -> Result<Json<Vec<SearchResultItem>>, (axum::http::StatusCode, String)> {
    let limit = req.limit as usize;

    // Blended search (local + external engines) — blocking work (ONNX embed,
    // engine HTTP via ureq) moved off the async runtime.
    let client = state.client.clone();
    let query = req.query.clone();
    let filters = req.filters.clone();
    let blended = tokio::task::spawn_blocking(move || {
        client.search_blended(&query, limit, &filters)
    })
    .await
    .map_err(|e| (
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        format!("Search task failed: {e}"),
    ))?
    .map_err(|e| (
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        format!("Search failed: {e}"),
    ))?;
    let (mut items, _sources) = blended;

    // Crawl-on-query learning: fire-and-forget — the resident process
    // indexes the external results in the background, so this query makes
    // the next one faster and better without adding response latency.
    // Learned content is persisted immediately (the shared-index client has
    // no storage of its own, and a killed node must not lose what it learned).
    let learner = state.client.clone();
    let index = state.index.clone();
    let index_path = state.index_path.clone();
    if learner.pending_crawl_count() > 0 {
        tokio::task::spawn_blocking(move || {
            let learned = learner.process_crawl_queue();
            if learned > 0 {
                tracing::info!("crawl-on-query: learned {learned} pages");
                if let Some(path) = index_path
                    && let Ok(storage) = oi_index::Storage::new(&path)
                    && let Err(e) = index.save(&storage)
                {
                    tracing::warn!("crawl-on-query: failed to persist learned pages: {e}");
                }
            }
        });
    }

    // P2P fan-out: ask network peers, re-score their answers on the same
    // absolute blend scale, and merge by score. (Peer scores arrive
    // query-relative-normalized from their index — incomparable as-is; and
    // appending after the blend already filled `limit` used to guarantee
    // every P2P result was truncated away.) With zero peers this is just a
    // fast local re-query whose results all dedup away.
    if let Ok(query_embedding) = state.client.embedder_arc().embed_query(&req.query) {
        let proto_request = proto::QueryRequest {
            request_id: uuid::Uuid::new_v4().to_string(),
            query_embedding: query_embedding
                .iter()
                .flat_map(|f| f.to_le_bytes())
                .collect(),
            query_text: req.query.clone(),
            filters: Some(req.filters.to_proto()),
            limit: req.limit,
            signed_ticket: vec![],
            sender_pubkey: String::new(),
            nonce: 0,
        };
        if let Ok(response) = state.handle.query(proto_request).await {
            let seen: std::collections::HashSet<String> =
                items.iter().map(|r| r.url.clone()).collect();
            let mut p2p_items: Vec<SearchResultItem> = proto_results_to_items(&response.results)
                .into_iter()
                .filter(|item| !seen.contains(&item.url))
                .collect();
            if !p2p_items.is_empty() {
                let client = state.client.clone();
                let query = req.query.clone();
                p2p_items = tokio::task::spawn_blocking(move || {
                    let mut p2p_items = p2p_items;
                    client.rescore_extra(&query, &mut p2p_items);
                    p2p_items
                })
                .await
                .unwrap_or_default();
                items.extend(p2p_items);
                items.sort_by(|a, b| b.final_score.total_cmp(&a.final_score));
                items.truncate(limit);
                for (i, r) in items.iter_mut().enumerate() {
                    r.rank = (i + 1) as u32;
                }
            }
        }
    }

    Ok(Json(items))
}

/// Convert proto SearchResult items to SearchResultItem.
fn proto_results_to_items(results: &[proto::SearchResult]) -> Vec<SearchResultItem> {
    results
        .iter()
        .map(|r| {
            let scores = r.scores.as_ref();
            let stake = r.stake.as_ref();
            let freshness = r.freshness.as_ref();
            SearchResultItem {
                result_id: r.result_id.clone(),
                rank: r.rank,
                title: r.title.clone(),
                url: r.url.clone(),
                snippet: r.snippet.clone(),
                content_md: r.content_md.clone(),
                semantic_score: scores.map(|s| s.semantic as f64).unwrap_or(0.0),
                bm25_score: scores.map(|s| s.bm25 as f64).unwrap_or(0.0),
                final_score: scores.map(|s| s.r#final as f64).unwrap_or(0.0),
                stake_truth: stake.map(|s| s.truth).unwrap_or(0),
                stake_usd: stake.map(|s| s.usd).unwrap_or(0),
                stake_type: stake
                    .map(|s| s.r#type.clone())
                    .unwrap_or_else(|| "domain".to_string()),
                reputation: stake.map(|s| s.rep as f64).unwrap_or(1.0),
                content_hash: freshness
                    .map(|f| f.content_hash.clone())
                    .unwrap_or_default(),
                extraction_quality: r.extraction_quality.clone(),
                source: "p2p".to_string(),
                entity: if r.entity.is_empty() {
                    None
                } else {
                    Some(r.entity.clone())
                },
                content_kind: if r.content_kind.is_empty() {
                    None
                } else {
                    Some(r.content_kind.clone())
                },
                crawled_at: freshness.and_then(|f| {
                    if f.crawled_at > 0 { Some(f.crawled_at) } else { None }
                }),
                published_at: freshness.and_then(|f| {
                    if f.published_at > 0 { Some(f.published_at as i64) } else { None }
                }),
                trust_verdict: "neutral".to_string(),
                reputation_votes: 0,
                disputed: false,
                facts: None,
                content_relevant: None,
                domain_first_seen_at: None,
                domain_url_count: None,
                domain_reputation_score: None,
                domain_total_votes: None,
            }
        })
        .collect()
}

/// Start the node HTTP API server.
///
/// `index` is the node's live shared index; `sdk_config` carries the
/// configured external engines. The API serves the full blended search
/// (local + external + P2P) with background crawl-on-query learning.
pub async fn run_node_api(
    port: u16,
    handle: NodeHandle,
    index: Arc<oi_index::HybridIndex>,
    index_path: Option<std::path::PathBuf>,
    sdk_config: oi_sdk::SdkConfig,
) -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let client = Arc::new(oi_sdk::OpenSonarXClient::with_shared_index(
        sdk_config,
        index.clone(),
    ));
    let state = Arc::new(ApiState { handle, client, index, index_path });

    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/search", post(search_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    println!("  HTTP API listening on http://0.0.0.0:{port}");
    axum::serve(listener, app).await?;
    Ok(())
}
