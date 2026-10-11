use rand::{rngs::StdRng, Rng, SeedableRng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::BufReader;

const MIN_HOLDOUT_QUERIES: usize = 40;
const MIN_SUPERIORITY_QUERIES: usize = 1_000;
const MIN_SUPERIORITY_STRATA: usize = 8;
const MIN_INDEPENDENT_JUDGES: usize = 2;

#[derive(Debug, Deserialize)]
struct Manifest {
    name: String,
    queries: Vec<ManifestQuery>,
}

#[derive(Debug, Clone, Deserialize)]
struct ManifestQuery {
    id: String,
    query: String,
    stratum: String,
}

#[derive(Debug, Deserialize)]
struct OpenReport {
    experiment: String,
    manifest_sha256: String,
    judged_during_capture: bool,
    captures: Vec<OpenCapture>,
}

#[derive(Debug, Deserialize)]
struct OpenCapture {
    pass: usize,
    manifest: ManifestQuery,
    results: Vec<CapturedResult>,
}

#[derive(Debug, Deserialize)]
struct MarketReport {
    experiment: String,
    manifest_sha256: String,
    engine: String,
    live_network: bool,
    mock_results: bool,
    judged_during_capture: bool,
    query_count: usize,
    nonempty: usize,
    complete_coverage_gate_passed: bool,
    cases: Vec<MarketCapture>,
}

#[derive(Debug, Deserialize)]
struct MarketCapture {
    manifest: ManifestQuery,
    results: Vec<CapturedResult>,
}

#[derive(Debug, Clone, Deserialize)]
struct CapturedResult {
    rank: u32,
    title: String,
    url: String,
    snippet: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Rubric {
    relevance: BTreeMap<String, String>,
    query_relative_authority: BTreeMap<String, String>,
    direct_answer_support: BTreeMap<String, String>,
    severe_error: String,
    unsafe_reliance: String,
    judge_confidence: BTreeMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct BlindPacket {
    experiment: String,
    created_at_unix_ms: u128,
    manifest_name: String,
    manifest_sha256: String,
    opensonarx_capture_sha256: String,
    market_capture_sha256: String,
    top_k_per_system: usize,
    blinding_nonce_commitment: String,
    system_identity_present: bool,
    rubric: Rubric,
    queries: Vec<BlindQuery>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BlindQuery {
    id: String,
    query: String,
    stratum: String,
    candidates: Vec<BlindCandidate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BlindCandidate {
    candidate_id: String,
    title: String,
    url: String,
    snippet: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct PrivateKey {
    experiment: String,
    packet_sha256: String,
    manifest_sha256: String,
    opensonarx_capture_sha256: String,
    market_capture_sha256: String,
    market_engine: String,
    entries: Vec<KeyEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
struct KeyEntry {
    query_id: String,
    candidate_id: String,
    canonical_url: String,
    placements: Vec<Placement>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Placement {
    system: String,
    rank: u32,
}

#[derive(Debug, Deserialize)]
struct JudgmentReport {
    experiment: String,
    packet_sha256: String,
    judge_id: String,
    system_identity_blinded: bool,
    #[serde(default)]
    external_to_implementation: bool,
    judgments: Vec<CandidateJudgment>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct CandidateJudgment {
    query_id: String,
    candidate_id: String,
    relevance: Option<u8>,
    query_relative_authority: Option<u8>,
    direct_answer_support: Option<u8>,
    severe_error: Option<bool>,
    unsafe_reliance: Option<bool>,
    judge_confidence: Option<u8>,
    #[serde(default, rename = "notes")]
    _notes: String,
}

#[derive(Debug, Serialize)]
struct PoolSummary {
    experiment: &'static str,
    packet_path: String,
    private_key_path: String,
    packet_sha256: String,
    manifest_sha256: String,
    query_count: usize,
    pooled_candidate_count: usize,
    shared_candidate_count: usize,
    system_identity_present_in_packet: bool,
    ready_for_blind_judgment: bool,
}

#[derive(Debug, Clone, Default)]
struct QueryMetrics {
    nonempty: f64,
    top1_relevance: f64,
    top1_authority: f64,
    top1_support: f64,
    top1_acceptable: f64,
    top1_severe_error: f64,
    top1_unsafe_reliance: f64,
    ndcg_relevance_at_5: f64,
    ndcg_authority_at_5: f64,
}

#[derive(Debug, Serialize)]
struct SystemMetrics {
    system: String,
    queries: usize,
    nonempty_rate: f64,
    mean_top1_relevance: f64,
    mean_top1_authority: f64,
    mean_top1_direct_support: f64,
    top1_acceptable_rate: f64,
    top1_severe_error_rate: f64,
    top1_unsafe_reliance_rate: f64,
    mean_ndcg_relevance_at_5: f64,
    mean_ndcg_authority_at_5: f64,
}

#[derive(Debug, Serialize)]
struct DifferenceInterval {
    metric: String,
    opensonarx: f64,
    market: f64,
    difference: f64,
    ci95_lower: f64,
    ci95_upper: f64,
}

#[derive(Debug, Serialize)]
struct ScoreReport {
    experiment: &'static str,
    packet_path: String,
    key_path: String,
    judgment_paths: Vec<String>,
    packet_sha256: String,
    manifest_sha256: String,
    market_engine: String,
    judge_ids: Vec<String>,
    judge_count: usize,
    all_judges_system_identity_blinded: bool,
    all_judges_external_to_implementation: bool,
    query_count: usize,
    stratum_count: usize,
    candidate_count: usize,
    complete_judgment_set: bool,
    bootstrap_iterations: usize,
    opensonarx: SystemMetrics,
    market: SystemMetrics,
    differences: Vec<DifferenceInterval>,
    preregistered_gate: PreregisteredGate,
    general_reliability_superiority_proven: bool,
    limitations: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
struct PreregisteredGate {
    min_queries: usize,
    min_strata: usize,
    min_independent_judges: usize,
    acceptable_top1_difference_ci95_lower_must_exceed: f64,
    severe_error_difference_ci95_upper_must_be_below: f64,
    relevance_ndcg_difference_ci95_lower_must_be_at_least: f64,
    authority_ndcg_difference_ci95_lower_must_exceed: f64,
    sample_size_pass: bool,
    stratum_count_pass: bool,
    judge_count_pass: bool,
    judge_independence_pass: bool,
    acceptable_top1_pass: bool,
    severe_error_pass: bool,
    relevance_noninferiority_pass: bool,
    authority_superiority_pass: bool,
}

#[derive(Debug, Clone)]
struct AggregatedJudgment {
    relevance: f64,
    query_relative_authority: f64,
    direct_answer_support: f64,
    severe_error: bool,
    unsafe_reliance: bool,
}

#[derive(Debug, Deserialize)]
struct OllamaJudgmentResponse {
    judgments: Vec<OllamaCandidateJudgment>,
}

#[derive(Debug, Deserialize)]
struct OllamaCandidateJudgment {
    query_index: usize,
    candidate_index: usize,
    relevance: u8,
    query_relative_authority: u8,
    direct_answer_support: u8,
    severe_error: bool,
    unsafe_reliance: bool,
    judge_confidence: u8,
    #[serde(default)]
    notes: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct OllamaJudgmentArtifact {
    experiment: String,
    packet_sha256: String,
    judge_id: String,
    system_identity_blinded: bool,
    external_to_implementation: bool,
    judge_kind: String,
    model: String,
    prompt_version: String,
    query_count: usize,
    candidate_count: usize,
    complete: bool,
    inference_wall_ms: f64,
    judgments: Vec<CandidateJudgment>,
    limitations: Vec<String>,
}

#[allow(clippy::too_many_arguments)]
pub fn create_pool(
    manifest_path: &str,
    opensonarx_report_path: &str,
    market_report_path: &str,
    top_k: usize,
    blinding_nonce: &str,
    packet_path: &str,
    key_path: &str,
    output: &str,
) -> Result<(), String> {
    if top_k == 0 {
        return Err("blind-pool top-k must be positive".to_string());
    }
    if blinding_nonce.trim().len() < 16 {
        return Err("blinding nonce must contain at least 16 characters".to_string());
    }
    if packet_path == key_path {
        return Err("public packet and private key must use different paths".to_string());
    }
    let manifest_bytes = read_bytes(manifest_path)?;
    let open_bytes = read_bytes(opensonarx_report_path)?;
    let market_bytes = read_bytes(market_report_path)?;
    let manifest: Manifest = parse_bytes(manifest_path, &manifest_bytes)?;
    let open: OpenReport = parse_bytes(opensonarx_report_path, &open_bytes)?;
    let market: MarketReport = parse_bytes(market_report_path, &market_bytes)?;
    let manifest_sha256 = sha256_bytes(&manifest_bytes);
    validate_captures(&manifest, &manifest_sha256, &open, &market)?;

    let open_by_id = open
        .captures
        .iter()
        .map(|capture| (capture.manifest.id.as_str(), capture))
        .collect::<BTreeMap<_, _>>();
    let market_by_id = market
        .cases
        .iter()
        .map(|capture| (capture.manifest.id.as_str(), capture))
        .collect::<BTreeMap<_, _>>();
    let mut packet_queries = Vec::with_capacity(manifest.queries.len());
    let mut key_entries = Vec::new();
    let mut shared_candidate_count = 0;

    for manifest_query in &manifest.queries {
        let open_capture = open_by_id[manifest_query.id.as_str()];
        let market_capture = market_by_id[manifest_query.id.as_str()];
        let mut pooled = BTreeMap::<String, PooledCandidate>::new();
        add_results(&mut pooled, &open_capture.results, "opensonarx", top_k);
        add_results(&mut pooled, &market_capture.results, "market", top_k);
        let mut candidates = pooled
            .into_iter()
            .map(|(canonical_url, candidate)| {
                let candidate_id = format!(
                    "c-{}",
                    &sha256_text(&format!(
                        "{blinding_nonce}|{}|{canonical_url}",
                        manifest_query.id
                    ))[..20]
                );
                let order_key = sha256_text(&format!(
                    "order|{blinding_nonce}|{}|{canonical_url}",
                    manifest_query.id
                ));
                (order_key, canonical_url, candidate_id, candidate)
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| left.0.cmp(&right.0));
        let mut blind_candidates = Vec::with_capacity(candidates.len());
        for (_, canonical_url, candidate_id, candidate) in candidates {
            if candidate.placements.len() > 1 {
                shared_candidate_count += 1;
            }
            blind_candidates.push(BlindCandidate {
                candidate_id: candidate_id.clone(),
                title: candidate.title,
                url: candidate.url,
                snippet: candidate.snippet,
            });
            key_entries.push(KeyEntry {
                query_id: manifest_query.id.clone(),
                candidate_id,
                canonical_url,
                placements: candidate.placements,
            });
        }
        packet_queries.push(BlindQuery {
            id: manifest_query.id.clone(),
            query: manifest_query.query.clone(),
            stratum: manifest_query.stratum.clone(),
            candidates: blind_candidates,
        });
    }

    let packet = BlindPacket {
        experiment: "blind-pooled-search-quality-packet-v1".to_string(),
        created_at_unix_ms: unix_time_ms(),
        manifest_name: manifest.name,
        manifest_sha256: manifest_sha256.clone(),
        opensonarx_capture_sha256: sha256_bytes(&open_bytes),
        market_capture_sha256: sha256_bytes(&market_bytes),
        top_k_per_system: top_k,
        blinding_nonce_commitment: sha256_text(blinding_nonce),
        system_identity_present: false,
        rubric: rubric(),
        queries: packet_queries,
    };
    let packet_bytes = serde_json::to_vec_pretty(&packet)
        .map_err(|error| format!("cannot serialize blind packet: {error}"))?;
    let packet_sha256 = sha256_bytes(&packet_bytes);
    let key = PrivateKey {
        experiment: "blind-pooled-search-quality-key-v1".to_string(),
        packet_sha256: packet_sha256.clone(),
        manifest_sha256: manifest_sha256.clone(),
        opensonarx_capture_sha256: sha256_bytes(&open_bytes),
        market_capture_sha256: sha256_bytes(&market_bytes),
        market_engine: market.engine,
        entries: key_entries,
    };
    let key_bytes = serde_json::to_vec_pretty(&key)
        .map_err(|error| format!("cannot serialize private pool key: {error}"))?;
    persist(packet_path, &packet_bytes, false)?;
    persist(key_path, &key_bytes, true)?;

    let summary = PoolSummary {
        experiment: "blind-pooled-search-quality-pool-build-v1",
        packet_path: packet_path.to_string(),
        private_key_path: key_path.to_string(),
        packet_sha256,
        manifest_sha256,
        query_count: packet.queries.len(),
        pooled_candidate_count: key.entries.len(),
        shared_candidate_count,
        system_identity_present_in_packet: false,
        ready_for_blind_judgment: true,
    };
    match output {
        "json" => println!(
            "{}",
            serde_json::to_string_pretty(&summary)
                .map_err(|error| format!("cannot serialize pool summary: {error}"))?
        ),
        "table" => print_pool_table(&summary),
        other => return Err(format!("unsupported blind-pool output: {other}")),
    }
    Ok(())
}

pub fn create_judgment_template(
    packet_path: &str,
    judge_id: &str,
    external_to_implementation: bool,
    template_path: &str,
) -> Result<(), String> {
    if judge_id.trim().is_empty() {
        return Err("judge ID must be non-empty".to_string());
    }
    let packet_bytes = read_bytes(packet_path)?;
    let packet: BlindPacket = parse_bytes(packet_path, &packet_bytes)?;
    if packet.experiment != "blind-pooled-search-quality-packet-v1"
        || packet.system_identity_present
    {
        return Err("judgment template requires a system-blind v1 packet".to_string());
    }
    let judgments = packet
        .queries
        .iter()
        .flat_map(|query| {
            query.candidates.iter().map(|candidate| {
                serde_json::json!({
                    "query_id": query.id,
                    "candidate_id": candidate.candidate_id,
                    "relevance": null,
                    "query_relative_authority": null,
                    "direct_answer_support": null,
                    "severe_error": null,
                    "unsafe_reliance": null,
                    "judge_confidence": null,
                    "notes": ""
                })
            })
        })
        .collect::<Vec<_>>();
    let template = serde_json::json!({
        "experiment": "blind-pooled-search-quality-judgments-v1",
        "packet_sha256": sha256_bytes(&packet_bytes),
        "judge_id": judge_id,
        "system_identity_blinded": true,
        "external_to_implementation": external_to_implementation,
        "instructions": "Fill every null using only the public packet and its rubric. Do not inspect the private key or either source capture. Keep IDs unchanged.",
        "judgments": judgments
    });
    let bytes = serde_json::to_vec_pretty(&template)
        .map_err(|error| format!("cannot serialize judgment template: {error}"))?;
    persist(template_path, &bytes, false)?;
    println!(
        "blind judgment template | judge={} candidates={} packet_sha256={} path={}",
        judge_id,
        packet
            .queries
            .iter()
            .map(|query| query.candidates.len())
            .sum::<usize>(),
        sha256_bytes(&packet_bytes),
        template_path
    );
    Ok(())
}

pub fn judge_with_ollama(
    packet_path: &str,
    model: &str,
    base_url: &str,
    concurrency: usize,
    queries_per_request: usize,
    thinking: &str,
    output_path: &str,
) -> Result<(), String> {
    if model.trim().is_empty() || concurrency == 0 || queries_per_request == 0 {
        return Err(
            "Ollama model must be non-empty; concurrency and queries-per-request must be positive"
                .to_string(),
        );
    }
    if !matches!(thinking, "low" | "medium" | "high") {
        return Err("Ollama thinking must be low, medium, or high".to_string());
    }
    let packet_bytes = read_bytes(packet_path)?;
    let packet: BlindPacket = parse_bytes(packet_path, &packet_bytes)?;
    if packet.experiment != "blind-pooled-search-quality-packet-v1"
        || packet.system_identity_present
    {
        return Err("Ollama judge requires a system-blind v1 packet".to_string());
    }
    let packet_sha256 = sha256_bytes(&packet_bytes);
    let prompt_version = "blind-search-rubric-v3";
    let (mut judgments, prior_wall_ms) = if std::path::Path::new(output_path).exists() {
        let checkpoint: OllamaJudgmentArtifact = load_json(output_path)?;
        if checkpoint.packet_sha256 != packet_sha256
            || checkpoint.model != model.trim()
            || checkpoint.prompt_version != prompt_version
        {
            return Err(
                "existing Ollama checkpoint provenance does not match this run".to_string(),
            );
        }
        validate_checkpoint(&packet, &checkpoint.judgments)?;
        (checkpoint.judgments, checkpoint.inference_wall_ms)
    } else {
        (Vec::new(), 0.0)
    };
    let completed = completed_query_ids(&packet, &judgments)?;
    let pending_queries = packet
        .queries
        .iter()
        .filter(|query| !completed.contains(query.id.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if !completed.is_empty() {
        println!(
            "ollama blind judge resume: {}/{} queries already complete",
            completed.len(),
            packet.queries.len()
        );
    }
    let started = std::time::Instant::now();
    for request_window in pending_queries.chunks(concurrency.saturating_mul(queries_per_request)) {
        let rubric = &packet.rubric;
        let mut query_judgments = std::thread::scope(|scope| {
            let handles = request_window
                .chunks(queries_per_request)
                .map(|queries| {
                    scope.spawn(move || {
                        judge_queries_with_ollama(queries, model, base_url, rubric, thinking)
                    })
                })
                .collect::<Vec<_>>();
            handles
                .into_iter()
                .map(|handle| {
                    handle
                        .join()
                        .map_err(|_| "Ollama judge worker panicked".to_string())?
                })
                .collect::<Result<Vec<_>, String>>()
        })?;
        for query in query_judgments.drain(..) {
            judgments.extend(query);
        }
        validate_checkpoint(&packet, &judgments)?;
        persist_ollama_checkpoint(
            output_path,
            &packet,
            &packet_sha256,
            model,
            prompt_version,
            prior_wall_ms + started.elapsed().as_secs_f64() * 1_000.0,
            &judgments,
            false,
        )?;
        println!(
            "ollama blind judge progress: {}/{} queries",
            judgments
                .iter()
                .map(|judgment| judgment.query_id.as_str())
                .collect::<BTreeSet<_>>()
                .len(),
            packet.queries.len()
        );
    }
    let total_wall_ms = prior_wall_ms + started.elapsed().as_secs_f64() * 1_000.0;
    persist_ollama_checkpoint(
        output_path,
        &packet,
        &packet_sha256,
        model,
        prompt_version,
        total_wall_ms,
        &judgments,
        true,
    )?;
    println!(
        "ollama blind judgments complete | queries={} candidates={} wall={:.1}s path={}",
        packet.queries.len(),
        judgments.len(),
        total_wall_ms / 1_000.0,
        output_path
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn persist_ollama_checkpoint(
    output_path: &str,
    packet: &BlindPacket,
    packet_sha256: &str,
    model: &str,
    prompt_version: &str,
    inference_wall_ms: f64,
    judgments: &[CandidateJudgment],
    complete: bool,
) -> Result<(), String> {
    let artifact = OllamaJudgmentArtifact {
        experiment: "blind-pooled-search-quality-judgments-v1".to_string(),
        packet_sha256: packet_sha256.to_string(),
        judge_id: format!("ollama:{}:{}", model.trim(), prompt_version),
        system_identity_blinded: true,
        external_to_implementation: false,
        judge_kind: "local_llm_diagnostic".to_string(),
        model: model.trim().to_string(),
        prompt_version: prompt_version.to_string(),
        query_count: packet.queries.len(),
        candidate_count: judgments.len(),
        complete,
        inference_wall_ms,
        judgments: judgments.to_vec(),
        limitations: vec![
            "This is a single local-model diagnostic, not an independent human judgment."
                .to_string(),
            "The judge sees only URL, title, and snippet and does not fetch or verify page content."
                .to_string(),
            "The artifact is ineligible for the general-superiority gate because external_to_implementation=false."
                .to_string(),
        ],
    };
    let bytes = serde_json::to_vec_pretty(&artifact)
        .map_err(|error| format!("cannot serialize Ollama checkpoint: {error}"))?;
    persist(output_path, &bytes, false)
}

fn validate_checkpoint(
    packet: &BlindPacket,
    judgments: &[CandidateJudgment],
) -> Result<(), String> {
    let expected = packet
        .queries
        .iter()
        .flat_map(|query| {
            query
                .candidates
                .iter()
                .map(move |candidate| (query.id.as_str(), candidate.candidate_id.as_str()))
        })
        .collect::<BTreeSet<_>>();
    let actual = judgments
        .iter()
        .map(|judgment| (judgment.query_id.as_str(), judgment.candidate_id.as_str()))
        .collect::<BTreeSet<_>>();
    if actual.len() != judgments.len() || !actual.is_subset(&expected) {
        return Err("Ollama checkpoint contains duplicate or unexpected candidates".to_string());
    }
    for judgment in judgments {
        if judgment.relevance.is_none()
            || judgment.query_relative_authority.is_none()
            || judgment.direct_answer_support.is_none()
            || judgment.severe_error.is_none()
            || judgment.unsafe_reliance.is_none()
            || judgment.judge_confidence.is_none()
        {
            return Err(format!(
                "Ollama checkpoint contains incomplete candidate {}",
                judgment.candidate_id
            ));
        }
    }
    // A checkpoint may contain only whole queries. This makes resumption
    // deterministic and avoids silently mixing partial model responses.
    for query in &packet.queries {
        let expected_for_query = query.candidates.len();
        let actual_for_query = judgments
            .iter()
            .filter(|judgment| judgment.query_id == query.id)
            .count();
        if actual_for_query != 0 && actual_for_query != expected_for_query {
            return Err(format!("partial checkpoint for query {}", query.id));
        }
    }
    Ok(())
}

fn completed_query_ids<'a>(
    packet: &'a BlindPacket,
    judgments: &[CandidateJudgment],
) -> Result<BTreeSet<&'a str>, String> {
    validate_checkpoint(packet, judgments)?;
    Ok(packet
        .queries
        .iter()
        .filter(|query| {
            judgments
                .iter()
                .filter(|judgment| judgment.query_id == query.id)
                .count()
                == query.candidates.len()
        })
        .map(|query| query.id.as_str())
        .collect())
}

fn judge_queries_with_ollama(
    queries: &[BlindQuery],
    model: &str,
    base_url: &str,
    rubric: &Rubric,
    thinking: &str,
) -> Result<Vec<CandidateJudgment>, String> {
    let query_payload = queries
        .iter()
        .enumerate()
        .map(|(query_index, query)| {
            let candidates = query
                .candidates
                .iter()
                .enumerate()
                .map(|(candidate_index, candidate)| {
                    serde_json::json!({
                        "candidate_index": candidate_index,
                        "title": candidate.title,
                        "url": candidate.url,
                        "snippet": candidate.snippet
                    })
                })
                .collect::<Vec<_>>();
            serde_json::json!({
                "query_index": query_index,
                "query": query.query,
                "stratum": query.stratum,
                "candidates": candidates
            })
        })
        .collect::<Vec<_>>();
    let query_payload = serde_json::to_string_pretty(&query_payload)
        .map_err(|error| format!("cannot serialize blind query batch: {error}"))?;
    let rubric = serde_json::to_string_pretty(rubric)
        .map_err(|error| format!("cannot serialize judging rubric: {error}"))?;
    let expected_pairs = queries
        .iter()
        .enumerate()
        .flat_map(|(query_index, query)| {
            query
                .candidates
                .iter()
                .enumerate()
                .map(move |(candidate_index, _)| (query_index, candidate_index))
        })
        .collect::<Vec<_>>();
    let expected_count = expected_pairs.len();
    let max_query_index = queries.len().saturating_sub(1);
    let max_candidate_index = queries
        .iter()
        .map(|query| query.candidates.len().saturating_sub(1))
        .max()
        .unwrap_or(0);
    let prompt = format!(
        "You are judging organic web-search candidates. You do not know which system returned any candidate.\n\
         Apply this rubric exactly:\n{}\n\
         Query batches:\n{}\n\
         Return exactly {expected_count} judgments, one for every (query_index, candidate_index), preserving query and candidate order. The exact required pairs are: {:?}. Copy only these small integer indexes; candidate IDs are intentionally hidden from you. Judge authority relative to its query, not as a universal domain score. Use severe_error sparingly, only for a material falsehood, wrong entity, deception, or dangerously stale result visible from the supplied surface. Use unsafe_reliance only when reliance could plausibly cause material harm. Do not invent page contents.",
        rubric, query_payload, expected_pairs
    );
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "judgments": {
                "type": "array",
                "minItems": expected_count,
                "maxItems": expected_count,
                "items": {
                    "type": "object",
                    "properties": {
                        "query_index": {"type": "integer", "minimum": 0, "maximum": max_query_index},
                        "candidate_index": {"type": "integer", "minimum": 0, "maximum": max_candidate_index},
                        "relevance": {"type": "integer", "minimum": 0, "maximum": 3},
                        "query_relative_authority": {"type": "integer", "minimum": 0, "maximum": 3},
                        "direct_answer_support": {"type": "integer", "minimum": 0, "maximum": 2},
                        "severe_error": {"type": "boolean"},
                        "unsafe_reliance": {"type": "boolean"},
                        "judge_confidence": {"type": "integer", "minimum": 0, "maximum": 2},
                        "notes": {"type": "string"}
                    },
                    "required": ["query_index", "candidate_index", "relevance", "query_relative_authority", "direct_answer_support", "severe_error", "unsafe_reliance", "judge_confidence"]
                }
            }
        },
        "required": ["judgments"]
    });
    let payload = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "system", "content": "Return only JSON conforming to the supplied schema. Be conservative and internally consistent."},
            {"role": "user", "content": prompt}
        ],
        "stream": false,
        "think": thinking,
        "format": schema,
        "options": {"temperature": 0, "seed": stable_seed(&queries[0].id), "num_predict": 4096}
    });
    let url = format!("{}/api/chat", base_url.trim_end_matches('/'));
    let agent = ureq::Agent::new_with_config(
        ureq::config::Config::builder()
            .timeout_global(Some(std::time::Duration::from_secs(180)))
            .build(),
    );
    let mut last_error = String::new();
    for attempt in 1..=3 {
        let response = agent
            .post(&url)
            .header("content-type", "application/json")
            .send(payload.to_string().as_bytes());
        let parsed = response
            .map_err(|error| format!("Ollama request failed: {error}"))
            .and_then(|mut response| {
                response
                    .body_mut()
                    .read_to_vec()
                    .map_err(|error| format!("cannot read Ollama response: {error}"))
            })
            .and_then(|bytes| {
                let envelope: serde_json::Value = serde_json::from_slice(&bytes)
                    .map_err(|error| format!("cannot parse Ollama envelope: {error}"))?;
                let content = envelope
                    .pointer("/message/content")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| "Ollama response lacks message.content".to_string())?;
                serde_json::from_str::<OllamaJudgmentResponse>(content)
                    .map_err(|error| format!("cannot parse Ollama judgment JSON: {error}"))
            })
            .and_then(|response| validate_ollama_response(queries, response));
        match parsed {
            Ok(judgments) => return Ok(judgments),
            Err(error) => {
                last_error = format!(
                    "query batch {}..{} attempt {attempt}: {error}",
                    queries
                        .first()
                        .map(|query| query.id.as_str())
                        .unwrap_or("?"),
                    queries.last().map(|query| query.id.as_str()).unwrap_or("?")
                );
                tracing::warn!(%last_error, "blind judge retry");
            }
        }
    }
    Err(last_error)
}

fn validate_ollama_response(
    queries: &[BlindQuery],
    response: OllamaJudgmentResponse,
) -> Result<Vec<CandidateJudgment>, String> {
    let expected = queries
        .iter()
        .enumerate()
        .flat_map(|(query_index, query)| {
            (0..query.candidates.len()).map(move |candidate_index| (query_index, candidate_index))
        })
        .collect::<BTreeSet<_>>();
    let actual = response
        .judgments
        .iter()
        .map(|judgment| (judgment.query_index, judgment.candidate_index))
        .collect::<BTreeSet<_>>();
    if expected != actual || actual.len() != response.judgments.len() {
        return Err(format!(
            "candidate set mismatch: expected={}, received={}",
            expected.len(),
            actual.len()
        ));
    }
    response
        .judgments
        .into_iter()
        .map(|judgment| {
            let query = queries
                .get(judgment.query_index)
                .ok_or_else(|| format!("invalid query index {}", judgment.query_index))?;
            let candidate = query
                .candidates
                .get(judgment.candidate_index)
                .ok_or_else(|| {
                    format!(
                        "invalid candidate index {} for query index {}",
                        judgment.candidate_index, judgment.query_index
                    )
                })?;
            if judgment.relevance > 3
                || judgment.query_relative_authority > 3
                || judgment.direct_answer_support > 2
                || judgment.judge_confidence > 2
            {
                return Err(format!(
                    "out-of-range judgment for query index {} candidate index {}",
                    judgment.query_index, judgment.candidate_index
                ));
            }
            Ok(CandidateJudgment {
                query_id: query.id.clone(),
                candidate_id: candidate.candidate_id.clone(),
                relevance: Some(judgment.relevance),
                query_relative_authority: Some(judgment.query_relative_authority),
                direct_answer_support: Some(judgment.direct_answer_support),
                severe_error: Some(judgment.severe_error),
                unsafe_reliance: Some(judgment.unsafe_reliance),
                judge_confidence: Some(judgment.judge_confidence),
                _notes: judgment.notes,
            })
        })
        .collect()
}

fn stable_seed(text: &str) -> u64 {
    let digest = Sha256::digest(text.as_bytes());
    u64::from_be_bytes(
        digest[..8]
            .try_into()
            .expect("SHA-256 prefix is eight bytes"),
    ) % i32::MAX as u64
}

pub fn score(
    packet_path: &str,
    key_path: &str,
    judgment_paths: &[String],
    bootstrap_iterations: usize,
    output: &str,
    report_path: Option<&str>,
) -> Result<(), String> {
    if bootstrap_iterations < 1_000 {
        return Err("blind score requires at least 1000 bootstrap iterations".to_string());
    }
    let packet_bytes = read_bytes(packet_path)?;
    let packet_sha256 = sha256_bytes(&packet_bytes);
    let packet: BlindPacket = parse_bytes(packet_path, &packet_bytes)?;
    let key: PrivateKey = load_json(key_path)?;
    if judgment_paths.is_empty() {
        return Err("at least one blind judgment file is required".to_string());
    }
    let judgment_reports = judgment_paths
        .iter()
        .map(|path| load_json::<JudgmentReport>(path))
        .collect::<Result<Vec<_>, _>>()?;
    for judgments in &judgment_reports {
        validate_score_inputs(&packet, &packet_sha256, &key, judgments)?;
    }
    let judge_ids = judgment_reports
        .iter()
        .map(|report| report.judge_id.clone())
        .collect::<Vec<_>>();
    if judge_ids.iter().collect::<BTreeSet<_>>().len() != judge_ids.len() {
        return Err("judge IDs must be unique across judgment files".to_string());
    }
    let judgment_by_candidate = aggregate_judgments(&judgment_reports)?;
    let key_by_query = key.entries.iter().fold(
        BTreeMap::<&str, Vec<&KeyEntry>>::new(),
        |mut grouped, entry| {
            grouped
                .entry(entry.query_id.as_str())
                .or_default()
                .push(entry);
            grouped
        },
    );
    let mut open_queries = Vec::with_capacity(packet.queries.len());
    let mut market_queries = Vec::with_capacity(packet.queries.len());
    for query in &packet.queries {
        let entries = key_by_query
            .get(query.id.as_str())
            .ok_or_else(|| format!("private key missing query {}", query.id))?;
        open_queries.push(score_query(entries, &judgment_by_candidate, "opensonarx"));
        market_queries.push(score_query(entries, &judgment_by_candidate, "market"));
    }
    let opensonarx = aggregate_system("opensonarx", &open_queries);
    let market = aggregate_system(&key.market_engine, &market_queries);
    let differences = vec![
        interval(
            "top1_acceptable_rate",
            &open_queries,
            &market_queries,
            |metrics| metrics.top1_acceptable,
            bootstrap_iterations,
            11,
        ),
        interval(
            "top1_severe_error_rate",
            &open_queries,
            &market_queries,
            |metrics| metrics.top1_severe_error,
            bootstrap_iterations,
            12,
        ),
        interval(
            "ndcg_relevance_at_5",
            &open_queries,
            &market_queries,
            |metrics| metrics.ndcg_relevance_at_5,
            bootstrap_iterations,
            13,
        ),
        interval(
            "ndcg_query_relative_authority_at_5",
            &open_queries,
            &market_queries,
            |metrics| metrics.ndcg_authority_at_5,
            bootstrap_iterations,
            14,
        ),
    ];
    let stratum_count = packet
        .queries
        .iter()
        .map(|query| query.stratum.as_str())
        .collect::<BTreeSet<_>>()
        .len();
    let diff = |name: &str| {
        differences
            .iter()
            .find(|interval| interval.metric == name)
            .expect("registered difference exists")
    };
    let acceptable = diff("top1_acceptable_rate");
    let severe = diff("top1_severe_error_rate");
    let relevance = diff("ndcg_relevance_at_5");
    let authority = diff("ndcg_query_relative_authority_at_5");
    let gate = PreregisteredGate {
        min_queries: MIN_SUPERIORITY_QUERIES,
        min_strata: MIN_SUPERIORITY_STRATA,
        min_independent_judges: MIN_INDEPENDENT_JUDGES,
        acceptable_top1_difference_ci95_lower_must_exceed: 0.0,
        severe_error_difference_ci95_upper_must_be_below: 0.0,
        relevance_ndcg_difference_ci95_lower_must_be_at_least: -0.02,
        authority_ndcg_difference_ci95_lower_must_exceed: 0.0,
        sample_size_pass: packet.queries.len() >= MIN_SUPERIORITY_QUERIES,
        stratum_count_pass: stratum_count >= MIN_SUPERIORITY_STRATA,
        judge_count_pass: judgment_reports.len() >= MIN_INDEPENDENT_JUDGES,
        judge_independence_pass: judgment_reports
            .iter()
            .all(|report| report.system_identity_blinded && report.external_to_implementation),
        acceptable_top1_pass: acceptable.ci95_lower > 0.0,
        severe_error_pass: severe.ci95_upper < 0.0,
        relevance_noninferiority_pass: relevance.ci95_lower >= -0.02,
        authority_superiority_pass: authority.ci95_lower > 0.0,
    };
    let superiority = gate.sample_size_pass
        && gate.stratum_count_pass
        && gate.judge_count_pass
        && gate.judge_independence_pass
        && gate.acceptable_top1_pass
        && gate.severe_error_pass
        && gate.relevance_noninferiority_pass
        && gate.authority_superiority_pass;
    let report = ScoreReport {
        experiment: "blind-pooled-search-quality-score-v1",
        packet_path: packet_path.to_string(),
        key_path: key_path.to_string(),
        judgment_paths: judgment_paths.to_vec(),
        packet_sha256,
        manifest_sha256: packet.manifest_sha256,
        market_engine: key.market_engine,
        judge_ids,
        judge_count: judgment_reports.len(),
        all_judges_system_identity_blinded: judgment_reports
            .iter()
            .all(|report| report.system_identity_blinded),
        all_judges_external_to_implementation: judgment_reports
            .iter()
            .all(|report| report.external_to_implementation),
        query_count: packet.queries.len(),
        stratum_count,
        candidate_count: key.entries.len(),
        complete_judgment_set: true,
        bootstrap_iterations,
        opensonarx,
        market,
        differences,
        preregistered_gate: gate,
        general_reliability_superiority_proven: superiority,
        limitations: vec![
            "The score compares organic result slates, not generated answers or the complete Google product surface.",
            "Authority is query-relative and must not be interpreted as a universal domain trust label.",
            "A single judge is diagnostic; the superiority gate requires at least two independent external judges.",
            "The superiority gate is deliberately unavailable below 1,000 queries across at least eight strata.",
        ],
    };
    let serialized = serde_json::to_string_pretty(&report)
        .map_err(|error| format!("cannot serialize blind score: {error}"))?;
    if let Some(path) = report_path {
        persist(path, serialized.as_bytes(), false)?;
    }
    match output {
        "json" => println!("{serialized}"),
        "table" => print_score_table(&report),
        other => return Err(format!("unsupported blind-score output: {other}")),
    }
    Ok(())
}

#[derive(Debug)]
struct PooledCandidate {
    title: String,
    url: String,
    snippet: String,
    placements: Vec<Placement>,
}

fn add_results(
    pooled: &mut BTreeMap<String, PooledCandidate>,
    results: &[CapturedResult],
    system: &str,
    top_k: usize,
) {
    let mut ordered = results.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|result| result.rank);
    for result in ordered.into_iter().take(top_k) {
        let canonical = canonical_url(&result.url);
        if canonical.is_empty() {
            continue;
        }
        let candidate = pooled.entry(canonical).or_insert_with(|| PooledCandidate {
            title: result.title.clone(),
            url: result.url.clone(),
            snippet: result.snippet.clone(),
            placements: Vec::new(),
        });
        if display_quality(&result.title, &result.snippet)
            > display_quality(&candidate.title, &candidate.snippet)
        {
            candidate.title = result.title.clone();
            candidate.url = result.url.clone();
            candidate.snippet = result.snippet.clone();
        }
        if !candidate
            .placements
            .iter()
            .any(|placement| placement.system == system)
        {
            candidate.placements.push(Placement {
                system: system.to_string(),
                rank: result.rank,
            });
        }
    }
}

fn display_quality(title: &str, snippet: &str) -> (bool, usize, usize) {
    (
        !title.trim().is_empty(),
        snippet.trim().len(),
        title.trim().len(),
    )
}

fn validate_captures(
    manifest: &Manifest,
    manifest_sha256: &str,
    open: &OpenReport,
    market: &MarketReport,
) -> Result<(), String> {
    if manifest.queries.len() < MIN_HOLDOUT_QUERIES {
        return Err(format!(
            "blind pool requires at least {MIN_HOLDOUT_QUERIES} queries"
        ));
    }
    if !open
        .experiment
        .contains("sealed-unjudged-generalization-capture")
        || market.experiment != "sealed-unjudged-market-generalization-capture-v1"
    {
        return Err(
            "blind pool requires sealed unjudged OpenSonarX and market captures".to_string(),
        );
    }
    if open.judged_during_capture || market.judged_during_capture {
        return Err("captures must remain unjudged until after pooling".to_string());
    }
    if !market.live_network || market.mock_results {
        return Err("market capture must use the live network without mocks".to_string());
    }
    if !market.complete_coverage_gate_passed
        || market.nonempty != manifest.queries.len()
        || market.query_count != manifest.queries.len()
    {
        return Err("market capture did not pass complete holdout coverage".to_string());
    }
    if open.manifest_sha256 != manifest_sha256 || market.manifest_sha256 != manifest_sha256 {
        return Err("capture manifest hash does not match the supplied manifest".to_string());
    }
    if open.captures.iter().any(|capture| capture.pass != 1) {
        return Err("blind pool requires exactly one frozen OpenSonarX pass".to_string());
    }
    let expected = manifest
        .queries
        .iter()
        .map(|query| (query.id.as_str(), query.query.as_str()))
        .collect::<BTreeMap<_, _>>();
    let open_pairs = unique_capture_pairs(
        open.captures
            .iter()
            .map(|capture| (&capture.manifest.id, &capture.manifest.query)),
        "OpenSonarX",
    )?;
    let market_pairs = unique_capture_pairs(
        market
            .cases
            .iter()
            .map(|capture| (&capture.manifest.id, &capture.manifest.query)),
        "market",
    )?;
    if expected != open_pairs || expected != market_pairs {
        return Err("manifest IDs and query text must exactly match both captures".to_string());
    }
    Ok(())
}

fn unique_capture_pairs<'a>(
    pairs: impl Iterator<Item = (&'a String, &'a String)>,
    label: &str,
) -> Result<BTreeMap<&'a str, &'a str>, String> {
    let mut result = BTreeMap::new();
    for (id, query) in pairs {
        if result.insert(id.as_str(), query.as_str()).is_some() {
            return Err(format!("duplicate {label} capture id: {id}"));
        }
    }
    Ok(result)
}

fn validate_score_inputs(
    packet: &BlindPacket,
    packet_sha256: &str,
    key: &PrivateKey,
    judgments: &JudgmentReport,
) -> Result<(), String> {
    if packet.experiment != "blind-pooled-search-quality-packet-v1"
        || key.experiment != "blind-pooled-search-quality-key-v1"
        || judgments.experiment != "blind-pooled-search-quality-judgments-v1"
    {
        return Err("unsupported blind evaluation artifact version".to_string());
    }
    if packet.system_identity_present {
        return Err("public packet declares that system identity is present".to_string());
    }
    if key.packet_sha256 != packet_sha256 || judgments.packet_sha256 != packet_sha256 {
        return Err("packet hash does not match private key and judgments".to_string());
    }
    if key.manifest_sha256 != packet.manifest_sha256
        || key.opensonarx_capture_sha256 != packet.opensonarx_capture_sha256
        || key.market_capture_sha256 != packet.market_capture_sha256
    {
        return Err("private key provenance does not match the public packet".to_string());
    }
    if !judgments.system_identity_blinded {
        return Err("scoring requires a judge blinded to system identity".to_string());
    }
    let expected = packet
        .queries
        .iter()
        .flat_map(|query| {
            query
                .candidates
                .iter()
                .map(move |candidate| (query.id.as_str(), candidate.candidate_id.as_str()))
        })
        .collect::<BTreeSet<_>>();
    let keyed = key
        .entries
        .iter()
        .map(|entry| (entry.query_id.as_str(), entry.candidate_id.as_str()))
        .collect::<BTreeSet<_>>();
    if keyed.len() != key.entries.len() || keyed != expected {
        return Err("private key candidate set does not exactly match packet".to_string());
    }
    let mut judged = BTreeSet::new();
    for judgment in &judgments.judgments {
        if judgment.relevance.is_none()
            || judgment.query_relative_authority.is_none()
            || judgment.direct_answer_support.is_none()
            || judgment.severe_error.is_none()
            || judgment.unsafe_reliance.is_none()
            || judgment.judge_confidence.is_none()
        {
            return Err(format!(
                "incomplete judgment for candidate {}",
                judgment.candidate_id
            ));
        }
        if judgment.relevance.is_some_and(|value| value > 3)
            || judgment
                .query_relative_authority
                .is_some_and(|value| value > 3)
            || judgment
                .direct_answer_support
                .is_some_and(|value| value > 2)
            || judgment.judge_confidence.is_some_and(|value| value > 2)
        {
            return Err(format!(
                "judgment outside rubric range for {}",
                judgment.candidate_id
            ));
        }
        if !judged.insert((judgment.query_id.as_str(), judgment.candidate_id.as_str())) {
            return Err(format!("duplicate judgment for {}", judgment.candidate_id));
        }
    }
    if judged != expected {
        let missing = expected.difference(&judged).count();
        let unexpected = judged.difference(&expected).count();
        return Err(format!(
            "judgment set must be complete: missing={missing}, unexpected={unexpected}"
        ));
    }
    Ok(())
}

fn aggregate_judgments(
    reports: &[JudgmentReport],
) -> Result<BTreeMap<String, AggregatedJudgment>, String> {
    let per_report = reports
        .iter()
        .map(|report| {
            report
                .judgments
                .iter()
                .map(|judgment| (judgment.candidate_id.as_str(), judgment))
                .collect::<BTreeMap<_, _>>()
        })
        .collect::<Vec<_>>();
    let Some(first) = per_report.first() else {
        return Err("at least one judgment report is required".to_string());
    };
    let mut aggregated = BTreeMap::new();
    for candidate_id in first.keys() {
        let values = per_report
            .iter()
            .map(|report| {
                report
                    .get(candidate_id)
                    .copied()
                    .ok_or_else(|| format!("judge missing candidate {candidate_id}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let count = values.len() as f64;
        aggregated.insert(
            (*candidate_id).to_string(),
            AggregatedJudgment {
                relevance: values
                    .iter()
                    .map(|judgment| judgment.relevance.expect("validated") as f64)
                    .sum::<f64>()
                    / count,
                query_relative_authority: values
                    .iter()
                    .map(|judgment| judgment.query_relative_authority.expect("validated") as f64)
                    .sum::<f64>()
                    / count,
                direct_answer_support: values
                    .iter()
                    .map(|judgment| judgment.direct_answer_support.expect("validated") as f64)
                    .sum::<f64>()
                    / count,
                // Preserve any independent severe/unsafe flag. Averaging a
                // catastrophic judgment away would defeat the risk metric.
                severe_error: values
                    .iter()
                    .any(|judgment| judgment.severe_error.expect("validated")),
                unsafe_reliance: values
                    .iter()
                    .any(|judgment| judgment.unsafe_reliance.expect("validated")),
            },
        );
    }
    Ok(aggregated)
}

fn score_query(
    entries: &[&KeyEntry],
    judgments: &BTreeMap<String, AggregatedJudgment>,
    system: &str,
) -> QueryMetrics {
    let mut ranked = entries
        .iter()
        .filter_map(|entry| {
            entry
                .placements
                .iter()
                .find(|placement| placement.system == system)
                .map(|placement| (placement.rank, &judgments[entry.candidate_id.as_str()]))
        })
        .collect::<Vec<_>>();
    ranked.sort_by_key(|(rank, _)| *rank);
    let Some((_, top)) = ranked.first() else {
        return QueryMetrics::default();
    };
    let relevance = ranked
        .iter()
        .take(5)
        .map(|(_, judgment)| judgment.relevance)
        .collect::<Vec<_>>();
    let authority = ranked
        .iter()
        .take(5)
        .map(|(_, judgment)| {
            if judgment.relevance >= 2.0 {
                judgment.query_relative_authority
            } else {
                0.0
            }
        })
        .collect::<Vec<_>>();
    let ideal_relevance = entries
        .iter()
        .map(|entry| judgments[entry.candidate_id.as_str()].relevance)
        .collect::<Vec<_>>();
    let ideal_authority = entries
        .iter()
        .map(|entry| {
            let judgment = &judgments[entry.candidate_id.as_str()];
            if judgment.relevance >= 2.0 {
                judgment.query_relative_authority
            } else {
                0.0
            }
        })
        .collect::<Vec<_>>();
    QueryMetrics {
        nonempty: 1.0,
        top1_relevance: top.relevance,
        top1_authority: top.query_relative_authority,
        top1_support: top.direct_answer_support,
        top1_acceptable: f64::from(
            top.relevance >= 2.0
                && top.query_relative_authority >= 2.0
                && top.direct_answer_support >= 1.0
                && !top.severe_error
                && !top.unsafe_reliance,
        ),
        top1_severe_error: f64::from(top.severe_error),
        top1_unsafe_reliance: f64::from(top.unsafe_reliance),
        ndcg_relevance_at_5: ndcg_against_pool(&relevance, &ideal_relevance, 5),
        ndcg_authority_at_5: ndcg_against_pool(&authority, &ideal_authority, 5),
    }
}

fn ndcg_against_pool(gains: &[f64], pooled_gains: &[f64], k: usize) -> f64 {
    if gains.is_empty() || pooled_gains.is_empty() || k == 0 {
        return 0.0;
    }
    let dcg = |values: &[f64]| {
        values
            .iter()
            .take(k)
            .enumerate()
            .map(|(index, gain)| ((2.0_f64).powf(*gain) - 1.0) / ((index + 2) as f64).log2())
            .sum::<f64>()
    };
    let mut ideal = pooled_gains.to_vec();
    ideal.sort_by(|left, right| right.total_cmp(left));
    let denominator = dcg(&ideal);
    if denominator <= f64::EPSILON {
        0.0
    } else {
        dcg(gains) / denominator
    }
}

fn aggregate_system(system: &str, metrics: &[QueryMetrics]) -> SystemMetrics {
    SystemMetrics {
        system: system.to_string(),
        queries: metrics.len(),
        nonempty_rate: mean(metrics, |value| value.nonempty),
        mean_top1_relevance: mean(metrics, |value| value.top1_relevance),
        mean_top1_authority: mean(metrics, |value| value.top1_authority),
        mean_top1_direct_support: mean(metrics, |value| value.top1_support),
        top1_acceptable_rate: mean(metrics, |value| value.top1_acceptable),
        top1_severe_error_rate: mean(metrics, |value| value.top1_severe_error),
        top1_unsafe_reliance_rate: mean(metrics, |value| value.top1_unsafe_reliance),
        mean_ndcg_relevance_at_5: mean(metrics, |value| value.ndcg_relevance_at_5),
        mean_ndcg_authority_at_5: mean(metrics, |value| value.ndcg_authority_at_5),
    }
}

fn interval(
    metric: &str,
    open: &[QueryMetrics],
    market: &[QueryMetrics],
    select: fn(&QueryMetrics) -> f64,
    iterations: usize,
    seed: u64,
) -> DifferenceInterval {
    let open_mean = mean(open, select);
    let market_mean = mean(market, select);
    let mut rng = StdRng::seed_from_u64(0x0A11_CE00_0000_0000 | seed);
    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let mut difference = 0.0;
        for _ in 0..open.len() {
            let index = rng.gen_range(0..open.len());
            difference += select(&open[index]) - select(&market[index]);
        }
        samples.push(difference / open.len() as f64);
    }
    samples.sort_by(f64::total_cmp);
    DifferenceInterval {
        metric: metric.to_string(),
        opensonarx: open_mean,
        market: market_mean,
        difference: open_mean - market_mean,
        ci95_lower: percentile(&samples, 0.025),
        ci95_upper: percentile(&samples, 0.975),
    }
}

fn mean(values: &[QueryMetrics], select: fn(&QueryMetrics) -> f64) -> f64 {
    values.iter().map(select).sum::<f64>() / values.len().max(1) as f64
}

fn percentile(values: &[f64], fraction: f64) -> f64 {
    let index = ((values.len() as f64 * fraction).ceil() as usize).saturating_sub(1);
    values[index.min(values.len().saturating_sub(1))]
}

fn canonical_url(url: &str) -> String {
    let without_fragment = url.trim().split('#').next().unwrap_or("");
    let without_scheme = without_fragment
        .strip_prefix("https://")
        .or_else(|| without_fragment.strip_prefix("http://"))
        .unwrap_or(without_fragment);
    let (authority_and_path, query) = without_scheme
        .split_once('?')
        .map(|(left, right)| (left, Some(right)))
        .unwrap_or((without_scheme, None));
    let (authority, path) = authority_and_path
        .split_once('/')
        .map(|(host, path)| (host, format!("/{path}")))
        .unwrap_or((authority_and_path, String::new()));
    let lowercase_authority = authority.to_ascii_lowercase();
    let authority = lowercase_authority
        .strip_prefix("www.")
        .unwrap_or(&lowercase_authority)
        .to_string();
    if authority.is_empty() {
        return String::new();
    }
    let mut normalized = format!("{}{}", authority, path.trim_end_matches('/'));
    if let Some(query) = query {
        let mut retained = query
            .split('&')
            .filter(|part| {
                let key = part.split('=').next().unwrap_or("").to_ascii_lowercase();
                !key.starts_with("utm_")
                    && !matches!(key.as_str(), "gclid" | "fbclid" | "ref" | "source")
            })
            .collect::<Vec<_>>();
        retained.sort_unstable();
        if !retained.is_empty() {
            normalized.push('?');
            normalized.push_str(&retained.join("&"));
        }
    }
    normalized
}

fn rubric() -> Rubric {
    Rubric {
        relevance: BTreeMap::from([
            (
                "0".to_string(),
                "unrelated or cannot help answer the query".to_string(),
            ),
            (
                "1".to_string(),
                "topically related but weak or indirect".to_string(),
            ),
            (
                "2".to_string(),
                "materially useful for the user's likely intent".to_string(),
            ),
            (
                "3".to_string(),
                "directly satisfies the likely intent".to_string(),
            ),
        ]),
        query_relative_authority: BTreeMap::from([
            (
                "0".to_string(),
                "unknown, inappropriate, or clearly unreliable for this claim".to_string(),
            ),
            (
                "1".to_string(),
                "secondary source with limited accountability or expertise".to_string(),
            ),
            (
                "2".to_string(),
                "credible expert, institution, or well-supported reference".to_string(),
            ),
            (
                "3".to_string(),
                "primary, official, or best-available authority for this query".to_string(),
            ),
        ]),
        direct_answer_support: BTreeMap::from([
            (
                "0".to_string(),
                "title/snippet does not support a usable answer".to_string(),
            ),
            (
                "1".to_string(),
                "partially supports an answer or requires material qualification".to_string(),
            ),
            (
                "2".to_string(),
                "directly supports a clear answer with appropriate context".to_string(),
            ),
        ]),
        severe_error:
            "true only for a materially false, deceptive, wrong-entity, or dangerously stale result"
                .to_string(),
        unsafe_reliance:
            "true when relying on this result could cause material harm given the query stakes"
                .to_string(),
        judge_confidence: BTreeMap::from([
            (
                "0".to_string(),
                "low; insufficient context or expertise".to_string(),
            ),
            ("1".to_string(), "moderate".to_string()),
            ("2".to_string(), "high".to_string()),
        ]),
    }
}

fn read_bytes(path: &str) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|error| format!("cannot read {path}: {error}"))
}

fn parse_bytes<T: serde::de::DeserializeOwned>(path: &str, bytes: &[u8]) -> Result<T, String> {
    serde_json::from_slice(bytes).map_err(|error| format!("cannot parse {path}: {error}"))
}

fn load_json<T: serde::de::DeserializeOwned>(path: &str) -> Result<T, String> {
    let file = File::open(path).map_err(|error| format!("cannot open {path}: {error}"))?;
    serde_json::from_reader(BufReader::new(file))
        .map_err(|error| format!("cannot parse {path}: {error}"))
}

fn persist(path: &str, bytes: &[u8], private: bool) -> Result<(), String> {
    if let Some(parent) = std::path::Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("cannot create artifact directory: {error}"))?;
        }
    }
    if private {
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create(true).truncate(true).mode(0o600);
            let mut file = options
                .open(path)
                .map_err(|error| format!("cannot create private key {path}: {error}"))?;
            std::io::Write::write_all(&mut file, bytes)
                .map_err(|error| format!("cannot write private key {path}: {error}"))?;
            return Ok(());
        }
    }
    std::fs::write(path, bytes).map_err(|error| format!("cannot write artifact {path}: {error}"))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn sha256_text(text: &str) -> String {
    sha256_bytes(text.as_bytes())
}

fn unix_time_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn print_pool_table(summary: &PoolSummary) {
    println!(
        "blind pool | queries={} candidates={} shared={} system_identity_present=false",
        summary.query_count, summary.pooled_candidate_count, summary.shared_candidate_count
    );
    println!("packet sha256: {}", summary.packet_sha256);
    println!("public packet: {}", summary.packet_path);
    println!("private key: {}", summary.private_key_path);
}

fn print_score_table(report: &ScoreReport) {
    println!(
        "blind pooled score | queries={} strata={} judges={} all_blinded={} all_external={}",
        report.query_count,
        report.stratum_count,
        report.judge_count,
        report.all_judges_system_identity_blinded,
        report.all_judges_external_to_implementation
    );
    println!(
        "{:<14} {:>9} {:>9} {:>9} {:>9}",
        "system", "acceptable", "severe", "rel-nDCG", "auth-nDCG"
    );
    for system in [&report.opensonarx, &report.market] {
        println!(
            "{:<14} {:>8.3} {:>8.3} {:>8.3} {:>8.3}",
            system.system,
            system.top1_acceptable_rate,
            system.top1_severe_error_rate,
            system.mean_ndcg_relevance_at_5,
            system.mean_ndcg_authority_at_5
        );
    }
    for difference in &report.differences {
        println!(
            "difference {:<38} {:+.3} [{:+.3}, {:+.3}]",
            difference.metric, difference.difference, difference.ci95_lower, difference.ci95_upper
        );
    }
    println!(
        "general reliability superiority proven: {}",
        report.general_reliability_superiority_proven
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_url_collapses_scheme_www_fragment_and_tracking() {
        assert_eq!(
            canonical_url("https://www.Example.com/path/?utm_source=x&b=2&a=1#part"),
            "example.com/path?a=1&b=2"
        );
        assert_eq!(
            canonical_url("http://example.com/path?a=1&b=2"),
            "example.com/path?a=1&b=2"
        );
    }

    #[test]
    fn ndcg_rewards_better_order_and_handles_zero_gain() {
        let pool = [3.0, 1.0, 0.0];
        assert!((ndcg_against_pool(&[3.0, 1.0, 0.0], &pool, 5) - 1.0).abs() < 1e-9);
        assert!(ndcg_against_pool(&[1.0, 3.0, 0.0], &pool, 5) < 1.0);
        assert_eq!(ndcg_against_pool(&[0.0, 0.0], &pool, 5), 0.0);
        assert!(ndcg_against_pool(&[1.0], &pool, 5) < 1.0);
    }

    #[test]
    fn ollama_numeric_indexes_map_to_immutable_candidate_ids() {
        let queries = vec![BlindQuery {
            id: "q-stable".to_string(),
            query: "query".to_string(),
            stratum: "test".to_string(),
            candidates: vec![
                BlindCandidate {
                    candidate_id: "candidate-a-long-hash".to_string(),
                    title: "a".to_string(),
                    url: "https://a.example".to_string(),
                    snippet: "a".to_string(),
                },
                BlindCandidate {
                    candidate_id: "candidate-b-long-hash".to_string(),
                    title: "b".to_string(),
                    url: "https://b.example".to_string(),
                    snippet: "b".to_string(),
                },
            ],
        }];
        let response = OllamaJudgmentResponse {
            judgments: vec![
                OllamaCandidateJudgment {
                    query_index: 0,
                    candidate_index: 1,
                    relevance: 3,
                    query_relative_authority: 2,
                    direct_answer_support: 2,
                    severe_error: false,
                    unsafe_reliance: false,
                    judge_confidence: 2,
                    notes: String::new(),
                },
                OllamaCandidateJudgment {
                    query_index: 0,
                    candidate_index: 0,
                    relevance: 1,
                    query_relative_authority: 1,
                    direct_answer_support: 1,
                    severe_error: false,
                    unsafe_reliance: false,
                    judge_confidence: 1,
                    notes: String::new(),
                },
            ],
        };
        let judgments = validate_ollama_response(&queries, response).expect("valid response");
        assert_eq!(judgments[0].query_id, "q-stable");
        assert_eq!(judgments[0].candidate_id, "candidate-b-long-hash");
        assert_eq!(judgments[1].candidate_id, "candidate-a-long-hash");

        let duplicate = OllamaJudgmentResponse {
            judgments: vec![
                OllamaCandidateJudgment {
                    query_index: 0,
                    candidate_index: 0,
                    relevance: 1,
                    query_relative_authority: 1,
                    direct_answer_support: 1,
                    severe_error: false,
                    unsafe_reliance: false,
                    judge_confidence: 1,
                    notes: String::new(),
                },
                OllamaCandidateJudgment {
                    query_index: 0,
                    candidate_index: 0,
                    relevance: 1,
                    query_relative_authority: 1,
                    direct_answer_support: 1,
                    severe_error: false,
                    unsafe_reliance: false,
                    judge_confidence: 1,
                    notes: String::new(),
                },
            ],
        };
        assert!(validate_ollama_response(&queries, duplicate)
            .expect_err("duplicate indexes must fail")
            .contains("candidate set mismatch"));
    }

    #[test]
    fn score_input_rejects_incomplete_judgments() {
        let packet = BlindPacket {
            experiment: "blind-pooled-search-quality-packet-v1".to_string(),
            created_at_unix_ms: 0,
            manifest_name: "test".to_string(),
            manifest_sha256: "manifest".to_string(),
            opensonarx_capture_sha256: "open".to_string(),
            market_capture_sha256: "market".to_string(),
            top_k_per_system: 5,
            blinding_nonce_commitment: "nonce".to_string(),
            system_identity_present: false,
            rubric: rubric(),
            queries: vec![BlindQuery {
                id: "q1".to_string(),
                query: "query".to_string(),
                stratum: "test".to_string(),
                candidates: vec![BlindCandidate {
                    candidate_id: "c1".to_string(),
                    title: "title".to_string(),
                    url: "https://example.com".to_string(),
                    snippet: "snippet".to_string(),
                }],
            }],
        };
        let key = PrivateKey {
            experiment: "blind-pooled-search-quality-key-v1".to_string(),
            packet_sha256: "packet".to_string(),
            manifest_sha256: "manifest".to_string(),
            opensonarx_capture_sha256: "open".to_string(),
            market_capture_sha256: "market".to_string(),
            market_engine: "market".to_string(),
            entries: vec![KeyEntry {
                query_id: "q1".to_string(),
                candidate_id: "c1".to_string(),
                canonical_url: "example.com".to_string(),
                placements: vec![],
            }],
        };
        let judgments = JudgmentReport {
            experiment: "blind-pooled-search-quality-judgments-v1".to_string(),
            packet_sha256: "packet".to_string(),
            judge_id: "judge".to_string(),
            system_identity_blinded: true,
            external_to_implementation: true,
            judgments: vec![],
        };
        assert!(validate_score_inputs(&packet, "packet", &key, &judgments)
            .expect_err("missing judgments must fail")
            .contains("complete"));
    }
}
