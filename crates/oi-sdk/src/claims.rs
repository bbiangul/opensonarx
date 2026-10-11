//! Claims extracted when a page enters the index, and the corroboration
//! signal computed from them at query time without any model call.
//!
//! Ingestion: a model lists the statements a passage makes, each with a
//! quote. A claim is kept only if its quote is located verbatim in the
//! passage by the same deterministic check the answer path uses. The kept
//! claims are embedded and stored with their source family.
//!
//! Query: the request is embedded and the nearest stored claims are fetched.
//! The best match is the head; stored claims close enough to the head say
//! the same thing, and the number of distinct source families among them is
//! the corroboration count. The whole query step is one embedding and a
//! vector lookup.
//!
//! Similarity of two claims here is embedding similarity. It does not tell
//! agreement from contradiction on the same subject; stance still needs a
//! model and is not part of this signal.

use oi_index::hnsw::HnswIndex;
use oi_types::{EvidencePassage, PassageClaimExtractor, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

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

/// Extract and verify the claims of one passage. Returns the kept records
/// and how many proposed claims were dropped because their quote was not
/// found in the passage.
pub fn ingest_passage(
    extractor: &dyn PassageClaimExtractor,
    passage: &EvidencePassage,
    policy: &ClaimPolicy,
) -> Result<(Vec<ClaimRecord>, usize)> {
    let proposed = extractor.passage_claims(passage, policy.max_claims_per_passage)?;
    let extractor_version = extractor.model_version();
    let family = crate::source::source_family(&passage.url);
    let mut kept = Vec::new();
    let mut rejected = 0;
    for claim in proposed.into_iter().take(policy.max_claims_per_passage) {
        match crate::answer::locate_quote(&passage.text, &claim.quote, policy.min_quote_chars) {
            Some(span) => kept.push(ClaimRecord {
                claim: claim.text,
                quote: span,
                url: passage.url.clone(),
                source_family: family.clone(),
                extractor: extractor_version.clone(),
            }),
            None => rejected += 1,
        }
    }
    Ok((kept, rejected))
}

/// A stored claim fetched for a request.
#[derive(Debug, Clone, Serialize)]
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

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm = |v: &[f32]| v.iter().map(|x| x * x).sum::<f32>().sqrt();
    let denominator = norm(a) * norm(b);
    if denominator > 0.0 {
        dot / denominator
    } else {
        0.0
    }
}

/// Stored claims with their embeddings, searchable by vector.
pub struct ClaimIndex {
    records: Vec<ClaimRecord>,
    vectors: Vec<Vec<f32>>,
    hnsw: HnswIndex,
}

