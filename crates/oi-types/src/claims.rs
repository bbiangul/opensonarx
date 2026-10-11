//! Claims stored when a page enters an index, and the corroboration count
//! read from them at query time.
//!
//! The types live here because three layers share them: the SDK extracts and
//! verifies claims, an index stores and searches them, and the network carries
//! them between nodes.

use crate::proto::opensonarx::{Scores, SearchResult};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashSet};

/// Numbers that decide what is stored and what counts as corroboration.
/// They are policy, versioned by `policy_id`, not knowledge about language.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimPolicy {
    pub policy_id: String,
    pub max_claims_per_passage: usize,
    /// A shorter quote locates too little text to back a claim.
    pub min_quote_chars: usize,
    /// Stored claims fetched for one request.
    pub top_k: usize,
    /// Below this similarity to the request a stored claim is not about it.
    pub min_query_similarity: f32,
    /// At or above this similarity to the head claim, a stored claim is
    /// counted as stating the same thing.
    pub min_claim_similarity: f32,
    pub min_source_families: usize,
}

impl Default for ClaimPolicy {
    /// The two similarity thresholds were chosen on development queries and
    /// then held fixed for a separate test (docs/experiments/
    /// INGEST-CLAIMS-2026-10-11.md). They are specific to the embedder used
    /// there, `arctic-embed-s`; another embedder needs its own.
    fn default() -> Self {
        Self {
            policy_id: "ingest-claims-v1".to_string(),
            max_claims_per_passage: 12,
            min_quote_chars: 20,
            top_k: 20,
            min_query_similarity: 0.725,
            min_claim_similarity: 0.841,
            min_source_families: 2,
        }
    }
}

/// One statement a retained passage makes, with the span that states it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimRecord {
    pub claim: String,
    /// The span as located in the passage, not as the model wrote it.
    pub quote: String,
    pub url: String,
    pub source_family: String,
    pub extractor: String,
}

/// A stored claim fetched for a request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchedClaim {
    pub claim: String,
    pub quote: String,
    pub url: String,
    pub source_family: String,
    pub query_similarity: f32,
    /// Similarity to the head claim; 1.0 for the head itself.
    pub head_similarity: f32,
}

/// What the stored claims say about a request.
#[derive(Debug, Clone, Serialize)]
pub struct Corroboration {
    /// Fetched claims in order of similarity to the request, before any
    /// threshold is applied.
    pub matches: Vec<MatchedClaim>,
    /// Distinct source families among the claims that pass both thresholds.
    pub supporting_families: usize,
    pub corroborated: bool,
}

pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm = |v: &[f32]| v.iter().map(|x| x * x).sum::<f32>().sqrt();
    let denominator = norm(a) * norm(b);
    if denominator > 0.0 {
        dot / denominator
    } else {
        0.0
    }
}

/// Distinct source families among the matches that state the head claim.
/// Zero when the head itself is not close enough to the request.
pub fn supporting_families(matches: &[MatchedClaim], policy: &ClaimPolicy) -> usize {
    let Some(head) = matches.first() else {
        return 0;
    };
    if head.query_similarity < policy.min_query_similarity {
        return 0;
    }
    matches
        .iter()
        .filter(|claim| {
            claim.query_similarity >= policy.min_query_similarity
                && claim.head_similarity >= policy.min_claim_similarity
        })
        .map(|claim| claim.source_family.as_str())
        .collect::<BTreeSet<_>>()
        .len()
}

/// Order fetched claims by similarity to the request, keep the closest
/// `top_k`, and fill in each one's similarity to the head from the vectors.
/// The same function serves one index and the merged answers of many nodes,
/// so both produce the same signal from the same claims.
///
/// Two nodes can hold the same claim (a page with chunks in two shards, or
/// two replicas). One copy is kept, identified by URL and claim text, so a
/// repeated claim does not take another claim's place among the `top_k`.
pub fn corroboration(
    mut fetched: Vec<(MatchedClaim, Vec<f32>)>,
    policy: &ClaimPolicy,
) -> Corroboration {
    fetched.sort_by(|a, b| b.0.query_similarity.total_cmp(&a.0.query_similarity));
    let mut seen: HashSet<(String, String)> = HashSet::new();
    fetched.retain(|(claim, _)| seen.insert((claim.url.clone(), claim.claim.clone())));
    fetched.truncate(policy.top_k);
    let head = fetched.first().map(|(_, vector)| vector.clone());
    let matches: Vec<MatchedClaim> = fetched
        .into_iter()
        .map(|(mut claim, vector)| {
            claim.head_similarity = head.as_deref().map_or(0.0, |head| cosine(head, &vector));
            claim
        })
        .collect();
    let supporting_families = supporting_families(&matches, policy);
    Corroboration {
        corroborated: supporting_families >= policy.min_source_families,
        supporting_families,
        matches,
    }
}

/// Separates a claim from its quote in the text field of a wire result.
const QUOTE_MARK: &str = "\n> ";
/// `content_kind` of a wire result that carries a claim.
pub const CLAIM_KIND: &str = "claim";

