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
//! and a third that keeps the runtime router but replaces its centroids:
//!
//! - by learned clusters: the centroids are the means of ten clusters found
//!   in the stored claims themselves (spherical k-means, fixed start), so the
//!   shards follow what the index actually holds.
//!
//! For each division and each number of shards asked, the claims fetched from
//! those shards are merged the way an entry node merges them, and the result
//! is compared with an exhaustive scan of every stored claim. Each shard is a
//! runtime `ClaimIndex`; no network is involved, so this measures what
//! routing loses, not what it costs in time.

use oi_sdk::claims::{corroboration, ClaimIndex, ClaimPolicy, ClaimRecord, MatchedClaim};
use oi_types::claims::cosine;
use oi_types::shard::learn_centroids;
use oi_types::{TopicRouter, BUILTIN_TOPICS};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

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

/// Each ingested passage's key with the claims it produced.
fn read_passage_records(claims_path: &str) -> Result<Vec<(String, Vec<ClaimRecord>)>, String> {
    let raw = std::fs::read_to_string(claims_path)
        .map_err(|error| format!("cannot read {claims_path}: {error}"))?;
    Ok(raw
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|line| {
            let key = line["key"].as_str()?.to_string();
            let records = line["records"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter_map(|record| serde_json::from_value(record).ok())
                .collect();
            Some((key, records))
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

/// The shards each URL has a passage in, when passages are classified by
/// `router` the way a node classifies the chunks of a page.
fn shards_of_urls(
    passages: &[(String, Vec<f32>)],
    router: &TopicRouter,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut shards: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (url, vector) in passages {
        shards
            .entry(url.clone())
            .or_default()
            .insert(router.classify(vector).to_string());
    }
    shards
}

/// Claims follow their page: every claim of a URL is stored in every shard
/// that holds one of that URL's passages.
fn divide_by_page(
    name: &'static str,
    stored: &[(ClaimRecord, Vec<f32>)],
    shards_of: &BTreeMap<String, BTreeSet<String>>,
    dimension: usize,
) -> Result<Division, String> {
    let mut division = Division {
        name,
        shards: BTreeMap::new(),
        sizes: BTreeMap::new(),
    };
    for (record, vector) in stored {
        for shard in shards_of.get(&record.url).into_iter().flatten() {
            let added = division
                .shards
                .entry(shard.clone())
                .or_insert_with(|| ClaimIndex::new(dimension))
                .add(record.clone(), vector.clone())?;
            if added {
                *division.sizes.entry(shard.clone()).or_default() += 1;
            }
        }
    }
    Ok(division)
}

pub fn run(
    claims_path: &str,
    reports: &[String],
    passage_reports: &[String],
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
    let all_vectors: Vec<Vec<f32>> = stored.iter().map(|(_, vector)| vector.clone()).collect();
    let learned = learn_centroids(&all_vectors, topic_shards.len(), 20);
    // The runtime router, given learned centroids. The threshold is lowered
    // so that no claim falls back to a "general" shard that has no centroid.
    let learned_router = TopicRouter::new(
        learned
            .iter()
            .enumerate()
            .map(|(cluster, centroid)| (format!("cluster-{cluster}"), centroid.clone()))
            .collect(),
    )
    .with_threshold(-1.0);
    let mut by_cluster = Division {
        name: "learned",
        shards: BTreeMap::new(),
        sizes: BTreeMap::new(),
    };
    for (record, vector) in &stored {
        let shard = learned_router.classify(vector).to_string();
        *by_cluster.sizes.entry(shard.clone()).or_default() += 1;
        by_cluster
            .shards
            .entry(shard)
            .or_insert_with(|| ClaimIndex::new(dimension))
            .add(record.clone(), vector.clone())?;
    }
    eprintln!("{} claims divided three ways", stored.len());

    // The division a node actually makes: passages are classified, and the
    // claims of a page live wherever the page has a passage.
    let mut page_divisions: Vec<(Division, TopicRouter)> = Vec::new();
    let mut passages_without_text = 0usize;
    if !passage_reports.is_empty() {
        let texts = crate::claim_ingest::retained_passages(passage_reports)?;
        let mut keyed: Vec<(String, String)> = Vec::new();
        for (key, records) in read_passage_records(claims_path)? {
            let Some(url) = records.first().map(|record| record.url.clone()) else {
                continue;
            };
            match texts.get(&key) {
                Some(passage) => keyed.push((url, passage.text.clone())),
                None => passages_without_text += 1,
            }
        }
        let mut passages: Vec<(String, Vec<f32>)> = Vec::with_capacity(keyed.len());
        for batch in keyed.chunks(32) {
            let batch_texts: Vec<&str> = batch.iter().map(|(_, text)| text.as_str()).collect();
            let vectors = embedder
                .embed_text_batch(&batch_texts)
                .map_err(|error| format!("embedding passages: {error}"))?;
            passages.extend(batch.iter().map(|(url, _)| url.clone()).zip(vectors));
        }
        let passage_vectors: Vec<Vec<f32>> =
            passages.iter().map(|(_, vector)| vector.clone()).collect();
        let page_learned_router = TopicRouter::new(
            learn_centroids(&passage_vectors, topic_shards.len(), 20)
                .into_iter()
                .enumerate()
                .map(|(cluster, centroid)| (format!("page-cluster-{cluster}"), centroid))
                .collect(),
        )
        .with_threshold(-1.0);
        let page_topic_router = TopicRouter::new(
            BUILTIN_TOPICS
                .iter()
                .zip(
                    embedder
                        .embed_text_batch(&descriptions)
                        .map_err(|error| format!("embedding topics: {error}"))?,
                )
                .map(|(topic, embedding)| (topic.shard_id.to_string(), embedding))
                .collect(),
        );
        for (name, page_router) in [
            ("page-topic", page_topic_router),
            ("page-learned", page_learned_router),
        ] {
            let division = divide_by_page(
                name,
                &stored,
                &shards_of_urls(&passages, &page_router),
                dimension,
            )?;
            page_divisions.push((division, page_router));
        }
        eprintln!(
            "{} passages classified; {passages_without_text} without text",
            passages.len()
        );
    }

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
        let cluster_order: Vec<String> = learned_router
            .rank_shards(&vector)
            .into_iter()
            .map(|(shard, _)| shard.to_string())
            .collect();
        let page_orders: Vec<Vec<String>> = page_divisions
            .iter()
            .map(|(_, page_router)| {
                page_router
                    .rank_shards(&vector)
                    .into_iter()
                    .map(|(shard, _)| shard.to_string())
                    .collect()
            })
            .collect();
        let mut asked_divisions: Vec<(&Division, &Vec<String>)> = vec![
            (&by_topic, &topic_order),
            (&by_hash, &hash_shards),
            (&by_cluster, &cluster_order),
        ];
        asked_divisions.extend(
            page_divisions
                .iter()
                .map(|(division, _)| division)
                .zip(&page_orders),
        );
        for (division, order) in asked_divisions {
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
    let mut shard_sizes: BTreeMap<&str, &BTreeMap<String, usize>> = BTreeMap::new();
    for division in [&by_topic, &by_hash, &by_cluster]
        .into_iter()
        .chain(page_divisions.iter().map(|(division, _)| division))
    {
        shard_sizes.insert(division.name, &division.sizes);
    }
    let copies_per_claim: BTreeMap<&str, f64> = shard_sizes
        .iter()
        .map(|(name, sizes)| {
            (
                *name,
                sizes.values().sum::<usize>() as f64 / stored.len().max(1) as f64,
            )
        })
        .collect();
    let report = json!({
        "schema_version": 1,
        "claims_path": claims_path,
        "policy": policy,
        "queries": queries.len(),
        "claims": stored.len(),
        "shard_sizes": shard_sizes,
        "copies_per_claim": copies_per_claim,
        "passages_without_text": passages_without_text,
        "exhaustive_scan_corroborated": exact_rows.iter().filter(|row| row["corroborated"] == true).count(),
        "summary": summary,
        "limitations": [
            "No network: each shard is an in-process claim index. This measures what routing to a subset of shards loses, not latency.",
            "In topic, hash and learned each claim is placed by its own embedding. In page-topic and page-learned the passages are classified and a claim lives in every shard holding a passage of its URL, which is what a node does.",
            "The passages are the evidence passages retained by the answer pipeline, not the node's own chunks of the page.",
            "For the hash division the shards asked are the first N in a fixed order, because nothing about a query points to a shard.",
            "The learned centroids are fitted on the same stored claims the queries are looked up in. No label is used, but a growing index would have to refit or keep centroids from an earlier snapshot.",
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
    fn learned_centroids_separate_two_obvious_groups() {
        let vectors = vec![
            vec![1.0, 0.0],
            vec![0.9, 0.1],
            vec![0.0, 1.0],
            vec![0.1, 0.9],
        ];
        let centroids = learn_centroids(&vectors, 2, 10);
        assert_eq!(centroids.len(), 2);
        let side = |vector: &[f32]| {
            usize::from(cosine(vector, &centroids[1]) > cosine(vector, &centroids[0]))
        };
        assert_eq!(side(&vectors[0]), side(&vectors[1]));
        assert_eq!(side(&vectors[2]), side(&vectors[3]));
        assert_ne!(side(&vectors[0]), side(&vectors[2]));
        // Same input, same clusters.
        assert_eq!(centroids, learn_centroids(&vectors, 2, 10));
    }

    #[test]
    fn a_url_hashes_to_one_of_the_shards_and_always_the_same_one() {
        let shard = hash_shard("https://example.org/a", 10);
        assert_eq!(shard, hash_shard("https://example.org/a", 10));
        assert!(shard.starts_with("hash-"));
        let bucket: usize = shard.trim_start_matches("hash-").parse().unwrap();
        assert!(bucket < 10);
    }
}
