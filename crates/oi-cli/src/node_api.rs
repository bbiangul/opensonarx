use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use oi_network::NodeHandle;
use oi_network::operator::OperatorBonds;
use oi_types::proto::opensonarx as proto;
use oi_types::traits::StakeRegistry;
use oi_types::{QueryFilters, SearchResultItem};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// What the node command knows about this node and hands to its HTTP API.
pub struct NodeContext {
    /// Shards this node announces.
    pub shards: Vec<String>,
    /// Version of the learned routing file, when one is in use.
    pub routing_version: Option<String>,
    /// The router and the shards whose chunks this node keeps, when pages
    /// the node learns are classified by a learned routing.
    pub routing: Option<(Arc<oi_types::TopicRouter>, Vec<String>)>,
    pub routing_table: Arc<tokio::sync::RwLock<oi_network::routing::RoutingTable>>,
    /// The stake registry attached to the node's index.
    pub stake_registry: Arc<oi_staking::InMemoryStakeRegistry>,
    /// The mirror file the registry is read from.
    pub stakes_path: Option<std::path::PathBuf>,
    /// The mirror of operators' bonds, when this node requires one of the
    /// peers it copies from.
    pub operator_bonds: Option<Arc<crate::operator_bonds::BondMirror>>,
    /// Times a peer was passed over or turned away for lack of a bond.
    pub unbonded_skips: Arc<std::sync::atomic::AtomicU64>,
}

/// Shared state for the node HTTP API.
struct ApiState {
    handle: NodeHandle,
    /// Blended-search client over the node's SHARED index: external engines,
    /// crawl-on-query learning, and spell correction on the warm path.
    client: Arc<oi_sdk::OpenSonarXClient>,
    /// The shared index + its on-disk location, for persisting learned pages.
    index: Arc<oi_index::HybridIndex>,
    index_path: Option<std::path::PathBuf>,
    context: NodeContext,
    /// Whether this node reads the pages it learns for claims (it has a model).
    reads_pages: bool,
    /// Modification time of the stake mirror when it was last loaded.
    stakes_loaded: std::sync::Mutex<Option<std::time::SystemTime>>,
}

