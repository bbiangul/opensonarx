use axum::{Json, Router, extract::State, routing::{get, post}};
use oi_network::NodeHandle;
use oi_types::proto::opensonarx as proto;
use oi_types::{QueryFilters, SearchResultItem};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Shared state for the node HTTP API.
struct ApiState {
    handle: NodeHandle,
    embedder: Arc<dyn oi_types::EmbeddingPipeline>,
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
    // Embed query to f32 LE bytes for the P2P protocol
    let query_embedding = state
        .embedder
        .embed_query(&req.query)
        .map(|emb| emb.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>())
        .unwrap_or_default();

    let proto_request = proto::QueryRequest {
        request_id: uuid::Uuid::new_v4().to_string(),
        query_embedding,
        query_text: req.query,
        filters: Some(req.filters.to_proto()),
        limit: req.limit,
        signed_ticket: vec![],
        sender_pubkey: String::new(),
        nonce: 0,
    };

    match state.handle.query(proto_request).await {
        Ok(response) => {
            let items = proto_results_to_items(&response.results);
            Ok(Json(items))
        }
        Err(e) => Err((
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("Query failed: {e}"),
        )),
    }
}

/// Convert proto SearchResult items to SearchResultItem.
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
pub async fn run_node_api(
    port: u16,
    handle: NodeHandle,
) -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let embedder = oi_sdk::OpenSonarXClient::default_embedder();
    let state = Arc::new(ApiState { handle, embedder });

    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/search", post(search_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    println!("  HTTP API listening on http://0.0.0.0:{port}");
    axum::serve(listener, app).await?;
    Ok(())
}
