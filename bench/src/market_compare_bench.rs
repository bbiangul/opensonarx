use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::File;
use std::io::BufReader;

#[derive(Debug, Deserialize)]
struct MarketFixture {
    schema_version: u32,
    capture: CaptureMetadata,
    cases: Vec<MarketCase>,
}

#[derive(Debug, Deserialize, Serialize)]
struct CaptureMetadata {
    captured_at: String,
    engine: String,
    locale: String,
    country: String,
    personalization: String,
    candidate_set: String,
    capture_method: String,
}

#[derive(Debug, Deserialize)]
struct MarketCase {
    id: String,
    query: String,
    intent: String,
    contract: SharedContract,
    google_results: Vec<ResultSnapshot>,
}

#[derive(Debug, Default, Deserialize)]
struct SharedContract {
    #[serde(default)]
    top1_any_url: Vec<String>,
    #[serde(default)]
    top1_forbidden_url: Vec<String>,
    #[serde(default)]
    top3_forbidden_url: Vec<String>,
    #[serde(default)]
    top1_surface_groups: Vec<Vec<String>>,
    #[serde(default)]
    min_distinct_domains_top3: usize,
    #[serde(default)]
    primary_url_any_top5: Vec<String>,
    #[serde(default)]
    top1_self_declaration_url: Vec<String>,
    #[serde(default)]
    require_dated_direct_hit_top3: bool,
    #[serde(default)]
    expected_opensonarx_answer_status: String,
    #[serde(default)]
    target_context: String,
    #[serde(default)]
    top3_entity_context_any: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct ResultSnapshot {
    rank: usize,
    title: String,
    url: String,
    #[serde(default)]
    published_at: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct LiveReportSnapshot {
    experiment: String,
    real_embedder: String,
    data_dir: String,
    reports: Vec<LiveCaseSnapshot>,
}

#[derive(Debug, Deserialize)]
struct LiveCaseSnapshot {
    id: String,
    query: String,
    intent: String,
    search_ms: f64,
    answer_status: String,
    quality_pass: bool,
    results: Vec<LiveResultSnapshot>,
}

#[derive(Debug, Deserialize)]
struct LiveResultSnapshot {
    decision_rank: usize,
    title: String,
    url: String,
    published_at: Option<i64>,
}

#[derive(Debug, Serialize)]
struct ContractMetrics {
    shared_score: f64,
    checks_passed: usize,
    checks_total: usize,
    top1_destination_ok: Option<bool>,
    top1_not_forbidden: Option<bool>,
    top1_functional_surface_ok: Option<bool>,
    top3_distinct_domains: usize,
    top3_diversity_ok: Option<bool>,
    top3_has_no_forbidden_url: Option<bool>,
    primary_hit_at_5: Option<bool>,
    top1_is_not_self_declaration: Option<bool>,
    dated_direct_hit_at_3: Option<bool>,
    top3_entity_context_ok: Option<bool>,
}

#[derive(Debug, Serialize)]
struct CaseComparison {
    id: String,
    query: String,
    intent: String,
    target_context: Option<String>,
    google_top1: String,
    opensonarx_top1: String,
    google: ContractMetrics,
    opensonarx: ContractMetrics,
    contract_delta: f64,
    opensonarx_answer_status: String,
    expected_opensonarx_answer_status: Option<String>,
    opensonarx_answer_policy_ok: Option<bool>,
    opensonarx_quality_gate_passed: bool,
    opensonarx_search_ms: f64,
}

#[derive(Debug, Serialize)]
struct MarketComparisonReport {
    evaluated_at_unix_ms: u128,
    experiment: &'static str,
    fixture_path: String,
    live_report_path: String,
    capture: CaptureMetadata,
    live_experiment: String,
    live_embedder: String,
    live_data_dir: String,
    cases: usize,
    google_mean_shared_contract_score: f64,
    opensonarx_mean_shared_contract_score: f64,
    opensonarx_wins: usize,
    ties: usize,
    opensonarx_losses: usize,
    opensonarx_answer_policy_pass_rate: f64,
    market_latency_compared: bool,
    market_superiority_proven: bool,
    cash_thesis_tested: bool,
    comparisons: Vec<CaseComparison>,
    limitations: Vec<&'static str>,
}

pub fn run(
    fixture_path: &str,
    live_report_path: &str,
    output: &str,
    report_path: Option<&str>,
) -> Result<(), String> {
    let fixture: MarketFixture = load_json(fixture_path)?;
    let live: LiveReportSnapshot = load_json(live_report_path)?;
    validate_fixture(&fixture)?;
    validate_live_report(&fixture, &live)?;
    let report = evaluate(fixture, live, fixture_path, live_report_path);
    let serialized = serde_json::to_string_pretty(&report)
        .map_err(|error| format!("cannot serialize market comparison: {error}"))?;
    if let Some(path) = report_path {
        persist_report(path, &serialized)?;
    }

    match output {
        "json" => println!("{serialized}"),
        "table" => print_table(&report),
        other => return Err(format!("unsupported market-compare output format: {other}")),
    }
    Ok(())
}

fn persist_report(path: &str, serialized: &str) -> Result<(), String> {
    if let Some(parent) = std::path::Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("cannot create market report directory: {error}"))?;
        }
    }
    std::fs::write(path, serialized)
        .map_err(|error| format!("cannot write market report {path}: {error}"))
}