impl ApiState {
    /// Load the stake mirror again when the file changed, so a bond recorded
    /// while the node runs shows without a restart.
    fn refresh_stakes(&self) {
        let Some(path) = &self.context.stakes_path else {
            return;
        };
        let Ok(modified) = std::fs::metadata(path).and_then(|meta| meta.modified()) else {
            return;
        };
        let Ok(mut loaded) = self.stakes_loaded.lock() else {
            return;
        };
        if *loaded == Some(modified) {
            return;
        }
        let path = path.to_string_lossy();
        oi_sdk::OpenSonarXClient::load_stakes_from(&path, &self.context.stake_registry);
        oi_sdk::OpenSonarXClient::load_stakes_from(&path, self.client.stake_registry());
        *loaded = Some(modified);
    }
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
                // With a passage extractor attached, the new pages are read
                // for claims before the index is saved.
                let stored = learner.process_claim_queue();
                if stored > 0 {
                    tracing::info!("crawl-on-query: stored {stored} verified claims");
                }
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
#[derive(Deserialize)]
struct ClaimsRequest {
    query: String,
}

#[derive(Serialize)]
struct ClaimsResponse {
    /// Source families holding a claim close to the nearest stored claim.
    supporting_families: usize,
    /// Whether that count reaches the policy's threshold.
    corroborated: bool,
    /// The nearest stored claims from this node and its peers, closest first.
    matches: Vec<oi_sdk::claims::MatchedClaim>,
    /// Embedding the query plus asking the network, in milliseconds.
    lookup_ms: f64,
    /// Claims stored on this node alone.
    local_claims: usize,
    /// How many of the returned claims came from other nodes.
    from_peers: usize,
    /// How many other nodes contributed a returned claim.
    peers_answering: usize,
    /// One entry per match, in the same order: where it is held and what is
    /// bonded behind its source.
    details: Vec<ClaimDetail>,
}

#[derive(Serialize)]
struct ClaimDetail {
    /// Id of the node that holds the claim.
    held_by: String,
    held_here: bool,
    /// Whether the claim is close enough to the question to be about it.
    about_question: bool,
    /// Whether the claim is counted with the head claim under the policy.
    counts_with_head: bool,
    /// Lamports bonded on chain behind the claim's source family, if any.
    bond_lamports: u64,
}

/// What the claims stored at ingestion say about a query, from this node and
/// the peers that serve its other shards. One query embedding and one
/// network round; no model is called.
async fn claims_handler(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<ClaimsRequest>,
) -> Result<Json<ClaimsResponse>, (axum::http::StatusCode, String)> {
    if req.query.trim().is_empty() {
        return Err((
            axum::http::StatusCode::BAD_REQUEST,
            "query must not be empty".to_string(),
        ));
    }
    let started = std::time::Instant::now();
    let embedder = state.client.embedder_arc();
    let query = req.query.clone();
    let vector = tokio::task::spawn_blocking(move || embedder.embed_query(&query))
        .await
        .map_err(|e| {
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("Embedding task failed: {e}"),
            )
        })?
        .map_err(|e| {
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("Embedding failed: {e}"),
            )
        })?;
    let policy = state.client.claim_policy().clone();
    let response = state
        .handle
        .query(proto::QueryRequest {
            request_id: uuid::Uuid::new_v4().to_string(),
            query_embedding: vector.iter().flat_map(|f| f.to_le_bytes()).collect(),
            query_text: req.query.clone(),
            limit: policy.top_k as u32,
            claims: true,
            ..Default::default()
        })
        .await
        .map_err(|e| {
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("Claim lookup failed: {e}"),
            )
        })?;
    state.refresh_stakes();
    // A claim's result id starts with the id of the node that holds it.
    let holders: std::collections::HashMap<(String, String), String> = response
        .results
        .iter()
        .rev()
        .filter_map(|result| {
            let holder = result.result_id.split(':').next()?.to_string();
            let claim = oi_sdk::claims::claims_from_results(std::slice::from_ref(result))
                .into_iter()
                .next()?
                .0;
            Some(((claim.url, claim.claim), holder))
        })
        .collect();
    let own = format!("{}:", state.handle.peer_id);
    let remote_holders: std::collections::BTreeSet<&str> = response
        .results
        .iter()
        .filter(|result| !result.result_id.starts_with(&own))
        .filter_map(|result| result.result_id.split(':').next())
        .collect();
    let from_peers = response
        .results
        .iter()
        .filter(|result| !result.result_id.starts_with(&own))
        .count();
    let peers_answering = remote_holders.len();
    let signal = oi_sdk::claims::corroboration(
        oi_sdk::claims::claims_from_results(&response.results),
        &policy,
    );
    Ok(Json(ClaimsResponse {
        details: signal
            .matches
            .iter()
            .map(|claim| {
                let held_by = holders
                    .get(&(claim.url.clone(), claim.claim.clone()))
                    .cloned()
                    .unwrap_or_default();
                ClaimDetail {
                    held_here: held_by == state.handle.peer_id.to_string(),
                    held_by,
                    about_question: claim.query_similarity >= policy.min_query_similarity,
                    counts_with_head: oi_types::claims::counts_with_head(claim, &policy),
                    bond_lamports: state
                        .context
                        .stake_registry
                        .get_stake(&claim.source_family)
                        .ok()
                        .flatten()
                        .map_or(0, |stake| stake.amount_lamports),
                }
            })
            .collect(),
        supporting_families: signal.supporting_families,
        corroborated: signal.corroborated,
        matches: signal.matches,
        lookup_ms: started.elapsed().as_secs_f64() * 1_000.0,
        local_claims: state.client.claim_count(),
        from_peers,
        peers_answering,
    }))
}

/// What this node is and holds, for an operator or a status page.
async fn status_handler(State(state): State<Arc<ApiState>>) -> Json<serde_json::Value> {
    state.refresh_stakes();
    let peers: Vec<serde_json::Value> = state
        .context
        .routing_table
        .read()
        .await
        .peer_summaries()
        .into_iter()
        .map(|(node_id, shards, vectors)| {
            let wallet = oi_network::operator::wallet_of_node(&node_id);
            serde_json::json!({
                "node_id": node_id,
                "shards": shards,
                "vectors": vectors,
                "operator_wallet": wallet.as_ref().map(oi_network::operator::wallet_address),
                // Only meaningful on a node that requires bonds.
                "bonded": state.context.operator_bonds.as_ref().map(|bonds| {
                    wallet.is_some_and(|wallet| bonds.in_good_standing(&wallet))
                }),
            })
        })
        .collect();
    let refusals: Vec<serde_json::Value> = state
        .index
        .refused_claims()
        .into_iter()
        .map(|refused| {
            let wallet = oi_network::operator::wallet_of_node(&refused.peer_id);
            serde_json::json!({
                "peer_id": refused.peer_id,
                "operator_wallet": wallet.as_ref().map(oi_network::operator::wallet_address),
                "url": refused.url,
                "claim": refused.claim,
                "quote": refused.quote,
            })
        })
        .collect();
    let bonds: Vec<serde_json::Value> = state
        .context
        .stake_registry
        .list_active_stakes()
        .unwrap_or_default()
        .into_iter()
        .map(|stake| serde_json::json!({ "domain": stake.domain, "lamports": stake.amount_lamports }))
        .collect();
    Json(serde_json::json!({
        "peer_id": state.handle.peer_id.to_string(),
        "operator_wallet": oi_network::operator::operator_wallet(&state.handle.peer_id)
            .as_ref()
            .map(oi_network::operator::wallet_address),
        "requires_bond": state.context.operator_bonds.is_some(),
        "unbonded_peers_passed_over": state
            .context
            .unbonded_skips
            .load(std::sync::atomic::Ordering::Acquire),
        "refusals": refusals,
        "shards": state.context.shards,
        "routing_version": state.context.routing_version,
        "reads_pages_for_claims": state.reads_pages,
        "pages": state.index.doc_count(),
        "claims": state.client.claim_count(),
        "claims_rejected_from_peers": state.index.rejected_claim_count(),
        "peers": peers,
        "bonds": bonds,
    }))
}

