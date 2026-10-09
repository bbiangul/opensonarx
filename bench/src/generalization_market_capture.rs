use oi_types::SearchEngine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

const MIN_HOLDOUT_QUERIES: usize = 40;

#[derive(Debug, Deserialize)]
struct Manifest {
    schema_version: u32,
    name: String,
    queries: Vec<ManifestQuery>,
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
    title: String,
    url: String,
    snippet: String,
    source: String,
}

#[derive(Debug, Serialize)]
struct QueryCapture {
    manifest: ManifestQuery,
    search_ms: f64,
    nonempty: bool,
    results: Vec<CapturedResult>,
}

#[derive(Debug, Serialize)]
struct Stats {
    count: usize,
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
    manifest_name: String,
    engine: String,
    live_network: bool,
    mock_results: bool,
    judged_during_capture: bool,
    top_k: usize,
    concurrency: usize,
    query_count: usize,
    nonempty: usize,
    complete_coverage_gate_passed: bool,
    latency: Stats,
    wall_ms: f64,
    throughput_qps: f64,
    cases: Vec<QueryCapture>,
    limitations: Vec<&'static str>,
}

pub fn run(
    manifest_path: &str,
    engine_name: &str,
    top_k: usize,
    concurrency: usize,
    output: &str,
    report_path: Option<&str>,
) -> Result<(), String> {
    if top_k == 0 || concurrency == 0 {
        return Err("top-k and concurrency must be positive".to_string());
    }
    let manifest_bytes = std::fs::read(manifest_path)
        .map_err(|error| format!("cannot read holdout manifest {manifest_path}: {error}"))?;
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| format!("cannot parse holdout manifest {manifest_path}: {error}"))?;
    validate_manifest(&manifest)?;
    let normalized_engine = engine_name.trim().to_ascii_lowercase();
    let engine = crate::engine_probe::build_engine(&normalized_engine)
        .ok_or_else(|| format!("unknown or uncredentialed market engine: {engine_name}"))?;

    let started = Instant::now();
    let mut cases = Vec::with_capacity(manifest.queries.len());
    for batch in manifest.queries.chunks(concurrency) {
        let mut captured = std::thread::scope(|scope| {
            let barrier = Arc::new(std::sync::Barrier::new(batch.len()));
            let handles = batch
                .iter()
                .map(|query| {
                    let barrier = Arc::clone(&barrier);
                    let engine = Arc::clone(&engine);
                    scope.spawn(move || {
                        barrier.wait();
                        capture_query(engine.as_ref(), query, top_k)
                    })
                })
                .collect::<Vec<_>>();
            handles
                .into_iter()
                .map(|handle| {
                    handle
                        .join()
                        .map_err(|_| "market capture worker panicked".to_string())
                })
                .collect::<Result<Vec<_>, String>>()
        })?;
        cases.append(&mut captured);
    }
    let wall_ms = started.elapsed().as_secs_f64() * 1_000.0;
    let nonempty = cases.iter().filter(|case| case.nonempty).count();
    let report = CaptureReport {
        captured_at_unix_ms: unix_time_ms(),
        experiment: "sealed-unjudged-market-generalization-capture-v1",
        manifest_path: manifest_path.to_string(),
        manifest_sha256: sha256_bytes(&manifest_bytes),
        manifest_name: manifest.name,
        engine: normalized_engine,
        live_network: true,
        mock_results: false,
        judged_during_capture: false,
        top_k,
        concurrency,
        query_count: manifest.queries.len(),
        nonempty,
        complete_coverage_gate_passed: nonempty == manifest.queries.len(),
        latency: stats(cases.iter().map(|case| case.search_ms).collect()),
        wall_ms,
        throughput_qps: cases.len() as f64 / (wall_ms / 1_000.0).max(f64::EPSILON),
        cases,
        limitations: vec![
            "This is a raw market-provider capture, not a Google UI or AI-answer capture.",
            "The artifact is intentionally unjudged; quality is evaluated only after both systems are frozen and pooled.",
            "An empty result vector cannot distinguish a legitimate zero-result query from every upstream failure mode.",
        ],
    };
    let serialized = serde_json::to_string_pretty(&report)
        .map_err(|error| format!("cannot serialize market capture: {error}"))?;
    if let Some(path) = report_path {
        persist(path, &serialized)?;
    }
    match output {
        "json" => println!("{serialized}"),
        "table" => print_table(&report),
        other => return Err(format!("unsupported market capture output: {other}")),
    }
    if nonempty != manifest.queries.len() {
        return Err(format!(
            "market capture coverage gate failed: {nonempty}/{} queries returned valid results",
            manifest.queries.len()
        ));
    }
    Ok(())
}

