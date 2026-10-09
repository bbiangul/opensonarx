//! Stage-2 decision metadata for a result slate.
//!
//! Stage 2 may reorder results only on evidence about a claim. Before a model
//! has read the request there is no claim, so every result carries the same
//! evidential state — nothing supports, nothing conflicts, everything is
//! unknown — and the decision order is the retrieval order. Once stance
//! judgments exist, `apply_claim_stance` moves supporting and contradicting
//! results inside the relevance band.
//!
//! Signals recorded here are observations about retained bytes and URLs.
//! None of them is a judgement about what a page says or who wrote it.

use crate::source::source_family;
use oi_index::uncertainty::{
    CashCalibration, ClaimAssessment, DecisionCandidate, DecisionPolicy, EvidenceState,
    OpenUncertainty, OpenUncertaintyConfig, SourceHistory, UncertaintyEstimate, UncertaintyMode,
};
use oi_types::{SearchDecisionMetadata, SearchResultItem};
use std::collections::HashMap;

const POLICY_ID: &str = "search-evidential-v4-claim-stance";
/// Stance may order candidates that are comparably relevant; it never
/// rescues a result whose relevance is further than this from the top one.
const RELEVANCE_BAND: f64 = 0.12;

/// A result's observed stance toward the claims of one answer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClaimStance {
    pub supports: usize,
    pub contradicts: usize,
}

/// Attach Stage-2 metadata without reordering: no claim has been assessed.
pub fn apply_decision_policy(query: &str, results: &mut [SearchResultItem]) {
    let estimator = estimator();
    let families = family_counts(results);
    for (index, result) in results.iter_mut().enumerate() {
        let rank = index as u32 + 1;
        let retrieval_rank = result
            .decision
            .as_ref()
            .map(|decision| decision.retrieval_rank)
            .unwrap_or(rank);
        let estimate = estimator.estimate(&assessment(query, result, ClaimStance::default()));
        result.rank = rank;
        result.decision = Some(metadata(
            retrieval_rank,
            rank,
            result.final_score,
            &estimate,
            observations(result, &families, ClaimStance::default()),
        ));
    }
}

/// Re-run Stage 2 with claim stance keyed by result URL. Results inside the
/// relevance band are ordered by the estimator's decision score; the rest
/// keep their retrieval order below the band.
pub fn apply_claim_stance(
    query: &str,
    results: &mut Vec<SearchResultItem>,
    stance: &HashMap<String, ClaimStance>,
) {
    if results.is_empty() {
        return;
    }
    let estimator = estimator();
    let families = family_counts(results);
    let strongest = results
        .iter()
        .map(|result| result.final_score)
        .max_by(f64::total_cmp)
        .unwrap_or_default();
    let candidates = std::mem::take(results)
        .into_iter()
        .enumerate()
        .map(|(index, result)| {
            let observed = stance.get(&result.url).copied().unwrap_or_default();
            let retrieval_rank = result
                .decision
                .as_ref()
                .map(|decision| decision.retrieval_rank as usize)
                .unwrap_or(index + 1);
            DecisionCandidate {
                estimate: estimator.estimate(&assessment(query, &result, observed)),
                base_relevance: result.final_score,
                licensed_cost: 0.0,
                retrieval_rank,
                item: (observations(&result, &families, observed), result),
            }
        })
        .collect();
    let mut decided = DecisionPolicy::default().rerank(candidates);
    decided.sort_by(|left, right| {
        let left_in_band = left.base_relevance + RELEVANCE_BAND >= strongest;
        let right_in_band = right.base_relevance + RELEVANCE_BAND >= strongest;
        right_in_band
            .cmp(&left_in_band)
            .then_with(|| {
                if left_in_band {
                    right.decision_score.total_cmp(&left.decision_score)
                } else {
                    std::cmp::Ordering::Equal
                }
            })
            .then_with(|| left.retrieval_rank.cmp(&right.retrieval_rank))
    });
    *results = decided
        .into_iter()
        .enumerate()
        .map(|(index, decision)| {
            let (mut signals, mut result) = decision.item;
            if decision.base_relevance + RELEVANCE_BAND < strongest {
                signals.push("outside_top_relevance_band".to_string());
            }
            let rank = index as u32 + 1;
            result.rank = rank;
            result.decision = Some(metadata(
                decision.retrieval_rank as u32,
                rank,
                decision.decision_score,
                &decision.estimate,
                signals,
            ));
            result
        })
        .collect();
}

