//! How the claim lookup behaves when the claims are divided among nodes, and
//! a query is sent to only some of them.
//!
//! Two ways of dividing the same stored claims into ten shards:
//!
//! - by subject, with the runtime's `TopicRouter`: each claim goes to the
//!   topic shard its embedding is closest to, and a query is routed to the
//!   topic shards its own embedding ranks highest;
//! - by a hash of the URL: shards are balanced, and nothing about a query
//!   says which shard holds what it needs.
//!
//! For each division and each number of shards asked, the claims fetched from
//! those shards are merged the way an entry node merges them, and the result
//! is compared with an exhaustive scan of every stored claim. Each shard is a
//! runtime `ClaimIndex`; no network is involved, so this measures what
//! routing loses, not what it costs in time.

use oi_sdk::claims::{corroboration, ClaimIndex, ClaimPolicy, ClaimRecord, MatchedClaim};
use oi_types::claims::cosine;
use oi_types::{TopicRouter, BUILTIN_TOPICS};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

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

fn hash_shard(url: &str, shards: usize) -> String {
    let digest = Sha256::digest(url.as_bytes());
    let bucket = u64::from_le_bytes(digest[..8].try_into().unwrap_or([0; 8])) as usize % shards;
    format!("hash-{bucket}")
}

fn identity(claim: &MatchedClaim) -> (&str, &str) {
    (claim.url.as_str(), claim.claim.as_str())
}

/// One way of dividing the claims: a shard per claim, and for a query the
/// shards in the order they would be asked.
struct Division {
    name: &'static str,
    shards: BTreeMap<String, ClaimIndex>,
    sizes: BTreeMap<String, usize>,
}