impl ClaimIndex {
    pub fn new(dimension: usize) -> Self {
        Self {
            records: Vec::new(),
            vectors: Vec::new(),
            hnsw: HnswIndex::new(dimension),
        }
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Store a claim with the embedding of its text.
    pub fn add(&mut self, record: ClaimRecord, vector: Vec<f32>) -> Result<()> {
        self.hnsw
            .insert(self.records.len().to_string(), vector.clone())
            .map_err(oi_types::Error::Internal)?;
        self.records.push(record);
        self.vectors.push(vector);
        Ok(())
    }

    /// The corroboration signal for a request, from its embedding alone.
    pub fn corroboration(&self, query_vector: &[f32], policy: &ClaimPolicy) -> Corroboration {
        let hits: Vec<(usize, f32)> = self
            .hnsw
            .search(query_vector, policy.top_k)
            .into_iter()
            .filter_map(|(id, similarity)| Some((id.parse::<usize>().ok()?, similarity)))
            .filter(|(index, _)| *index < self.records.len())
            .collect();
        let head_vector = hits.first().map(|(index, _)| &self.vectors[*index]);
        let matches: Vec<MatchedClaim> = hits
            .iter()
            .map(|(index, query_similarity)| {
                let record = &self.records[*index];
                MatchedClaim {
                    claim: record.claim.clone(),
                    quote: record.quote.clone(),
                    url: record.url.clone(),
                    source_family: record.source_family.clone(),
                    query_similarity: *query_similarity,
                    head_similarity: head_vector
                        .map(|head| cosine(head, &self.vectors[*index]))
                        .unwrap_or(0.0),
                }
            })
            .collect();
        let supporting_families = supporting_families(&matches, policy);
        Corroboration {
            corroborated: supporting_families >= policy.min_source_families,
            supporting_families,
            matches,
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use oi_types::PassageClaim;

    struct Scripted(Vec<PassageClaim>);

    impl PassageClaimExtractor for Scripted {
        fn passage_claims(
            &self,
            _passage: &EvidencePassage,
            _max_claims: usize,
        ) -> Result<Vec<PassageClaim>> {
            Ok(self.0.clone())
        }

        fn model_version(&self) -> String {
            "scripted".to_string()
        }
    }

    fn passage(url: &str, text: &str) -> EvidencePassage {
        EvidencePassage {
            id: "p1".to_string(),
            url: url.to_string(),
            title: String::new(),
            text: text.to_string(),
            retained: true,
            published_at: None,
            observed_at: None,
        }
    }

    fn record(claim: &str, family: &str) -> ClaimRecord {
        ClaimRecord {
            claim: claim.to_string(),
            quote: claim.to_string(),
            url: format!("https://{family}/page"),
            source_family: family.to_string(),
            extractor: "scripted".to_string(),
        }
    }

    #[test]
    fn a_claim_is_kept_only_when_its_quote_is_in_the_passage() {
        let text = "Antarctica is the largest desert on Earth. It covers 14 million square km.";
        let extractor = Scripted(vec![
            PassageClaim {
                text: "Antarctica is the largest desert.".to_string(),
                quote: "Antarctica is the largest desert on Earth.".to_string(),
            },
            PassageClaim {
                text: "The Sahara is the largest desert.".to_string(),
                quote: "The Sahara is the largest desert on Earth.".to_string(),
            },
        ]);
        let (kept, rejected) = ingest_passage(
            &extractor,
            &passage("https://www.example.com/deserts", text),
            &ClaimPolicy::default(),
        )
        .unwrap();
        assert_eq!(rejected, 1);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].quote, "Antarctica is the largest desert on Earth.");
        assert_eq!(kept[0].source_family, "example.com");
        assert_eq!(kept[0].extractor, "scripted");
    }

    #[test]
    fn corroboration_counts_families_that_state_the_head_claim() {
        let mut index = ClaimIndex::new(3);
        // Two families state the same thing; a third states something else
        // on the same topic; the fourth repeats a family.
        index
            .add(record("a", "one.org"), vec![1.0, 0.0, 0.0])
            .unwrap();
        index
            .add(record("a2", "two.org"), vec![0.98, 0.2, 0.0])
            .unwrap();
        index
            .add(record("b", "three.org"), vec![0.6, 0.8, 0.0])
            .unwrap();
        index
            .add(record("a3", "one.org"), vec![0.99, 0.1, 0.0])
            .unwrap();
        let policy = ClaimPolicy {
            top_k: 4,
            min_query_similarity: 0.5,
            min_claim_similarity: 0.9,
            ..ClaimPolicy::default()
        };
        let signal = index.corroboration(&[1.0, 0.0, 0.0], &policy);
        assert_eq!(signal.matches.len(), 4);
        assert_eq!(signal.matches[0].claim, "a");
        assert_eq!(signal.supporting_families, 2);
        assert!(signal.corroborated);

        // A request nothing stored is close to has no reference claim.
        let far = index.corroboration(&[0.0, 0.0, 1.0], &policy);
        assert_eq!(far.supporting_families, 0);
        assert!(!far.corroborated);

        // One family alone is not corroboration.
        let mut single = ClaimIndex::new(3);
        single
            .add(record("a", "one.org"), vec![1.0, 0.0, 0.0])
            .unwrap();
        single
            .add(record("a3", "one.org"), vec![0.99, 0.1, 0.0])
            .unwrap();
        assert!(!single.corroboration(&[1.0, 0.0, 0.0], &policy).corroborated);
    }
}