fn estimator() -> OpenUncertainty {
    let config = OpenUncertaintyConfig::default();
    OpenUncertainty::new(config.clone(), CashCalibration::fit(&[], &config))
}

fn family_counts(results: &[SearchResultItem]) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for result in results {
        *counts.entry(source_family(&result.url)).or_default() += 1;
    }
    counts
}

fn assessment(query: &str, result: &SearchResultItem, stance: ClaimStance) -> ClaimAssessment {
    ClaimAssessment {
        claim_ref: format!("search:{}:{}", stable_ref(query), result.result_id),
        task: "search/slate".to_string(),
        horizon: "request".to_string(),
        policy_id: POLICY_ID.to_string(),
        source_history: SourceHistory::default(),
        evidence: EvidenceState {
            support_mass: stance.supports as f64,
            conflict_mass: stance.contradicts as f64,
            // Text that was never retained cannot be relied on for a claim.
            critical_missing: result.content_md.trim().is_empty(),
            ..EvidenceState::default()
        },
        conviction: None,
        deterministic_outcome: None,
    }
}

fn observations(
    result: &SearchResultItem,
    families: &HashMap<String, usize>,
    stance: ClaimStance,
) -> Vec<String> {
    let mut signals = Vec::new();
    signals.push(
        if result.content_md.trim().is_empty() {
            "content_not_retained"
        } else {
            "content_retained"
        }
        .to_string(),
    );
    if result.published_at.is_some() {
        signals.push("publication_time_declared".to_string());
    }
    if result.crawled_at.is_some() {
        signals.push("observation_time_recorded".to_string());
    }
    if families
        .get(&source_family(&result.url))
        .is_some_and(|count| *count > 1)
    {
        signals.push("source_family_repeated_in_slate".to_string());
    }
    if stance.supports > 0 {
        signals.push(format!("supports_claims:{}", stance.supports));
    }
    if stance.contradicts > 0 {
        signals.push(format!("contradicts_claims:{}", stance.contradicts));
    }
    if stance == ClaimStance::default() {
        signals.push("no_claim_stance_assessed".to_string());
    }
    signals
}

fn metadata(
    retrieval_rank: u32,
    decision_rank: u32,
    decision_score: f64,
    estimate: &UncertaintyEstimate,
    evidence_signals: Vec<String>,
) -> SearchDecisionMetadata {
    SearchDecisionMetadata {
        retrieval_rank,
        decision_rank,
        decision_score,
        policy_id: estimate.policy_id.clone(),
        uncertainty_mode: match estimate.mode {
            UncertaintyMode::Deterministic => "deterministic",
            UncertaintyMode::Calibrated => "calibrated",
            UncertaintyMode::EvidentialOnly => "evidential_only",
        }
        .to_string(),
        p_reliable: estimate.p_reliable,
        interval_90: estimate.interval_90,
        support: estimate.support,
        conflict: estimate.conflict,
        ignorance: estimate.ignorance,
        critical_missing: estimate.critical_missing,
        independence: estimate.independence,
        dependency_uncertainty: estimate.uncertainty.dependency_uncertainty,
        staleness_uncertainty: estimate.uncertainty.staleness_uncertainty,
        distribution_shift: estimate.uncertainty.distribution_shift,
        cash_eligible: estimate.cash.eligible,
        cash_calibrated: estimate.cash.calibrated,
        cash_log_bayes_factor: estimate.cash.log_bayes_factor,
        evidence_signals,
        evidence_lineage: None,
    }
}

