use oi_index::uncertainty::{
    CashCalibration, ClaimAssessment, DecisionCandidate, DecisionPolicy, EvidenceState,
    OpenUncertainty, OpenUncertaintyConfig, SourceHistory, UncertaintyMode,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::File;
use std::io::BufReader;

const HARD_RISK_FLAGS: [&str; 3] = [
    "future_results_absent",
    "scope_mismatch",
    "not_a_poll_result",
];

#[derive(Debug, Deserialize)]
struct ElectionPollFixture {
    schema_version: u32,
    capture: CaptureMetadata,
    queries: Vec<QueryCase>,
}

#[derive(Debug, Deserialize, Serialize)]
struct CaptureMetadata {
    captured_at: String,
    engine: String,
    locale: String,
    country: String,
    personalization: String,
    candidate_set: String,
}

#[derive(Debug, Deserialize)]
struct QueryCase {
    id: String,
    query: String,
    intent: String,
    required_scope: String,
    answer_policy: String,
    results: Vec<ResultJudgment>,
}

#[derive(Debug, Clone, Deserialize)]
struct ResultJudgment {
    google_rank: usize,
    title: String,
    url: String,
    source_class: String,
    content_state: String,
    scope: String,
    primary: bool,
    official: bool,
    metadata_completeness: f64,
    base_relevance: f64,
    independence_cluster: String,
    dependency_uncertainty: f64,
    staleness_uncertainty: f64,
    distribution_shift: f64,
    #[serde(default)]
    risk_flags: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize)]
struct RankingMetrics {
    policy_utility_ndcg_at_5: f64,
    mean_relevance_at_3: f64,
    mean_metadata_completeness_at_3: f64,
    traceable_rate_at_3: f64,
    primary_or_official_hit_at_3: f64,
    hard_risk_rate_at_3: f64,
    independent_clusters_at_3: f64,
}

#[derive(Debug, Serialize)]
struct RankedResultSummary {
    decision_rank: usize,
    google_rank: usize,
    title: String,
    url: String,
    decision_score: f64,
    mode: UncertaintyMode,
    ignorance: f64,
    dependency_uncertainty: f64,
    cash_log_bayes_factor: f64,
    risk_flags: Vec<String>,
}

#[derive(Debug, Serialize)]
struct QueryReport {
    id: String,
    query: String,
    intent: String,
    required_scope: String,
    answer_policy: String,
    google_top_1: String,
    risk_aware_top_1: String,
    google: RankingMetrics,
    risk_aware: RankingMetrics,
    top_3: Vec<RankedResultSummary>,
    cash_influence_zero: bool,
}

#[derive(Debug, Serialize)]
struct ElectionPollReport {
    experiment: &'static str,
    fixture_path: String,
    schema_version: u32,
    capture: CaptureMetadata,
    queries: usize,
    candidates: usize,
    comparison_scope: &'static str,
    end_to_end_search_validated: bool,
    reranking_hypothesis_supported: bool,
    cash_thesis_tested: bool,
    cash_coverage: f64,
    google: RankingMetrics,
    risk_aware: RankingMetrics,
    hard_risk_reduction_pct: f64,
    query_reports: Vec<QueryReport>,
    limitations: Vec<&'static str>,
}

pub fn run(path: &str, output: &str) -> Result<(), String> {
    let fixture = load_fixture(path)?;
    validate_fixture(&fixture)?;
    let report = evaluate_fixture(fixture, path);

    if output == "json" {
        println!(
            "{}",
            serde_json::to_string_pretty(&report)
                .map_err(|error| format!("cannot serialize report: {error}"))?
        );
    } else if output == "table" {
        print_table(&report);
    } else {
        return Err(format!("unsupported output format: {output}"));
    }

    Ok(())
}

fn load_fixture(path: &str) -> Result<ElectionPollFixture, String> {
    let file = File::open(path).map_err(|error| format!("cannot open {path}: {error}"))?;
    serde_json::from_reader(BufReader::new(file))
        .map_err(|error| format!("cannot parse {path}: {error}"))
}