/// A fetched claim as a wire result: the absolute similarity to the request
/// is both scores, and the claim's vector travels with it so the receiver can
/// compare claims from different nodes without embedding them again.
pub fn claim_to_result(claim: &MatchedClaim, vector: &[f32], result_id: String) -> SearchResult {
    SearchResult {
        result_id,
        url: claim.url.clone(),
        content_md: format!("{}{QUOTE_MARK}{}", claim.claim, claim.quote),
        scores: Some(Scores {
            semantic: claim.query_similarity,
            bm25: 0.0,
            r#final: claim.query_similarity,
        }),
        source: claim.source_family.clone(),
        content_kind: CLAIM_KIND.to_string(),
        embedding: vector
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect(),
        ..SearchResult::default()
    }
}

/// The claims carried by wire results, each with its vector.
pub fn claims_from_results(results: &[SearchResult]) -> Vec<(MatchedClaim, Vec<f32>)> {
    results
        .iter()
        .filter(|result| result.content_kind == CLAIM_KIND)
        .map(|result| {
            let (claim, quote) = result
                .content_md
                .split_once(QUOTE_MARK)
                .unwrap_or((result.content_md.as_str(), ""));
            let vector = result
                .embedding
                .chunks_exact(4)
                .map(|bytes| f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
                .collect();
            (
                MatchedClaim {
                    claim: claim.to_string(),
                    quote: quote.to_string(),
                    url: result.url.clone(),
                    source_family: result.source.clone(),
                    query_similarity: result.scores.as_ref().map_or(0.0, |scores| scores.semantic),
                    head_similarity: 0.0,
                },
                vector,
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fetched(
        claim: &str,
        family: &str,
        similarity: f32,
        vector: [f32; 3],
    ) -> (MatchedClaim, Vec<f32>) {
        (
            MatchedClaim {
                claim: claim.to_string(),
                quote: format!("{claim} (quoted)"),
                url: format!("https://{family}/page"),
                source_family: family.to_string(),
                query_similarity: similarity,
                head_similarity: 0.0,
            },
            vector.to_vec(),
        )
    }

    #[test]
    fn corroboration_orders_by_request_similarity_and_counts_families_near_the_head() {
        let policy = ClaimPolicy {
            top_k: 3,
            min_query_similarity: 0.5,
            min_claim_similarity: 0.9,
            ..ClaimPolicy::default()
        };
        // Given out of order, and with one claim beyond top_k.
        let signal = corroboration(
            vec![
                fetched("b", "three.org", 0.7, [0.6, 0.8, 0.0]),
                fetched("a2", "two.org", 0.8, [0.98, 0.2, 0.0]),
                fetched("a", "one.org", 0.9, [1.0, 0.0, 0.0]),
                fetched("far", "four.org", 0.1, [0.0, 0.0, 1.0]),
            ],
            &policy,
        );
        assert_eq!(signal.matches.len(), 3);
        assert_eq!(signal.matches[0].claim, "a");
        assert!((signal.matches[0].head_similarity - 1.0).abs() < 1e-6);
        assert_eq!(signal.supporting_families, 2);
        assert!(signal.corroborated);
        assert!(!corroboration(Vec::new(), &policy).corroborated);
    }

    #[test]
    fn a_claim_returned_by_two_nodes_is_counted_once() {
        let policy = ClaimPolicy {
            top_k: 2,
            min_query_similarity: 0.5,
            min_claim_similarity: 0.9,
            ..ClaimPolicy::default()
        };
        // The same claim arrives from two nodes; with both copies kept, the
        // second family's claim would fall outside top_k.
        let signal = corroboration(
            vec![
                fetched("a", "one.org", 0.9, [1.0, 0.0, 0.0]),
                fetched("a", "one.org", 0.9, [1.0, 0.0, 0.0]),
                fetched("a2", "two.org", 0.8, [0.98, 0.2, 0.0]),
            ],
            &policy,
        );
        assert_eq!(signal.matches.len(), 2);
        assert_eq!(signal.matches[1].claim, "a2");
        assert_eq!(signal.supporting_families, 2);
    }

    #[test]
    fn a_claim_survives_the_trip_through_a_wire_result() {
        let (claim, vector) = fetched(
            "Antarctica is the largest desert.",
            "example.org",
            0.83,
            [0.1, -0.2, 0.3],
        );
        let result = claim_to_result(&claim, &vector, "n1:7".to_string());
        assert_eq!(result.content_kind, CLAIM_KIND);
        let back = claims_from_results(&[result, SearchResult::default()]);
        assert_eq!(back.len(), 1, "a result that is not a claim is ignored");
        assert_eq!(back[0].0.claim, claim.claim);
        assert_eq!(back[0].0.quote, claim.quote);
        assert_eq!(back[0].0.source_family, "example.org");
        assert!((back[0].0.query_similarity - 0.83).abs() < 1e-6);
        assert_eq!(back[0].1, vector);
    }
}
