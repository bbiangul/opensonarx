use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
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

/// Gateway-style flat request (`POST /v1/search`), matching what the hosted
/// gateway and the shipped Python client speak — so the same client code
/// works against a local node unchanged.
#[derive(Deserialize)]
struct GatewayStyleRequest {
    query: String,
    #[serde(default = "default_limit")]
    top_k: u32,
    #[serde(default)]
    min_trust: Option<String>,
    #[serde(default)]
    freshness: Option<String>,
    #[serde(default)]
    site: Option<String>,
    #[serde(default)]
    types: Vec<String>,
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
    if req.query.trim().is_empty() {
        return Err((
            axum::http::StatusCode::BAD_REQUEST,
            "query must not be empty".to_string(),
        ));
    }
    let limit = req.limit as usize;

    // Blended search (local + external engines) — blocking work (ONNX embed,
    // engine HTTP via ureq) moved off the async runtime.
    let client = state.client.clone();
    let query = req.query.clone();
    let filters = req.filters.clone();
    let blended =
        tokio::task::spawn_blocking(move || client.search_blended(&query, limit, &filters))
            .await
            .map_err(|e| {
                (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Search task failed: {e}"),
                )
            })?
            .map_err(|e| {
                (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Search failed: {e}"),
                )
            })?;
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
            claims: false,
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
                    r.decision = None;
                }
                oi_sdk::evidence::apply_decision_policy(&req.query, &mut items);
            }
        }
    }

    Ok(Json(items))
}

async fn answer_handler(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<SearchRequest>,
) -> Result<Json<oi_sdk::AnsweredSearch>, (axum::http::StatusCode, String)> {
    if req.query.trim().is_empty() {
        return Err((
            axum::http::StatusCode::BAD_REQUEST,
            "query must not be empty".to_string(),
        ));
    }
    let client = state.client.clone();
    let query = req.query;
    let filters = req.filters;
    let limit = req.limit as usize;
    let answer =
        tokio::task::spawn_blocking(move || client.search_answered(&query, limit, &filters))
            .await
            .map_err(|error| {
                (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Answer task failed: {error}"),
                )
            })?
            .map_err(|error| {
                (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Answer failed: {error}"),
                )
            })?;
    Ok(Json(answer))
}

/// Minimal human search UI served at `/` — the same trust-priced index,
/// usable by people without any client code. Inline HTML/JS, no assets.
async fn ui_handler() -> axum::response::Html<&'static str> {
    axum::response::Html(UI_HTML)
}

const UI_HTML: &str = r##"<!doctype html>
<html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>OpenSonarX</title>
<style>
 body{font-family:-apple-system,system-ui,sans-serif;max-width:760px;margin:2rem auto;padding:0 1rem;color:#1a1a1a}
 h1{font-size:1.4rem} h1 span{color:#888;font-weight:400;font-size:.9rem}
 form{display:flex;gap:.5rem;margin:1rem 0}
 input[type=text]{flex:1;padding:.6rem .8rem;font-size:1rem;border:1px solid #ccc;border-radius:8px}
 button{padding:.6rem 1.1rem;font-size:1rem;border:0;border-radius:8px;background:#1a1a1a;color:#fff;cursor:pointer}
 label{font-size:.85rem;color:#555}
 .r{padding:.8rem 0;border-bottom:1px solid #eee}
 .r a{font-size:1.05rem;text-decoration:none;color:#0b57d0} .r a:hover{text-decoration:underline}
 .meta{font-size:.78rem;color:#777;margin:.15rem 0}
 .badge{display:inline-block;padding:.05rem .45rem;border-radius:99px;font-size:.72rem;margin-left:.4rem;vertical-align:1px}
 .neutral{background:#fff3cd;color:#7a5d00}.trusted,.highly_trusted{background:#d1e7dd;color:#0f5132}
 .unverified,.external{background:#f1f3f4;color:#666}
 .snippet{font-size:.9rem;color:#333;margin-top:.2rem}
 .stake{color:#0f5132;font-weight:600}
 #status{color:#888;font-size:.85rem}
</style></head><body>
<h1>OpenSonarX <span>&mdash; relevance plus explicit evidential uncertainty</span></h1>
<form id="f"><input type="text" id="q" placeholder="Search&hellip;" autofocus>
<button>Search</button></form>
<p id="status"></p><div id="out"></div>
<script>
const f=document.getElementById('f'),q=document.getElementById('q'),out=document.getElementById('out'),st=document.getElementById('status');
f.addEventListener('submit',async e=>{
 e.preventDefault(); if(!q.value.trim())return;
 st.textContent='searching…'; out.innerHTML='';
 const body={query:q.value,top_k:8};
 const t0=performance.now();
 try{
  const res=await fetch('/v1/search',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify(body)});
  const data=await res.json();
  const ms=(performance.now()-t0).toFixed(0);
  const rs=data.results||[];
  st.textContent=rs.length+' results in '+ms+'ms';
  const esc=s=>s.replace(/[&<>"]/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;'}[c]));
  for(const r of rs){
   const d=document.createElement('div'); d.className='r';
   const stake=r.stake_usd>0?' <span class="stake">$'+r.stake_usd+' disclosed stake (not relevance)</span>':'';
   const decision=r.decision||{};
   const uncertainty=decision.uncertainty_mode||'not_run';
   d.innerHTML='<a href="'+esc(r.url)+'" rel="noopener">'+esc(r.title||r.url)+'</a>'+
    '<div class="meta">'+esc(r.url)+' <span class="badge neutral">'+esc(uncertainty)+'</span>'+stake+
    ' · relevance '+(r.final_score||0).toFixed(3)+' · '+esc(r.source)+'</div>'+
    '<div class="snippet">'+esc((r.content_relevant||r.snippet||'').slice(0,300))+'</div>';
   out.appendChild(d);
  }
 }catch(err){ st.textContent='error: '+err; }
});
</script></body></html>"##;

/// Gateway-compatible endpoint: flat request shape, `{"results": [...]}`
/// response — the contract of the hosted gateway's /v1/search, served by the
/// local node so SDK/tool examples work without a hosted deployment.
async fn gateway_search_handler(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<GatewayStyleRequest>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    let inner = SearchRequest {
        query: req.query,
        limit: req.top_k,
        filters: QueryFilters {
            min_trust: req.min_trust,
            freshness: req.freshness,
            site: req.site,
            types: req.types,
            ..Default::default()
        },
    };
    let Json(items) = search_handler(State(state), Json(inner)).await?;
    Ok(Json(serde_json::json!({ "results": items })))
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
                    if f.crawled_at > 0 {
                        Some(f.crawled_at)
                    } else {
                        None
                    }
                }),
                published_at: freshness.and_then(|f| {
                    if f.published_at > 0 {
                        Some(f.published_at as i64)
                    } else {
                        None
                    }
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
                decision: None,
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
    let state = Arc::new(ApiState {
        handle,
        client,
        index,
        index_path,
    });

    let app = Router::new()
        .route("/", get(ui_handler))
        .route("/health", get(health_handler))
        .route("/search", post(search_handler))
        .route("/answer", post(answer_handler))
        .route("/v1/search", post(gateway_search_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    println!("  HTTP API listening on http://0.0.0.0:{port}");
    axum::serve(listener, app).await?;
    Ok(())
}
