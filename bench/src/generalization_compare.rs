use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::io::BufReader;

#[derive(Debug, Deserialize)]
struct Manifest {
    name: String,
    queries: Vec<ManifestQuery>,
}

#[derive(Debug, Deserialize)]
struct ManifestQuery {
    id: String,
    query: String,
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
    manifest: ManifestQuery,
    search_ms: f64,
    answer: Option<OpenAnswer>,
    results: Vec<OpenResult>,
}

#[derive(Debug, Deserialize)]
struct OpenAnswer {
    status: String,
    uncertainty_mode: String,
}

#[derive(Debug, Deserialize)]
struct OpenResult {
    url: String,
    uncertainty_mode: String,
}

#[derive(Debug, Deserialize)]
struct GoogleReport {
    experiment: String,
    manifest_sha256: String,
    judged_during_capture: bool,
    cases: Vec<GoogleCapture>,
}

#[derive(Debug, Deserialize)]
struct GoogleCapture {
    id: String,
    query: String,
    wall_ms: f64,
    unusual_traffic: bool,
    ai_overview_visible: bool,
    results: Vec<GoogleResult>,
}

#[derive(Debug, Deserialize)]
struct GoogleResult {
    url: String,
}

#[derive(Debug, Serialize)]
struct LatencyStats {
    count: usize,
    p50_ms: f64,
    p95_ms: f64,
    p99_ms: f64,
}

#[derive(Debug, Serialize)]
struct Coverage {
    queries: usize,
    nonempty: usize,
    rate: f64,
    mean_results_at_5: f64,
    mean_distinct_domains_at_5: f64,
    at_least_three_domains: usize,
}

#[derive(Debug, Serialize)]
struct MissingCase {
    id: String,
    query: String,
    opensonarx_nonempty: bool,
    google_nonempty: bool,
}

#[derive(Debug, Serialize)]
struct Report {
    experiment: &'static str,
    manifest_name: String,
    manifest_path: String,
    opensonarx_report_path: String,
    google_report_path: String,
    manifest_sha256: String,
    queries: usize,
    opensonarx: Coverage,
    google: Coverage,
    both_nonempty: usize,
    google_only_nonempty: usize,
    opensonarx_only_nonempty: usize,
    neither_nonempty: usize,
    same_top1_host: usize,
    opensonarx_answer_status_counts: BTreeMap<String, usize>,
    opensonarx_substantive_answers: usize,
    opensonarx_explicit_abstentions: usize,
    opensonarx_uncertainty_applicable_queries: usize,
    google_ai_overview_visible: usize,
    google_unusual_traffic: usize,
    opensonarx_backend_latency: LatencyStats,
    google_browser_latency: LatencyStats,
    latency_comparable: bool,
    relevance_judged: bool,
    trustworthiness_judged: bool,
    general_reliability_superiority_proven: bool,
    market_superiority_proven: bool,
    missing_cases: Vec<MissingCase>,
    limitations: Vec<&'static str>,
}

pub fn run(
    manifest_path: &str,
    opensonarx_report_path: &str,
    google_report_path: &str,
    output: &str,
    report_path: Option<&str>,
) -> Result<(), String> {
    let manifest: Manifest = load_json(manifest_path)?;
    let opensonarx: OpenReport = load_json(opensonarx_report_path)?;
    let google: GoogleReport = load_json(google_report_path)?;
    validate(&manifest, &opensonarx, &google)?;
    let report = evaluate(
        manifest,
        opensonarx,
        google,
        manifest_path,
        opensonarx_report_path,
        google_report_path,
    );
    let serialized = serde_json::to_string_pretty(&report)
        .map_err(|error| format!("cannot serialize generalization comparison: {error}"))?;
    if let Some(path) = report_path {
        persist_report(path, &serialized)?;
    }
    match output {
        "json" => println!("{serialized}"),
        "table" => print_table(&report),
        other => {
            return Err(format!(
                "unsupported generalization comparison output: {other}"
            ))
        }
    }
    Ok(())
}

fn load_json<T: serde::de::DeserializeOwned>(path: &str) -> Result<T, String> {
    let file = File::open(path).map_err(|error| format!("cannot open {path}: {error}"))?;
    serde_json::from_reader(BufReader::new(file))
        .map_err(|error| format!("cannot parse {path}: {error}"))
}

