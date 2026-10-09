use oi_sdk::{AnswerTrace, OpenSonarXClient, SdkConfig, SearchAnswer};
use oi_types::{EvidenceLineageMetadata, QueryFilters};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::io::Read;
use std::time::Instant;

const MIN_HOLDOUT_QUERIES: usize = 40;

#[derive(Debug, Deserialize)]
struct Manifest {
    schema_version: u32,
    name: String,
    sealed_at: String,
    selection: serde_json::Value,
    queries: Vec<ManifestQuery>,
}

#[derive(Debug, Deserialize)]
struct PriorHoldoutRef {
    path: String,
    sha256: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct ManifestQuery {
    id: String,
    query: String,
    source: String,
    source_id: String,
    language: String,
    locale: String,
    country: String,
    stratum: String,
}

#[derive(Debug, Serialize)]
struct CapturedResult {
    rank: u32,
    retrieval_rank: u32,
    decision_rank: u32,
    title: String,
    url: String,
    snippet: String,
    source: String,
    published_at: Option<i64>,
    crawled_at: Option<u64>,
    relevance_score: f64,
    decision_score: f64,
    uncertainty_mode: String,
    ignorance: f64,
    independence: f64,
    cash_log_bayes_factor: f64,
    critical_missing: bool,
    p_reliable: Option<f64>,
    support: f64,
    conflict: f64,
    evidence_signals: Vec<String>,
    evidence_lineage: Option<EvidenceLineageMetadata>,
}

#[derive(Debug, Serialize)]
struct QueryCapture {
    pass: usize,
    manifest: ManifestQuery,
    classified_intent: String,
    search_ms: f64,
    succeeded: bool,
    error: Option<String>,
    answer: Option<SearchAnswer>,
    sources_used: Vec<String>,
    hydrated_pages: usize,
    trace: Option<AnswerTrace>,
    results: Vec<CapturedResult>,
}

#[derive(Debug, Serialize)]
struct Stats {
    count: usize,
    min_ms: f64,
    p50_ms: f64,
    p95_ms: f64,
    p99_ms: f64,
    max_ms: f64,
}

#[derive(Debug, Serialize)]
struct CaptureReport {
    captured_at_unix_ms: u128,
    experiment: &'static str,
    manifest_path: String,
    manifest_sha256: String,
    evaluated_binary_sha256: String,
    manifest_name: String,
    manifest_sealed_at: String,
    selection: serde_json::Value,
    development_overlap_rejected: bool,
    prior_holdout_overlap_rejected: bool,
    judged_during_capture: bool,
    real_embedder: String,
    data_dir: String,
    discovery_engine_override: Option<String>,
    passes: usize,
    concurrency: usize,
    top_k: usize,
    crawl_enabled: bool,
    initial_documents: usize,
    final_documents: usize,
    query_count: usize,
    request_count: usize,
    succeeded: usize,
    failed: usize,
    nonempty_result_slates: usize,
    answer_status_counts: BTreeMap<String, usize>,
    intent_counts: BTreeMap<String, usize>,
    search_latency: Stats,
    wall_ms: f64,
    throughput_qps: f64,
    background_crawl_ms: f64,
    background_crawled_pages: usize,
    captures: Vec<QueryCapture>,
    limitations: Vec<&'static str>,
}

#[allow(clippy::too_many_arguments)]
pub fn run(
    manifest_path: &str,
    data_dir: &str,
    engine_override: Option<&str>,
    passes: usize,
    concurrency: usize,
    top_k: usize,
    crawl: bool,
    output: &str,
    report_path: Option<&str>,
) -> Result<(), String> {
    if passes == 0 || concurrency == 0 || top_k == 0 {
        return Err("passes, concurrency, and top-k must be positive".to_string());
    }
    let manifest_bytes = std::fs::read(manifest_path)
        .map_err(|error| format!("cannot read holdout manifest {manifest_path}: {error}"))?;
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| format!("cannot parse holdout manifest {manifest_path}: {error}"))?;
    validate_manifest(&manifest)?;
    let prior_holdout_overlap_rejected = validate_prior_holdouts(&manifest)?;

