//! Aggregate the independent live-evaluation artifacts into one fail-closed
//! readiness verdict. A green answer contract must never conceal broken web
//! discovery, and a strong curated comparison must never be reported as proof
//! of broad superiority or of the cash-at-risk thesis.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashSet};
use std::fs::File;
use std::io::BufReader;

const MAX_ARTIFACT_AGE_MS: u128 = 24 * 60 * 60 * 1_000;
const CLOCK_SKEW_MS: u128 = 5 * 60 * 1_000;
const MIN_LIVE_CASES: usize = 68;
const MIN_MARKET_CASES: usize = 9;
const MIN_PROVIDER_ATTEMPTS_PER_ENGINE: usize = 24;

fn default_provider_concurrency() -> usize {
    1
}

#[derive(Debug, Clone, Copy)]
pub struct Thresholds {
    pub min_healthy_providers: usize,
    pub max_cold_search_p99_ms: f64,
    pub max_resident_search_p95_ms: f64,
    pub max_resident_search_p99_ms: f64,
    pub min_resident_qps: f64,
}

#[derive(Debug, Deserialize)]
struct ProviderReport {
    #[serde(default)]
    captured_at_unix_ms: u128,
    live_network: bool,
    mock_results: bool,
    required_healthy_engines: usize,
    healthy_engines: usize,
    rounds: usize,
    #[serde(default = "default_provider_concurrency")]
    concurrency: usize,
    queries: Vec<String>,
    engines: Vec<ProviderSummary>,
}

#[derive(Debug, Deserialize)]
struct ProviderSummary {
    engine: String,
    healthy: bool,
}

#[derive(Debug, Deserialize)]
struct LiveReport {
    #[serde(default)]
    captured_at_unix_ms: u128,
    experiment: String,
    answer_mode: bool,
    real_embedder: String,
    data_dir: String,
    initial_documents: usize,
    final_documents: usize,
    passes: usize,
    warmup_passes: usize,
    concurrency: usize,
    query_variants: bool,
    cases: usize,
    throughput_qps: f64,
    quality_pass_rate: f64,
    search_p95_ms: f64,
    search_p99_ms: f64,
    answer_hydration_p99_ms: f64,
    background_crawl_p99_ms: f64,
    reports: Vec<LiveCase>,
}

#[derive(Debug, Deserialize)]
struct LiveCase {
    intent: String,
    answer_status: String,
    hydrated_pages: usize,
    crawled_pages: usize,
    sources: Vec<String>,
    quality_pass: bool,
    #[serde(default)]
    answer_evidence_lineage: Vec<Option<LineageSnapshot>>,
    results: Vec<LiveResult>,
}

#[derive(Debug, Deserialize)]
struct LineageSnapshot {
    underlying_unit_kind: String,
}

#[derive(Debug, Deserialize)]
struct LiveResult {
    uncertainty_mode: String,
    cash_log_bayes_factor: f64,
}

#[derive(Debug, Deserialize)]
struct MarketReport {
    #[serde(default)]
    evaluated_at_unix_ms: u128,
    experiment: String,
    live_data_dir: String,
    cases: usize,
    google_mean_shared_contract_score: f64,
    opensonarx_mean_shared_contract_score: f64,
    opensonarx_answer_policy_pass_rate: f64,
    market_latency_compared: bool,
    market_superiority_proven: bool,
    cash_thesis_tested: bool,
    comparisons: Vec<MarketCase>,
}

#[derive(Debug, Deserialize)]
struct MarketCase {
    opensonarx_answer_policy_ok: Option<bool>,
    opensonarx_quality_gate_passed: bool,
}

#[derive(Debug, Serialize)]
struct GateCheck {
    id: &'static str,
    scope: &'static str,
    passed: bool,
    observed: String,
    requirement: String,
}

#[derive(Debug, Serialize)]
struct ReadinessReport {
    evaluated_at_unix_ms: u128,
    experiment: &'static str,
    artifact_paths: ArtifactPaths,
    thresholds: ThresholdReport,
    search_core_ready: bool,
    research_claims_validated: bool,
    overall_ready: bool,
    failed_search_core_gates: Vec<&'static str>,
    unvalidated_research_claims: Vec<&'static str>,
    checks: Vec<GateCheck>,
}

#[derive(Debug, Serialize)]
struct ArtifactPaths {
    provider_report: String,
    cold_report: String,
    resident_report: String,
    market_report: String,
}

