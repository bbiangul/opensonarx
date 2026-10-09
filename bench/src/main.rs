#![allow(
    clippy::useless_conversion,
    clippy::type_complexity,
    clippy::manual_div_ceil,
    clippy::clone_on_copy,
    clippy::needless_range_loop
)]

mod agent_cost;
mod answer_eval;
mod answer_eval_squad;
mod beir;
mod bench_quality;
mod dataset;
mod dist_sim;
mod election_polls_bench;
mod embed;
mod engine_probe;
mod facts_bench;
mod facts_dataset;
mod facts_metrics;
mod generalization_blind;
mod generalization_capture;
mod generalization_compare;
mod generalization_market_capture;
mod generator;
mod image_gen;
mod market_compare_bench;
mod market_latency_bench;
mod metrics;
mod msmarco;
mod production_readiness_bench;
mod scale_ingest;
mod scaling;
mod sealqa;
mod seed;
mod tuning;
mod uncertainty_bench;

use axum::{
    extract::{Json, State},
    routing::{get, post},
    Router,
};
use clap::{Parser, Subcommand};
use oi_embed::MockEmbedder;
use oi_index::HybridIndex;
use oi_types::traits::SearchIndex;
use oi_types::{ChunkMeta, ContentChunk, EmbeddingPipeline, QueryFilters};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;