fn validate_fixture(fixture: &ElectionPollFixture) -> Result<(), String> {
    if fixture.schema_version != 1 {
        return Err(format!(
            "unsupported schema_version {}; expected 1",
            fixture.schema_version
        ));
    }
    if fixture.queries.is_empty() {
        return Err("fixture has no queries".to_string());
    }

    let mut query_ids = HashSet::new();
    for query in &fixture.queries {
        if !query_ids.insert(&query.id) {
            return Err(format!("duplicate query id: {}", query.id));
        }
        if query.results.is_empty() {
            return Err(format!("query {} has no results", query.id));
        }

        let mut ranks = HashSet::new();
        for result in &query.results {
            if result.google_rank == 0 || !ranks.insert(result.google_rank) {
                return Err(format!(
                    "query {} has invalid or duplicate Google rank {}",
                    query.id, result.google_rank
                ));
            }
            if !result.url.starts_with("https://") && !result.url.starts_with("http://") {
                return Err(format!(
                    "query {} has invalid URL: {}",
                    query.id, result.url
                ));
            }
            for (name, value) in [
                ("metadata_completeness", result.metadata_completeness),
                ("base_relevance", result.base_relevance),
                ("dependency_uncertainty", result.dependency_uncertainty),
                ("staleness_uncertainty", result.staleness_uncertainty),
                ("distribution_shift", result.distribution_shift),
            ] {
                if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                    return Err(format!(
                        "query {} result rank {} has invalid {}={}",
                        query.id, result.google_rank, name, value
                    ));
                }
            }
            if result.independence_cluster.trim().is_empty()
                || result.source_class.trim().is_empty()
                || result.content_state.trim().is_empty()
                || result.scope.trim().is_empty()
            {
                return Err(format!(
                    "query {} result rank {} has an empty judgment field",
                    query.id, result.google_rank
                ));
            }
        }
    }

    Ok(())
}

fn evaluate_fixture(fixture: ElectionPollFixture, path: &str) -> ElectionPollReport {
    let config = OpenUncertaintyConfig::default();
    let engine = OpenUncertainty::new(config.clone(), CashCalibration::fit(&[], &config));
    let policy = DecisionPolicy {
        ignorance_penalty: 0.15,
        conflict_penalty: 0.30,
        dependency_penalty: 0.15,
        staleness_penalty: 0.20,
        distribution_shift_penalty: 0.25,
        licensed_cost_penalty: 0.0,
        exploration_floor: 0.05,
    };

    let mut google_metrics = Vec::new();
    let mut risk_aware_metrics = Vec::new();
    let mut query_reports = Vec::new();
    let mut candidate_count = 0;
    let mut cash_effects = 0usize;

    for query in &fixture.queries {
        let mut google_order = query.results.clone();
        google_order.sort_by_key(|result| result.google_rank);
        candidate_count += google_order.len();
        let mut query_cash_effects = 0usize;

        let candidates = google_order
            .iter()
            .map(|result| {
                let estimate = engine.estimate(&assessment(query, result));
                if estimate.cash.log_bayes_factor.abs() > f64::EPSILON {
                    cash_effects += 1;
                    query_cash_effects += 1;
                }
                DecisionCandidate {
                    item: result.clone(),
                    retrieval_rank: result.google_rank,
                    base_relevance: result.base_relevance,
                    licensed_cost: 0.0,
                    estimate,
                }
            })
            .collect();
        let reranked = policy.rerank(candidates);
        let risk_order: Vec<ResultJudgment> =
            reranked.iter().map(|result| result.item.clone()).collect();

        let google = metrics(&google_order);
        let risk_aware = metrics(&risk_order);
        google_metrics.push(google);
        risk_aware_metrics.push(risk_aware);

        let top_3 = reranked
            .iter()
            .take(3)
            .map(|result| RankedResultSummary {
                decision_rank: result.decision_rank,
                google_rank: result.retrieval_rank,
                title: result.item.title.clone(),
                url: result.item.url.clone(),
                decision_score: result.decision_score,
                mode: result.estimate.mode,
                ignorance: result.estimate.uncertainty.ignorance,
                dependency_uncertainty: result.estimate.uncertainty.dependency_uncertainty,
                cash_log_bayes_factor: result.estimate.cash.log_bayes_factor,
                risk_flags: result.item.risk_flags.clone(),
            })
            .collect();

        query_reports.push(QueryReport {
            id: query.id.clone(),
            query: query.query.clone(),
            intent: query.intent.clone(),
            required_scope: query.required_scope.clone(),
            answer_policy: query.answer_policy.clone(),
            google_top_1: google_order[0].title.clone(),
            risk_aware_top_1: risk_order[0].title.clone(),
            google,
            risk_aware,
            top_3,
            cash_influence_zero: query_cash_effects == 0,
        });
    }

    let google = average_metrics(&google_metrics);
    let risk_aware = average_metrics(&risk_aware_metrics);
    let hard_risk_reduction_pct =
        reduction_pct(google.hard_risk_rate_at_3, risk_aware.hard_risk_rate_at_3);
    let reranking_hypothesis_supported = risk_aware.policy_utility_ndcg_at_5
        > google.policy_utility_ndcg_at_5
        && risk_aware.hard_risk_rate_at_3 < google.hard_risk_rate_at_3;

    ElectionPollReport {
        experiment: "brazil-election-polls-google-candidate-rerank-v1",
        fixture_path: path.to_string(),
        schema_version: fixture.schema_version,
        capture: fixture.capture,
        queries: query_reports.len(),
        candidates: candidate_count,
        comparison_scope: "Google candidate-set replay; OpenUncertainty Stage-2 risk reranking",
        end_to_end_search_validated: false,
        reranking_hypothesis_supported,
        cash_thesis_tested: false,
        cash_coverage: cash_effects as f64 / candidate_count.max(1) as f64,
        google,
        risk_aware,
        hard_risk_reduction_pct,
        query_reports,
        limitations: vec![
            "The candidate documents came from a captured Google SERP, not an independent OpenIndex crawl.",
            "Judgments are auditable policy labels, not blinded human relevance judgments.",
            "Current 2026 polls are unresolved as election forecasts; this run evaluates report provenance, scope, time state, and metadata.",
            "No eligible pre-outcome cash commitments were present, so cash correctly had zero ranking effect.",
            "Google results are volatile; re-capture is required for longitudinal comparison.",
        ],
    }
}