#[derive(Debug, Serialize)]
struct ThresholdReport {
    min_healthy_providers: usize,
    max_cold_search_p99_ms: f64,
    max_resident_search_p95_ms: f64,
    max_resident_search_p99_ms: f64,
    min_resident_qps: f64,
    max_artifact_age_hours: u128,
}

pub fn run(
    provider_report_path: &str,
    cold_report_path: &str,
    resident_report_path: &str,
    market_report_path: &str,
    thresholds: Thresholds,
    output: &str,
    report_path: Option<&str>,
) -> Result<(), String> {
    validate_thresholds(thresholds)?;
    let provider: ProviderReport = load_json(provider_report_path)?;
    let cold: LiveReport = load_json(cold_report_path)?;
    let resident: LiveReport = load_json(resident_report_path)?;
    let market: MarketReport = load_json(market_report_path)?;
    let now = unix_time_ms();
    let report = evaluate(
        provider,
        cold,
        resident,
        market,
        thresholds,
        now,
        ArtifactPaths {
            provider_report: provider_report_path.to_string(),
            cold_report: cold_report_path.to_string(),
            resident_report: resident_report_path.to_string(),
            market_report: market_report_path.to_string(),
        },
    );
    let serialized = serde_json::to_string_pretty(&report)
        .map_err(|error| format!("cannot serialize readiness report: {error}"))?;
    if let Some(path) = report_path {
        persist_report(path, &serialized)?;
    }
    match output {
        "json" => println!("{serialized}"),
        "table" => print_table(&report),
        other => {
            return Err(format!(
                "unsupported production-readiness output format: {other}"
            ))
        }
    }
    if !report.overall_ready {
        return Err(format!(
            "overall gate is not ready: {} search-core failure(s), {} research claim(s) unvalidated",
            report.failed_search_core_gates.len(),
            report.unvalidated_research_claims.len()
        ));
    }
    Ok(())
}

