//! The corroboration signal when "says the same thing" is a model's judgment
//! made at ingestion, instead of an embedding-similarity cut.
//!
//! For each query the head is the nearest stored claim. A relation judge
//! compares the head with its nearest stored claims from other source
//! families, the index stores the relations, and the signal is read back
//! from them. The judge sees two stored claims and never the query.
//!
//! A node would judge every claim as it is stored. Here only the heads of
//! the measured queries are judged, which gives the same relations for those
//! heads at a fraction of the model calls. Judgments are appended to a cache
//! file as they finish, so an interrupted run resumes without repeating them.
//!
//! Everything that decides is runtime code: `ClaimIndex`, the
//! `ClaimRelationJudge` implementation of `ChatModel`, and `ClaimPolicy`.

use crate::claim_ingest::{read_report, stem};
use oi_sdk::claims::{ClaimIndex, ClaimPolicy, ClaimRecord, ClaimRelation};
use oi_sdk::{AnswerModelConfig, ChatModel};
use oi_types::ClaimRelationJudge;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Write;

type Identity = (String, String);

fn identity(record: &ClaimRecord) -> Identity {
    (record.url.clone(), record.claim.clone())
}

fn read_records(claims_path: &str) -> Result<Vec<ClaimRecord>, String> {
    let raw = std::fs::read_to_string(claims_path)
        .map_err(|error| format!("cannot read {claims_path}: {error}"))?;
    Ok(raw
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
        .collect())
}

/// Store the relations of one cache line in the index. Returns how many.
fn apply(index: &mut ClaimIndex, positions: &HashMap<Identity, usize>, line: &Value) -> usize {
    let key = |value: &Value| -> Option<usize> {
        positions
            .get(&(
                value["url"].as_str()?.to_string(),
                value["claim"].as_str()?.to_string(),
            ))
            .copied()
    };
    let Some(head) = key(&line["head"]) else {
        return 0;
    };
    let mut applied = 0;
    for candidate in line["candidates"].as_array().into_iter().flatten() {
        let relation = serde_json::from_value::<ClaimRelation>(candidate["relation"].clone());
        if let (Some(other), Ok(relation)) = (key(candidate), relation) {
            index.set_relation(head, other, relation);
            applied += 1;
        }
    }
    applied
}