#[derive(Parser)]
#[command(name = "oi-bench", about = "OpenSonarX benchmark tools")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the benchmark HTTP server
    Server {
        /// Number of documents to seed
        #[arg(short = 'n', long, default_value_t = 10_000)]
        docs: usize,

        /// Port to listen on
        #[arg(short, long, default_value_t = 3333)]
        port: u16,
    },
    /// Run search quality benchmarks
    Benchmark {
        /// Number of test cases to generate
        #[arg(short = 'n', long, default_value_t = 1000)]
        cases: usize,

        /// Embedding pipeline to use (mock or minilm)
        #[arg(short, long, default_value = "mock")]
        pipeline: String,

        /// Output format (table or json)
        #[arg(short, long, default_value = "table")]
        output: String,

        /// Modality: text, image, or all
        #[arg(short, long, default_value = "text")]
        modality: String,
    },
    /// Run a parameter sweep over hybrid search configs
    Sweep {
        /// Number of test cases to generate
        #[arg(short = 'n', long, default_value_t = 200)]
        cases: usize,

        /// Embedding pipeline to use
        #[arg(short, long, default_value = "mock")]
        pipeline: String,
    },
    /// Evaluate against a BEIR benchmark dataset (scifact, nfcorpus)
    Beir {
        /// Dataset name
        #[arg(short, long, default_value = "scifact")]
        dataset: String,

        /// Embedding pipeline to use (mock or minilm)
        #[arg(short, long, default_value = "minilm")]
        pipeline: String,

        /// Output format (table or json)
        #[arg(short, long, default_value = "table")]
        output: String,

        /// Semantic weight (0.0-1.0). BM25 weight = 1 - semantic_weight.
        #[arg(short = 'w', long)]
        semantic_weight: Option<f64>,

        /// Enable cross-encoder reranking (requires --features reranker)
        #[arg(long)]
        rerank: bool,
    },
    /// Sweep semantic/BM25 weights on a BEIR dataset (embed once, evaluate many)
    BeirSweep {
        /// Dataset name
        #[arg(short, long, default_value = "scifact")]
        dataset: String,

        /// Embedding pipeline to use
        #[arg(short, long, default_value = "minilm")]
        pipeline: String,
    },
    /// Evaluate on MS MARCO passage ranking (8.8M passages, MRR@10)
    Marco {
        /// Embedding pipeline to use (mock or minilm)
        #[arg(short, long, default_value = "minilm")]
        pipeline: String,

        /// Output format (table or json)
        #[arg(short, long, default_value = "table")]
        output: String,

        /// Semantic weight (0.0-1.0). BM25 weight = 1 - semantic_weight.
        #[arg(short = 'w', long)]
        semantic_weight: Option<f64>,

        /// Limit corpus size (e.g. 100000 for quick test)
        #[arg(short, long)]
        limit: Option<usize>,

        /// Enable cross-encoder reranking (requires --features reranker)
        #[arg(long)]
        rerank: bool,
    },
    /// Simulate distributed search across virtual nodes
    DistSim {
        /// Number of virtual nodes
        #[arg(short = 'n', long, default_value_t = 4)]
        nodes: usize,

        /// Query fanout (number of nodes queried per search)
        #[arg(short, long, default_value_t = 3)]
        fanout: usize,

        /// Corpus distribution mode: "uniform" or "zipf"
        #[arg(short, long, default_value = "uniform")]
        distribution: String,

        /// Embedding pipeline
        #[arg(short, long, default_value = "minilm")]
        pipeline: String,

        /// Limit corpus size
        #[arg(short, long)]
        limit: Option<usize>,

        /// Semantic weight
        #[arg(short = 'w', long)]
        semantic_weight: Option<f64>,
    },
    /// Analyze query latency scaling per component across corpus sizes
    Scaling {
        /// Comma-separated corpus sizes (supports K/M suffixes, e.g. "1k,5k,10k,50k")
        #[arg(short = 's', long)]
        sizes: Option<String>,

        /// Number of queries per corpus size
        #[arg(short = 'q', long, default_value_t = scaling::default_num_queries())]
        queries: usize,

        /// Output format (table or json)
        #[arg(short, long, default_value = "table")]
        output: String,

        /// Vector-only mode (skip BM25, set bm25_weight=0.0)
        #[arg(long)]
        vector_only: bool,
    },
    /// Ingest N cached MS MARCO passages into a target index (scale testing)
    ScaleIngest {
        /// Number of passages to ingest
        #[arg(short, long, default_value_t = 100_000)]
        limit: usize,
        /// Target index directory
        #[arg(short, long)]
        index_path: String,
    },
    /// Evaluate against LongSeal benchmark (254 questions, per-question indexing)
    SealQa {
        /// Embedding pipeline to use (mock or minilm)
        #[arg(short, long, default_value = "minilm")]
        pipeline: String,

        /// Output format (table or json)
        #[arg(short, long, default_value = "table")]
        output: String,

        /// Semantic weight (0.0-1.0). BM25 weight = 1 - semantic_weight.
        #[arg(short = 'w', long)]
        semantic_weight: Option<f64>,

        /// Document set size: 12, 20, or 30
        #[arg(short = 'd', long, default_value_t = 30)]
        docs: usize,

        /// Enable cross-encoder reranking (requires --features reranker)
        #[arg(long)]
        rerank: bool,
    },
    /// Evaluate fact extraction pipelines on multilingual benchmark data
    FactsBench {
        /// Fact extraction pipeline (mock or gliner)
        #[arg(short, long, default_value = "mock")]
        pipeline: String,

        /// Output format (table, json, or verbose)
        #[arg(short, long, default_value = "table")]
        output: String,

        /// Comma-separated language codes to skip (e.g. "zh,ja")
        #[arg(long)]
        skip_langs: Option<String>,
    },
    /// Compare agent total cost-to-answer across search backends (Serper, Exa, Tavily, OpenIndex)
    AgentCost {
        /// Embedding pipeline to use
        #[arg(short, long, default_value = "minilm")]
        pipeline: String,

        /// Output format (table or json)
        #[arg(short, long, default_value = "table")]
        output: String,

        /// Comma-separated backends to test (serper,exa,tavily,openindex)
        #[arg(short, long, default_value = "serper,exa,tavily,openindex")]
        backends: String,

        /// Number of search results per query
        #[arg(short = 'k', long, default_value_t = 5)]
        top_k: usize,

        /// Gemini model to use
        #[arg(short, long, default_value = "gemini-flash-latest")]
        model: String,
    },
    /// Evaluate the answer path (model extraction + stance + deterministic
    /// assembly) on fixed cases; needs a chat model served by Ollama
    AnswerEval {
        /// JSONL case file
        #[arg(long, default_value = "bench/fixtures/answer-eval-cases-v1.jsonl")]
        cases: String,

        /// Where the model is served: ollama (local) or openrouter (reads
        /// OPENROUTER_API_KEY from the environment)
        #[arg(long, default_value = "ollama")]
        provider: String,

        /// Model name on that provider
        #[arg(long)]
        model: String,

        /// Model for the stance and responsiveness judgments (default: the
        /// same as --model, which then only names the extractor)
        #[arg(long)]
        judge_model: Option<String>,

        /// Server base URL (default: the provider's)
        #[arg(long)]
        base_url: Option<String>,

        /// Cases evaluated at the same time
        #[arg(long, default_value_t = 1)]
        concurrency: usize,

        /// Skip the responsiveness judge (reproduces the pipeline before it)
        #[arg(long)]
        no_responsiveness: bool,

        /// Reasoning effort for models that expose one (low, medium, high)
        #[arg(long)]
        think: Option<String>,

        /// Override the answer policy's distinct-source requirement (use 1
        /// for single-document datasets such as SQuAD)
        #[arg(long)]
        min_source_families: Option<usize>,

        /// Output JSON report
        #[arg(long, default_value = "answer_eval_results.json")]
        output: String,
    },
    /// Convert a SQuAD 2.0 JSON file into answer-eval cases (no model needed)
    AnswerEvalImportSquad {
        /// Path to a SQuAD 2.0 file, e.g. dev-v2.0.json
        #[arg(long)]
        input: String,

        /// Number of cases, half answerable and half unanswerable
        #[arg(long, default_value_t = 200)]
        sample: usize,

        /// Label hashed with each question id to choose the sample; a new
        /// label yields a new, disjoint-by-chance sample
        #[arg(long)]
        label: String,

        /// Earlier sample files whose case ids must not be drawn again
        #[arg(long)]
        exclude: Vec<String>,

        /// Output JSONL case file
        #[arg(long)]
        output: String,
    },
    /// Validate OpenUncertainty cash ablation, guards, calibration, and throughput
    Uncertainty {
        /// Number of resolved synthetic claims used to learn the cash likelihood
        #[arg(long, default_value_t = 20_000)]
        train_cases: usize,

        /// Number of held-out forward synthetic claims
        #[arg(long, default_value_t = 20_000)]
        eval_cases: usize,

        /// Number of hot-path estimates used for throughput measurement
        #[arg(long, default_value_t = 1_000_000)]
        throughput_claims: usize,

        /// Output format (table or json)
        #[arg(short, long, default_value = "table")]
        output: String,
    },
    /// Evaluate OpenUncertainty on a leakage-checked forward JSONL dataset
    UncertaintyDataset {
        /// Path to the versioned JSONL cohort
        #[arg(short, long)]
        dataset: String,

        /// Output format (table or json)
        #[arg(short, long, default_value = "table")]
        output: String,

        /// Deterministic cluster-bootstrap iterations for the Brier improvement interval
        #[arg(long, default_value_t = 2_000)]
        bootstrap_iterations: usize,
    },
    /// Replay a captured Google SERP and apply the election-poll trust policy
    ElectionPolls {
        /// Versioned Google SERP snapshot with auditable result judgments
        #[arg(
            short,
            long,
            default_value = "bench/fixtures/brazil-election-polls-google-2026-08-21.json"
        )]
        dataset: String,

        /// Output format (table or json)
        #[arg(short, long, default_value = "table")]
        output: String,
    },
    /// Probe keyless search adapters over multiple live queries without mocks
    EngineProbe {
        /// Comma-separated adapters: bing,brave-html,duckduckgo,google,qwant,stract,wikipedia,yep
        #[arg(
            long,
            default_value = "bing,brave-html,duckduckgo,google,qwant,stract,yep"
        )]
        engines: String,

        /// Queries to probe; repeat this flag to build a representative set
        #[arg(
            long = "query",
            default_values_t = vec![
                "claude".to_string(),
                "python online compiler".to_string(),
                "pesquisa eleitoral presidente brasil hoje".to_string(),
            ]
        )]
        queries: Vec<String>,

        /// Maximum results requested from each adapter
        #[arg(short = 'k', long, default_value_t = 5)]
        limit: usize,

        /// Serial rounds per provider; use multiple rounds to expose volatility
        #[arg(long, default_value_t = 1)]
        rounds: usize,

        /// Simultaneous raw requests per provider (use production concurrency for capacity gates)
        #[arg(long, default_value_t = 1)]
        concurrency: usize,

        /// Fail the probe unless at least this many adapters pass every query
        #[arg(long, default_value_t = 2)]
        min_healthy: usize,

        /// Output format (table or json)
        #[arg(short, long, default_value = "table")]
        output: String,

        /// Also persist the complete JSON report, including on a failed gate
        #[arg(long)]
        report_path: Option<String>,
    },
    /// Compare a dated market SERP snapshot with a real OpenSonarX live report
    MarketCompare {
        /// Versioned market candidate-set fixture
        #[arg(
            long,
            default_value = "bench/fixtures/google-multi-intent-br-2026-08-22.json"
        )]
        fixture: String,

        /// Complete JSON emitted by live-search --report-path
        #[arg(long)]
        live_report: String,

        /// Output format (table or json)
        #[arg(short, long, default_value = "table")]
        output: String,

        /// Also persist the complete JSON comparison report
        #[arg(long)]
        report_path: Option<String>,
    },
    /// Audit exploratory Google-browser and OpenSonarX-backend latency samples
    MarketLatency {
        /// Versioned raw latency fixture; the evaluator refuses a comparability claim
        #[arg(
            long,
            default_value = "bench/fixtures/google-vs-opensonarx-latency-exploratory-2026-08-22.json"
        )]
        fixture: String,

        /// Output format (table or json)
        #[arg(short, long, default_value = "table")]
        output: String,

        /// Also persist the recalculated JSON report
        #[arg(long)]
        report_path: Option<String>,
    },
    /// Capture a sealed, unjudged, non-overlapping live-search holdout
    GeneralizationCapture {
        /// Query-only manifest with an auditable independent selection recipe
        #[arg(
            long,
            default_value = "bench/fixtures/generalization-msmarco-holdout-v1.json"
        )]
        manifest: String,

        /// Isolated persistent state directory for the real production path
        #[arg(long)]
        data_dir: String,

        /// Override production discovery adapters
        #[arg(long)]
        engines: Option<String>,

        /// Repetitions over the sealed manifest
        #[arg(long, default_value_t = 1)]
        passes: usize,

        /// Maximum simultaneous searches sharing the production client
        #[arg(long, default_value_t = 4)]
        concurrency: usize,

        /// Maximum results captured per query
        #[arg(short = 'k', long, default_value_t = 5)]
        top_k: usize,

        /// Process the real crawl-on-query queue after every batch
        #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
        crawl: bool,

        /// Output format (table or json)
        #[arg(short, long, default_value = "table")]
        output: String,

        /// Persist the complete raw capture for later blind judging
        #[arg(long)]
        report_path: Option<String>,
    },
    /// Capture one raw live market engine against the identical sealed holdout
    GeneralizationMarketCapture {
        /// Frozen query-only manifest
        #[arg(
            long,
            default_value = "bench/fixtures/generalization-msmarco-holdout-v1.json"
        )]
        manifest: String,

        /// Credentialed or keyless market adapter to capture
        #[arg(long, default_value = "serper")]
        engine: String,

        /// Maximum results captured per query
        #[arg(short = 'k', long, default_value_t = 5)]
        top_k: usize,

        /// Maximum simultaneous requests to the market provider
        #[arg(long, default_value_t = 4)]
        concurrency: usize,

        /// Output format (table or json)
        #[arg(short, long, default_value = "table")]
        output: String,

        /// Persist the complete raw capture for later blind pooling
        #[arg(long)]
        report_path: Option<String>,
    },
    /// Pool frozen OpenSonarX and market slates without exposing system identity
    GeneralizationBlindPool {
        /// Frozen query-only manifest
        #[arg(long)]
        manifest: String,

        /// Raw JSON emitted by generalization-capture
        #[arg(long)]
        opensonarx_report: String,

        /// Raw JSON emitted by generalization-market-capture
        #[arg(long)]
        market_report: String,

        /// Maximum results pooled from each system
        #[arg(short = 'k', long, default_value_t = 5)]
        top_k: usize,

        /// Precommitted randomization nonce (at least 16 characters)
        #[arg(long)]
        blinding_nonce: String,

        /// Public system-blind packet for the judge
        #[arg(long)]
        packet_path: String,

        /// Private system/rank mapping; keep this away from the judge
        #[arg(long)]
        key_path: String,

        /// Output format (table or json)
        #[arg(short, long, default_value = "table")]
        output: String,
    },
    /// Create a complete null-valued judgment form bound to a blind packet
    GeneralizationBlindTemplate {
        /// Immutable public blind packet
        #[arg(long)]
        packet: String,

        /// Stable anonymous identifier for this judge
        #[arg(long)]
        judge_id: String,

        /// Whether the judge is organizationally external to implementation
        #[arg(long, default_value_t = false, action = clap::ArgAction::Set)]
        external_to_implementation: bool,

        /// Destination for the judgment form
        #[arg(long)]
        template_path: String,
    },
    /// Produce a system-blind diagnostic judgment with a real local Ollama model
    GeneralizationBlindOllamaJudge {
        /// Immutable public blind packet
        #[arg(long)]
        packet: String,

        /// Installed Ollama generation model
        #[arg(long, default_value = "gpt-oss:20b")]
        model: String,

        /// Ollama API origin
        #[arg(long, default_value = "http://127.0.0.1:11434")]
        base_url: String,

        /// Simultaneous model requests
        #[arg(long, default_value_t = 1)]
        concurrency: usize,

        /// Blind queries grouped into one model request
        #[arg(long, default_value_t = 4)]
        queries_per_request: usize,

        /// Supported gpt-oss reasoning level: low, medium, or high
        #[arg(long, default_value = "low")]
        thinking: String,

        /// Destination for complete diagnostic judgments
        #[arg(long)]
        judgments_path: String,
    },
    /// Score complete blind judgments and calculate paired confidence intervals
    GeneralizationBlindScore {
        /// Immutable public blind packet
        #[arg(long)]
        packet: String,

        /// Private system/rank mapping produced with the packet
        #[arg(long)]
        key: String,

        /// Complete judgments bound to the packet SHA-256; repeat for independent judges
        #[arg(long, required = true)]
        judgments: Vec<String>,

        /// Deterministic query-cluster bootstrap iterations
        #[arg(long, default_value_t = 10_000)]
        bootstrap_iterations: usize,

        /// Output format (table or json)
        #[arg(short, long, default_value = "table")]
        output: String,

        /// Persist the complete scored report
        #[arg(long)]
        report_path: Option<String>,
    },
    /// Compare objective coverage of two sealed, still-unjudged holdout captures
    GeneralizationCompare {
        /// Frozen query-only manifest
        #[arg(
            long,
            default_value = "bench/fixtures/generalization-msmarco-holdout-v1.json"
        )]
        manifest: String,

        /// Raw JSON emitted by generalization-capture
        #[arg(long)]
        opensonarx_report: String,

        /// Raw visible Google SERP capture for the identical manifest
        #[arg(
            long,
            default_value = "bench/fixtures/google-generalization-msmarco-capture-v1.json"
        )]
        google_report: String,

        /// Output format (table or json)
        #[arg(short, long, default_value = "table")]
        output: String,

        /// Persist the objective comparison report
        #[arg(long)]
        report_path: Option<String>,
    },
    /// Aggregate live artifacts into an explicit production-readiness verdict
    ProductionReadiness {
        /// JSON emitted by engine-probe --report-path
        #[arg(long)]
        provider_report: String,

        /// Fresh-index answered JSON emitted by live-search --report-path
        #[arg(long)]
        cold_report: String,

        /// Reused-index multi-pass JSON emitted by live-search --report-path
        #[arg(long)]
        resident_report: String,

        /// JSON emitted by market-compare --report-path
        #[arg(long)]
        market_report: String,

        /// Minimum independently healthy discovery providers
        #[arg(long, default_value_t = 2)]
        min_healthy_providers: usize,

        /// Maximum cold search p99 latency in milliseconds
        #[arg(long, default_value_t = 10_000.0)]
        max_cold_search_p99_ms: f64,

        /// Maximum resident search p95 latency in milliseconds
        #[arg(long, default_value_t = 2_000.0)]
        max_resident_search_p95_ms: f64,

        /// Maximum resident search p99 latency in milliseconds
        #[arg(long, default_value_t = 4_000.0)]
        max_resident_search_p99_ms: f64,

        /// Minimum resident throughput at the benchmark concurrency
        #[arg(long, default_value_t = 10.0)]
        min_resident_qps: f64,

        /// Output format (table or json)
        #[arg(short, long, default_value = "table")]
        output: String,

        /// Also persist the aggregate JSON verdict
        #[arg(long)]
        report_path: Option<String>,
    },
    /// Generate a benchmark dataset file
    Generate {
        /// Number of test cases to generate
        #[arg(short = 'n', long, default_value_t = 5000)]
        cases: usize,

        /// Output file path
        #[arg(short, long, default_value = "bench/datasets/text_5k.json")]
        output: String,
    },
}