fn evaluate(
    provider: ProviderReport,
    cold: LiveReport,
    resident: LiveReport,
    market: MarketReport,
    thresholds: Thresholds,
    now: u128,
    artifact_paths: ArtifactPaths,
) -> ReadinessReport {
    let mut checks = Vec::new();
    let mut add = |id, scope, passed, observed: String, requirement: String| {
        checks.push(GateCheck {
            id,
            scope,
            passed,
            observed,
            requirement,
        });
    };

    let artifact_times = [
        provider.captured_at_unix_ms,
        cold.captured_at_unix_ms,
        resident.captured_at_unix_ms,
        market.evaluated_at_unix_ms,
    ];
    let artifacts_fresh = artifact_times
        .iter()
        .all(|timestamp| artifact_is_fresh(*timestamp, now));
    add(
        "fresh_artifacts",
        "search_core",
        artifacts_fresh,
        format!("timestamps_ms={artifact_times:?}; now_ms={now}"),
        "all four artifacts must be generated within 24h and not be future-dated".to_string(),
    );

    let coherent_data_dir = cold.data_dir == resident.data_dir
        && cold.data_dir == market.live_data_dir
        && cold.captured_at_unix_ms <= resident.captured_at_unix_ms
        && cold.captured_at_unix_ms <= market.evaluated_at_unix_ms;
    add(
        "coherent_run_lineage",
        "search_core",
        coherent_data_dir,
        format!(
            "cold={:?}; resident={:?}; market={:?}",
            cold.data_dir, resident.data_dir, market.live_data_dir
        ),
        "cold, resident, and market artifacts must reference one data directory and valid run order"
            .to_string(),
    );

    add(
        "real_provider_probe",
        "search_core",
        provider.live_network && !provider.mock_results,
        format!(
            "live_network={}; mock_results={}",
            provider.live_network, provider.mock_results
        ),
        "provider probe must use the live network with no mock results".to_string(),
    );
    let attempts_per_engine = provider.rounds.saturating_mul(provider.queries.len());
    let probe_depth_ok = provider.rounds >= 3
        && provider.queries.len() >= 3
        && attempts_per_engine >= MIN_PROVIDER_ATTEMPTS_PER_ENGINE;
    add(
        "provider_probe_depth",
        "search_core",
        probe_depth_ok,
        format!(
            "rounds={}; queries={}; attempts_per_engine={}",
            provider.rounds,
            provider.queries.len(),
            attempts_per_engine
        ),
        format!(
            "at least 3 rounds, 3 query classes, and {MIN_PROVIDER_ATTEMPTS_PER_ENGINE} attempts per engine"
        ),
    );
    let provider_capacity_ok = provider.concurrency >= cold.concurrency;
    add(
        "provider_probe_capacity",
        "search_core",
        provider_capacity_ok,
        format!(
            "provider_concurrency={}; cold_e2e_concurrency={}",
            provider.concurrency, cold.concurrency
        ),
        "provider redundancy must be probed at or above the cold production concurrency"
            .to_string(),
    );
    let healthy_names = provider
        .engines
        .iter()
        .filter(|engine| engine.healthy)
        .map(|engine| engine.engine.as_str())
        .collect::<Vec<_>>();
    let required_healthy = thresholds
        .min_healthy_providers
        .max(provider.required_healthy_engines);
    let redundancy_ok =
        provider.healthy_engines >= required_healthy && healthy_names.len() >= required_healthy;
    add(
        "provider_redundancy",
        "search_core",
        redundancy_ok,
        format!(
            "healthy={}/{}; providers={healthy_names:?}",
            provider.healthy_engines, required_healthy
        ),
        "independent providers must pass every semantic-health attempt".to_string(),
    );

    let cold_profile_ok = cold.experiment.contains("answered-production-path")
        && cold.answer_mode
        && !cold.real_embedder.to_ascii_lowercase().contains("mock")
        && cold.initial_documents == 0
        && cold.final_documents > cold.initial_documents
        && cold.passes == 1
        && cold.warmup_passes == 0
        && cold.query_variants
        && cold.cases >= MIN_LIVE_CASES
        && cold.concurrency > 1;
    add(
        "cold_production_profile",
        "search_core",
        cold_profile_ok,
        format!(
            "experiment={:?}; model={:?}; docs={}->{}; passes={}; warmup={}; variants={}; cases={}; concurrency={}",
            cold.experiment,
            cold.real_embedder,
            cold.initial_documents,
            cold.final_documents,
            cold.passes,
            cold.warmup_passes,
            cold.query_variants,
            cold.cases,
            cold.concurrency
        ),
        format!(
            "fresh real-model answered run, no warmup, variants, concurrency, and at least {MIN_LIVE_CASES} cases"
        ),
    );
    let cold_quality_ok = perfect_quality(&cold);
    add(
        "cold_quality_contracts",
        "search_core",
        cold_quality_ok,
        format!(
            "quality={:.3}; failed_cases={}",
            cold.quality_pass_rate,
            cold.reports
                .iter()
                .filter(|case| !case.quality_pass)
                .count()
        ),
        "all cold multi-intent contracts must pass".to_string(),
    );

    let web_sources = recognized_web_sources(&cold);
    let hydrated_pages = cold
        .reports
        .iter()
        .map(|case| case.hydrated_pages)
        .sum::<usize>();
    let crawled_pages = cold
        .reports
        .iter()
        .map(|case| case.crawled_pages)
        .sum::<usize>();
    let live_acquisition_ok = !web_sources.is_empty() && hydrated_pages > 0 && crawled_pages > 0;
    add(
        "cold_live_acquisition",
        "search_core",
        live_acquisition_ok,
        format!(
            "web_sources={web_sources:?}; hydrated_pages={hydrated_pages}; crawled_pages={crawled_pages}"
        ),
        "the E2E run must integrate a live web provider, synchronous hydration, and background crawling"
            .to_string(),
    );

    let intents = cold
        .reports
        .iter()
        .map(|case| case.intent.as_str())
        .collect::<HashSet<_>>();
    let answer_modes = cold
        .reports
        .iter()
        .map(|case| case.answer_status.as_str())
        .collect::<HashSet<_>>();
    let required_intents = [
        "navigational",
        "transactional",
        "tool",
        "current_events",
        "factual",
        "due_diligence",
    ];
    let required_answers = [
        "destination",
        "tool_results",
        "extractive",
        "insufficient_evidence",
    ];
    let coverage_ok = required_intents
        .iter()
        .all(|intent| intents.contains(intent))
        && required_answers
            .iter()
            .all(|mode| answer_modes.contains(mode));
    add(
        "multi_intent_decision_coverage",
        "search_core",
        coverage_ok,
        format!("intents={intents:?}; answers={answer_modes:?}"),
        "six intent classes and all four answer/abstention modes must be exercised".to_string(),
    );

    let result_count = cold
        .reports
        .iter()
        .map(|case| case.results.len())
        .sum::<usize>();
    let unsafe_cash_effects = cold
        .reports
        .iter()
        .flat_map(|case| &case.results)
        .filter(|result| result.cash_log_bayes_factor.abs() > f64::EPSILON)
        .count();
    let decision_modes = cold
        .reports
        .iter()
        .flat_map(|case| &case.results)
        .map(|result| result.uncertainty_mode.as_str())
        .collect::<HashSet<_>>();
    let decision_safety_ok = result_count >= cold.cases && unsafe_cash_effects == 0;
    add(
        "decision_metadata_safety",
        "search_core",
        decision_safety_ok,
        format!(
            "results={result_count}; modes={decision_modes:?}; nonzero_uncalibrated_cash={unsafe_cash_effects}"
        ),
        "every case must return decided results and uncalibrated cash must have exactly zero effect"
            .to_string(),
    );

    let cold_latency_ok = cold.search_p99_ms <= thresholds.max_cold_search_p99_ms
        && cold.answer_hydration_p99_ms <= 5_000.0
        && cold.background_crawl_p99_ms <= 3_000.0;
    add(
        "cold_latency_slo",
        "search_core",
        cold_latency_ok,
        format!(
            "search_p99={:.1}ms; hydration_p99={:.1}ms; background_crawl_p99={:.1}ms",
            cold.search_p99_ms, cold.answer_hydration_p99_ms, cold.background_crawl_p99_ms
        ),
        format!(
            "search p99 <= {:.1}ms, hydration p99 <= 5000ms, background crawl p99 <= 3000ms",
            thresholds.max_cold_search_p99_ms
        ),
    );

    let resident_profile_ok = resident.experiment.contains("answered-production-path")
        && resident.answer_mode
        && !resident.real_embedder.to_ascii_lowercase().contains("mock")
        && resident.initial_documents >= cold.final_documents
        && resident.final_documents >= resident.initial_documents
        && resident.passes >= 3
        && resident.warmup_passes == 0
        && resident.query_variants
        && resident.cases >= MIN_LIVE_CASES * 3
        && resident.concurrency == cold.concurrency;
    add(
        "resident_production_profile",
        "search_core",
        resident_profile_ok,
        format!(
            "docs={}->{}; passes={}; cases={}; concurrency={}",
            resident.initial_documents,
            resident.final_documents,
            resident.passes,
            resident.cases,
            resident.concurrency
        ),
        "same retained corpus, at least 3 full variant passes, real model, and matching concurrency"
            .to_string(),
    );
    add(
        "resident_quality_contracts",
        "search_core",
        perfect_quality(&resident),
        format!(
            "quality={:.3}; failed_cases={}",
            resident.quality_pass_rate,
            resident
                .reports
                .iter()
                .filter(|case| !case.quality_pass)
                .count()
        ),
        "all resident multi-pass contracts must pass".to_string(),
    );
    let resident_slo_ok = resident.search_p95_ms <= thresholds.max_resident_search_p95_ms
        && resident.search_p99_ms <= thresholds.max_resident_search_p99_ms
        && resident.answer_hydration_p99_ms <= 2_500.0
        && resident.background_crawl_p99_ms <= 2_000.0
        && resident.throughput_qps >= thresholds.min_resident_qps;
    add(
        "resident_latency_throughput_slo",
        "search_core",
        resident_slo_ok,
        format!(
            "search_p95={:.1}ms; search_p99={:.1}ms; hydration_p99={:.1}ms; background_crawl_p99={:.1}ms; qps={:.1}",
            resident.search_p95_ms,
            resident.search_p99_ms,
            resident.answer_hydration_p99_ms,
            resident.background_crawl_p99_ms,
            resident.throughput_qps
        ),
        format!(
            "search p95 <= {:.1}ms, p99 <= {:.1}ms, hydration p99 <= 2500ms, crawl p99 <= 2000ms, qps >= {:.1}",
            thresholds.max_resident_search_p95_ms,
            thresholds.max_resident_search_p99_ms,
            thresholds.min_resident_qps
        ),
    );

    let market_integrity_ok = market
        .experiment
        .contains("google-vs-live-opensonarx-multi-intent")
        && market.cases >= MIN_MARKET_CASES
        && market.comparisons.len() == market.cases
        && approx_one(market.opensonarx_answer_policy_pass_rate)
        && market.comparisons.iter().all(|case| {
            case.opensonarx_quality_gate_passed
                && case.opensonarx_answer_policy_ok.is_none_or(|passed| passed)
        });
    add(
        "market_comparison_integrity",
        "search_core",
        market_integrity_ok,
        format!(
            "experiment={:?}; cases={}; answer_policy={:.3}",
            market.experiment, market.cases, market.opensonarx_answer_policy_pass_rate
        ),
        format!(
            "dated transparent comparison with at least {MIN_MARKET_CASES} cases and all answer policies passing"
        ),
    );
    let limited_market_quality_ok = market.google_mean_shared_contract_score.is_finite()
        && market.opensonarx_mean_shared_contract_score.is_finite()
        && market.opensonarx_mean_shared_contract_score >= market.google_mean_shared_contract_score;
    add(
        "limited_market_candidate_quality",
        "search_core",
        limited_market_quality_ok,
        format!(
            "Google={:.3}; OpenSonarX={:.3}",
            market.google_mean_shared_contract_score, market.opensonarx_mean_shared_contract_score
        ),
        "OpenSonarX must be no worse on the same transparent candidate-set contracts; this is not a superiority claim"
            .to_string(),
    );

    add(
        "comparable_market_latency",
        "research_claim",
        market.market_latency_compared,
        format!("market_latency_compared={}", market.market_latency_compared),
        "measure Google and OpenSonarX latency under a comparable repeated protocol".to_string(),
    );
    add(
        "broad_market_superiority",
        "research_claim",
        market.market_superiority_proven,
        format!(
            "market_superiority_proven={}",
            market.market_superiority_proven
        ),
        "blinded, broad, repeated relevance evaluation must support superiority".to_string(),
    );
    add(
        "cash_predictive_thesis",
        "research_claim",
        market.cash_thesis_tested,
        format!("cash_thesis_tested={}", market.cash_thesis_tested),
        "prospective resolved outcomes must show calibrated claim-specific cash adds held-out predictive value"
            .to_string(),
    );
    let lineage_kinds = cold
        .reports
        .iter()
        .flat_map(|case| &case.answer_evidence_lineage)
        .filter_map(Option::as_ref)
        .map(|lineage| lineage.underlying_unit_kind.as_str())
        .collect::<BTreeSet<_>>();
    let generalized_lineage = lineage_kinds.len() >= 3;
    add(
        "generalized_claim_lineage",
        "research_claim",
        generalized_lineage,
        format!("underlying_unit_kinds={lineage_kinds:?}"),
        "at least three materially different claim classes must expose structured underlying-unit lineage"
            .to_string(),
    );
    let calibrated_claim_results = cold
        .reports
        .iter()
        .flat_map(|case| &case.results)
        .filter(|result| result.uncertainty_mode == "calibrated_probability")
        .count();
    add(
        "live_calibrated_uncertainty",
        "research_claim",
        calibrated_claim_results > 0,
        format!("calibrated_probability_results={calibrated_claim_results}"),
        "at least one eligible live claim class must produce an outcome-calibrated probability"
            .to_string(),
    );

    let search_core_ready = checks
        .iter()
        .filter(|check| check.scope == "search_core")
        .all(|check| check.passed);
    let research_claims_validated = checks
        .iter()
        .filter(|check| check.scope == "research_claim")
        .all(|check| check.passed);
    let failed_search_core_gates = checks
        .iter()
        .filter(|check| check.scope == "search_core" && !check.passed)
        .map(|check| check.id)
        .collect();
    let unvalidated_research_claims = checks
        .iter()
        .filter(|check| check.scope == "research_claim" && !check.passed)
        .map(|check| check.id)
        .collect();
    ReadinessReport {
        evaluated_at_unix_ms: now,
        experiment: "opensonarx-production-readiness-v1",
        artifact_paths,
        thresholds: ThresholdReport {
            min_healthy_providers: thresholds.min_healthy_providers,
            max_cold_search_p99_ms: thresholds.max_cold_search_p99_ms,
            max_resident_search_p95_ms: thresholds.max_resident_search_p95_ms,
            max_resident_search_p99_ms: thresholds.max_resident_search_p99_ms,
            min_resident_qps: thresholds.min_resident_qps,
            max_artifact_age_hours: MAX_ARTIFACT_AGE_MS / (60 * 60 * 1_000),
        },
        search_core_ready,
        research_claims_validated,
        overall_ready: search_core_ready && research_claims_validated,
        failed_search_core_gates,
        unvalidated_research_claims,
        checks,
    }
}