fn capture_query(engine: &dyn SearchEngine, query: &ManifestQuery, top_k: usize) -> QueryCapture {
    let started = Instant::now();
    let results = engine
        .search(&query.query, top_k)
        .into_iter()
        .enumerate()
        .filter(|(_, result)| {
            result.url.starts_with("https://") || result.url.starts_with("http://")
        })
        .map(|(index, result)| CapturedResult {
            rank: if result.rank == 0 {
                index as u32 + 1
            } else {
                result.rank
            },
            title: result.title,
            url: result.url,
            snippet: result.snippet,
            source: result.source,
        })
        .collect::<Vec<_>>();
    QueryCapture {
        manifest: query.clone(),
        search_ms: started.elapsed().as_secs_f64() * 1_000.0,
        nonempty: !results.is_empty(),
        results,
    }
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
            "market holdout has {} queries; at least {MIN_HOLDOUT_QUERIES} are required",
            manifest.queries.len()
        ));
    }
    let mut ids = HashSet::new();
    let mut queries = HashSet::new();
    for query in &manifest.queries {
        if !ids.insert(query.id.as_str()) {
            return Err(format!("duplicate holdout id: {}", query.id));
        }
        let normalized = normalize_query(&query.query);
        if normalized.is_empty() || !queries.insert(normalized) {
            return Err(format!("duplicate or empty holdout query: {}", query.query));
        }
    }
    Ok(())
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

fn stats(mut values: Vec<f64>) -> Stats {
    values.sort_by(f64::total_cmp);
    let count = values.len();
    let percentile = |fraction: f64| {
        let index = ((count as f64 * fraction).ceil() as usize).saturating_sub(1);
        values[index.min(count.saturating_sub(1))]
    };
    Stats {
        count,
        p50_ms: percentile(0.50),
        p95_ms: percentile(0.95),
        p99_ms: percentile(0.99),
        max_ms: values.last().copied().unwrap_or_default(),
    }
}

fn sha256_bytes(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn persist(path: &str, serialized: &str) -> Result<(), String> {
    if let Some(parent) = std::path::Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("cannot create market capture directory: {error}"))?;
        }
    }
    std::fs::write(path, serialized)
        .map_err(|error| format!("cannot write market capture {path}: {error}"))
}

fn unix_time_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn print_table(report: &CaptureReport) {
    println!(
        "sealed unjudged market capture | engine={} queries={} nonempty={} mocks=false",
        report.engine, report.query_count, report.nonempty
    );
    println!(
        "latency p50/p95/p99={:.1}/{:.1}/{:.1}ms | throughput={:.2}qps | judged=false",
        report.latency.p50_ms, report.latency.p95_ms, report.latency.p99_ms, report.throughput_qps
    );
    println!("manifest sha256: {}", report.manifest_sha256);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn market_capture_manifest_requires_unique_large_holdout() {
        let mut manifest = Manifest {
            schema_version: 1,
            name: "test".to_string(),
            queries: (0..MIN_HOLDOUT_QUERIES)
                .map(|index| ManifestQuery {
                    id: format!("q-{index}"),
                    query: format!("independent query {index}"),
                    source: "test".to_string(),
                    source_id: index.to_string(),
                    language: "en".to_string(),
                    locale: "en-US".to_string(),
                    country: "US".to_string(),
                    stratum: "test".to_string(),
                })
                .collect(),
        };
        validate_manifest(&manifest).expect("valid manifest");
        manifest.queries[1].id = manifest.queries[0].id.clone();
        assert!(validate_manifest(&manifest).is_err());
    }
}