#[derive(Deserialize)]
struct LearnRequest {
    urls: Vec<String>,
}

/// Index the given pages now and read them for claims. An operator's
/// instruction to the node, so it answers only when the node was started
/// with `OI_NODE_LEARN=1`.
async fn learn_handler(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<LearnRequest>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    let allowed = std::env::var("OI_NODE_LEARN")
        .is_ok_and(|value| matches!(value.trim(), "1" | "true" | "yes"));
    if !allowed {
        return Err((
            axum::http::StatusCode::FORBIDDEN,
            "start the node with OI_NODE_LEARN=1 to accept pages to index".to_string(),
        ));
    }
    let client = state.client.clone();
    let index = state.index.clone();
    let index_path = state.index_path.clone();
    let started = std::time::Instant::now();
    let (pages, claims) = tokio::task::spawn_blocking(move || {
        let pages = client.learn_urls(req.urls);
        let claims = client.process_claim_queue();
        if let Some(path) = index_path
            && let Ok(storage) = oi_index::Storage::new(&path)
            && let Err(error) = index.save(&storage)
        {
            tracing::warn!("learn: failed to persist the index: {error}");
        }
        (pages, claims)
    })
    .await
    .map_err(|error| {
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("Learn task failed: {error}"),
        )
    })?;
    Ok(Json(serde_json::json!({
        "pages_indexed": pages,
        "claims_stored": claims,
        "claims": state.client.claim_count(),
        "seconds": started.elapsed().as_secs_f64(),
    })))
}

/// A page that shows this node at work: what it holds, who its peers are,
/// and what the stored claims say about a question.
async fn demo_handler() -> axum::response::Html<&'static str> {
    axum::response::Html(DEMO_HTML)
}

const DEMO_HTML: &str = include_str!("node_demo.html");

pub async fn run_node_api(
    port: u16,
    handle: NodeHandle,
    index: Arc<oi_index::HybridIndex>,
    index_path: Option<std::path::PathBuf>,
    sdk_config: oi_sdk::SdkConfig,
    context: NodeContext,
) -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // With OI_EXTRACT_CLAIMS=1 and an answer model configured, the node
    // reads each page it learns for claims.
    let extract_claims = std::env::var("OI_EXTRACT_CLAIMS")
        .is_ok_and(|value| matches!(value.trim(), "1" | "true" | "yes"));
    let passage_model = sdk_config.answer_model.clone().filter(|_| extract_claims);
    let mut client = oi_sdk::OpenSonarXClient::with_shared_index(sdk_config, index.clone());
    if let Some((router, shards)) = context.routing.clone() {
        client = client.with_topic_router(router).with_shard_filter(shards);
    }
    let reads_pages = passage_model.is_some();
    if let Some(model) = passage_model {
        client = client.with_passage_claim_extractor(Arc::new(oi_sdk::ChatModel::new(model)));
        println!("  Claims: pages are read for claims after they are indexed");
    }
    let client = Arc::new(client);
    let state = Arc::new(ApiState {
        handle,
        client,
        index,
        index_path,
        context,
        reads_pages,
        stakes_loaded: std::sync::Mutex::new(None),
    });

    let app = Router::new()
        .route("/", get(ui_handler))
        .route("/health", get(health_handler))
        .route("/search", post(search_handler))
        .route("/answer", post(answer_handler))
        .route("/claims", post(claims_handler))
        .route("/status", get(status_handler))
        .route("/learn", post(learn_handler))
        .route("/demo", get(demo_handler))
        .route("/v1/search", post(gateway_search_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    println!("  HTTP API listening on http://0.0.0.0:{port}");
    axum::serve(listener, app).await?;
    Ok(())
}