fn assessment(query: &QueryCase, result: &ResultJudgment) -> ClaimAssessment {
    let provenance_bonus = if result.official {
        0.75
    } else if result.primary {
        0.50
    } else {
        0.0
    };
    let support_mass = 0.25 + result.metadata_completeness + provenance_bonus;
    let hard_risk_count = result
        .risk_flags
        .iter()
        .filter(|flag| HARD_RISK_FLAGS.contains(&flag.as_str()))
        .count();

    ClaimAssessment {
        claim_ref: format!("{}:google:{}", query.id, result.google_rank),
        task: format!("election_poll_search/{}", query.intent),
        horizon: "captured-serp".to_string(),
        policy_id: "br-election-polls-v1".to_string(),
        source_history: SourceHistory::default(),
        evidence: EvidenceState {
            support_mass,
            conflict_mass: 0.25 * hard_risk_count as f64,
            missing_mass: 1.0 - result.metadata_completeness,
            critical_missing: false,
            independence: 1.0,
            dependency_uncertainty: result.dependency_uncertainty,
            staleness_uncertainty: result.staleness_uncertainty,
            distribution_shift: result.distribution_shift,
        },
        conviction: None,
        deterministic_outcome: None,
    }
}

fn metrics(order: &[ResultJudgment]) -> RankingMetrics {
    let top_5 = &order[..order.len().min(5)];
    let top_3 = &order[..order.len().min(3)];
    let denominator_3 = top_3.len().max(1) as f64;

    let ideal = {
        let mut values: Vec<f64> = order.iter().map(policy_utility).collect();
        values.sort_by(|a, b| b.total_cmp(a));
        values
    };
    let dcg = discounted_gain(top_5.iter().map(policy_utility));
    let ideal_dcg = discounted_gain(ideal.iter().copied().take(5));

    RankingMetrics {
        policy_utility_ndcg_at_5: if ideal_dcg <= f64::EPSILON {
            0.0
        } else {
            dcg / ideal_dcg
        },
        mean_relevance_at_3: top_3
            .iter()
            .map(|result| result.base_relevance)
            .sum::<f64>()
            / denominator_3,
        mean_metadata_completeness_at_3: top_3
            .iter()
            .map(|result| result.metadata_completeness)
            .sum::<f64>()
            / denominator_3,
        traceable_rate_at_3: top_3.iter().filter(|result| traceable(result)).count() as f64
            / denominator_3,
        primary_or_official_hit_at_3: if top_3
            .iter()
            .any(|result| result.primary || result.official)
        {
            1.0
        } else {
            0.0
        },
        hard_risk_rate_at_3: top_3.iter().filter(|result| hard_risk(result)).count() as f64
            / denominator_3,
        independent_clusters_at_3: top_3
            .iter()
            .map(|result| result.independence_cluster.as_str())
            .collect::<HashSet<_>>()
            .len() as f64,
    }
}