    std::fs::create_dir_all(data_dir)
        .map_err(|error| format!("cannot create holdout data dir {data_dir}: {error}"))?;
    let search_engines = parse_engine_override(engine_override)?;
    let config = SdkConfig {
        local_index_path: Some(
            std::path::Path::new(data_dir)
                .join("index")
                .to_string_lossy()
                .to_string(),
        ),
        brave_api_key: nonempty_env("BRAVE_API_KEY"),
        serper_api_key: nonempty_env("SERPER_API_KEY"),
        search_engines,
        crawl_on_query: true,
        crawl_on_query_max_urls: 3,
        stakes_path: Some(
            std::path::Path::new(data_dir)
                .join("stakes.json")
                .to_string_lossy()
                .to_string(),
        ),
        creator_ledger_path: Some(
            std::path::Path::new(data_dir)
                .join("creator_ledger.json")
                .to_string_lossy()
                .to_string(),
        ),
        ..Default::default()
    };
    let client = OpenSonarXClient::new(config);
    let embedder_name = client.embedder().name().to_string();
    if embedder_name.to_ascii_lowercase().contains("mock") {
        return Err(format!(
            "refusing generalization capture with mock embedder '{embedder_name}'; run with --features onnx"
        ));
    }

    let manifest_sha256 = sha256_bytes(&manifest_bytes);
    let current_exe = std::env::current_exe()
        .map_err(|error| format!("cannot resolve evaluated binary: {error}"))?;
    let evaluated_binary_sha256 = sha256_file(&current_exe)?;
    let initial_documents = client.indexed_document_count();
    let request_count = manifest.queries.len() * passes;
    let mut captures = Vec::with_capacity(request_count);
    let mut background_crawl_ms = 0.0;
    let mut background_crawled_pages = 0;
    let benchmark_started = Instant::now();

    for pass in 1..=passes {
        for batch in manifest.queries.chunks(concurrency) {
            let mut batch_captures = if batch.len() == 1 {
                vec![capture_query(&client, &batch[0], pass, top_k)]
            } else {
                std::thread::scope(|scope| {
                    let barrier = std::sync::Arc::new(std::sync::Barrier::new(batch.len()));
                    let handles = batch
                        .iter()
                        .map(|query| {
                            let barrier = std::sync::Arc::clone(&barrier);
                            let client = &client;
                            scope.spawn(move || {
                                barrier.wait();
                                capture_query(client, query, pass, top_k)
                            })
                        })
                        .collect::<Vec<_>>();
                    handles
                        .into_iter()
                        .map(|handle| {
                            handle
                                .join()
                                .map_err(|_| "holdout capture worker panicked".to_string())
                        })
                        .collect::<Result<Vec<_>, String>>()
                })?
            };
            if crawl {
                let crawl_started = Instant::now();
                background_crawled_pages += client.process_crawl_queue();
                background_crawl_ms += crawl_started.elapsed().as_secs_f64() * 1_000.0;
            }
            captures.append(&mut batch_captures);
        }
    }

    let wall_ms = benchmark_started.elapsed().as_secs_f64() * 1_000.0;
    let succeeded = captures.iter().filter(|capture| capture.succeeded).count();
    let nonempty_result_slates = captures
        .iter()
        .filter(|capture| !capture.results.is_empty())
        .count();
    let mut answer_status_counts = BTreeMap::new();
    let mut intent_counts = BTreeMap::new();
    for capture in &captures {
        *intent_counts
            .entry(capture.classified_intent.clone())
            .or_insert(0) += 1;
        let status = capture
            .answer
            .as_ref()
            .map(|answer| answer.status.as_str())
            .unwrap_or("error");
        *answer_status_counts.entry(status.to_string()).or_insert(0) += 1;
    }
    let search_latency = stats(captures.iter().map(|capture| capture.search_ms).collect());
    let report = CaptureReport {
        captured_at_unix_ms: unix_time_ms(),
        experiment: "sealed-unjudged-generalization-capture-v1",
        manifest_path: manifest_path.to_string(),
        manifest_sha256,
        evaluated_binary_sha256,
        manifest_name: manifest.name,
        manifest_sealed_at: manifest.sealed_at,
        selection: manifest.selection,
        development_overlap_rejected: true,
        prior_holdout_overlap_rejected,
        judged_during_capture: false,
        real_embedder: embedder_name,
        data_dir: data_dir.to_string(),
        discovery_engine_override: engine_override.map(str::to_string),
        passes,
        concurrency,
        top_k,
        crawl_enabled: crawl,
        initial_documents,
        final_documents: client.indexed_document_count(),
        query_count: manifest.queries.len(),
        request_count,
        succeeded,
        failed: request_count - succeeded,
        nonempty_result_slates,
        answer_status_counts,
        intent_counts,
        search_latency,
        wall_ms,
        throughput_qps: request_count as f64 / (wall_ms / 1_000.0).max(f64::EPSILON),
        background_crawl_ms,
        background_crawled_pages,
        captures,
        limitations: vec![
            "This artifact is an unjudged capture. It measures coverage and behavior but contains no relevance or trust verdict.",
            "The query manifest is visible after sealing; production behavior must not be changed from individual failures before a scored report is frozen.",
            "MS MARCO answer-seeking queries do not represent navigation, local search, current events, commerce, or non-English traffic.",
            "General market-superiority claims require additional strata, pooled vendor results, blinded judgments, and confidence intervals.",
        ],
    };

