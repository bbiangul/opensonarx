use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::File;
use std::io::BufReader;

#[derive(Debug, Deserialize)]
struct Fixture {
    schema_version: u32,
    capture: Capture,
    protocols: Protocols,
    cases: Vec<Case>,
    exact_retry_after_capture: ExactRetry,
    claimed_comparable: bool,
}

#[derive(Debug, Deserialize, Serialize)]
struct Capture {
    captured_at: String,
    locale: String,
    country: String,
    google_session: String,
    google_url_template: String,
    opensonarx_report_path: String,
    opensonarx_data_dir: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct Protocols {
    google: Protocol,
    opensonarx: Protocol,
}

#[derive(Debug, Deserialize, Serialize)]
struct Protocol {
    measured_surface: String,
    execution: String,
    rounds: usize,
    stop_condition: String,
}

#[derive(Debug, Deserialize)]
struct Case {
    id: String,
    intent: String,
    query_relation: QueryRelation,
    google_query: String,
    opensonarx_query: String,
    google_samples: Vec<GoogleSample>,
    opensonarx_samples_ms: Vec<f64>,
    opensonarx_quality_passes: usize,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum QueryRelation {
    Exact,
    LocalizedEquivalent,
}

#[derive(Debug, Deserialize)]
struct GoogleSample {
    latency_ms: f64,
    organic_ready: bool,
}

#[derive(Debug, Deserialize, Serialize)]
struct ExactRetry {
    query: String,
    attempts: usize,
    successful_organic_results: usize,
    observed_outcome: String,
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
struct CaseReport {
    id: String,
    intent: String,
    query_relation: &'static str,
    google_query: String,
    opensonarx_query: String,
    google_organic_ready: usize,
    google: Stats,
    opensonarx_quality_passes: usize,
    opensonarx: Stats,
}

#[derive(Debug, Serialize)]
struct Report {
    experiment: &'static str,
    fixture_path: String,
    capture: Capture,
    protocols: Protocols,
    cases: usize,
    exact_query_cases: usize,
    localized_equivalent_cases: usize,
    google_organic_ready: usize,
    opensonarx_quality_passes: usize,
    google_all_cases: Stats,
    opensonarx_all_cases: Stats,
    google_exact_query_subset: Stats,
    opensonarx_exact_query_subset: Stats,
    exact_retry_after_capture: ExactRetry,
    market_latency_compared: bool,
    cases_detail: Vec<CaseReport>,
    limitations: Vec<&'static str>,
}

pub fn run(fixture_path: &str, output: &str, report_path: Option<&str>) -> Result<(), String> {
    let fixture: Fixture = load_json(fixture_path)?;
    validate(&fixture)?;
    let report = evaluate(fixture, fixture_path);
    let serialized = serde_json::to_string_pretty(&report)
        .map_err(|error| format!("cannot serialize market latency report: {error}"))?;
    if let Some(path) = report_path {
        persist_report(path, &serialized)?;
    }

    match output {
        "json" => println!("{serialized}"),
        "table" => print_table(&report),
        other => return Err(format!("unsupported market-latency output format: {other}")),
    }
    Ok(())
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
                .map_err(|error| format!("cannot create latency report directory: {error}"))?;
        }
    }
    std::fs::write(path, serialized)
        .map_err(|error| format!("cannot write latency report {path}: {error}"))
}

fn validate(fixture: &Fixture) -> Result<(), String> {
    if fixture.schema_version != 1 {
        return Err(format!(
            "unsupported market latency schema {}; expected 1",
            fixture.schema_version
        ));
    }
    if fixture.claimed_comparable {
        return Err("exploratory fixture must not claim comparable latency".to_string());
    }
    if fixture.protocols.google.measured_surface == fixture.protocols.opensonarx.measured_surface {
        return Err(
            "fixture must describe the two actually different measured surfaces".to_string(),
        );
    }
    if fixture.protocols.google.rounds == 0
        || fixture.protocols.opensonarx.rounds == 0
        || fixture.cases.is_empty()
    {
        return Err("market latency fixture requires cases and non-zero rounds".to_string());
    }

    let mut ids = HashSet::new();
    for case in &fixture.cases {
        if !ids.insert(&case.id) {
            return Err(format!("duplicate market latency case: {}", case.id));
        }
        if case.google_samples.len() != fixture.protocols.google.rounds {
            return Err(format!(
                "{} has {} Google samples; expected {}",
                case.id,
                case.google_samples.len(),
                fixture.protocols.google.rounds
            ));
        }
        if case.opensonarx_samples_ms.len() != fixture.protocols.opensonarx.rounds {
            return Err(format!(
                "{} has {} OpenSonarX samples; expected {}",
                case.id,
                case.opensonarx_samples_ms.len(),
                fixture.protocols.opensonarx.rounds
            ));
        }
        if case
            .google_samples
            .iter()
            .any(|sample| !sample.latency_ms.is_finite() || sample.latency_ms <= 0.0)
            || case
                .opensonarx_samples_ms
                .iter()
                .any(|sample| !sample.is_finite() || *sample < 0.0)
        {
            return Err(format!("{} contains an invalid latency sample", case.id));
        }
        if case
            .google_samples
            .iter()
            .any(|sample| !sample.organic_ready)
        {
            return Err(format!(
                "{} contains a Google sample without an organic result",
                case.id
            ));
        }
        if case.opensonarx_quality_passes != fixture.protocols.opensonarx.rounds {
            return Err(format!(
                "{} has {}/{} OpenSonarX quality passes; latency audit requires every counted sample to pass quality",
                case.id, case.opensonarx_quality_passes, fixture.protocols.opensonarx.rounds
            ));
        }
        match case.query_relation {
            QueryRelation::Exact if case.google_query != case.opensonarx_query => {
                return Err(format!(
                    "{} is marked exact but its queries differ",
                    case.id
                ));
            }
            QueryRelation::LocalizedEquivalent if case.google_query == case.opensonarx_query => {
                return Err(format!(
                    "{} is marked localized-equivalent but its queries are identical",
                    case.id
                ));
            }
            _ => {}
        }
    }
    if !fixture
        .cases
        .iter()
        .any(|case| case.query_relation == QueryRelation::LocalizedEquivalent)
    {
        return Err("fixture must preserve the observed localized-query mismatch".to_string());
    }
    if fixture.exact_retry_after_capture.attempts == 0
        || fixture.exact_retry_after_capture.successful_organic_results
            > fixture.exact_retry_after_capture.attempts
    {
        return Err("invalid exact-query retry audit".to_string());
    }
    Ok(())
}

fn evaluate(fixture: Fixture, fixture_path: &str) -> Report {
    let mut google_all = Vec::new();
    let mut opensonarx_all = Vec::new();
    let mut google_exact = Vec::new();
    let mut opensonarx_exact = Vec::new();
    let mut google_ready = 0;
    let mut opensonarx_quality_passes = 0;
    let mut cases_detail = Vec::new();

    for case in fixture.cases {
        let google_times = case
            .google_samples
            .iter()
            .map(|sample| sample.latency_ms)
            .collect::<Vec<_>>();
        let case_ready = case
            .google_samples
            .iter()
            .filter(|sample| sample.organic_ready)
            .count();
        google_ready += case_ready;
        opensonarx_quality_passes += case.opensonarx_quality_passes;
        google_all.extend(&google_times);
        opensonarx_all.extend(&case.opensonarx_samples_ms);
        if case.query_relation == QueryRelation::Exact {
            google_exact.extend(&google_times);
            opensonarx_exact.extend(&case.opensonarx_samples_ms);
        }
        cases_detail.push(CaseReport {
            id: case.id,
            intent: case.intent,
            query_relation: match case.query_relation {
                QueryRelation::Exact => "exact",
                QueryRelation::LocalizedEquivalent => "localized_equivalent",
            },
            google_query: case.google_query,
            opensonarx_query: case.opensonarx_query,
            google_organic_ready: case_ready,
            google: stats(google_times),
            opensonarx_quality_passes: case.opensonarx_quality_passes,
            opensonarx: stats(case.opensonarx_samples_ms),
        });
    }

    let exact_query_cases = cases_detail
        .iter()
        .filter(|case| case.query_relation == "exact")
        .count();
    Report {
        experiment: "google-browser-vs-opensonarx-backend-latency-exploratory-v1",
        fixture_path: fixture_path.to_string(),
        capture: fixture.capture,
        protocols: fixture.protocols,
        cases: cases_detail.len(),
        exact_query_cases,
        localized_equivalent_cases: cases_detail.len() - exact_query_cases,
        google_organic_ready: google_ready,
        opensonarx_quality_passes,
        google_all_cases: stats(google_all),
        opensonarx_all_cases: stats(opensonarx_all),
        google_exact_query_subset: stats(google_exact),
        opensonarx_exact_query_subset: stats(opensonarx_exact),
        exact_retry_after_capture: fixture.exact_retry_after_capture,
        market_latency_compared: false,
        cases_detail,
        limitations: vec![
            "Google measures browser wall time through network and DOM rendering; OpenSonarX measures backend search_ms only.",
            "Google ran sequentially in a signed-out browser session; OpenSonarX ran three passes at concurrency four over a retained corpus.",
            "Eight queries are text-identical; the tool query is only a localized semantic equivalent.",
            "A later exact tool-query retry was blocked by Google's unusual-traffic page and is not counted as a latency sample.",
            "These measurements do not support a market-latency winner or Google-scale claim.",
        ],
    }
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

fn print_table(report: &Report) {
    println!(
        "exploratory market latency | cases={} exact={} localized={} | comparable=false",
        report.cases, report.exact_query_cases, report.localized_equivalent_cases
    );
    println!(
        "Google browser wall→organic: n={} ready={} p50={:.1}ms p95={:.1}ms p99={:.1}ms",
        report.google_all_cases.count,
        report.google_organic_ready,
        report.google_all_cases.p50_ms,
        report.google_all_cases.p95_ms,
        report.google_all_cases.p99_ms
    );
    println!(
        "OpenSonarX backend search_ms: n={} quality={} p50={:.3}ms p95={:.3}ms p99={:.3}ms",
        report.opensonarx_all_cases.count,
        report.opensonarx_quality_passes,
        report.opensonarx_all_cases.p50_ms,
        report.opensonarx_all_cases.p95_ms,
        report.opensonarx_all_cases.p99_ms
    );
    println!(
        "Exact-query subset: Google p50/p95/p99={:.1}/{:.1}/{:.1}ms; OpenSonarX={:.3}/{:.3}/{:.3}ms",
        report.google_exact_query_subset.p50_ms,
        report.google_exact_query_subset.p95_ms,
        report.google_exact_query_subset.p99_ms,
        report.opensonarx_exact_query_subset.p50_ms,
        report.opensonarx_exact_query_subset.p95_ms,
        report.opensonarx_exact_query_subset.p99_ms
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Fixture {
        serde_json::from_str(include_str!(
            "../fixtures/google-vs-opensonarx-latency-exploratory-2026-08-22.json"
        ))
        .expect("versioned market latency fixture")
    }

    #[test]
    fn versioned_fixture_is_fail_closed_and_recomputes_expected_percentiles() {
        let fixture = fixture();
        validate(&fixture).expect("valid fixture");
        let report = evaluate(fixture, "fixture.json");
        assert_eq!(report.cases, 9);
        assert_eq!(report.exact_query_cases, 8);
        assert_eq!(report.localized_equivalent_cases, 1);
        assert_eq!(report.google_organic_ready, 27);
        assert_eq!(report.opensonarx_quality_passes, 27);
        assert_eq!(report.google_all_cases.p50_ms, 215.0);
        assert_eq!(report.google_all_cases.p95_ms, 407.0);
        assert_eq!(report.google_all_cases.p99_ms, 414.0);
        assert!((report.opensonarx_all_cases.p50_ms - 1.957417).abs() < 1e-9);
        assert!((report.opensonarx_all_cases.p95_ms - 31.020208).abs() < 1e-9);
        assert!((report.opensonarx_all_cases.p99_ms - 2400.03525).abs() < 1e-9);
        assert!(!report.market_latency_compared);
    }

    #[test]
    fn exact_label_rejects_different_query_text() {
        let mut fixture = fixture();
        let localized = fixture
            .cases
            .iter_mut()
            .find(|case| case.query_relation == QueryRelation::LocalizedEquivalent)
            .expect("localized case");
        localized.query_relation = QueryRelation::Exact;
        assert!(validate(&fixture)
            .expect_err("mismatched exact query must fail")
            .contains("marked exact"));
    }
}
