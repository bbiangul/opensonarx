//! Calibrated probability that a complete answer is correct.
//!
//! A probability is emitted only where the uncertainty engine allows one:
//! the claim's checklist is complete, enough resolved outcomes exist for
//! answers assembled in the same evidence state, and the declared shift
//! between the calibration data and the place of use is below the engine's
//! limit. Everywhere else the answer stays evidential and carries no number.
//!
//! The table is data. It is fitted and evaluated by `oi-bench
//! answer-calibrate` on held-out cases and names its target, its datasets and
//! its limits; nothing in this module estimates anything on its own.

use crate::answer::SearchAnswer;
use crate::checklist::EvidenceChecklist;
use oi_index::uncertainty::{
    CashCalibration, ClaimAssessment, EvidenceState, OpenUncertainty, OpenUncertaintyConfig,
    SourceHistory, UncertaintyMode,
};
use serde::{Deserialize, Serialize};

/// Resolved outcomes of complete answers in one evidence state: the weakest
/// answering claim was supported by this many source families (the largest
/// count also covers larger ones) and, when `reported_gap` is set, the
/// extractor did or did not note that the passages lack something.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalibrationBin {
    pub supporting_families: usize,
    /// None when the table does not split on the extractor's gap note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reported_gap: Option<bool>,
    pub correct: u32,
    pub incorrect: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnswerCalibration {
    /// Recorded on every answer that carries a probability from this table.
    pub calibration_id: String,
    /// What "correct" meant when the outcomes were resolved.
    pub target: String,
    /// How far the place of use is from the calibration data, in [0, 1], as
    /// declared by whoever deploys the table. It widens every interval and,
    /// at the engine's out-of-distribution limit, suppresses the probability.
    pub declared_distribution_shift: f64,
    /// Whether the table beat its declared baseline on held-out outcomes.
    /// A table that did not is kept as a record and never produces a
    /// probability.
    #[serde(default)]
    pub accepted: bool,
    pub bins: Vec<CalibrationBin>,
    /// How the table was produced and evaluated; not interpreted here.
    #[serde(default)]
    pub provenance: serde_json::Value,
}

impl AnswerCalibration {
    pub fn load(path: &str) -> oi_types::Result<Self> {
        let raw = std::fs::read_to_string(path)?;
        serde_json::from_str(&raw)
            .map_err(|error| oi_types::Error::Serialization(format!("{path}: {error}")))
    }

    /// The bin for a claim supported by `families` source families in an
    /// answer whose extractor did (`reported_gap`) or did not note a gap: an
    /// exact family match, or the largest family bin when `families` exceeds
    /// it. A table that does not split on the gap note matches either value.
    /// A count below the smallest bin has no reference class.
    pub fn bin_for(&self, families: usize, reported_gap: bool) -> Option<CalibrationBin> {
        let candidates: Vec<&CalibrationBin> = self
            .bins
            .iter()
            .filter(|bin| bin.reported_gap.is_none_or(|gap| gap == reported_gap))
            .collect();
        let largest = candidates.iter().map(|bin| bin.supporting_families).max()?;
        candidates
            .into_iter()
            .find(|bin| bin.supporting_families == families.min(largest))
            .copied()
    }