pub fn run(
    claims_path: &str,
    reports: &[String],
    fanouts: &[usize],
    output: &str,
) -> Result<(), String> {
    let records = read_records(claims_path)?;
    if records.is_empty() {
        return Err(format!("{claims_path} has no claim records"));
    }
    let policy = ClaimPolicy::default();
    let embedder = oi_sdk::OpenSonarXClient::default_embedder();
    let dimension = embedder.dimension();

    // The topic router exactly as a node builds it: one centroid per
    // built-in topic, the embedding of the topic's description.
    let descriptions: Vec<&str> = BUILTIN_TOPICS
        .iter()
        .map(|topic| topic.description)
        .collect();
    let centroids = BUILTIN_TOPICS
        .iter()
        .zip(
            embedder
                .embed_text_batch(&descriptions)
                .map_err(|error| format!("embedding topics: {error}"))?,
        )
        .map(|(topic, embedding)| (topic.shard_id.to_string(), embedding))
        .collect();
    let router = TopicRouter::new(centroids);
    let topic_shards: Vec<String> = BUILTIN_TOPICS
        .iter()
        .map(|topic| topic.shard_id.to_string())
        .collect();
    let hash_shards: Vec<String> = (0..topic_shards.len())
        .map(|n| format!("hash-{n}"))
        .collect();

    let mut by_topic = Division {
        name: "topic",
        shards: BTreeMap::new(),
        sizes: BTreeMap::new(),
    };
    let mut by_hash = Division {
        name: "hash",
        shards: BTreeMap::new(),
        sizes: BTreeMap::new(),
    };
    let mut stored: Vec<(ClaimRecord, Vec<f32>)> = Vec::with_capacity(records.len());
    for batch in records.chunks(64) {
        let texts: Vec<&str> = batch.iter().map(|record| record.claim.as_str()).collect();
        let vectors = embedder
            .embed_text_batch(&texts)
            .map_err(|error| format!("embedding claims: {error}"))?;
        for (record, vector) in batch.iter().zip(vectors) {
            for (division, shard) in [
                (&mut by_topic, router.classify(&vector).to_string()),
                (&mut by_hash, hash_shard(&record.url, hash_shards.len())),
            ] {
                *division.sizes.entry(shard.clone()).or_default() += 1;
                division
                    .shards
                    .entry(shard)
                    .or_insert_with(|| ClaimIndex::new(dimension))
                    .add(record.clone(), vector.clone())?;
            }
            stored.push((record.clone(), vector));
        }
    }
    eprintln!("{} claims divided two ways", stored.len());

    let mut queries: Vec<(String, Value, String)> = Vec::new();
    for path in reports {
        let raw = std::fs::read_to_string(path)
            .map_err(|error| format!("cannot read {path}: {error}"))?;
        let report: Value =
            serde_json::from_str(&raw).map_err(|error| format!("{path}: {error}"))?;
        let stem = std::path::Path::new(path)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or(path)
            .to_string();
        for result in report["results"].as_array().into_iter().flatten() {
            queries.push((
                stem.clone(),
                result["id"].clone(),
                result["query"].as_str().unwrap_or_default().to_string(),
            ));
        }
    }

    // variant -> per-query rows and running totals
    let mut variants: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    let mut totals: BTreeMap<String, (f64, usize, usize, usize)> = BTreeMap::new();
    let mut exact_rows = Vec::new();
    for (report, id, query) in &queries {
        let vector = embedder
            .embed_query(query)
            .map_err(|error| format!("embedding query: {error}"))?;
        let exact = corroboration(
            stored
                .iter()
                .map(|(record, stored_vector)| {
                    (
                        MatchedClaim {
                            claim: record.claim.clone(),
                            quote: record.quote.clone(),
                            url: record.url.clone(),
                            source_family: record.source_family.clone(),
                            query_similarity: cosine(&vector, stored_vector),
                            head_similarity: 0.0,
                        },
                        stored_vector.clone(),
                    )
                })
                .collect(),
            &policy,
        );
        exact_rows.push(json!({
            "report": report, "id": id,
            "supporting_families": exact.supporting_families,
            "corroborated": exact.corroborated,
        }));

        let topic_order: Vec<String> = router
            .rank_shards(&vector)
            .into_iter()
            .map(|(shard, _)| shard.to_string())
            .collect();
        for (division, order) in [(&by_topic, &topic_order), (&by_hash, &hash_shards)] {
            for &fanout in fanouts {
                let asked: Vec<&String> = order.iter().take(fanout).collect();
                let fetched = asked
                    .iter()
                    .filter_map(|shard| division.shards.get(*shard))
                    .flat_map(|shard| shard.fetch(&vector, policy.top_k))
                    .collect();
                let signal = corroboration(fetched, &policy);
                let found = exact
                    .matches
                    .iter()
                    .filter(|claim| {
                        signal
                            .matches
                            .iter()
                            .any(|other| identity(other) == identity(claim))
                    })
                    .count();
                let recall = found as f64 / exact.matches.len().max(1) as f64;
                let same_head = match (signal.matches.first(), exact.matches.first()) {
                    (Some(a), Some(b)) => identity(a) == identity(b),
                    (None, None) => true,
                    _ => false,
                };
                let name = format!("{}-{fanout}", division.name);
                let total = totals.entry(name.clone()).or_default();
                total.0 += recall;
                total.1 += usize::from(same_head);
                total.2 += usize::from(signal.corroborated == exact.corroborated);
                total.3 += usize::from(signal.corroborated);
                variants.entry(name).or_default().push(json!({
                    "report": report, "id": id,
                    "supporting_families": signal.supporting_families,
                    "corroborated": signal.corroborated,
                }));
            }
        }
    }

    let n = queries.len().max(1) as f64;
    let summary: BTreeMap<String, Value> = totals
        .iter()
        .map(|(name, (recall, head, label, corroborated))| {
            (
                name.clone(),
                json!({
                    "recall_of_the_nearest_claims": recall / n,
                    "same_head_claim": head,
                    "same_label": label,
                    "corroborated": corroborated,
                }),
            )
        })
        .collect();
    let report = json!({
        "schema_version": 1,
        "claims_path": claims_path,
        "policy": policy,
        "queries": queries.len(),
        "claims": stored.len(),
        "shard_sizes": { "topic": by_topic.sizes, "hash": by_hash.sizes },
        "exhaustive_scan_corroborated": exact_rows.iter().filter(|row| row["corroborated"] == true).count(),
        "summary": summary,
        "limitations": [
            "No network: each shard is an in-process claim index. This measures what routing to a subset of shards loses, not latency.",
            "Each claim is placed by its own embedding; a node places chunks, and the claims of a page would follow their chunk.",
            "For the hash division the shards asked are the first N in a fixed order, because nothing about a query points to a shard.",
            "Every query is covered by the stored claims."
        ],
        "exact": exact_rows,
        "variants": variants,
    });
    let body = serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?;
    std::fs::write(output, body + "\n")
        .map_err(|error| format!("cannot write {output}: {error}"))?;
    println!(
        "claim-routing → {output}\n{}\n{}",
        serde_json::to_string_pretty(&report["shard_sizes"]).unwrap_or_default(),
        serde_json::to_string_pretty(&report["summary"]).unwrap_or_default()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_url_hashes_to_one_of_the_shards_and_always_the_same_one() {
        let shard = hash_shard("https://example.org/a", 10);
        assert_eq!(shard, hash_shard("https://example.org/a", 10));
        assert!(shard.starts_with("hash-"));
        let bucket: usize = shard.trim_start_matches("hash-").parse().unwrap();
        assert!(bucket < 10);
    }
}