fn validate_thresholds(thresholds: Thresholds) -> Result<(), String> {
    if thresholds.min_healthy_providers == 0 {
        return Err("min-healthy-providers must be positive".to_string());
    }
    for (name, value) in [
        ("max-cold-search-p99-ms", thresholds.max_cold_search_p99_ms),
        (
            "max-resident-search-p95-ms",
            thresholds.max_resident_search_p95_ms,
        ),
        (
            "max-resident-search-p99-ms",
            thresholds.max_resident_search_p99_ms,
        ),
        ("min-resident-qps", thresholds.min_resident_qps),
    ] {
        if !value.is_finite() || value <= 0.0 {
            return Err(format!("{name} must be a finite positive number"));
        }
    }
    Ok(())
}

fn artifact_is_fresh(timestamp: u128, now: u128) -> bool {
    timestamp > 0
        && timestamp <= now.saturating_add(CLOCK_SKEW_MS)
        && now.saturating_sub(timestamp) <= MAX_ARTIFACT_AGE_MS
}

fn perfect_quality(report: &LiveReport) -> bool {
    approx_one(report.quality_pass_rate)
        && report.reports.len() == report.cases
        && report.reports.iter().all(|case| case.quality_pass)
}

fn approx_one(value: f64) -> bool {
    value.is_finite() && (value - 1.0).abs() <= 1e-9
}