    /// Attach a probability to each answering claim of a complete answer.
    /// Does nothing to partial, abstained or incomplete answers.
    pub fn apply(&self, answer: &mut SearchAnswer, checklist: &EvidenceChecklist) {
        if !self.accepted
            || answer.status != "extractive"
            || checklist.critical_missing
            || !checklist.complete_answer_supported
        {
            return;
        }
        let reported_gap = checklist
            .requirements
            .iter()
            .any(|item| item.id.starts_with(crate::answer::MODEL_REPORTED_GAP));
        let config = OpenUncertaintyConfig::default();
        let estimator = OpenUncertainty::new(config.clone(), CashCalibration::fit(&[], &config));
        let mut calibrated = false;
        for (index, claim) in answer.claims.iter_mut().enumerate() {
            if claim.status != "corroborated" {
                continue;
            }
            let Some(lineage) = &claim.evidence_lineage else {
                continue;
            };
            let Some(bin) = self.bin_for(lineage.reporting_source_families, reported_gap) else {
                continue;
            };
            let estimate = estimator.estimate(&ClaimAssessment {
                claim_ref: format!("claim:{index}"),
                task: "answer/correctness".to_string(),
                horizon: "request".to_string(),
                policy_id: self.calibration_id.clone(),
                // The reference class is the bin: answers assembled in the
                // same evidence state, with their resolved outcomes.
                source_history: SourceHistory {
                    eligible_random_passes: bin.correct,
                    eligible_random_failures: bin.incorrect,
                },
                evidence: EvidenceState {
                    support_mass: lineage.reporting_source_families as f64,
                    conflict_mass: 0.0,
                    missing_mass: 0.0,
                    critical_missing: false,
                    independence: 1.0,
                    // Unresolved derivation is part of every outcome in the
                    // bin. Identical wording across families is not: it is
                    // extra evidence of a shared origin and widens the
                    // interval.
                    dependency_uncertainty: if lineage.shared_wording_families > 0 {
                        1.0
                    } else {
                        0.0
                    },
                    staleness_uncertainty: 0.0,
                    distribution_shift: self.declared_distribution_shift,
                },
                conviction: None,
                deterministic_outcome: None,
            });
            if estimate.mode == UncertaintyMode::Calibrated {
                claim.p_correct = estimate.p_reliable;
                claim.interval_90 = estimate.interval_90;
                calibrated = true;
            }
        }
        if calibrated {
            answer.uncertainty_mode = "calibrated".to_string();
            answer.calibration_id = Some(self.calibration_id.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::answer::{AnswerClaim, ClaimEvidenceLineage};

    fn table(bins: &[(usize, u32, u32)], shift: f64) -> AnswerCalibration {
        AnswerCalibration {
            calibration_id: "test-calibration".to_string(),
            target: "matches a reference answer".to_string(),
            declared_distribution_shift: shift,
            accepted: true,
            bins: bins
                .iter()
                .map(
                    |&(supporting_families, correct, incorrect)| CalibrationBin {
                        supporting_families,
                        reported_gap: None,
                        correct,
                        incorrect,
                    },
                )
                .collect(),
            provenance: serde_json::Value::Null,
        }
    }

    fn claim(status: &str, families: usize, shared: usize) -> AnswerClaim {
        AnswerClaim {
            text: "A claim.".to_string(),
            status: status.to_string(),
            citations: Vec::new(),
            evidence_lineage: Some(ClaimEvidenceLineage {
                reporting_source_families: families,
                contradicting_source_families: 0,
                verbatim_citations: families,
                shared_wording_families: shared,
                independence_status: "unresolved".to_string(),
            }),
            support: 0.5,
            conflict: 0.0,
            ignorance: 0.5,
            p_correct: None,
            interval_90: None,
        }
    }

    fn answer(status: &str, claims: Vec<AnswerClaim>) -> SearchAnswer {
        SearchAnswer {
            status: status.to_string(),
            intent: "question".to_string(),
            summary: "A claim.".to_string(),
            claims,
            uncertainty_mode: "evidential_only".to_string(),
            average_ignorance: None,
            warning: None,
            calibration_id: None,
        }
    }

    fn complete() -> EvidenceChecklist {
        EvidenceChecklist {
            policy_id: "p".to_string(),
            complete_answer_supported: true,
            critical_missing: false,
            requirements: Vec::new(),
        }
    }

    #[test]
    fn complete_answer_gets_the_bin_rate_with_an_interval_around_it() {
        let calibration = table(&[(2, 90, 10)], 0.0);
        let mut answer = answer("extractive", vec![claim("corroborated", 2, 0)]);
        calibration.apply(&mut answer, &complete());
        let p = answer.claims[0].p_correct.unwrap();
        // Beta(1 + 90, 1 + 10) mean.
        assert!((p - 91.0 / 102.0).abs() < 1e-9);
        let (lower, upper) = answer.claims[0].interval_90.unwrap();
        assert!(lower < p && p < upper);
        assert_eq!(answer.uncertainty_mode, "calibrated");
        assert_eq!(answer.calibration_id.as_deref(), Some("test-calibration"));
    }

    #[test]
    fn no_probability_without_enough_outcomes_or_a_reference_class() {
        // 19 outcomes: below the engine's minimum of 20.
        let thin = table(&[(2, 18, 1)], 0.0);
        let mut answer_thin = answer("extractive", vec![claim("corroborated", 2, 0)]);
        thin.apply(&mut answer_thin, &complete());
        assert_eq!(answer_thin.claims[0].p_correct, None);
        assert_eq!(answer_thin.uncertainty_mode, "evidential_only");
        assert_eq!(answer_thin.calibration_id, None);

        // One family, but only a two-family bin exists.
        let calibration = table(&[(2, 90, 10)], 0.0);
        let mut single = answer("extractive", vec![claim("corroborated", 1, 0)]);
        calibration.apply(&mut single, &complete());
        assert_eq!(single.claims[0].p_correct, None);
    }

    #[test]
    fn larger_family_counts_use_the_largest_bin_only() {
        let calibration = table(&[(1, 70, 30), (2, 90, 10)], 0.0);
        assert_eq!(
            calibration.bin_for(1, false).unwrap().supporting_families,
            1
        );
        assert_eq!(calibration.bin_for(2, true).unwrap().supporting_families, 2);
        assert_eq!(
            calibration.bin_for(5, false).unwrap().supporting_families,
            2
        );
        assert!(calibration.bin_for(0, false).is_none());
        // A hole in the table is not filled from the bin below it.
        let holed = table(&[(1, 70, 30), (3, 95, 5)], 0.0);
        assert!(holed.bin_for(2, false).is_none());
        assert_eq!(holed.bin_for(4, false).unwrap().supporting_families, 3);
    }

    #[test]
    fn a_table_split_on_the_gap_note_uses_the_matching_bin() {
        let mut calibration = table(&[(2, 35, 1), (2, 21, 12)], 0.0);
        calibration.bins[0].reported_gap = Some(false);
        calibration.bins[1].reported_gap = Some(true);
        assert_eq!(calibration.bin_for(2, false).unwrap().correct, 35);
        assert_eq!(calibration.bin_for(3, true).unwrap().correct, 21);

        let with_gap = |gap: bool| {
            let mut answer = answer("extractive", vec![claim("corroborated", 2, 0)]);
            let mut checklist = complete();
            if gap {
                checklist
                    .requirements
                    .push(crate::checklist::EvidenceRequirement {
                        id: format!("{} founding year", crate::answer::MODEL_REPORTED_GAP),
                        claim_index: None,
                        critical: false,
                        status: crate::checklist::RequirementStatus::Missing,
                        evidence_refs: Vec::new(),
                    });
            }
            calibration.apply(&mut answer, &checklist);
            answer.claims[0].p_correct.unwrap()
        };
        assert!((with_gap(false) - 36.0 / 38.0).abs() < 1e-9);
        assert!((with_gap(true) - 22.0 / 35.0).abs() < 1e-9);
    }

    #[test]
    fn a_table_that_was_not_accepted_never_produces_a_probability() {
        let mut calibration = table(&[(2, 90, 10)], 0.0);
        calibration.accepted = false;
        let mut answer = answer("extractive", vec![claim("corroborated", 2, 0)]);
        calibration.apply(&mut answer, &complete());
        assert_eq!(answer.claims[0].p_correct, None);
        assert_eq!(answer.uncertainty_mode, "evidential_only");
        // A table file without the field is not accepted either.
        let parsed: AnswerCalibration = serde_json::from_str(
            r#"{"calibration_id":"x","target":"t","declared_distribution_shift":0.0,"bins":[]}"#,
        )
        .unwrap();
        assert!(!parsed.accepted);
    }

    #[test]
    fn partial_abstained_and_incomplete_answers_stay_evidential() {
        let calibration = table(&[(2, 90, 10)], 0.0);
        for status in ["partial_answer", "insufficient_evidence", "results_only"] {
            let mut other = answer(status, vec![claim("corroborated", 2, 0)]);
            calibration.apply(&mut other, &complete());
            assert_eq!(other.claims[0].p_correct, None, "{status}");
        }
        let mut pending = answer("extractive", vec![claim("corroborated", 2, 0)]);
        let mut checklist = complete();
        checklist.critical_missing = true;
        calibration.apply(&mut pending, &checklist);
        assert_eq!(pending.claims[0].p_correct, None);

        // Only the claims that answer get a number.
        let mut mixed = answer(
            "extractive",
            vec![claim("corroborated", 2, 0), claim("off_request", 2, 0)],
        );
        calibration.apply(&mut mixed, &complete());
        assert!(mixed.claims[0].p_correct.is_some());
        assert_eq!(mixed.claims[1].p_correct, None);
    }

    #[test]
    fn declared_shift_widens_the_interval_and_suppresses_at_the_limit() {
        let width = |shift: f64, shared: usize| {
            let mut answer = answer("extractive", vec![claim("corroborated", 2, shared)]);
            table(&[(2, 90, 10)], shift).apply(&mut answer, &complete());
            answer.claims[0]
                .interval_90
                .map(|(lower, upper)| upper - lower)
        };
        let base = width(0.0, 0).unwrap();
        assert!(width(0.5, 0).unwrap() > base);
        assert!(width(0.0, 2).unwrap() > base, "shared wording widens");
        assert_eq!(
            width(0.8, 0),
            None,
            "at the engine's limit there is no probability"
        );
    }
}
