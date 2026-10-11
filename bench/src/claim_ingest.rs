//! Ingestion-time claims over the passages of `answer-live` reports, and the
//! corroboration signal they give each query with no model call.
//!
//! `claim-ingest` runs the SDK's ingestion step (`oi_sdk::claims`) on every
//! retained passage of the given reports, once per distinct passage, the way
//! a node would when the page entered its index. `claim-signal` then builds
//! the claim index and, for each query of the reports, embeds the query and
//! reads the corroboration signal, timing only that query-side work.
//!
//! The passages are the ones those same queries retrieved, so every query is
//! covered by the index. This measures the signal and its cost when the
//! index has the pages; it says nothing about coverage.

use oi_sdk::claims::{ingest_passage, ClaimIndex, ClaimPolicy, ClaimRecord};
use oi_sdk::{AnswerModelConfig, ChatModel};
use oi_types::EvidencePassage;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;

pub(crate) fn read_report(path: &str) -> Result<Value, String> {
    let raw =
        std::fs::read_to_string(path).map_err(|error| format!("cannot read {path}: {error}"))?;
    serde_json::from_str(&raw).map_err(|error| format!("{path}: {error}"))
}

pub(crate) fn stem(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(path)
        .to_string()
}

fn passage_key(url: &str, text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(url.as_bytes());
    hasher.update(b"\n");
    hasher.update(text.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Every distinct retained passage of the reports, keyed by URL and text.
pub(crate) fn retained_passages(
    reports: &[String],
) -> Result<BTreeMap<String, EvidencePassage>, String> {
    let mut passages = BTreeMap::new();
    for path in reports {
        let report = read_report(path)?;
        for result in report["results"].as_array().into_iter().flatten() {
            for passage in result["passages"].as_array().into_iter().flatten() {
                if passage["retained"] != true {
                    continue;
                }
                let url = passage["url"].as_str().unwrap_or_default();
                let text = passage["text"].as_str().unwrap_or_default();
                if url.is_empty() || text.trim().is_empty() {
                    continue;
                }
                passages
                    .entry(passage_key(url, text))
                    .or_insert_with(|| EvidencePassage {
                        id: "p1".to_string(),
                        url: url.to_string(),
                        title: passage["title"].as_str().unwrap_or_default().to_string(),
                        text: text.to_string(),
                        retained: true,
                        published_at: None,
                        observed_at: None,
                    });
            }
        }
    }
    Ok(passages)
}

pub fn ingest(
    reports: &[String],
    provider: &str,
    model: &str,
    concurrency: usize,
    output: &str,
) -> Result<(), String> {
    let passages = retained_passages(reports)?;
    let extractor = ChatModel::new(match provider {
        "ollama" => AnswerModelConfig::new(model),
        "openrouter" => AnswerModelConfig::openrouter(model),
        other => return Err(format!("unknown --provider {other:?}")),
    });
    let policy = ClaimPolicy::default();

    // One line per ingested passage, appended as it finishes, so an
    // interrupted run resumes without repeating model calls.
    let done: BTreeSet<String> = std::fs::read_to_string(output)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|line| line["key"].as_str().map(str::to_string))
        .collect();
    let pending: Vec<(&String, &EvidencePassage)> = passages
        .iter()
        .filter(|(key, _)| !done.contains(*key))
        .collect();
    eprintln!(
        "{} distinct retained passages, {} already ingested, {} to go",
        passages.len(),
        done.len(),
        pending.len()
    );
    let started = std::time::Instant::now();
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(output)
        .map_err(|error| format!("cannot open {output}: {error}"))?;
    let (mut kept, mut rejected, mut errors) = (0usize, 0usize, 0usize);
    for batch in pending.chunks(concurrency.max(1)) {
        let lines: Vec<Value> = std::thread::scope(|scope| {
            let workers: Vec<_> = batch
                .iter()
                .map(|(key, passage)| {
                    let extractor = &extractor;
                    let policy = &policy;
                    scope.spawn(move || {
                        let started = std::time::Instant::now();
                        match ingest_passage(extractor, passage, policy) {
                            Ok((records, rejected)) => json!({
                                "key": key,
                                "url": passage.url,
                                "records": records,
                                "rejected_quotes": rejected,
                                "ms": started.elapsed().as_secs_f64() * 1_000.0,
                            }),
                            Err(error) => json!({
                                "key": key,
                                "url": passage.url,
                                "records": [],
                                "error": error.to_string(),
                            }),
                        }
                    })
                })
                .collect();
            workers
                .into_iter()
                .map(|worker| {
                    worker
                        .join()
                        .unwrap_or_else(|_| json!({ "error": "panicked" }))
                })
                .collect()
        });
        for line in lines {
            kept += line["records"].as_array().map_or(0, Vec::len);
            rejected += line["rejected_quotes"].as_u64().unwrap_or(0) as usize;
            errors += usize::from(line.get("error").is_some());
            writeln!(file, "{line}").map_err(|error| format!("cannot write {output}: {error}"))?;
        }
    }
    println!(
        "claim-ingest → {output}\n  passages ingested now: {}\n  claims kept: {kept}\n  claims dropped (quote not in passage): {rejected}\n  model errors: {errors}\n  wall time: {:.0} s at concurrency {concurrency}",
        pending.len(),
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

fn percentile(values: &[f64], quantile: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let index = ((sorted.len() as f64 - 1.0) * quantile).round() as usize;
    Some(sorted[index.min(sorted.len() - 1)])
}

fn latency(values: &[f64]) -> Value {
    json!({
        "n": values.len(),
        "p50_ms": percentile(values, 0.50),
        "p95_ms": percentile(values, 0.95),
        "max_ms": percentile(values, 1.0),
    })
}

pub fn signal(
    claims_path: &str,
    reports: &[String],
    policy: ClaimPolicy,
    output: &str,
) -> Result<(), String> {
    let raw = std::fs::read_to_string(claims_path)
        .map_err(|error| format!("cannot read {claims_path}: {error}"))?;
    let records: Vec<ClaimRecord> = raw
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .flat_map(|line| {
            line["records"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter_map(|record| serde_json::from_value(record).ok())
        })
        .collect();
    if records.is_empty() {
        return Err(format!("{claims_path} has no claim records"));
    }

    let embedder = oi_sdk::OpenSonarXClient::default_embedder();
    let started = std::time::Instant::now();
    let mut index = ClaimIndex::new(embedder.dimension());
    // Kept beside the index for the exhaustive comparison below.
    let mut stored: Vec<(ClaimRecord, Vec<f32>)> = Vec::new();
    for batch in records.chunks(64) {
        let texts: Vec<&str> = batch.iter().map(|record| record.claim.as_str()).collect();
        let vectors = embedder
            .embed_text_batch(&texts)
            .map_err(|error| format!("embedding claims: {error}"))?;
        for (record, vector) in batch.iter().zip(vectors) {
            stored.push((record.clone(), vector.clone()));
            index
                .add(record.clone(), vector)
                .map_err(|error| error.to_string())?;
        }
    }
    let build_ms = started.elapsed().as_secs_f64() * 1_000.0;
    eprintln!(
        "{} claims embedded and indexed in {:.0} ms",
        index.len(),
        build_ms
    );

    let mut results = Vec::new();
    let (mut embed_times, mut lookup_times, mut total_times) = (Vec::new(), Vec::new(), Vec::new());
    for path in reports {
        let report = read_report(path)?;
        let report_stem = stem(path);
        for result in report["results"].as_array().into_iter().flatten() {
            let query = result["query"].as_str().unwrap_or_default();
            let started = std::time::Instant::now();
            let vector = embedder
                .embed_query(query)
                .map_err(|error| format!("embedding query: {error}"))?;
            let embed_ms = started.elapsed().as_secs_f64() * 1_000.0;
            let started = std::time::Instant::now();
            let corroboration = index.corroboration(&vector, &policy);
            let lookup_ms = started.elapsed().as_secs_f64() * 1_000.0;
            // Not timed: the same signal from an exhaustive scan of every
            // stored claim, to see what the approximate index misses.
            let exact = oi_sdk::claims::corroboration(
                stored
                    .iter()
                    .map(|(record, stored_vector)| {
                        (
                            oi_sdk::claims::MatchedClaim {
                                claim: record.claim.clone(),
                                quote: record.quote.clone(),
                                url: record.url.clone(),
                                source_family: record.source_family.clone(),
                                query_similarity: oi_types::claims::cosine(&vector, stored_vector),
                                head_similarity: 0.0,
                            },
                            stored_vector.clone(),
                        )
                    })
                    .collect(),
                &policy,
            );
            embed_times.push(embed_ms);
            lookup_times.push(lookup_ms);
            total_times.push(embed_ms + lookup_ms);
            results.push(json!({
                "report": report_stem,
                "id": result["id"],
                "query": query,
                "supporting_families": corroboration.supporting_families,
                "corroborated": corroboration.corroborated,
                "embed_ms": embed_ms,
                "lookup_ms": lookup_ms,
                "matches": corroboration.matches,
                "exact": {
                    "supporting_families": exact.supporting_families,
                    "corroborated": exact.corroborated,
                    "matches": exact.matches,
                },
            }));
        }
    }
    let corroborated = results
        .iter()
        .filter(|result| result["corroborated"] == true)
        .count();
    let summary = json!({
        "claims": index.len(),
        "embedder": embedder.name(),
        "claims_embed_and_index_ms": build_ms,
        "queries": results.len(),
        "corroborated": corroborated,
        "query_embed": latency(&embed_times),
        "claim_lookup": latency(&lookup_times),
        "query_total": latency(&total_times),
    });
    let report = json!({
        "schema_version": 1,
        "claims_path": claims_path,
        "policy": policy,
        "summary": summary,
        "limitations": [
            "Every query is covered: the claim index holds the passages these same queries retrieved.",
            "Claim similarity is embedding similarity and does not tell agreement from contradiction.",
            "Latency is the query-side work only: one query embedding and one vector lookup, single process, one query at a time."
        ],
        "results": results,
    });
    let body = serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?;
    std::fs::write(output, body + "\n")
        .map_err(|error| format!("cannot write {output}: {error}"))?;
    println!(
        "claim-signal → {output}\n{}",
        serde_json::to_string_pretty(&report["summary"]).unwrap_or_default()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_passage_is_ingested_once_however_many_queries_retrieved_it() {
        let directory =
            std::env::temp_dir().join(format!("oi-claim-ingest-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let report = |name: &str, text: &str| {
            let path = directory.join(name);
            std::fs::write(
                &path,
                json!({ "results": [
                    { "id": "q1", "query": "q", "passages": [
                        { "id": "p1", "url": "https://a.org/x", "title": "A", "text": text, "retained": true },
                        { "id": "p2", "url": "https://b.org/y", "title": "B", "text": "snippet", "retained": false }
                    ] }
                ] })
                .to_string(),
            )
            .unwrap();
            path.to_string_lossy().to_string()
        };
        let passages = retained_passages(&[
            report("one.json", "same text"),
            report("two.json", "same text"),
        ])
        .unwrap();
        assert_eq!(
            passages.len(),
            1,
            "unretained snippets and repeats are skipped"
        );
        assert_eq!(passages.values().next().unwrap().url, "https://a.org/x");
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn percentiles_of_a_small_sample() {
        assert_eq!(percentile(&[3.0, 1.0, 2.0], 0.5), Some(2.0));
        assert_eq!(percentile(&[], 0.5), None);
    }
}