fn load_json<T: serde::de::DeserializeOwned>(path: &str) -> Result<T, String> {
    let file = File::open(path).map_err(|error| format!("cannot open {path}: {error}"))?;
    serde_json::from_reader(BufReader::new(file))
        .map_err(|error| format!("cannot parse {path}: {error}"))
}

fn validate_fixture(fixture: &MarketFixture) -> Result<(), String> {
    if fixture.schema_version != 1 {
        return Err(format!(
            "unsupported market fixture schema {}; expected 1",
            fixture.schema_version
        ));
    }
    if fixture.cases.is_empty() {
        return Err("market fixture has no cases".to_string());
    }
    let mut ids = HashSet::new();
    for case in &fixture.cases {
        if !ids.insert(&case.id) {
            return Err(format!("duplicate market case id: {}", case.id));
        }
        if case.google_results.is_empty() {
            return Err(format!("market case {} has no Google results", case.id));
        }
        let mut ranks = HashSet::new();
        for result in &case.google_results {
            if result.rank == 0 || !ranks.insert(result.rank) {
                return Err(format!(
                    "market case {} has invalid or duplicate rank {}",
                    case.id, result.rank
                ));
            }
            if !is_http_url(&result.url) {
                return Err(format!(
                    "market case {} has invalid URL {}",
                    case.id, result.url
                ));
            }
        }
        for group in &case.contract.top1_surface_groups {
            if group.is_empty() {
                return Err(format!(
                    "market case {} has an empty surface-term group",
                    case.id
                ));
            }
        }
    }
    Ok(())
}

fn validate_live_report(fixture: &MarketFixture, live: &LiveReportSnapshot) -> Result<(), String> {
    if !live.experiment.contains("answered-production-path") {
        return Err("market comparison requires an answered production-path report".to_string());
    }
    if live.real_embedder.to_ascii_lowercase().contains("mock") {
        return Err("market comparison refuses a mock live report".to_string());
    }
    for case in &fixture.cases {
        let Some(live_case) = live.reports.iter().find(|report| report.id == case.id) else {
            return Err(format!("live report is missing market case {}", case.id));
        };
        if live_case.query != case.query {
            return Err(format!(
                "query mismatch for {}: fixture={:?} live={:?}",
                case.id, case.query, live_case.query
            ));
        }
        if live_case.intent != case.intent {
            return Err(format!(
                "intent mismatch for {}: fixture={} live={}",
                case.id, case.intent, live_case.intent
            ));
        }
        if live_case.results.is_empty() {
            return Err(format!("live report has no results for {}", case.id));
        }
    }
    Ok(())
}