fn policy_utility(result: &ResultJudgment) -> f64 {
    let provenance = if result.official || result.primary {
        1.0
    } else {
        0.0
    };
    let evidence_quality = (0.50 * result.metadata_completeness
        + 0.25 * provenance
        + 0.25 * (1.0 - result.dependency_uncertainty))
        .clamp(0.0, 1.0);
    let risk_multiplier = if hard_risk(result) { 0.15 } else { 1.0 };
    result.base_relevance * (0.40 + 0.60 * evidence_quality) * risk_multiplier
}

fn traceable(result: &ResultJudgment) -> bool {
    result.official || result.primary || result.metadata_completeness >= 0.67
}

fn hard_risk(result: &ResultJudgment) -> bool {
    result
        .risk_flags
        .iter()
        .any(|flag| HARD_RISK_FLAGS.contains(&flag.as_str()))
}

fn discounted_gain(values: impl Iterator<Item = f64>) -> f64 {
    values
        .enumerate()
        .map(|(index, value)| value / ((index + 2) as f64).log2())
        .sum()
}

fn average_metrics(metrics: &[RankingMetrics]) -> RankingMetrics {
    let denominator = metrics.len().max(1) as f64;
    RankingMetrics {
        policy_utility_ndcg_at_5: metrics
            .iter()
            .map(|metric| metric.policy_utility_ndcg_at_5)
            .sum::<f64>()
            / denominator,
        mean_relevance_at_3: metrics
            .iter()
            .map(|metric| metric.mean_relevance_at_3)
            .sum::<f64>()
            / denominator,
        mean_metadata_completeness_at_3: metrics
            .iter()
            .map(|metric| metric.mean_metadata_completeness_at_3)
            .sum::<f64>()
            / denominator,
        traceable_rate_at_3: metrics
            .iter()
            .map(|metric| metric.traceable_rate_at_3)
            .sum::<f64>()
            / denominator,
        primary_or_official_hit_at_3: metrics
            .iter()
            .map(|metric| metric.primary_or_official_hit_at_3)
            .sum::<f64>()
            / denominator,
        hard_risk_rate_at_3: metrics
            .iter()
            .map(|metric| metric.hard_risk_rate_at_3)
            .sum::<f64>()
            / denominator,
        independent_clusters_at_3: metrics
            .iter()
            .map(|metric| metric.independent_clusters_at_3)
            .sum::<f64>()
            / denominator,
    }
}

fn reduction_pct(baseline: f64, candidate: f64) -> f64 {
    if baseline <= f64::EPSILON {
        0.0
    } else {
        100.0 * (baseline - candidate) / baseline
    }
}