// --- Server types ---

struct AppState {
    index: HybridIndex,
    doc_count: usize,
}

#[derive(Deserialize)]
struct SearchRequest {
    query: String,
    #[serde(default = "default_top_k")]
    top_k: usize,
    #[serde(default)]
    freshness: Option<String>,
    #[serde(default)]
    types: Vec<String>,
}

fn default_top_k() -> usize {
    10
}

#[derive(Serialize)]
struct SearchResponse {
    results: Vec<SearchResultOut>,
    total: usize,
    query_ms: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    timings: Option<oi_index::QueryTimings>,
}

#[derive(Serialize)]
struct SearchResultOut {
    rank: u32,
    url: String,
    title: String,
    score: f64,
    domain: String,
}

#[derive(Serialize)]
struct StatsResponse {
    doc_count: usize,
    status: &'static str,
}

async fn health() -> &'static str {
    "ok"
}

async fn stats(State(state): State<Arc<AppState>>) -> Json<StatsResponse> {
    Json(StatsResponse {
        doc_count: state.doc_count,
        status: "ready",
    })
}

async fn search_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SearchRequest>,
) -> Json<SearchResponse> {
    let embedding = embed::text_to_embedding(&req.query, 384);
    let filters = QueryFilters {
        freshness: req.freshness,
        types: req.types,
        ..Default::default()
    };

    let (results, timings) = state
        .index
        .query_timed(&embedding, &req.query, &filters, req.top_k)
        .unwrap();
    let query_ms = timings.total_ms;

    let total = results.len();
    let out: Vec<SearchResultOut> = results
        .into_iter()
        .map(|r| SearchResultOut {
            rank: r.rank,
            url: r.url,
            title: r.title,
            score: r.final_score,
            domain: r.snippet.clone(),
        })
        .collect();

    Json(SearchResponse {
        results: out,
        total,
        query_ms,
        timings: Some(timings),
    })
}