    let serialized = serde_json::to_string_pretty(&report)
        .map_err(|error| format!("cannot serialize generalization capture: {error}"))?;
    if let Some(path) = report_path {
        persist_report(path, &serialized)?;
    }
    match output {
        "json" => println!("{serialized}"),
        "table" => print_table(&report),
        other => return Err(format!("unsupported generalization output format: {other}")),
    }
    Ok(())
}

fn validate_manifest(manifest: &Manifest) -> Result<(), String> {
    if manifest.schema_version != 1 {
        return Err(format!(
            "unsupported generalization manifest schema {}; expected 1",
            manifest.schema_version
        ));
    }
    if manifest.queries.len() < MIN_HOLDOUT_QUERIES {
        return Err(format!(
            "generalization holdout has {} queries; at least {MIN_HOLDOUT_QUERIES} are required",
            manifest.queries.len()
        ));
    }
    let seen = development_query_texts()?
        .iter()
        .map(|query| normalize_query(query))
        .collect::<HashSet<_>>();
    let mut ids = HashSet::new();
    let mut queries = HashSet::new();
    for query in &manifest.queries {
        if query.id.trim().is_empty() || query.query.trim().is_empty() {
            return Err("holdout query ids and text must be non-empty".to_string());
        }
        if !ids.insert(query.id.as_str()) {
            return Err(format!("duplicate holdout id: {}", query.id));
        }
        let normalized = normalize_query(&query.query);
        if !queries.insert(normalized.clone()) {
            return Err(format!(
                "duplicate normalized holdout query: {}",
                query.query
            ));
        }
        if seen.contains(&normalized) {
            return Err(format!(
                "holdout query overlaps the development regression suite: {}",
                query.query
            ));
        }
    }
    Ok(())
}

fn validate_prior_holdouts(manifest: &Manifest) -> Result<bool, String> {
    let Some(raw_refs) = manifest.selection.get("prior_holdouts_excluded") else {
        return Ok(false);
    };
    let refs: Vec<PriorHoldoutRef> = serde_json::from_value(raw_refs.clone())
        .map_err(|error| format!("invalid prior_holdouts_excluded declaration: {error}"))?;
    if refs.is_empty() {
        return Err("prior_holdouts_excluded must not be an empty array".to_string());
    }

    let mut seen_paths = HashSet::new();
    for prior_ref in refs {
        if !seen_paths.insert(prior_ref.path.clone()) {
            return Err(format!(
                "duplicate prior holdout reference: {}",
                prior_ref.path
            ));
        }
        let bytes = std::fs::read(&prior_ref.path).map_err(|error| {
            format!(
                "cannot read prior holdout manifest {}: {error}",
                prior_ref.path
            )
        })?;
        let actual_sha256 = sha256_bytes(&bytes);
        if actual_sha256 != prior_ref.sha256 {
            return Err(format!(
                "prior holdout hash mismatch for {}: declared {}, got {}",
                prior_ref.path, prior_ref.sha256, actual_sha256
            ));
        }
        let prior: Manifest = serde_json::from_slice(&bytes).map_err(|error| {
            format!(
                "cannot parse prior holdout manifest {}: {error}",
                prior_ref.path
            )
        })?;
        validate_no_overlap_with_prior(&manifest.queries, &prior.queries, &prior_ref.path)?;
    }
    Ok(true)
}

fn validate_no_overlap_with_prior(
    current: &[ManifestQuery],
    prior: &[ManifestQuery],
    prior_name: &str,
) -> Result<(), String> {
    let prior_ids = prior
        .iter()
        .map(|query| query.id.as_str())
        .collect::<HashSet<_>>();
    let prior_queries = prior
        .iter()
        .map(|query| normalize_query(&query.query))
        .collect::<HashSet<_>>();
    for query in current {
        if prior_ids.contains(query.id.as_str())
            || prior_queries.contains(&normalize_query(&query.query))
        {
            return Err(format!(
                "holdout query overlaps prior frozen manifest {prior_name}: {}",
                query.query
            ));
        }
    }
    Ok(())
}