#[allow(clippy::too_many_arguments)]
pub fn run(
    claims_path: &str,
    reports: &[String],
    provider: &str,
    model: &str,
    concurrency: usize,
    cache_path: &str,
    limit: Option<usize>,
    output: &str,
) -> Result<(), String> {
    let records = read_records(claims_path)?;
    if records.is_empty() {
        return Err(format!("{claims_path} has no claim records"));
    }
    let policy = ClaimPolicy::default();
    let judge = ChatModel::new(match provider {
        "ollama" => AnswerModelConfig::new(model),
        "openrouter" => AnswerModelConfig::openrouter(model),
        other => return Err(format!("unknown --provider {other:?}")),
    });
    let embedder = oi_sdk::OpenSonarXClient::default_embedder();
    let mut index = ClaimIndex::new(embedder.dimension());
    let mut positions: HashMap<Identity, usize> = HashMap::new();
    for batch in records.chunks(64) {
        let texts: Vec<&str> = batch.iter().map(|record| record.claim.as_str()).collect();
        let vectors = embedder
            .embed_text_batch(&texts)
            .map_err(|error| format!("embedding claims: {error}"))?;
        for (record, vector) in batch.iter().zip(vectors) {
            let position = index.len();
            if index.add(record.clone(), vector)? {
                positions.insert(identity(record), position);
            }
        }
    }
    eprintln!("{} claims indexed", index.len());

    let mut queries: Vec<(String, Value, String, Vec<f32>)> = Vec::new();
    for path in reports {
        let report = read_report(path)?;
        for result in report["results"].as_array().into_iter().flatten() {
            let query = result["query"].as_str().unwrap_or_default().to_string();
            let vector = embedder
                .embed_query(&query)
                .map_err(|error| format!("embedding query: {error}"))?;
            queries.push((stem(path), result["id"].clone(), query, vector));
        }
    }
    if let Some(limit) = limit {
        queries.truncate(limit);
    }

    // Relations judged in an earlier run.
    let cached = std::fs::read_to_string(cache_path).unwrap_or_default();
    let mut judged_heads: BTreeSet<usize> = BTreeSet::new();
    let mut pairs_from_cache = 0usize;
    for line in cached
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|line| line.get("error").is_none())
    {
        pairs_from_cache += apply(&mut index, &positions, &line);
        let head = (
            line["head"]["url"].as_str().unwrap_or_default().to_string(),
            line["head"]["claim"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
        );
        if let Some(position) = positions.get(&head) {
            judged_heads.insert(*position);
        }
    }

    // The heads that can carry a label and have not been judged yet.
    let pending: Vec<usize> = queries
        .iter()
        .filter_map(|(_, _, _, vector)| index.nearest(vector, policy.top_k))
        .filter(|(_, similarity)| *similarity >= policy.min_query_similarity)
        .map(|(position, _)| position)
        .collect::<BTreeSet<usize>>()
        .into_iter()
        .filter(|position| !judged_heads.contains(position))
        .collect();
    eprintln!(
        "{} queries; {} heads already judged ({pairs_from_cache} pairs), {} to judge",
        queries.len(),
        judged_heads.len(),
        pending.len()
    );

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(cache_path)
        .map_err(|error| format!("cannot open {cache_path}: {error}"))?;
    let (mut calls, mut errors, mut judge_ms) = (0usize, 0usize, Vec::new());
    for batch in pending.chunks(concurrency.max(1)) {
        let jobs: Vec<(usize, Vec<usize>)> = batch
            .iter()
            .map(|position| (*position, index.neighbours_to_judge(*position, &policy)))
            .collect();
        let lines: Vec<Value> = std::thread::scope(|scope| {
            let workers: Vec<_> = jobs
                .iter()
                .map(|(position, neighbours)| {
                    let (index, judge) = (&index, &judge);
                    scope.spawn(move || {
                        let head = index.record(*position).cloned();
                        let candidates: Vec<ClaimRecord> = neighbours
                            .iter()
                            .filter_map(|other| index.record(*other).cloned())
                            .collect();
                        let Some(head) = head else {
                            return json!({ "error": "no such claim" });
                        };
                        let texts: Vec<String> =
                            candidates.iter().map(|c| c.claim.clone()).collect();
                        let started = std::time::Instant::now();
                        match judge.relate(&head.claim, &texts) {
                            Ok(relations) => json!({
                                "head": { "url": head.url, "claim": head.claim, "source_family": head.source_family },
                                "judge": judge.model_version(),
                                "ms": started.elapsed().as_secs_f64() * 1_000.0,
                                "candidates": candidates.iter().zip(relations).map(|(candidate, relation)| json!({
                                    "url": candidate.url,
                                    "claim": candidate.claim,
                                    "source_family": candidate.source_family,
                                    "relation": relation,
                                })).collect::<Vec<_>>(),
                            }),
                            Err(error) => json!({
                                "head": { "url": head.url, "claim": head.claim },
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
            calls += 1;
            if line.get("error").is_some() {
                errors += 1;
            } else {
                apply(&mut index, &positions, &line);
                judge_ms.extend(line["ms"].as_f64());
            }
            writeln!(file, "{line}")
                .map_err(|error| format!("cannot write {cache_path}: {error}"))?;
        }
        eprintln!("judged {calls} of {} heads", pending.len());
    }

    let mut relation_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut results = Vec::new();
    let mut lookup_ms = Vec::new();
    for (report, id, query, vector) in &queries {
        let started = std::time::Instant::now();
        let judged = index.judged_corroboration(vector, &policy);
        lookup_ms.push(started.elapsed().as_secs_f64() * 1_000.0);
        let similar = index.corroboration(vector, &policy);
        results.push(json!({
            "report": report,
            "id": id,
            "query": query,
            "head": judged.head,
            "judged": judged.support,
            "judged_corroborated": judged.corroborated,
            "contested": judged.contested,
            "agreeing": judged.agreeing,
            "contradicting": judged.contradicting,
            "similar_supporting_families": similar.supporting_families,
            "similar_corroborated": similar.corroborated,
        }));
    }
    let head_positions: BTreeSet<usize> = queries
        .iter()
        .filter_map(|(_, _, _, vector)| index.nearest(vector, policy.top_k))
        .filter(|(_, similarity)| *similarity >= policy.min_query_similarity)
        .map(|(position, _)| position)
        .collect();
    for position in &head_positions {
        for (_, relation) in index.related(*position) {
            let name = serde_json::to_value(relation)
                .ok()
                .and_then(|value| value.as_str().map(str::to_string))
                .unwrap_or_default();
            *relation_counts.entry(name).or_default() += 1;
        }
    }
    let count = |field: &str| {
        results
            .iter()
            .filter(|result| result[field] == true)
            .count()
    };
    lookup_ms.sort_by(f64::total_cmp);
    judge_ms.sort_by(f64::total_cmp);
    let median = |values: &[f64]| values.get(values.len() / 2).copied();
    let summary = json!({
        "claims": index.len(),
        "embedder": embedder.name(),
        "judge": judge.model_version(),
        "queries": results.len(),
        "heads_about_their_query": head_positions.len(),
        "judge_calls_now": calls,
        "judge_errors_now": errors,
        "judge_ms_p50": median(&judge_ms),
        "relations_of_the_heads": relation_counts,
        "judged_corroborated": count("judged_corroborated"),
        "contested": count("contested"),
        "similar_corroborated": count("similar_corroborated"),
        "judged_lookup_ms_p50": median(&lookup_ms),
    });
    let report = json!({
        "schema": "opensonarx.claim_judged.v1",
        "claims_path": claims_path,
        "relations_cache": cache_path,
        "policy": policy,
        "summary": summary,
        "limitations": [
            "Only the heads of the measured queries are judged; a node would judge every claim as it is stored.",
            "Relations are kept in memory for this measurement; they are not saved with the index or replicated.",
            "Every query is covered: the claim index holds the passages these same queries retrieved.",
            "The judged lookup time excludes the query embedding."
        ],
        "results": results,
    });
    let body = serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?;
    std::fs::write(output, body + "\n")
        .map_err(|error| format!("cannot write {output}: {error}"))?;
    println!(
        "claim-judged → {output}\n{}",
        serde_json::to_string_pretty(&report["summary"]).unwrap_or_default()
    );
    if errors > 0 {
        return Err(format!(
            "{errors} judge calls failed; run again to retry them"
        ));
    }
    Ok(())
}