// --- Benchmark logic ---

fn get_embedder(pipeline_name: &str) -> Arc<dyn EmbeddingPipeline> {
    match pipeline_name {
        "mock" => Arc::new(MockEmbedder::default()),
        #[cfg(feature = "onnx")]
        "minilm" => {
            tracing::info!("Loading Arctic Embed S model...");
            Arc::new(oi_embed::MiniLmEmbedder::new().expect("Failed to load MiniLM"))
        }
        #[cfg(feature = "arctic-m")]
        "arctic-m" => {
            tracing::info!("Loading Arctic Embed M v2.0 model...");
            Arc::new(oi_embed::ArcticMEmbedder::new().expect("Failed to load Arctic M"))
        }
        #[cfg(feature = "arctic-l")]
        "arctic-l" => {
            tracing::info!("Loading Arctic Embed L v2.0 model...");
            Arc::new(oi_embed::ArcticLEmbedder::new().expect("Failed to load Arctic L"))
        }
        #[cfg(feature = "clip")]
        "clip" => {
            tracing::info!("Loading Jina CLIP v1 model...");
            Arc::new(oi_embed::ClipEmbedder::new().expect("Failed to load CLIP"))
        }
        #[cfg(feature = "ollama")]
        other if other.starts_with("ollama:") => {
            let model = other.strip_prefix("ollama:").unwrap();
            tracing::info!("Connecting to Ollama model: {model}...");
            Arc::new(oi_embed::OllamaEmbedder::new(model).expect("Failed to connect to Ollama"))
        }
        other => {
            eprintln!(
                "Unknown pipeline: {other}. Available: mock, minilm, arctic-m, arctic-l, clip, ollama:<model>"
            );
            std::process::exit(1);
        }
    }
}