fn capture_query(
    client: &OpenSonarXClient,
    query: &ManifestQuery,
    pass: usize,
    top_k: usize,
) -> QueryCapture {
    let filters = QueryFilters {
        locale: Some(query.locale.clone()),
        gl: Some(query.country.to_ascii_lowercase()),
        ..Default::default()
    };
    let started = Instant::now();
    match client.search_answered(&query.query, top_k, &filters) {
        Ok(response) => {
            let results = response
                .results
                .iter()
                .map(|result| {
                    let decision = result.decision.as_ref();
                    CapturedResult {
                        rank: result.rank,
                        retrieval_rank: decision
                            .map(|value| value.retrieval_rank)
                            .unwrap_or(result.rank),
                        decision_rank: decision
                            .map(|value| value.decision_rank)
                            .unwrap_or(result.rank),
                        title: result.title.clone(),
                        url: result.url.clone(),
                        snippet: result.snippet.clone(),
                        source: result.source.clone(),
                        published_at: result.published_at,
                        crawled_at: result.crawled_at,
                        relevance_score: result.final_score,
                        decision_score: decision
                            .map(|value| value.decision_score)
                            .unwrap_or(result.final_score),
                        uncertainty_mode: decision
                            .map(|value| value.uncertainty_mode.clone())
                            .unwrap_or_else(|| "not_run".to_string()),
                        ignorance: decision.map(|value| value.ignorance).unwrap_or(1.0),
                        independence: decision.map(|value| value.independence).unwrap_or(1.0),
                        cash_log_bayes_factor: decision
                            .map(|value| value.cash_log_bayes_factor)
                            .unwrap_or(0.0),
                        p_reliable: decision.and_then(|value| value.p_reliable),
                        critical_missing: decision
                            .map(|value| value.critical_missing)
                            .unwrap_or(true),
                        support: decision.map(|value| value.support).unwrap_or(0.0),
                        conflict: decision.map(|value| value.conflict).unwrap_or(0.0),
                        evidence_signals: decision
                            .map(|value| value.evidence_signals.clone())
                            .unwrap_or_default(),
                        evidence_lineage: decision.and_then(|value| value.evidence_lineage.clone()),
                    }
                })
                .collect();
            QueryCapture {
                pass,
                manifest: query.clone(),
                classified_intent: response.answer.intent.clone(),
                search_ms: started.elapsed().as_secs_f64() * 1_000.0,
                succeeded: true,
                error: None,
                answer: Some(response.answer),
                sources_used: response.sources_used,
                hydrated_pages: response.hydrated_pages,
                trace: Some(response.trace),
                results,
            }
        }
        Err(error) => QueryCapture {
            pass,
            manifest: query.clone(),
            classified_intent: "unclassified".to_string(),
            search_ms: started.elapsed().as_secs_f64() * 1_000.0,
            succeeded: false,
            error: Some(error.to_string()),
            answer: None,
            sources_used: Vec::new(),
            hydrated_pages: 0,
            trace: None,
            results: Vec::new(),
        },
    }
}