fn evaluate(
    fixture: MarketFixture,
    live: LiveReportSnapshot,
    fixture_path: &str,
    live_report_path: &str,
) -> MarketComparisonReport {
    let mut comparisons = Vec::new();
    for case in &fixture.cases {
        let live_case = live
            .reports
            .iter()
            .find(|report| report.id == case.id)
            .expect("validated live case");
        let mut google_results = case.google_results.clone();
        google_results.sort_by_key(|result| result.rank);
        let mut opensonarx_results = live_case
            .results
            .iter()
            .map(|result| ResultSnapshot {
                rank: result.decision_rank,
                title: result.title.clone(),
                url: result.url.clone(),
                published_at: result.published_at,
            })
            .collect::<Vec<_>>();
        opensonarx_results.sort_by_key(|result| result.rank);

        let google_metrics = score(&case.contract, &google_results);
        let opensonarx_metrics = score(&case.contract, &opensonarx_results);
        let expected = (!case.contract.expected_opensonarx_answer_status.is_empty())
            .then(|| case.contract.expected_opensonarx_answer_status.clone());
        comparisons.push(CaseComparison {
            id: case.id.clone(),
            query: case.query.clone(),
            intent: case.intent.clone(),
            target_context: (!case.contract.target_context.is_empty())
                .then(|| case.contract.target_context.clone()),
            google_top1: google_results
                .first()
                .map(|result| result.url.clone())
                .unwrap_or_default(),
            opensonarx_top1: opensonarx_results
                .first()
                .map(|result| result.url.clone())
                .unwrap_or_default(),
            contract_delta: opensonarx_metrics.shared_score - google_metrics.shared_score,
            google: google_metrics,
            opensonarx: opensonarx_metrics,
            opensonarx_answer_status: live_case.answer_status.clone(),
            opensonarx_answer_policy_ok: expected
                .as_ref()
                .map(|status| status == &live_case.answer_status),
            expected_opensonarx_answer_status: expected,
            opensonarx_quality_gate_passed: live_case.quality_pass,
            opensonarx_search_ms: live_case.search_ms,
        });
    }

    let google_mean = mean(
        &comparisons
            .iter()
            .map(|comparison| comparison.google.shared_score)
            .collect::<Vec<_>>(),
    );
    let opensonarx_mean = mean(
        &comparisons
            .iter()
            .map(|comparison| comparison.opensonarx.shared_score)
            .collect::<Vec<_>>(),
    );
    let opensonarx_wins = comparisons
        .iter()
        .filter(|comparison| comparison.contract_delta > 1e-9)
        .count();
    let opensonarx_losses = comparisons
        .iter()
        .filter(|comparison| comparison.contract_delta < -1e-9)
        .count();
    let ties = comparisons.len() - opensonarx_wins - opensonarx_losses;
    let answer_checks = comparisons
        .iter()
        .filter_map(|comparison| comparison.opensonarx_answer_policy_ok)
        .collect::<Vec<_>>();

    MarketComparisonReport {
        evaluated_at_unix_ms: unix_time_ms(),
        experiment: "dated-google-vs-live-opensonarx-multi-intent-v1",
        fixture_path: fixture_path.to_string(),
        live_report_path: live_report_path.to_string(),
        capture: fixture.capture,
        live_experiment: live.experiment,
        live_embedder: live.real_embedder,
        live_data_dir: live.data_dir,
        cases: comparisons.len(),
        google_mean_shared_contract_score: google_mean,
        opensonarx_mean_shared_contract_score: opensonarx_mean,
        opensonarx_wins,
        ties,
        opensonarx_losses,
        opensonarx_answer_policy_pass_rate: answer_checks
            .iter()
            .filter(|passed| **passed)
            .count() as f64
            / answer_checks.len().max(1) as f64,
        market_latency_compared: false,
        market_superiority_proven: false,
        cash_thesis_tested: false,
        comparisons,
        limitations: vec![
            "The Google side is a dated signed-out visible-SERP snapshot, not a Google API or repeated randomized sample.",
            "The OpenSonarX side is a later live production-path report over its retained corpus; Google latency was not measured comparably.",
            "Contracts are transparent policy judgments, not blinded human relevance labels.",
            "Google AI answers, ads, carousels, knowledge panels, and personalization are outside this candidate-set comparison.",
            "A higher contract score on nine selected cases does not establish broad or Google-scale superiority.",
            "No eligible cash position appeared, so the economic-conviction thesis is not tested.",
        ],
    }
}