fn run_benchmark(cases: usize, pipeline_name: &str, output_format: &str, modality: &str) {
    let embedder = get_embedder(pipeline_name);
    let dim = embedder.dimension();

    tracing::info!(
        "Generating {cases} test cases with pipeline '{}' (dim={dim}) modality={modality}",
        embedder.name()
    );

    let (text_dataset, image_dataset) = match modality {
        "text" => (Some(generator::generate_text_dataset(cases)), None),
        "image" => (None, Some(generator::generate_image_dataset(cases))),
        "all" => (
            Some(generator::generate_text_dataset(cases)),
            Some(generator::generate_image_dataset(cases / 5)),
        ),
        other => {
            eprintln!("Unknown modality: {other}. Available: text, image, all");
            std::process::exit(1);
        }
    };

    // Combine cases from text and image datasets
    let mut all_cases = Vec::new();
    if let Some(ref ds) = text_dataset {
        all_cases.extend(ds.cases.iter().cloned());
    }
    if let Some(ref ds) = image_dataset {
        all_cases.extend(ds.cases.iter().cloned());
    }
    let dataset = dataset::BenchmarkDataset {
        name: format!("opensonarx-{modality}"),
        description: format!("{} cases, modality={modality}", all_cases.len()),
        cases: all_cases,
    };

    // Build an index from deduplicated documents in the dataset.
    // Many cases share the same underlying content (same topic templates),
    // so we deduplicate by content to avoid inflating the index with duplicates.
    // We track content->indexed_id so recall checks can match correctly.
    let index = HybridIndex::new(dim);
    let mut content_to_id: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();

    // Collect unique docs, separating text and image for batched embedding
    struct DocInfo {
        doc_id: String,
        content: String,
        title: String,
        embed_text: Option<String>, // text to embed (None = image doc)
        image_desc: Option<String>, // image description (for image docs)
    }

    let mut docs = Vec::new();
    for case in &dataset.cases {
        for doc in &case.relevant_docs {
            if content_to_id.contains_key(&doc.content) {
                continue;
            }
            content_to_id.insert(doc.content.clone(), doc.doc_id.clone());

            if let Some(ref desc) = doc.image_description {
                docs.push(DocInfo {
                    doc_id: doc.doc_id.clone(),
                    content: doc.content.clone(),
                    title: doc.title.clone(),
                    embed_text: if embedder.supports_images() {
                        None
                    } else {
                        Some(desc.clone())
                    },
                    image_desc: if embedder.supports_images() {
                        Some(desc.clone())
                    } else {
                        None
                    },
                });
            } else {
                docs.push(DocInfo {
                    doc_id: doc.doc_id.clone(),
                    content: doc.content.clone(),
                    title: doc.title.clone(),
                    embed_text: Some(doc.content.clone()),
                    image_desc: None,
                });
            }
        }
    }

    // Batch-embed all text docs in one pass (32 per batch to cap memory)
    let ingest_start = Instant::now();
    let text_indices: Vec<usize> = docs
        .iter()
        .enumerate()
        .filter_map(|(i, d)| d.embed_text.as_ref().map(|_| i))
        .collect();
    let text_contents: Vec<&str> = text_indices
        .iter()
        .map(|&i| docs[i].embed_text.as_deref().unwrap())
        .collect();

    let text_embeddings = oi_embed::batch::embed_in_batches(embedder.as_ref(), &text_contents, 32)
        .unwrap_or_else(|_| vec![vec![]; text_contents.len()]);

    // Map text embeddings back to doc indices
    let mut embeddings: Vec<Option<Vec<f32>>> = vec![None; docs.len()];
    for (batch_idx, &doc_idx) in text_indices.iter().enumerate() {
        if !text_embeddings[batch_idx].is_empty() {
            embeddings[doc_idx] = Some(text_embeddings[batch_idx].clone());
        }
    }

    // Embed image docs individually (can't batch with text)
    for (i, doc) in docs.iter().enumerate() {
        if let Some(ref desc) = doc.image_desc {
            let label = image_gen::all_templates()
                .iter()
                .find(|t| t.description == *desc)
                .map(|t| t.label.clone())
                .unwrap_or_else(|| "solid_black".to_string());
            let (png_bytes, _) = image_gen::generate_image(&label);
            embeddings[i] = embedder.embed_image(&png_bytes).ok();
        }
    }

    // Build chunks
    let all_chunks: Vec<ContentChunk> = docs
        .iter()
        .enumerate()
        .map(|(i, doc)| ContentChunk {
            chunk_id: doc.doc_id.clone(),
            content: doc.content.clone(),
            embedding: embeddings[i].clone(),
            seq: 0,
            meta: Some(ChunkMeta {
                url: format!("https://bench.test/{}", doc.doc_id),
                title: doc.title.clone(),
                domain: "bench.test".to_string(),
                content_hash: format!("hash_{}", doc.doc_id),
                crawled_at: Some(1700000000),
                content_type: Some("text/html".to_string()),
                entity: None,
                content_kind: None,
                ..Default::default()
            }),
        })
        .collect();

    index.ingest(&all_chunks).unwrap();
    let ingest_ms = ingest_start.elapsed().as_secs_f64() * 1000.0;
    let docs_per_sec = if ingest_ms > 0.0 {
        all_chunks.len() as f64 / (ingest_ms / 1000.0)
    } else {
        0.0
    };
    tracing::info!(
        "Indexed {} unique documents (deduplicated from {} total) in {:.0}ms ({:.0} docs/sec)",
        all_chunks.len(),
        dataset.cases.len() * 4,
        ingest_ms,
        docs_per_sec,
    );

    // Run queries and collect metrics per difficulty + cross-modal
    let mut results_by_diff: std::collections::HashMap<String, DifficultyMetrics> =
        std::collections::HashMap::new();

    let mut total_latency_ms = Vec::new();
    let mut embed_latency_ms = Vec::new();
    let mut search_latency_ms = Vec::new();

    for case in &dataset.cases {
        let is_cross_modal = case.metadata.modality == "text-to-image";

        let diff_key = if is_cross_modal {
            "cross-modal".to_string()
        } else {
            case.difficulty.to_string()
        };
        let entry = results_by_diff.entry(diff_key).or_default();

        entry.total += 1;

        let start = Instant::now();
        let query_emb = match embedder.embed_query(&case.query) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let embed_ms = start.elapsed().as_secs_f64() * 1000.0;
        embed_latency_ms.push(embed_ms);

        let search_start = Instant::now();
        let filters = QueryFilters::default();
        let search_results = match index.query(&query_emb, &case.query, &filters, 10) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let search_ms = search_start.elapsed().as_secs_f64() * 1000.0;
        search_latency_ms.push(search_ms);

        let latency_ms = start.elapsed().as_secs_f64() * 1000.0;
        total_latency_ms.push(latency_ms);

        let retrieved_ids: Vec<&str> = search_results
            .iter()
            .map(|r| r.url.rsplit('/').next().unwrap_or(""))
            .collect();

        // Map each case's doc_ids to the indexed chunk_ids via content matching.
        // After dedup, doc-50-rel3 may have been indexed as doc-0-rel3 if
        // they share the same content.
        let relevant_ids: Vec<String> = case
            .relevant_docs
            .iter()
            .filter(|d| d.relevance > 0)
            .filter_map(|d| content_to_id.get(&d.content).cloned())
            .collect();
        let relevant_refs: Vec<&str> = relevant_ids.iter().map(|s| s.as_str()).collect();

        let relevance_map: Vec<(String, u8)> = case
            .relevant_docs
            .iter()
            .filter_map(|d| {
                content_to_id
                    .get(&d.content)
                    .map(|id| (id.clone(), d.relevance))
            })
            .collect();
        let relevance_refs: Vec<(&str, u8)> = relevance_map
            .iter()
            .map(|(s, r)| (s.as_str(), *r))
            .collect();

        entry.recall_1 += metrics::recall_at_k(&relevant_refs, &retrieved_ids, 1);
        entry.recall_5 += metrics::recall_at_k(&relevant_refs, &retrieved_ids, 5);
        entry.recall_10 += metrics::recall_at_k(&relevant_refs, &retrieved_ids, 10);
        entry.mrr += metrics::mrr(&relevant_refs, &retrieved_ids);
        entry.ndcg_10 += metrics::ndcg_at_k(&relevance_refs, &retrieved_ids, 10);
    }

    // Quality gate evaluation
    let quality_result = bench_quality::evaluate_quality_gate(&dataset.cases);
    let fpr = bench_quality::evaluate_false_positive_rate(&dataset.cases);

    // Compute latency percentiles
    total_latency_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
    embed_latency_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
    search_latency_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p50 = percentile(&total_latency_ms, 50.0);
    let p95 = percentile(&total_latency_ms, 95.0);
    let p99 = percentile(&total_latency_ms, 99.0);
    let embed_p50 = percentile(&embed_latency_ms, 50.0);
    let embed_p95 = percentile(&embed_latency_ms, 95.0);
    let search_p50 = percentile(&search_latency_ms, 50.0);
    let search_p95 = percentile(&search_latency_ms, 95.0);

    match output_format {
        "json" => {
            let output = serde_json::json!({
                "pipeline": pipeline_name,
                "total_cases": cases,
                "difficulties": results_by_diff.iter().map(|(k, v)| {
                    let n = v.total as f64;
                    serde_json::json!({
                        "difficulty": k,
                        "cases": v.total,
                        "recall_1": v.recall_1 / n,
                        "recall_5": v.recall_5 / n,
                        "recall_10": v.recall_10 / n,
                        "mrr": v.mrr / n,
                        "ndcg_10": v.ndcg_10 / n,
                    })
                }).collect::<Vec<_>>(),
                "quality_gate": {
                    "adversarial_rejection_rate": quality_result.rejection_rate,
                    "false_positive_rate": fpr,
                },
                "latency": {
                    "p50_ms": p50,
                    "p95_ms": p95,
                    "p99_ms": p99,
                },
            });
            println!("{}", serde_json::to_string_pretty(&output).unwrap());
        }
        _ => {
            println!("\n=== OpenSonarX Benchmark Results ===");
            println!("Pipeline: {}  |  Cases: {}\n", pipeline_name, cases);
            println!(
                "{:<15} {:>6} {:>10} {:>10} {:>10} {:>8} {:>10}",
                "Difficulty", "Cases", "Recall@1", "Recall@5", "Recall@10", "MRR", "nDCG@10"
            );
            println!("{}", "-".repeat(75));

            for diff in &["easy", "medium", "hard", "adversarial", "cross-modal"] {
                if let Some(v) = results_by_diff.get(*diff) {
                    let n = v.total as f64;
                    if *diff == "adversarial" {
                        println!(
                            "{:<15} {:>6}    rejection_rate: {:.1}%",
                            diff,
                            v.total,
                            quality_result.rejection_rate * 100.0
                        );
                    } else {
                        println!(
                            "{:<15} {:>6} {:>10.2} {:>10.2} {:>10.2} {:>8.2} {:>10.2}",
                            diff,
                            v.total,
                            v.recall_1 / n,
                            v.recall_5 / n,
                            v.recall_10 / n,
                            v.mrr / n,
                            v.ndcg_10 / n,
                        );
                    }
                }
            }

            println!("\nQuality gate false positive rate: {:.1}%", fpr * 100.0);
            println!(
                "\nLatency (end-to-end): p50={:.1}ms  p95={:.1}ms  p99={:.1}ms",
                p50, p95, p99
            );
            println!("  Embed:  p50={:.1}ms  p95={:.1}ms", embed_p50, embed_p95);
            println!(
                "  Search: p50={:.1}ms  p95={:.1}ms\n",
                search_p50, search_p95
            );
        }
    }
}