fn normalize_query(query: &str) -> String {
    query
        .chars()
        .map(|character| {
            if character.is_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn sha256_file(path: &std::path::Path) -> Result<String, String> {
    let mut file = File::open(path)
        .map_err(|error| format!("cannot open {} for hashing: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("cannot hash {}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn stats(mut values: Vec<f64>) -> Stats {
    values.sort_by(f64::total_cmp);
    let count = values.len();
    let percentile = |fraction: f64| {
        let index = ((count as f64 * fraction).ceil() as usize).saturating_sub(1);
        values[index.min(count.saturating_sub(1))]
    };
    Stats {
        count,
        min_ms: values.first().copied().unwrap_or_default(),
        p50_ms: percentile(0.50),
        p95_ms: percentile(0.95),
        p99_ms: percentile(0.99),
        max_ms: values.last().copied().unwrap_or_default(),
    }
}

fn persist_report(path: &str, serialized: &str) -> Result<(), String> {
    if let Some(parent) = std::path::Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("cannot create capture directory: {error}"))?;
        }
    }
    std::fs::write(path, serialized)
        .map_err(|error| format!("cannot write capture report {path}: {error}"))
}

fn unix_time_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn print_table(report: &CaptureReport) {
    println!(
        "sealed unjudged generalization capture | queries={} requests={} | succeeded={} nonempty={} failed={}",
        report.query_count,
        report.request_count,
        report.succeeded,
        report.nonempty_result_slates,
        report.failed
    );
    println!(
        "latency p50/p95/p99={:.1}/{:.1}/{:.1}ms | throughput={:.2}qps | docs={}→{} | judged=false",
        report.search_latency.p50_ms,
        report.search_latency.p95_ms,
        report.search_latency.p99_ms,
        report.throughput_qps,
        report.initial_documents,
        report.final_documents
    );
    println!("answer statuses: {:?}", report.answer_status_counts);
    println!("classified intents: {:?}", report.intent_counts);
    println!("binary sha256: {}", report.evaluated_binary_sha256);
    println!("manifest sha256: {}", report.manifest_sha256);
}

const DEVELOPMENT_QUERIES: &str = include_str!("../fixtures/development-queries-seen.json");

/// Queries already inspected during development. A holdout that repeats one
/// of them is not a holdout.
fn development_query_texts() -> Result<Vec<String>, String> {
    #[derive(serde::Deserialize)]
    struct Seen {
        queries: Vec<String>,
    }
    serde_json::from_str::<Seen>(DEVELOPMENT_QUERIES)
        .map(|seen| seen.queries)
        .map_err(|error| format!("development query fixture is invalid: {error}"))
}

pub(crate) fn nonempty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

pub(crate) fn parse_engine_override(
    engine_override: Option<&str>,
) -> Result<Option<Vec<oi_sdk::config::SearchEngineConfig>>, String> {
    let Some(raw) = engine_override else {
        return Ok(None);
    };
    let mut configs = Vec::new();
    for (index, name) in raw
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .enumerate()
    {
        let canonical = name.to_ascii_lowercase();
        let api_key = match canonical.as_str() {
            "brave" => Some(
                nonempty_env("BRAVE_API_KEY")
                    .ok_or_else(|| "--engines brave requires BRAVE_API_KEY".to_string())?,
            ),
            "serper" => Some(
                nonempty_env("SERPER_API_KEY")
                    .ok_or_else(|| "--engines serper requires SERPER_API_KEY".to_string())?,
            ),
            "bing" | "brave-html" | "brave_html" | "duckduckgo" | "ddg" | "google" | "qwant"
            | "stract" | "yep" => None,
            _ => return Err(format!("unknown --engines adapter: {name}")),
        };
        configs.push(oi_sdk::config::SearchEngineConfig {
            name: canonical,
            api_key,
            enabled: true,
            priority: (index + 1) as u32,
        });
    }
    if configs.is_empty() {
        return Err("--engines requires at least one adapter".to_string());
    }
    Ok(Some(configs))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(index: usize) -> ManifestQuery {
        ManifestQuery {
            id: format!("q-{index}"),
            query: format!("unseen independent query {index}"),
            source: "test".to_string(),
            source_id: index.to_string(),
            language: "en".to_string(),
            locale: "en-US".to_string(),
            country: "US".to_string(),
            stratum: "answer_seeking".to_string(),
        }
    }

    fn manifest() -> Manifest {
        Manifest {
            schema_version: 1,
            name: "test".to_string(),
            sealed_at: "2026-08-22".to_string(),
            selection: serde_json::json!({"seed": "test"}),
            queries: (0..MIN_HOLDOUT_QUERIES).map(query).collect(),
        }
    }

    #[test]
    fn rejects_small_or_duplicate_holdouts() {
        let mut too_small = manifest();
        too_small.queries.pop();
        assert!(validate_manifest(&too_small)
            .expect_err("small holdout must fail")
            .contains("at least"));

        let mut duplicate = manifest();
        duplicate.queries[1].query = duplicate.queries[0].query.clone();
        assert!(validate_manifest(&duplicate)
            .expect_err("duplicate query must fail")
            .contains("duplicate normalized"));
    }

    #[test]
    fn rejects_seen_development_queries() {
        let mut holdout = manifest();
        holdout.queries[0].query = "claude".to_string();
        assert!(validate_manifest(&holdout)
            .expect_err("seen query must fail")
            .contains("development regression"));
    }

    #[test]
    fn rejects_overlap_with_a_prior_frozen_holdout() {
        let current = manifest();
        let mut prior = manifest();
        prior.queries[0].id = "prior-id".to_string();

        assert!(
            validate_no_overlap_with_prior(&current.queries, &prior.queries, "prior.json")
                .expect_err("reused query text must fail")
                .contains("prior frozen manifest")
        );

        for (index, query) in prior.queries.iter_mut().enumerate() {
            query.id = format!("prior-{index}");
            query.query = format!("different prior query {index}");
        }
        validate_no_overlap_with_prior(&current.queries, &prior.queries, "prior.json")
            .expect("disjoint prior manifest must pass");
    }
}