fn unix_time_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn score(contract: &SharedContract, results: &[ResultSnapshot]) -> ContractMetrics {
    let top = results.first();
    let top_surface = top
        .map(|result| fold(&format!("{} {}", result.title, result.url)))
        .unwrap_or_default();
    let top1_destination_ok = (!contract.top1_any_url.is_empty())
        .then(|| top.is_some_and(|result| contains_any(&result.url, &contract.top1_any_url)));
    let top1_not_forbidden = (!contract.top1_forbidden_url.is_empty()).then(|| {
        top.is_some_and(|result| !contains_any(&result.url, &contract.top1_forbidden_url))
    });
    let top1_functional_surface_ok = (!contract.top1_surface_groups.is_empty()).then(|| {
        contract
            .top1_surface_groups
            .iter()
            .all(|group| group.iter().any(|term| top_surface.contains(&fold(term))))
    });
    let top3 = results.iter().take(3).collect::<Vec<_>>();
    let top3_distinct_domains = top3
        .iter()
        .map(|result| domain(&result.url))
        .filter(|domain| !domain.is_empty())
        .collect::<HashSet<_>>()
        .len();
    let top3_diversity_ok = (contract.min_distinct_domains_top3 > 0)
        .then(|| top3_distinct_domains >= contract.min_distinct_domains_top3);
    let top3_has_no_forbidden_url = (!contract.top3_forbidden_url.is_empty()).then(|| {
        top3.iter()
            .all(|result| !contains_any(&result.url, &contract.top3_forbidden_url))
    });
    let primary_hit_at_5 = (!contract.primary_url_any_top5.is_empty()).then(|| {
        results
            .iter()
            .take(5)
            .any(|result| contains_any(&result.url, &contract.primary_url_any_top5))
    });
    let top1_is_not_self_declaration =
        (!contract.top1_self_declaration_url.is_empty()).then(|| {
            top.is_some_and(|result| {
                !contains_any(&result.url, &contract.top1_self_declaration_url)
            })
        });
    let dated_direct_hit_at_3 = contract.require_dated_direct_hit_top3.then(|| {
        top3.iter().any(|result| {
            (result.published_at.is_some() || url_has_date(&result.url))
                && !contains_any(&result.url, &contract.top3_forbidden_url)
        })
    });
    let top3_entity_context_ok = (!contract.top3_entity_context_any.is_empty()).then(|| {
        top3.iter().all(|result| {
            contains_any(
                &format!("{} {}", result.title, result.url),
                &contract.top3_entity_context_any,
            )
        })
    });
    let checks = [
        top1_destination_ok,
        top1_not_forbidden,
        top1_functional_surface_ok,
        top3_diversity_ok,
        top3_has_no_forbidden_url,
        primary_hit_at_5,
        top1_is_not_self_declaration,
        dated_direct_hit_at_3,
        top3_entity_context_ok,
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    let checks_passed = checks.iter().filter(|passed| **passed).count();

    ContractMetrics {
        shared_score: checks_passed as f64 / checks.len().max(1) as f64,
        checks_passed,
        checks_total: checks.len(),
        top1_destination_ok,
        top1_not_forbidden,
        top1_functional_surface_ok,
        top3_distinct_domains,
        top3_diversity_ok,
        top3_has_no_forbidden_url,
        primary_hit_at_5,
        top1_is_not_self_declaration,
        dated_direct_hit_at_3,
        top3_entity_context_ok,
    }
}

fn contains_any(text: &str, patterns: &[String]) -> bool {
    let text = text.to_ascii_lowercase();
    patterns
        .iter()
        .any(|pattern| text.contains(&pattern.to_ascii_lowercase()))
}

fn is_http_url(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("http://")
}

fn domain(url: &str) -> String {
    url.split("//")
        .nth(1)
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("")
        .trim_start_matches("www.")
        .to_ascii_lowercase()
}

fn url_has_date(url: &str) -> bool {
    let bytes = url.as_bytes();
    bytes.windows(4).any(|window| {
        matches!(window, [b'/', b'2', b'0', b'2']) || matches!(window, [b'-', b'2', b'0', b'2'])
    })
}

fn fold(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|character| match character {
            'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            other => other,
        })
        .collect()
}

fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len().max(1) as f64
}