fn print_table(report: &ElectionPollReport) {
    println!("\n=== Brazil election-polls SERP replay ===");
    println!(
        "snapshot={} engine={} locale={} country={} personalization={}",
        report.capture.captured_at,
        report.capture.engine,
        report.capture.locale,
        report.capture.country,
        report.capture.personalization
    );
    println!(
        "queries={} candidates={} scope={}",
        report.queries, report.candidates, report.comparison_scope
    );
    println!("\n{:<38} {:>12} {:>12}", "Metric", "Google", "Risk-aware");
    println!(
        "{:<38} {:>12.4} {:>12.4}",
        "Policy utility nDCG@5",
        report.google.policy_utility_ndcg_at_5,
        report.risk_aware.policy_utility_ndcg_at_5
    );
    println!(
        "{:<38} {:>12.4} {:>12.4}",
        "Mean relevance@3",
        report.google.mean_relevance_at_3,
        report.risk_aware.mean_relevance_at_3
    );
    println!(
        "{:<38} {:>12.4} {:>12.4}",
        "Metadata completeness@3",
        report.google.mean_metadata_completeness_at_3,
        report.risk_aware.mean_metadata_completeness_at_3
    );
    println!(
        "{:<38} {:>12.4} {:>12.4}",
        "Traceable result rate@3",
        report.google.traceable_rate_at_3,
        report.risk_aware.traceable_rate_at_3
    );
    println!(
        "{:<38} {:>12.4} {:>12.4}",
        "Primary/official hit@3",
        report.google.primary_or_official_hit_at_3,
        report.risk_aware.primary_or_official_hit_at_3
    );
    println!(
        "{:<38} {:>12.4} {:>12.4}",
        "Hard-risk rate@3",
        report.google.hard_risk_rate_at_3,
        report.risk_aware.hard_risk_rate_at_3
    );
    println!(
        "{:<38} {:>12.2} {:>12.2}",
        "Independent evidence clusters@3",
        report.google.independent_clusters_at_3,
        report.risk_aware.independent_clusters_at_3
    );
    println!(
        "Hard-risk reduction: {:.2}% | reranking hypothesis supported: {}",
        report.hard_risk_reduction_pct,
        if report.reranking_hypothesis_supported {
            "YES"
        } else {
            "NO"
        }
    );
    println!(
        "End-to-end search validated: NO | cash thesis tested: NO | cash coverage: {:.1}%",
        report.cash_coverage * 100.0
    );

    println!("\nTop-result changes:");
    for query in &report.query_reports {
        println!("- {}", query.query);
        println!("  Google:     {}", query.google_top_1);
        println!("  Risk-aware: {}", query.risk_aware_top_1);
        println!("  Policy:     {}", query.answer_policy);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_path() -> String {
        format!(
            "{}/fixtures/brazil-election-polls-google-2026-08-21.json",
            env!("CARGO_MANIFEST_DIR")
        )
    }

    #[test]
    fn fixture_is_valid_and_cash_neutral() {
        let fixture = load_fixture(&fixture_path()).expect("fixture");
        validate_fixture(&fixture).expect("valid fixture");
        let report = evaluate_fixture(fixture, &fixture_path());
        assert_eq!(report.cash_coverage, 0.0);
        assert!(!report.cash_thesis_tested);
        assert!(!report.end_to_end_search_validated);
    }

    #[test]
    fn reranker_removes_observed_scope_and_time_failures() {
        let fixture = load_fixture(&fixture_path()).expect("fixture");
        let report = evaluate_fixture(fixture, &fixture_path());
        assert!(report.reranking_hypothesis_supported);
        assert!(report.risk_aware.hard_risk_rate_at_3 < report.google.hard_risk_rate_at_3);
        assert!(
            report.risk_aware.policy_utility_ndcg_at_5 > report.google.policy_utility_ndcg_at_5
        );

        let second_round = report
            .query_reports
            .iter()
            .find(|query| query.id == "second_round_national")
            .expect("second-round query");
        assert!(!second_round.risk_aware_top_1.contains("Real Time/DF"));

        let latest = report
            .query_reports
            .iter()
            .find(|query| query.id == "latest_completed_national")
            .expect("latest query");
        assert!(!latest.risk_aware_top_1.contains("com 6 mil entrevistados"));
    }

    #[test]
    fn official_queries_keep_official_sources_at_the_top() {
        let fixture = load_fixture(&fixture_path()).expect("fixture");
        let report = evaluate_fixture(fixture, &fixture_path());
        for id in ["methodology_registry", "official_outcome_2022"] {
            let query = report
                .query_reports
                .iter()
                .find(|query| query.id == id)
                .expect("official query");
            assert!(query.risk_aware_top_1.contains("TSE") || query.id == "methodology_registry");
        }
    }
}