fn recognized_web_sources(report: &LiveReport) -> BTreeSet<&str> {
    const PROVIDERS: &[&str] = &[
        "bing",
        "brave",
        "brave-html",
        "duckduckgo",
        "google",
        "mojeek",
        "qwant",
        "searxng",
        "serper",
        "stract",
        "yep",
    ];
    report
        .reports
        .iter()
        .flat_map(|case| &case.sources)
        .map(String::as_str)
        .filter(|source| PROVIDERS.contains(source))
        .collect()
}

fn load_json<T: serde::de::DeserializeOwned>(path: &str) -> Result<T, String> {
    let file = File::open(path).map_err(|error| format!("cannot open {path}: {error}"))?;
    serde_json::from_reader(BufReader::new(file))
        .map_err(|error| format!("cannot parse {path}: {error}"))
}

fn persist_report(path: &str, serialized: &str) -> Result<(), String> {
    if let Some(parent) = std::path::Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("cannot create readiness report directory: {error}"))?;
        }
    }
    std::fs::write(path, serialized)
        .map_err(|error| format!("cannot write readiness report {path}: {error}"))
}

fn unix_time_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn print_table(report: &ReadinessReport) {
    println!(
        "production readiness | search_core={} research_claims={} overall={}",
        verdict(report.search_core_ready),
        verdict(report.research_claims_validated),
        verdict(report.overall_ready)
    );
    for check in &report.checks {
        println!(
            "{} {:<16} {:<34} observed={} | required={}",
            verdict(check.passed),
            check.scope,
            check.id,
            check.observed,
            check.requirement
        );
    }
}

fn verdict(passed: bool) -> &'static str {
    if passed {
        "PASS"
    } else {
        "FAIL"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_freshness_is_fail_closed() {
        assert!(!artifact_is_fresh(0, 10_000));
        assert!(artifact_is_fresh(9_000, 10_000));
        assert!(!artifact_is_fresh(1, MAX_ARTIFACT_AGE_MS.saturating_add(2)));
        assert!(!artifact_is_fresh(10_000 + CLOCK_SKEW_MS + 1, 10_000));
    }

    #[test]
    fn thresholds_reject_non_finite_or_zero_values() {
        let valid = Thresholds {
            min_healthy_providers: 2,
            max_cold_search_p99_ms: 10_000.0,
            max_resident_search_p95_ms: 2_000.0,
            max_resident_search_p99_ms: 4_000.0,
            min_resident_qps: 10.0,
        };
        assert!(validate_thresholds(valid).is_ok());
        assert!(validate_thresholds(Thresholds {
            min_resident_qps: f64::NAN,
            ..valid
        })
        .is_err());
        assert!(validate_thresholds(Thresholds {
            min_healthy_providers: 0,
            ..valid
        })
        .is_err());
    }
}