fn print_table(report: &MarketComparisonReport) {
    println!(
        "dated market comparison | cases={} | Google={:.1}% OpenSonarX={:.1}% | wins/ties/losses={}/{}/{} | market_superiority_proven=false",
        report.cases,
        report.google_mean_shared_contract_score * 100.0,
        report.opensonarx_mean_shared_contract_score * 100.0,
        report.opensonarx_wins,
        report.ties,
        report.opensonarx_losses
    );
    println!(
        "{:<31} {:<15} {:>8} {:>8} {:>8}  {}",
        "case", "intent", "Google", "OpenSX", "delta", "answer"
    );
    for comparison in &report.comparisons {
        println!(
            "{:<31} {:<15} {:>7.1}% {:>7.1}% {:+7.1}%  {}",
            comparison.id,
            comparison.intent,
            comparison.google.shared_score * 100.0,
            comparison.opensonarx.shared_score * 100.0,
            comparison.contract_delta * 100.0,
            comparison.opensonarx_answer_status
        );
        println!("  Google:     {}", comparison.google_top1);
        println!("  OpenSonarX: {}", comparison.opensonarx_top1);
    }
    println!(
        "OpenSonarX answer-policy pass rate: {:.1}% | Google latency comparison: unavailable",
        report.opensonarx_answer_policy_pass_rate * 100.0
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(rank: usize, title: &str, url: &str) -> ResultSnapshot {
        ResultSnapshot {
            rank,
            title: title.to_string(),
            url: url.to_string(),
            published_at: None,
        }
    }

    #[test]
    fn tool_contract_requires_functional_surface_and_three_domains() {
        let contract = SharedContract {
            top1_forbidden_url: vec!["python.org/".to_string()],
            top1_surface_groups: vec![
                vec!["python".to_string()],
                vec!["compiler".to_string(), "editor".to_string()],
            ],
            min_distinct_domains_top3: 3,
            ..Default::default()
        };
        let results = vec![
            result(1, "Python Online Compiler", "https://one.test/python"),
            result(2, "Python IDE", "https://two.test/python"),
            result(3, "Python Editor", "https://three.test/python"),
        ];
        let metrics = score(&contract, &results);
        assert_eq!(metrics.checks_passed, metrics.checks_total);
        assert_eq!(metrics.top3_distinct_domains, 3);
    }

    #[test]
    fn due_diligence_penalizes_self_declaration_at_top_one() {
        let contract = SharedContract {
            top1_self_declaration_url: vec!["entity.test/".to_string()],
            min_distinct_domains_top3: 2,
            ..Default::default()
        };
        let results = vec![
            result(1, "We are legitimate", "https://entity.test/"),
            result(2, "Launch coverage", "https://news.test/entity"),
        ];
        let metrics = score(&contract, &results);
        assert_eq!(metrics.top1_is_not_self_declaration, Some(false));
        assert_eq!(metrics.shared_score, 0.5);
    }

    #[test]
    fn due_diligence_rejects_same_name_results_outside_entity_context() {
        let contract = SharedContract {
            top3_entity_context_any: vec!["google".to_string(), "monashees".to_string()],
            ..Default::default()
        };
        let results = vec![
            result(
                1,
                "Gama Fund: Performance | PitchBook",
                "https://pitchbook.test/gama-fund",
            ),
            result(
                2,
                "Google and Monashees launch Gama Fund",
                "https://news.test/gama-fund",
            ),
        ];
        let metrics = score(&contract, &results);
        assert_eq!(metrics.top3_entity_context_ok, Some(false));
        assert_eq!(metrics.shared_score, 0.0);
    }

    #[test]
    fn dated_direct_hit_uses_metadata_or_a_date_bearing_url() {
        let contract = SharedContract {
            require_dated_direct_hit_top3: true,
            top3_forbidden_url: vec!["/tudo-sobre/".to_string()],
            ..Default::default()
        };
        let hub = vec![result(
            1,
            "Latest news",
            "https://news.test/tudo-sobre/topic",
        )];
        assert_eq!(score(&contract, &hub).dated_direct_hit_at_3, Some(false));
        let article = vec![result(
            1,
            "Latest news",
            "https://news.test/2026/08/22/article",
        )];
        assert_eq!(score(&contract, &article).dated_direct_hit_at_3, Some(true));
    }
}
