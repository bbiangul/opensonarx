//! Stored claims with their embeddings, searchable by vector.
//!
//! A claim lookup is a plain nearest-neighbour search. The score of a match
//! is its cosine similarity to the request, which means the same thing on
//! every node, so matches from different nodes can be merged by it.

use crate::hnsw::HnswIndex;
use oi_types::claims::{
    ClaimPolicy, ClaimRecord, ClaimRelation, Corroboration, JudgedCorroboration, JudgedSupport,
    MatchedClaim,
};
use std::collections::{BTreeSet, HashMap, HashSet};

pub struct ClaimIndex {
    records: Vec<ClaimRecord>,
    vectors: Vec<Vec<f32>>,
    hnsw: HnswIndex,
    /// (url, claim) pairs already stored; a claim received twice is kept once.
    seen: HashSet<(String, String)>,
    /// Relations a judge gave between two stored claims, keyed by their
    /// positions with the lower one first.
    relations: HashMap<(usize, usize), ClaimRelation>,
}

impl ClaimIndex {
    pub fn new(dimension: usize) -> Self {
        Self {
            records: Vec::new(),
            vectors: Vec::new(),
            hnsw: HnswIndex::new(dimension),
            seen: HashSet::new(),
            relations: HashMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Store a claim with the embedding of its text. Returns `false` when
    /// the same claim from the same URL is already stored.
    pub fn add(&mut self, record: ClaimRecord, vector: Vec<f32>) -> Result<bool, String> {
        if !self.seen.insert((record.url.clone(), record.claim.clone())) {
            return Ok(false);
        }
        self.hnsw
            .insert(self.records.len().to_string(), vector.clone())?;
        self.records.push(record);
        self.vectors.push(vector);
        Ok(true)
    }

    /// Claims stored after sequence number `from_seq`, oldest first, at most
    /// `max`. A claim's sequence number is its position in insertion order,
    /// starting at 1, so a replica can ask for what it has not seen yet.
    pub fn since(&self, from_seq: u64, max: usize) -> Vec<(u64, ClaimRecord, Vec<f32>)> {
        self.records
            .iter()
            .zip(&self.vectors)
            .enumerate()
            .skip(from_seq as usize)
            .take(max)
            .map(|(position, (record, vector))| {
                (position as u64 + 1, record.clone(), vector.clone())
            })
            .collect()
    }

    /// The stored claims of the given pages, with their vectors: what a
    /// replica is sent along with those pages.
    pub fn for_urls(&self, urls: &HashSet<String>) -> Vec<(ClaimRecord, Vec<f32>)> {
        self.records
            .iter()
            .zip(&self.vectors)
            .filter(|(record, _)| urls.contains(&record.url))
            .map(|(record, vector)| (record.clone(), vector.clone()))
            .collect()
    }

    /// Every stored claim with its vector, in insertion order: what is
    /// written to disk and what a replica needs.
    pub fn entries(&self) -> Vec<(ClaimRecord, Vec<f32>)> {
        self.records
            .iter()
            .cloned()
            .zip(self.vectors.iter().cloned())
            .collect()
    }

    /// The stored claims nearest to a request, each with its vector, in
    /// order of similarity.
    pub fn fetch(&self, query_vector: &[f32], top_k: usize) -> Vec<(MatchedClaim, Vec<f32>)> {
        self.search(query_vector, top_k)
            .into_iter()
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

    /// Positions of the stored claims nearest to a vector, nearest first,
    /// each with its similarity.
    fn search(&self, vector: &[f32], top_k: usize) -> Vec<(usize, f32)> {
        self.hnsw
            .search(vector, top_k)
            .into_iter()
            .filter_map(|(id, similarity)| Some((id.parse::<usize>().ok()?, similarity)))
            .filter(|(position, _)| *position < self.records.len())
            .collect()
    }

    /// The position of the stored claim nearest to a request and its
    /// similarity to it: the head of what `fetch` returns for `top_k`.
    pub fn nearest(&self, query_vector: &[f32], top_k: usize) -> Option<(usize, f32)> {
        self.search(query_vector, top_k).into_iter().next()
    }

    pub fn record(&self, position: usize) -> Option<&ClaimRecord> {
        self.records.get(position)
    }

    /// The stored claims a judge should compare the claim at `position`
    /// with: its nearest claims from other source families that it has no
    /// relation with yet, nearest first.
    pub fn neighbours_to_judge(&self, position: usize, policy: &ClaimPolicy) -> Vec<usize> {
        let (Some(record), Some(vector)) = (self.records.get(position), self.vectors.get(position))
        else {
            return Vec::new();
        };
        self.search(vector, policy.judge_pool)
            .into_iter()
            .map(|(other, _)| other)
            .filter(|other| *other != position)
            .filter(|other| self.records[*other].source_family != record.source_family)
            .take(policy.judge_neighbors)
            .filter(|other| self.relation(position, *other).is_none())
            .collect()
    }

    pub fn relation(&self, a: usize, b: usize) -> Option<ClaimRelation> {
        self.relations.get(&(a.min(b), a.max(b))).copied()
    }

    /// Store what a judge said about two stored claims.
    pub fn set_relation(&mut self, a: usize, b: usize, relation: ClaimRelation) {
        if a != b && a < self.records.len() && b < self.records.len() {
            self.relations.insert((a.min(b), a.max(b)), relation);
        }
    }

    /// The claims related to the one at `position`, with the relation.
    pub fn related(&self, position: usize) -> Vec<(usize, ClaimRelation)> {
        let mut related: Vec<(usize, ClaimRelation)> = self
            .relations
            .iter()
            .filter_map(|(&(a, b), &relation)| match position {
                p if p == a => Some((b, relation)),
                p if p == b => Some((a, relation)),
                _ => None,
            })
            .collect();
        related.sort_by_key(|(other, _)| *other);
        related
    }

    /// Who agrees with and who contradicts the claim at `position`, by
    /// source family, from the stored relations.
    pub fn judged_support(&self, position: usize) -> JudgedSupport {
        let Some(record) = self.records.get(position) else {
            return JudgedSupport::default();
        };
        let related = self.related(position);
        let families = |wanted: ClaimRelation| -> BTreeSet<&str> {
            related
                .iter()
                .filter(|(_, relation)| *relation == wanted)
                .map(|(other, _)| self.records[*other].source_family.as_str())
                .collect()
        };
        let mut agreeing = families(ClaimRelation::Same);
        agreeing.insert(record.source_family.as_str());
        JudgedSupport {
            agreeing_families: agreeing.len(),
            contradicting_families: families(ClaimRelation::Contradicts).len(),
            judged: related.len(),
        }
    }

    /// The judged signal for a request: the nearest stored claim and what
    /// the stored relations say about it. No model is called.
    pub fn judged_corroboration(
        &self,
        query_vector: &[f32],
        policy: &ClaimPolicy,
    ) -> JudgedCorroboration {
        let Some((position, query_similarity)) = self.nearest(query_vector, policy.top_k) else {
            return JudgedCorroboration {
                head: None,
                support: JudgedSupport::default(),
                corroborated: false,
                contested: false,
                agreeing: Vec::new(),
                contradicting: Vec::new(),
            };
        };
        let record = &self.records[position];
        let support = self.judged_support(position);
        let about_the_request = query_similarity >= policy.min_query_similarity;
        let with_relation = |wanted: ClaimRelation| -> Vec<ClaimRecord> {
            self.related(position)
                .into_iter()
                .filter(|(_, relation)| *relation == wanted)
                .map(|(other, _)| self.records[other].clone())
                .collect()
        };
        JudgedCorroboration {
            head: Some(MatchedClaim {
                claim: record.claim.clone(),
                quote: record.quote.clone(),
                url: record.url.clone(),
                source_family: record.source_family.clone(),
                query_similarity,
                head_similarity: 1.0,
            }),
            corroborated: about_the_request
                && support.agreeing_families >= policy.min_source_families,
            contested: about_the_request && support.contradicting_families > 0,
            agreeing: with_relation(ClaimRelation::Same),
            contradicting: with_relation(ClaimRelation::Contradicts),
            support,
        }
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
    fn judged_support_counts_families_by_the_stored_relations() {
        let mut index = ClaimIndex::new(3);
        // Four claims close to each other from four families, and a second
        // claim from the first family.
        for (name, family, vector) in [
            ("a", "one.org", [1.0, 0.0, 0.0]),
            ("a-same", "two.org", [0.99, 0.1, 0.0]),
            ("a-against", "three.org", [0.98, 0.2, 0.0]),
            ("a-other", "four.org", [0.97, 0.25, 0.0]),
            ("a-own", "one.org", [0.96, 0.28, 0.0]),
        ] {
            index.add(record(name, family), vector.to_vec()).unwrap();
        }
        let policy = ClaimPolicy::default();
        // The claim's own family is never a candidate.
        assert_eq!(index.neighbours_to_judge(0, &policy), [1, 2, 3]);
        assert_eq!(index.judged_support(0).agreeing_families, 1);

        index.set_relation(0, 1, ClaimRelation::Same);
        index.set_relation(2, 0, ClaimRelation::Contradicts);
        index.set_relation(0, 3, ClaimRelation::Different);
        let support = index.judged_support(0);
        assert_eq!(support.agreeing_families, 2);
        assert_eq!(support.contradicting_families, 1);
        assert_eq!(support.judged, 3);
        // Seen from the other side of the same relation.
        assert_eq!(index.judged_support(1).agreeing_families, 2);
        assert_eq!(index.relation(1, 0), Some(ClaimRelation::Same));
        // Already judged pairs are not offered again.
        assert!(index.neighbours_to_judge(0, &policy).is_empty());
        assert_eq!(index.nearest(&[1.0, 0.0, 0.0], 5).map(|(p, _)| p), Some(0));

        let policy = ClaimPolicy {
            min_query_similarity: 0.5,
            ..ClaimPolicy::default()
        };
        let signal = index.judged_corroboration(&[1.0, 0.0, 0.0], &policy);
        assert_eq!(
            signal.head.as_ref().map(|head| head.claim.as_str()),
            Some("a")
        );
        assert!(signal.corroborated && signal.contested);
        assert_eq!(signal.agreeing.len(), 1);
        assert_eq!(signal.contradicting[0].claim, "a-against");
        // A request nothing stored is about gets neither label.
        let far = index.judged_corroboration(&[0.0, 0.0, 1.0], &policy);
        assert!(!far.corroborated && !far.contested);
    }

    #[test]
    fn a_repeated_claim_is_stored_once_and_claims_are_found_by_page() {
        let mut index = ClaimIndex::new(3);
        assert!(
            index
                .add(record("a", "one.org"), vec![1.0, 0.0, 0.0])
                .unwrap()
        );
        assert!(
            !index
                .add(record("a", "one.org"), vec![1.0, 0.0, 0.0])
                .unwrap()
        );
        assert!(
            index
                .add(record("a", "two.org"), vec![1.0, 0.0, 0.0])
                .unwrap()
        );
        assert_eq!(index.len(), 2);
        let urls: HashSet<String> = ["https://two.org/page".to_string()].into();
        let found = index.for_urls(&urls);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0.source_family, "two.org");
    }

    #[test]
    fn claims_after_a_sequence_number_come_back_in_order_with_their_numbers() {
        let mut index = ClaimIndex::new(3);
        for name in ["a", "b", "c"] {
            index
                .add(record(name, "one.org"), vec![1.0, 0.0, 0.0])
                .unwrap();
        }
        let later: Vec<(u64, String)> = index
            .since(1, 10)
            .into_iter()
            .map(|(seq, record, _)| (seq, record.claim))
            .collect();
        assert_eq!(later, [(2, "b".to_string()), (3, "c".to_string())]);
        assert_eq!(index.since(0, 1).len(), 1);
        assert!(index.since(3, 10).is_empty());
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