fn validate(
    manifest: &Manifest,
    opensonarx: &OpenReport,
    google: &GoogleReport,
) -> Result<(), String> {
    if !opensonarx
        .experiment
        .contains("sealed-unjudged-generalization")
        || !google
            .experiment
            .contains("sealed-google-visible-serp-generalization")
    {
        return Err("comparison requires the two sealed generalization captures".to_string());
    }
    if opensonarx.judged_during_capture || google.judged_during_capture {
        return Err("capture must remain unjudged until after both systems run".to_string());
    }
    if opensonarx.manifest_sha256 != google.manifest_sha256 {
        return Err("capture manifest hashes do not match".to_string());
    }
    if manifest.queries.len() < 40 {
        return Err("generalization comparison requires at least 40 queries".to_string());
    }
    let expected = manifest
        .queries
        .iter()
        .map(|query| (query.id.as_str(), query.query.as_str()))
        .collect::<BTreeMap<_, _>>();
    let open = opensonarx
        .captures
        .iter()
        .map(|capture| {
            (
                capture.manifest.id.as_str(),
                capture.manifest.query.as_str(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let market = google
        .cases
        .iter()
        .map(|capture| (capture.id.as_str(), capture.query.as_str()))
        .collect::<BTreeMap<_, _>>();
    if expected != open || expected != market {
        return Err("manifest IDs/queries do not exactly match both captures".to_string());
    }
    Ok(())
}

fn evaluate(
    manifest: Manifest,
    opensonarx: OpenReport,
    google: GoogleReport,
    manifest_path: &str,
    opensonarx_report_path: &str,
    google_report_path: &str,
) -> Report {
    let open_by_id = opensonarx
        .captures
        .iter()
        .map(|capture| (capture.manifest.id.as_str(), capture))
        .collect::<BTreeMap<_, _>>();
    let google_by_id = google
        .cases
        .iter()
        .map(|capture| (capture.id.as_str(), capture))
        .collect::<BTreeMap<_, _>>();
    let mut both_nonempty = 0;
    let mut google_only_nonempty = 0;
    let mut opensonarx_only_nonempty = 0;
    let mut neither_nonempty = 0;
    let mut same_top1_host = 0;
    let mut missing_cases = Vec::new();
    let mut answer_status_counts = BTreeMap::new();
    let mut opensonarx_substantive_answers = 0;
    let mut opensonarx_explicit_abstentions = 0;
    let mut opensonarx_uncertainty_applicable_queries = 0;

    for query in &manifest.queries {
        let open = open_by_id[query.id.as_str()];
        let market = google_by_id[query.id.as_str()];
        let open_nonempty = !open.results.is_empty();
        let market_nonempty = !market.results.is_empty();
        match (open_nonempty, market_nonempty) {
            (true, true) => both_nonempty += 1,
            (false, true) => google_only_nonempty += 1,
            (true, false) => opensonarx_only_nonempty += 1,
            (false, false) => neither_nonempty += 1,
        }
        if !open_nonempty || !market_nonempty {
            missing_cases.push(MissingCase {
                id: query.id.clone(),
                query: query.query.clone(),
                opensonarx_nonempty: open_nonempty,
                google_nonempty: market_nonempty,
            });
        }
        if open_nonempty
            && market_nonempty
            && host(&open.results[0].url) == host(&market.results[0].url)
        {
            same_top1_host += 1;
        }
        let status = open
            .answer
            .as_ref()
            .map(|answer| answer.status.as_str())
            .unwrap_or("error");
        *answer_status_counts.entry(status.to_string()).or_insert(0) += 1;
        if matches!(status, "extractive" | "destination" | "tool_results") {
            opensonarx_substantive_answers += 1;
        }
        if status == "insufficient_evidence" {
            opensonarx_explicit_abstentions += 1;
        }
        if open.answer.as_ref().is_some_and(|answer| {
            answer.uncertainty_mode != "not_applicable" && answer.uncertainty_mode != "not_run"
        }) || open.results.iter().any(|result| {
            result.uncertainty_mode != "not_applicable" && result.uncertainty_mode != "not_run"
        }) {
            opensonarx_uncertainty_applicable_queries += 1;
        }
    }

    let opensonarx_coverage = coverage(
        manifest.queries.len(),
        opensonarx
            .captures
            .iter()
            .map(|capture| capture.results.iter().map(|result| result.url.as_str())),
    );
    let google_coverage = coverage(
        manifest.queries.len(),
        google
            .cases
            .iter()
            .map(|capture| capture.results.iter().map(|result| result.url.as_str())),
    );
    let open_latencies = opensonarx
        .captures
        .iter()
        .map(|capture| capture.search_ms)
        .collect();
    let google_latencies = google.cases.iter().map(|capture| capture.wall_ms).collect();

    Report {
        experiment: "sealed-unjudged-generalization-objective-comparison-v1",
        manifest_name: manifest.name,
        manifest_path: manifest_path.to_string(),
        opensonarx_report_path: opensonarx_report_path.to_string(),
        google_report_path: google_report_path.to_string(),
        manifest_sha256: opensonarx.manifest_sha256,
        queries: manifest.queries.len(),
        opensonarx: opensonarx_coverage,
        google: google_coverage,
        both_nonempty,
        google_only_nonempty,
        opensonarx_only_nonempty,
        neither_nonempty,
        same_top1_host,
        opensonarx_answer_status_counts: answer_status_counts,
        opensonarx_substantive_answers,
        opensonarx_explicit_abstentions,
        opensonarx_uncertainty_applicable_queries,
        google_ai_overview_visible: google
            .cases
            .iter()
            .filter(|capture| capture.ai_overview_visible)
            .count(),
        google_unusual_traffic: google
            .cases
            .iter()
            .filter(|capture| capture.unusual_traffic)
            .count(),
        opensonarx_backend_latency: latency_stats(open_latencies),
        google_browser_latency: latency_stats(google_latencies),
        latency_comparable: false,
        relevance_judged: false,
        trustworthiness_judged: false,
        general_reliability_superiority_proven: false,
        market_superiority_proven: false,
        missing_cases,
        limitations: vec![
            "Coverage and slate diversity are objective observations; relevance and trustworthiness remain unjudged.",
            "OpenSonarX latency is backend search time while Google latency is browser-to-visible-result wall time.",
            "Google AI Overview presence is not evidence that its answer is correct; OpenSonarX abstention is not evidence that its returned slate is reliable.",
            "This stratum contains 48 English MS MARCO answer-seeking queries and does not represent general web traffic.",
            "No product may be tuned against individual failures before a scored, blinded report for this frozen binary is archived.",
        ],
    }
}

fn coverage<'a, I, J>(queries: usize, slates: I) -> Coverage
where
    I: Iterator<Item = J>,
    J: Iterator<Item = &'a str>,
{
    let mut nonempty = 0;
    let mut results = 0;
    let mut domains = 0;
    let mut at_least_three_domains = 0;
    for slate in slates {
        let urls = slate.take(5).collect::<Vec<_>>();
        if !urls.is_empty() {
            nonempty += 1;
        }
        results += urls.len();
        let distinct = urls.iter().map(|url| host(url)).collect::<HashSet<_>>();
        domains += distinct.len();
        if distinct.len() >= 3 {
            at_least_three_domains += 1;
        }
    }
    Coverage {
        queries,
        nonempty,
        rate: nonempty as f64 / queries.max(1) as f64,
        mean_results_at_5: results as f64 / queries.max(1) as f64,
        mean_distinct_domains_at_5: domains as f64 / queries.max(1) as f64,
        at_least_three_domains,
    }
}

fn host(url: &str) -> String {
    url.split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(url)
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .trim_start_matches("www.")
        .to_ascii_lowercase()
}

fn latency_stats(mut values: Vec<f64>) -> LatencyStats {
    values.sort_by(f64::total_cmp);
    let count = values.len();
    let percentile = |fraction: f64| {
        let index = ((count as f64 * fraction).ceil() as usize).saturating_sub(1);
        values[index.min(count.saturating_sub(1))]
    };
    LatencyStats {
        count,
        p50_ms: percentile(0.50),
        p95_ms: percentile(0.95),
        p99_ms: percentile(0.99),
    }
}

fn persist_report(path: &str, serialized: &str) -> Result<(), String> {
    if let Some(parent) = std::path::Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("cannot create comparison directory: {error}"))?;
        }
    }
    std::fs::write(path, serialized)
        .map_err(|error| format!("cannot write generalization report {path}: {error}"))
}

fn print_table(report: &Report) {
    println!(
        "sealed unjudged generalization | queries={} | relevance/trust judged=false",
        report.queries
    );
    println!(
        "coverage: OpenSonarX={}/{} ({:.1}%) Google={}/{} ({:.1}%) | Google-only={} Open-only={}",
        report.opensonarx.nonempty,
        report.queries,
        report.opensonarx.rate * 100.0,
        report.google.nonempty,
        report.queries,
        report.google.rate * 100.0,
        report.google_only_nonempty,
        report.opensonarx_only_nonempty
    );
    println!(
        "mean results/domains@5: OpenSonarX={:.2}/{:.2} Google={:.2}/{:.2} | same top1 host={}",
        report.opensonarx.mean_results_at_5,
        report.opensonarx.mean_distinct_domains_at_5,
        report.google.mean_results_at_5,
        report.google.mean_distinct_domains_at_5,
        report.same_top1_host
    );
    println!(
        "answers: OpenSonarX substantive={} abstentions={} uncertainty-applicable={} | Google AI Overview visible={}",
        report.opensonarx_substantive_answers,
        report.opensonarx_explicit_abstentions,
        report.opensonarx_uncertainty_applicable_queries,
        report.google_ai_overview_visible
    );
    println!(
        "latency (not comparable): Open backend p50/p95/p99={:.1}/{:.1}/{:.1}ms; Google browser={:.1}/{:.1}/{:.1}ms",
        report.opensonarx_backend_latency.p50_ms,
        report.opensonarx_backend_latency.p95_ms,
        report.opensonarx_backend_latency.p99_ms,
        report.google_browser_latency.p50_ms,
        report.google_browser_latency.p95_ms,
        report.google_browser_latency.p99_ms
    );
    println!("general_reliability_superiority_proven=false");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage_counts_empty_and_duplicate_domain_slates() {
        let slates = [
            vec![
                "https://one.test/a",
                "https://one.test/b",
                "https://two.test/c",
            ],
            vec![],
        ];
        let metrics = coverage(2, slates.iter().map(|slate| slate.iter().copied()));
        assert_eq!(metrics.nonempty, 1);
        assert_eq!(metrics.mean_results_at_5, 1.5);
        assert_eq!(metrics.mean_distinct_domains_at_5, 1.0);
        assert_eq!(metrics.at_least_three_domains, 0);
    }

    #[test]
    fn host_normalization_ignores_www_and_paths() {
        assert_eq!(host("https://www.Example.com/a?q=1"), "example.com");
    }
}
