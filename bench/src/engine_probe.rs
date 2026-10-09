use oi_types::SearchEngine;
use serde::Serialize;
use std::sync::Arc;
use std::time::Instant;

#[derive(Serialize)]
struct ProbeAttempt {
    engine: String,
    round: usize,
    query: String,
    latency_ms: f64,
    result_count: usize,
    valid_url_count: usize,
    relevant_result_count: usize,
    top_urls: Vec<String>,
    top_relevant_urls: Vec<String>,
    healthy: bool,
}

#[derive(Serialize)]
struct EngineSummary {
    engine: String,
    successful_queries: usize,
    total_queries: usize,
    healthy: bool,
    latency_p50_ms: f64,
    latency_max_ms: f64,
}

#[derive(Serialize)]
struct ProbeReport {
    captured_at_unix_ms: u128,
    live_network: bool,
    mock_results: bool,
    required_healthy_engines: usize,
    healthy_engines: usize,
    rounds: usize,
    concurrency: usize,
    queries: Vec<String>,
    attempts: Vec<ProbeAttempt>,
    engines: Vec<EngineSummary>,
}

pub fn run(
    engine_names: &str,
    queries: &[String],
    limit: usize,
    rounds: usize,
    concurrency: usize,
    min_healthy: usize,
    output: &str,
    report_path: Option<&str>,
) -> Result<(), String> {
    if queries.is_empty() {
        return Err("engine probe requires at least one query".to_string());
    }
    if limit == 0 {
        return Err("engine probe limit must be greater than zero".to_string());
    }
    if rounds == 0 {
        return Err("engine probe rounds must be greater than zero".to_string());
    }
    if concurrency == 0 {
        return Err("engine probe concurrency must be greater than zero".to_string());
    }

    let selected = engine_names
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| {
            build_engine(name)
                .map(|engine| (name.to_ascii_lowercase(), engine))
                .ok_or_else(|| format!("unknown or credentialed engine: {name}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if selected.is_empty() {
        return Err("engine probe requires at least one engine".to_string());
    }

    // Providers run concurrently. Within each provider, requests run in
    // explicit bounded batches so serial stability and production-like burst
    // capacity can be measured as separate artifacts.
    let attempts = std::thread::scope(|scope| {
        let handles = selected
            .into_iter()
            .map(|(name, engine)| {
                scope.spawn(move || {
                    let work = (1..=rounds)
                        .flat_map(|round| queries.iter().cloned().map(move |query| (round, query)))
                        .collect::<Vec<_>>();
                    let mut engine_attempts = Vec::with_capacity(work.len());
                    for batch in work.chunks(concurrency) {
                        let batch_attempts = std::thread::scope(|batch_scope| {
                            let barrier = Arc::new(std::sync::Barrier::new(batch.len()));
                            let handles = batch
                                .iter()
                                .cloned()
                                .map(|(round, query)| {
                                    let name = name.clone();
                                    let engine = Arc::clone(&engine);
                                    let barrier = Arc::clone(&barrier);
                                    batch_scope.spawn(move || {
                                        barrier.wait();
                                        probe_attempt(&name, engine.as_ref(), round, query, limit)
                                    })
                                })
                                .collect::<Vec<_>>();
                            handles
                                .into_iter()
                                .filter_map(|handle| handle.join().ok())
                                .collect::<Vec<_>>()
                        });
                        engine_attempts.extend(batch_attempts);
                    }
                    engine_attempts
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().unwrap_or_default())
            .collect::<Vec<_>>()
    });

    let mut summaries = attempts
        .iter()
        .map(|attempt| attempt.engine.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .map(|engine| {
            let engine_attempts = attempts
                .iter()
                .filter(|attempt| attempt.engine == engine)
                .collect::<Vec<_>>();
            let successful_queries = engine_attempts
                .iter()
                .filter(|attempt| attempt.healthy)
                .count();
            let mut latencies = engine_attempts
                .iter()
                .map(|attempt| attempt.latency_ms)
                .collect::<Vec<_>>();
            latencies.sort_by(f64::total_cmp);
            EngineSummary {
                engine,
                successful_queries,
                total_queries: engine_attempts.len(),
                // A production fallback must work across the whole probe set;
                // one lucky head query is not provider health.
                healthy: successful_queries == engine_attempts.len(),
                latency_p50_ms: percentile(&latencies, 0.50),
                latency_max_ms: latencies.last().copied().unwrap_or(0.0),
            }
        })
        .collect::<Vec<_>>();
    summaries.sort_by(|left, right| left.engine.cmp(&right.engine));

    let healthy_engines = summaries.iter().filter(|engine| engine.healthy).count();
    let report = ProbeReport {
        captured_at_unix_ms: unix_time_ms(),
        live_network: true,
        mock_results: false,
        required_healthy_engines: min_healthy,
        healthy_engines,
        rounds,
        concurrency,
        queries: queries.to_vec(),
        attempts,
        engines: summaries,
    };
    let serialized = serde_json::to_string_pretty(&report)
        .map_err(|error| format!("cannot serialize engine probe: {error}"))?;
    if let Some(path) = report_path {
        persist_report(path, &serialized, "engine probe")?;
    }
    match output {
        "json" => println!("{serialized}"),
        "table" => print_table(&report),
        other => return Err(format!("unknown engine-probe output format: {other}")),
    }
    if healthy_engines < min_healthy {
        return Err(format!(
            "provider redundancy gate failed: {healthy_engines} healthy engines, {min_healthy} required"
        ));
    }
    Ok(())
}

fn unix_time_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn persist_report(path: &str, serialized: &str, label: &str) -> Result<(), String> {
    if let Some(parent) = std::path::Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("cannot create {label} report directory: {error}"))?;
        }
    }
    std::fs::write(path, serialized)
        .map_err(|error| format!("cannot write {label} report {path}: {error}"))
}

pub(crate) fn build_engine(name: &str) -> Option<Arc<dyn SearchEngine>> {
    match name.to_ascii_lowercase().as_str() {
        "bing" => Some(Arc::new(oi_sdk::engines::bing::BingScraper)),
        "brave" => std::env::var("BRAVE_API_KEY")
            .ok()
            .filter(|key| !key.trim().is_empty())
            .map(|key| Arc::new(oi_sdk::engines::brave::BraveSearch::new(key)) as Arc<_>),
        "brave-html" | "brave_html" => Some(Arc::new(oi_sdk::engines::brave_html::BraveHtml)),
        "duckduckgo" | "ddg" => Some(Arc::new(oi_sdk::engines::duckduckgo::DuckDuckGo)),
        "google" => Some(Arc::new(oi_sdk::engines::google::GoogleScraper)),
        "qwant" => Some(Arc::new(oi_sdk::engines::qwant::Qwant)),
        "serper" => std::env::var("SERPER_API_KEY")
            .ok()
            .filter(|key| !key.trim().is_empty())
            .map(|key| Arc::new(oi_sdk::engines::serper::Serper::new(key)) as Arc<_>),
        "stract" => Some(Arc::new(oi_sdk::engines::stract::Stract)),
        "wikipedia" => Some(Arc::new(oi_sdk::engines::wikipedia::WikipediaSearch)),
        "yep" => Some(Arc::new(oi_sdk::engines::yep::Yep)),
        _ => None,
    }
}

fn percentile(values: &[f64], quantile: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let index = ((values.len() - 1) as f64 * quantile).round() as usize;
    values[index]
}

fn print_table(report: &ProbeReport) {
    println!(
        "live provider probe | mocks=false | queries={} rounds={} engines={} healthy={}/{} required",
        report.queries.len(),
        report.rounds,
        report.engines.len(),
        report.healthy_engines,
        report.required_healthy_engines
    );
    println!("provider request concurrency={}", report.concurrency);
    println!(
        "{:<14} {:>8} {:>12} {:>12}  top URL",
        "engine", "health", "p50", "max"
    );
    for engine in &report.engines {
        let top_url = report
            .attempts
            .iter()
            .find(|attempt| attempt.engine == engine.engine && attempt.healthy)
            .and_then(|attempt| attempt.top_relevant_urls.first())
            .map(String::as_str)
            .unwrap_or("-");
        println!(
            "{:<14} {:>3}/{:<4} {:>9.1}ms {:>9.1}ms  {}",
            engine.engine,
            engine.successful_queries,
            engine.total_queries,
            engine.latency_p50_ms,
            engine.latency_max_ms,
            top_url
        );
    }
    for attempt in report.attempts.iter().filter(|attempt| !attempt.healthy) {
        println!(
            "FAIL engine={} round={} query={:?} latency={:.1}ms results={} valid_urls={} relevant_results={} top_urls={:?}",
            attempt.engine,
            attempt.round,
            attempt.query,
            attempt.latency_ms,
            attempt.result_count,
            attempt.valid_url_count,
            attempt.relevant_result_count,
            attempt.top_urls,
        );
    }
}

fn probe_attempt(
    name: &str,
    engine: &dyn SearchEngine,
    round: usize,
    query: String,
    limit: usize,
) -> ProbeAttempt {
    let started = Instant::now();
    let results = engine.search(&query, limit);
    let latency_ms = started.elapsed().as_secs_f64() * 1_000.0;
    let valid_urls = results
        .iter()
        .filter(|result| result.url.starts_with("https://") || result.url.starts_with("http://"))
        .collect::<Vec<_>>();
    let relevant_urls = valid_urls
        .iter()
        .filter(|result| oi_sdk::engines::provider_surface_topic_relevant(&query, result))
        .collect::<Vec<_>>();
    ProbeAttempt {
        engine: name.to_string(),
        round,
        query,
        latency_ms,
        result_count: results.len(),
        valid_url_count: valid_urls.len(),
        relevant_result_count: relevant_urls.len(),
        top_urls: valid_urls
            .iter()
            .take(3)
            .map(|result| result.url.clone())
            .collect(),
        top_relevant_urls: relevant_urls
            .iter()
            .take(3)
            .map(|result| result.url.clone())
            .collect(),
        // Syntactically valid URLs are not enough: malformed provider markup
        // can parse an unrelated page into a plausible-looking result vector.
        healthy: !relevant_urls.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_is_deterministic_for_small_probe_sets() {
        assert_eq!(percentile(&[], 0.50), 0.0);
        assert_eq!(percentile(&[1.0, 2.0, 9.0], 0.50), 2.0);
        assert_eq!(percentile(&[1.0, 2.0, 9.0], 1.00), 9.0);
    }

    #[test]
    fn engine_selector_rejects_unknown_and_accepts_keyless_adapters() {
        assert!(build_engine("unknown").is_none());
        for name in [
            "bing",
            "brave-html",
            "ddg",
            "google",
            "qwant",
            "stract",
            "wikipedia",
            "yep",
        ] {
            assert!(build_engine(name).is_some(), "engine={name}");
        }
    }
}