fn stable_ref(query: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(query.as_bytes());
    digest[..6]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(url: &str, score: f64, content: &str) -> SearchResultItem {
        SearchResultItem {
            result_id: url.to_string(),
            rank: 0,
            url: url.to_string(),
            title: "t".to_string(),
            snippet: "s".to_string(),
            content_md: content.to_string(),
            semantic_score: score,
            bm25_score: 0.0,
            final_score: score,
            stake_truth: 0,
            stake_usd: 0,
            stake_type: "none".to_string(),
            reputation: 0.0,
            content_hash: String::new(),
            extraction_quality: "full".to_string(),
            source: "local".to_string(),
            entity: None,
            content_kind: None,
            crawled_at: None,
            published_at: None,
            trust_verdict: "unverified".to_string(),
            reputation_votes: 0,
            disputed: false,
            facts: None,
            content_relevant: None,
            domain_first_seen_at: None,
            domain_url_count: None,
            domain_reputation_score: None,
            domain_total_votes: None,
            decision: None,
        }
    }

    fn urls(results: &[SearchResultItem]) -> Vec<&str> {
        results.iter().map(|result| result.url.as_str()).collect()
    }

    #[test]
    fn without_claim_stance_the_decision_order_is_the_retrieval_order() {
        let mut results = vec![
            result("https://a.test/1", 0.90, ""),
            result("https://b.test/2", 0.85, "retained"),
            result("https://b.test/3", 0.40, "retained"),
        ];
        apply_decision_policy("q", &mut results);
        assert_eq!(
            urls(&results),
            vec!["https://a.test/1", "https://b.test/2", "https://b.test/3"]
        );
        for (index, result) in results.iter().enumerate() {
            let decision = result.decision.as_ref().unwrap();
            assert_eq!(decision.retrieval_rank, index as u32 + 1);
            assert_eq!(decision.decision_rank, index as u32 + 1);
            assert_eq!(decision.uncertainty_mode, "evidential_only");
            assert_eq!(decision.support, 0.0);
            assert_eq!(decision.conflict, 0.0);
            assert_eq!(decision.ignorance, 1.0);
            assert_eq!(decision.p_reliable, None);
            assert_eq!(decision.cash_log_bayes_factor, 0.0);
        }
        let first = results[0].decision.as_ref().unwrap();
        assert!(first.critical_missing);
        assert!(
            first
                .evidence_signals
                .contains(&"content_not_retained".to_string())
        );
        let second = results[1].decision.as_ref().unwrap();
        assert!(!second.critical_missing);
        assert!(
            second
                .evidence_signals
                .contains(&"source_family_repeated_in_slate".to_string())
        );
    }

    #[test]
    fn supporting_result_moves_above_an_unassessed_one_inside_the_band() {
        let mut results = vec![
            result("https://a.test/1", 0.90, "retained"),
            result("https://b.test/2", 0.85, "retained"),
        ];
        apply_decision_policy("q", &mut results);
        let stance = HashMap::from([(
            "https://b.test/2".to_string(),
            ClaimStance {
                supports: 1,
                contradicts: 0,
            },
        )]);
        apply_claim_stance("q", &mut results, &stance);
        assert_eq!(urls(&results), vec!["https://b.test/2", "https://a.test/1"]);
        let top = results[0].decision.as_ref().unwrap();
        assert_eq!(top.retrieval_rank, 2);
        assert_eq!(top.decision_rank, 1);
        assert!(top.support > 0.0);
    }

    #[test]
    fn stance_cannot_rescue_a_result_outside_the_relevance_band() {
        let mut results = vec![
            result("https://a.test/1", 0.90, "retained"),
            result("https://b.test/2", 0.40, "retained"),
        ];
        apply_decision_policy("q", &mut results);
        let stance = HashMap::from([(
            "https://b.test/2".to_string(),
            ClaimStance {
                supports: 3,
                contradicts: 0,
            },
        )]);
        apply_claim_stance("q", &mut results, &stance);
        assert_eq!(urls(&results), vec!["https://a.test/1", "https://b.test/2"]);
        assert!(
            results[1]
                .decision
                .as_ref()
                .unwrap()
                .evidence_signals
                .contains(&"outside_top_relevance_band".to_string())
        );
    }

    #[test]
    fn contradicting_result_carries_conflict_and_does_not_outrank_support() {
        let mut results = vec![
            result("https://a.test/1", 0.90, "retained"),
            result("https://b.test/2", 0.88, "retained"),
        ];
        apply_decision_policy("q", &mut results);
        let stance = HashMap::from([
            (
                "https://a.test/1".to_string(),
                ClaimStance {
                    supports: 0,
                    contradicts: 1,
                },
            ),
            (
                "https://b.test/2".to_string(),
                ClaimStance {
                    supports: 1,
                    contradicts: 0,
                },
            ),
        ]);
        apply_claim_stance("q", &mut results, &stance);
        assert_eq!(urls(&results), vec!["https://b.test/2", "https://a.test/1"]);
        assert!(results[1].decision.as_ref().unwrap().conflict > 0.0);
    }
}