fn run_sweep_command(cases: usize, pipeline_name: &str) {
    let embedder = get_embedder(pipeline_name);
    let dim = embedder.dimension();

    tracing::info!(
        "Generating {cases} test cases with pipeline '{}' (dim={dim})",
        embedder.name()
    );

    let dataset = generator::generate_text_dataset(cases);
    let sweep_config = tuning::SweepConfig::default();

    let total_combos = sweep_config.semantic_weights.len()
        * sweep_config.ef_search_values.len()
        * sweep_config.pool_multipliers.len()
        * sweep_config.fusion_methods.len();

    tracing::info!("Running sweep: {total_combos} configurations...");

    let results = tuning::run_sweep(&dataset, embedder, &sweep_config);

    println!("\n=== Sweep Results (sorted by Recall@10) ===");
    println!("Pipeline: {}  |  Cases: {}\n", pipeline_name, cases);
    println!(
        "{:>4} {:<55} {:>7} {:>7} {:>7} {:>7} {:>7} {:>8}",
        "Rank", "Config", "R@1", "R@5", "R@10", "MRR", "nDCG", "Lat(ms)"
    );
    println!("{}", "-".repeat(102));

    for (i, r) in results.iter().enumerate().take(20) {
        println!("{:>4} {}", i + 1, r);
    }
    println!();
}

