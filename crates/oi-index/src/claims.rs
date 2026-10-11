//! Stored claims with their embeddings, searchable by vector.
//!
//! A claim lookup is a plain nearest-neighbour search. The score of a match
//! is its cosine similarity to the request, which means the same thing on
//! every node, so matches from different nodes can be merged by it.

use crate::hnsw::HnswIndex;
use oi_types::claims::{ClaimPolicy, ClaimRecord, Corroboration, MatchedClaim};

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
    pub fn add(&mut self, record: ClaimRecord, vector: Vec<f32>) -> Result<(), String> {
        self.hnsw
            .insert(self.records.len().to_string(), vector.clone())?;
        self.records.push(record);
        self.vectors.push(vector);
        Ok(())
    }

    /// The stored claims nearest to a request, each with its vector, in
    /// order of similarity.
    pub fn fetch(&self, query_vector: &[f32], top_k: usize) -> Vec<(MatchedClaim, Vec<f32>)> {
        self.hnsw
            .search(query_vector, top_k)
            .into_iter()
            .filter_map(|(id, similarity)| Some((id.parse::<usize>().ok()?, similarity)))
            .filter(|(index, _)| *index < self.records.len())
            .map(|(index, query_similarity)| {
                let record = &self.records[index];
                (
                    MatchedClaim {
                        claim: record.claim.clone(),
                        quote: record.quote.clone(),
                        url: record.url.clone(),
                        source_family: record.source_family.clone(),
                        query_similarity,
                        head_similarity: 0.0,
                    },
                    self.vectors[index].clone(),
                )
            })
            .collect()
    }

    /// The corroboration signal for a request, from its embedding alone.
    pub fn corroboration(&self, query_vector: &[f32], policy: &ClaimPolicy) -> Corroboration {
        oi_types::claims::corroboration(self.fetch(query_vector, policy.top_k), policy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn fetch_returns_each_claim_with_its_vector() {
        let mut index = ClaimIndex::new(3);
        index
            .add(record("a", "one.org"), vec![1.0, 0.0, 0.0])
            .unwrap();
        let fetched = index.fetch(&[1.0, 0.0, 0.0], 5);
        assert_eq!(fetched.len(), 1);
        assert_eq!(fetched[0].0.claim, "a");
        assert_eq!(fetched[0].1, vec![1.0, 0.0, 0.0]);
        assert!((fetched[0].0.query_similarity - 1.0).abs() < 1e-5);
    }
}