#[derive(Default)]
struct DifficultyMetrics {
    total: usize,
    recall_1: f64,
    recall_5: f64,
    recall_10: f64,
    mrr: f64,
    ndcg_10: f64,
}

fn percentile(sorted: &[f64], pct: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((pct / 100.0) * (sorted.len() - 1) as f64) as usize;
    sorted[idx.min(sorted.len() - 1)]
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match cli.command {
        Command::Server { docs, port } => {
            tracing::info!("Seeding index with {} documents...", docs);
            let index = seed::seed_index(docs);
            let doc_count = index.doc_count();
            tracing::info!("Index ready: {} documents", doc_count);

            let state = Arc::new(AppState { index, doc_count });

            let app = Router::new()
                .route("/health", get(health))
                .route("/stats", get(stats))
                .route("/search", post(search_handler))
                .with_state(state);

            let addr = format!("0.0.0.0:{}", port);
            tracing::info!("Listening on {}", addr);

            let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
            axum::serve(listener, app).await.unwrap();
        }
        Command::Benchmark {
            cases,
            pipeline,
            output,
            modality,
        } => {
            run_benchmark(cases, &pipeline, &output, &modality);
        }
        Command::Beir {
            dataset,
            pipeline,
            output,
            semantic_weight,
            rerank,
        } => {
            beir::run_beir(&dataset, &pipeline, &output, semantic_weight, rerank);
        }
        Command::BeirSweep { dataset, pipeline } => {
            beir::sweep_beir(&dataset, &pipeline);
        }
        Command::Marco {
            pipeline,
            output,
            semantic_weight,
            limit,
            rerank,
        } => {
            msmarco::run_msmarco(&pipeline, &output, limit, semantic_weight, rerank);
        }
        Command::DistSim {
            nodes,
            fanout,
            distribution,
            pipeline,
            limit,
            semantic_weight,
        } => {
            dist_sim::run_dist_sim(
                nodes,
                fanout,
                &distribution,
                &pipeline,
                limit,
                semantic_weight,
            );
        }
        Command::Sweep { cases, pipeline } => {
            run_sweep_command(cases, &pipeline);
        }
        Command::Scaling {
            sizes,
            queries,
            output,
            vector_only,
        } => {
            let size_list = sizes
                .map(|s| scaling::parse_sizes(&s))
                .unwrap_or_else(scaling::default_sizes);
            scaling::run_scaling(&size_list, queries, &output, vector_only);
        }
        Command::ScaleIngest { limit, index_path } => {
            scale_ingest::run_scale_ingest(limit, &index_path);
        }
        Command::SealQa {
            pipeline,
            output,
            semantic_weight,
            docs,
            rerank,
        } => {
            sealqa::run_sealqa(&pipeline, &output, semantic_weight, docs, rerank);
        }
        Command::FactsBench {
            pipeline,
            output,
            skip_langs,
        } => {
            let skip: Vec<String> = skip_langs
                .unwrap_or_default()
                .split(',')
                .filter(|s| !s.is_empty())
                .map(|s| s.trim().to_string())
                .collect();
            facts_bench::run_facts_bench(&pipeline, &output, &skip);
        }
        Command::AgentCost {
            pipeline,
            output,
            backends,
            top_k,
            model,
        } => {
            agent_cost::run_agent_cost(&pipeline, &output, &backends, top_k, &model).await;
        }
        Command::AnswerEval {
            cases,
            provider,
            model,
            judge_model,
            base_url,
            concurrency,
            no_responsiveness,
            think,
            min_source_families,
            output,
        } => {
            if let Err(error) = answer_eval::run(
                &cases,
                &provider,
                &model,
                judge_model.as_deref(),
                base_url.as_deref(),
                think.as_deref(),
                min_source_families,
                concurrency,
                !no_responsiveness,
                &output,
            ) {
                eprintln!("answer-eval failed: {error}");
                std::process::exit(2);
            }
        }
        Command::AnswerEvalImportSquad {
            input,
            sample,
            label,
            exclude,
            output,
        } => {
            if let Err(error) = answer_eval_squad::run(&input, sample, &label, &exclude, &output) {
                eprintln!("answer-eval-import-squad failed: {error}");
                std::process::exit(2);
            }
        }
        Command::Uncertainty {
            train_cases,
            eval_cases,
            throughput_claims,
            output,
        } => {
            uncertainty_bench::run_uncertainty_benchmark(
                train_cases,
                eval_cases,
                throughput_claims,
                &output,
            );
        }
        Command::UncertaintyDataset {
            dataset,
            output,
            bootstrap_iterations,
        } => {
            if let Err(error) =
                uncertainty_bench::run_uncertainty_dataset(&dataset, &output, bootstrap_iterations)
            {
                eprintln!("uncertainty dataset evaluation failed: {error}");
                std::process::exit(2);
            }
        }
        Command::ElectionPolls { dataset, output } => {
            if let Err(error) = election_polls_bench::run(&dataset, &output) {
                eprintln!("election-polls benchmark failed: {error}");
                std::process::exit(2);
            }
        }
        Command::EngineProbe {
            engines,
            queries,
            limit,
            rounds,
            concurrency,
            min_healthy,
            output,
            report_path,
        } => {
            if let Err(error) = engine_probe::run(
                &engines,
                &queries,
                limit,
                rounds,
                concurrency,
                min_healthy,
                &output,
                report_path.as_deref(),
            ) {
                eprintln!("engine-probe failed: {error}");
                std::process::exit(2);
            }
        }
        Command::MarketCompare {
            fixture,
            live_report,
            output,
            report_path,
        } => {
            if let Err(error) =
                market_compare_bench::run(&fixture, &live_report, &output, report_path.as_deref())
            {
                eprintln!("market-compare failed: {error}");
                std::process::exit(2);
            }
        }
        Command::MarketLatency {
            fixture,
            output,
            report_path,
        } => {
            if let Err(error) = market_latency_bench::run(&fixture, &output, report_path.as_deref())
            {
                eprintln!("market-latency failed: {error}");
                std::process::exit(2);
            }
        }
        Command::GeneralizationCapture {
            manifest,
            data_dir,
            engines,
            passes,
            concurrency,
            top_k,
            crawl,
            output,
            report_path,
        } => {
            if let Err(error) = generalization_capture::run(
                &manifest,
                &data_dir,
                engines.as_deref(),
                passes,
                concurrency,
                top_k,
                crawl,
                &output,
                report_path.as_deref(),
            ) {
                eprintln!("generalization-capture failed: {error}");
                std::process::exit(2);
            }
        }
        Command::GeneralizationMarketCapture {
            manifest,
            engine,
            top_k,
            concurrency,
            output,
            report_path,
        } => {
            if let Err(error) = generalization_market_capture::run(
                &manifest,
                &engine,
                top_k,
                concurrency,
                &output,
                report_path.as_deref(),
            ) {
                eprintln!("generalization-market-capture failed: {error}");
                std::process::exit(2);
            }
        }
        Command::GeneralizationBlindPool {
            manifest,
            opensonarx_report,
            market_report,
            top_k,
            blinding_nonce,
            packet_path,
            key_path,
            output,
        } => {
            if let Err(error) = generalization_blind::create_pool(
                &manifest,
                &opensonarx_report,
                &market_report,
                top_k,
                &blinding_nonce,
                &packet_path,
                &key_path,
                &output,
            ) {
                eprintln!("generalization-blind-pool failed: {error}");
                std::process::exit(2);
            }
        }
        Command::GeneralizationBlindTemplate {
            packet,
            judge_id,
            external_to_implementation,
            template_path,
        } => {
            if let Err(error) = generalization_blind::create_judgment_template(
                &packet,
                &judge_id,
                external_to_implementation,
                &template_path,
            ) {
                eprintln!("generalization-blind-template failed: {error}");
                std::process::exit(2);
            }
        }
        Command::GeneralizationBlindOllamaJudge {
            packet,
            model,
            base_url,
            concurrency,
            queries_per_request,
            thinking,
            judgments_path,
        } => {
            if let Err(error) = generalization_blind::judge_with_ollama(
                &packet,
                &model,
                &base_url,
                concurrency,
                queries_per_request,
                &thinking,
                &judgments_path,
            ) {
                eprintln!("generalization-blind-ollama-judge failed: {error}");
                std::process::exit(2);
            }
        }
        Command::GeneralizationBlindScore {
            packet,
            key,
            judgments,
            bootstrap_iterations,
            output,
            report_path,
        } => {
            if let Err(error) = generalization_blind::score(
                &packet,
                &key,
                &judgments,
                bootstrap_iterations,
                &output,
                report_path.as_deref(),
            ) {
                eprintln!("generalization-blind-score failed: {error}");
                std::process::exit(2);
            }
        }
        Command::GeneralizationCompare {
            manifest,
            opensonarx_report,
            google_report,
            output,
            report_path,
        } => {
            if let Err(error) = generalization_compare::run(
                &manifest,
                &opensonarx_report,
                &google_report,
                &output,
                report_path.as_deref(),
            ) {
                eprintln!("generalization-compare failed: {error}");
                std::process::exit(2);
            }
        }
        Command::ProductionReadiness {
            provider_report,
            cold_report,
            resident_report,
            market_report,
            min_healthy_providers,
            max_cold_search_p99_ms,
            max_resident_search_p95_ms,
            max_resident_search_p99_ms,
            min_resident_qps,
            output,
            report_path,
        } => {
            let thresholds = production_readiness_bench::Thresholds {
                min_healthy_providers,
                max_cold_search_p99_ms,
                max_resident_search_p95_ms,
                max_resident_search_p99_ms,
                min_resident_qps,
            };
            if let Err(error) = production_readiness_bench::run(
                &provider_report,
                &cold_report,
                &resident_report,
                &market_report,
                thresholds,
                &output,
                report_path.as_deref(),
            ) {
                eprintln!("production-readiness failed: {error}");
                std::process::exit(2);
            }
        }
        Command::Generate { cases, output } => {
            let dataset = generator::generate_text_dataset(cases);
            let json = serde_json::to_string_pretty(&dataset).unwrap();

            // Ensure parent directory exists
            if let Some(parent) = std::path::Path::new(&output).parent() {
                std::fs::create_dir_all(parent).ok();
            }

            std::fs::write(&output, &json).unwrap();
            println!("Generated {} cases -> {}", cases, output);
        }
    }
}
