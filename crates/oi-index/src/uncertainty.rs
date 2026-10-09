//! Claim-level uncertainty estimation and risk-aware decision ranking.
//!
//! This module is intentionally separate from Stage-1 retrieval. It treats cash
//! as a diagnostic until a conditional, forward-validated model establishes
//! incremental information beyond the no-cash baseline. Marginal cash/outcome
//! association never updates a probability. Raw domain stake never enters relevance.

use serde::{Deserialize, Serialize};

const EPS: f64 = 1e-9;
const Z_90: f64 = 1.644_853_626_951_472_2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UncertaintyMode {
    Deterministic,
    Calibrated,
    EvidentialOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeterministicOutcome {
    Pass,
    Fail,
    Unresolved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettlementGrade {
    /// A reproducible predicate over retained inputs.
    Deterministic,
    /// A pre-declared predicate over a machine-observable future event.
    EventBound,
    /// No objective capital-settlement predicate. Cash is not truth evidence.
    EvidenceOnly,
}

/// Money placed at risk on one exact claim.
///
/// Every continuous input is clamped by the estimator. The signal deliberately
/// includes loss, time, scope, funding independence, and hedge information so a
/// headline amount is not mistaken for comparable economic exposure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvictionSignal {
    pub bond_usd: f64,
    pub max_loss_fraction: f64,
    pub lock_duration_days: f64,
    pub predicate_coverage: f64,
    pub funding_independence: f64,
    pub unhedged_fraction: f64,
    pub posted_before_outcome: bool,
    pub settlement_grade: SettlementGrade,
}

impl ConvictionSignal {
    pub fn is_eligible(&self) -> bool {
        self.posted_before_outcome
            && self.bond_usd.is_finite()
            && self.bond_usd > 0.0
            && self.max_loss_fraction.is_finite()
            && self.max_loss_fraction > 0.0
            && self.lock_duration_days.is_finite()
            && self.lock_duration_days > 0.0
            && self.predicate_coverage.is_finite()
            && self.predicate_coverage > 0.0
            && self.funding_independence.is_finite()
            && self.funding_independence > 0.0
            && self.unhedged_fraction.is_finite()
            && self.unhedged_fraction > 0.0
            && self.settlement_grade != SettlementGrade::EvidenceOnly
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CashBucket {
    None,
    Low,
    Medium,
    High,
}

impl CashBucket {
    fn index(self) -> usize {
        match self {
            Self::None => 0,
            Self::Low => 1,
            Self::Medium => 2,
            Self::High => 3,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenUncertaintyConfig {
    /// Eligible randomized outcomes required for probabilistic mode.
    pub min_source_outcomes: u32,
    /// Resolved calibration cases required in a cash bucket and its baseline.
    pub min_cash_bucket_outcomes: u32,
    /// Claim-class reference amount. Cash is log-scaled relative to this value.
    pub reference_bond_usd: f64,
    /// Amount normalization saturates at this multiple of the reference bond.
    pub bond_cap_multiple: f64,
    /// Duration at which the capital-time component reaches full weight.
    pub reference_lock_days: f64,
    /// Absolute cap on learned cash log-likelihood contribution.
    pub max_abs_cash_log_bayes_factor: f64,
    /// At or above this OOD score, no calibrated probability is emitted.
    pub ood_fallback_threshold: f64,
    pub prior_alpha: f64,
    pub prior_beta: f64,
}

impl Default for OpenUncertaintyConfig {
    fn default() -> Self {
        Self {
            min_source_outcomes: 20,
            min_cash_bucket_outcomes: 20,
            reference_bond_usd: 1_000.0,
            bond_cap_multiple: 100.0,
            reference_lock_days: 30.0,
            // Cash can move prior odds by at most 3x in either direction in MVP-1.
            max_abs_cash_log_bayes_factor: 3.0_f64.ln(),
            ood_fallback_threshold: 0.8,
            prior_alpha: 1.0,
            prior_beta: 1.0,
        }
    }
}

impl OpenUncertaintyConfig {
    pub fn effective_cash_exposure(&self, signal: &ConvictionSignal) -> f64 {
        if !signal.is_eligible() {
            return 0.0;
        }

        let reference = self.reference_bond_usd.max(EPS);
        let cap = self.bond_cap_multiple.max(1.0);
        let capped_amount = signal.bond_usd.max(0.0).min(reference * cap);
        let amount = (1.0 + capped_amount / reference).ln() / (1.0 + cap).ln();
        let time = (signal.lock_duration_days.max(0.0) / self.reference_lock_days.max(EPS))
            .clamp(0.0, 1.0);

        (amount
            * signal.max_loss_fraction.clamp(0.0, 1.0)
            * time
            * signal.predicate_coverage.clamp(0.0, 1.0)
            * signal.funding_independence.clamp(0.0, 1.0)
            * signal.unhedged_fraction.clamp(0.0, 1.0))
        .clamp(0.0, 1.0)
    }

    pub fn cash_bucket(&self, signal: Option<&ConvictionSignal>) -> CashBucket {
        let exposure = signal
            .map(|s| self.effective_cash_exposure(s))
            .unwrap_or(0.0);
        if exposure <= 0.0 {
            CashBucket::None
        } else if exposure < 0.2 {
            CashBucket::Low
        } else if exposure < 0.6 {
            CashBucket::Medium
        } else {
            CashBucket::High
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedClaim {
    pub reliable: bool,
    pub conviction: Option<ConvictionSignal>,
    /// True only for an outcome sampled under the declared calibration design.
    pub eligible_for_calibration: bool,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct CashBucketStats {
    pub passes: u32,
    pub failures: u32,
}

impl CashBucketStats {
    pub fn total(self) -> u32 {
        self.passes + self.failures
    }

    fn smoothed_probability(self) -> f64 {
        (self.passes as f64 + 0.5) / (self.total() as f64 + 1.0)
    }
}

/// Descriptive relationship between economic conviction and outcomes.
///
/// This object is fit per claim class/task/horizon. Reusing it across classes
/// without a validated hierarchical model is invalid. Even within a class,
/// marginal bucket rates are not an incremental Bayes factor conditional on
/// source history and evidence. They must not update reliance probabilities.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CashCalibration {
    pub buckets: [CashBucketStats; 4],
    pub eligible_cases: u32,
}

impl CashCalibration {
    pub fn fit(cases: &[ResolvedClaim], config: &OpenUncertaintyConfig) -> Self {
        let mut buckets = [CashBucketStats::default(); 4];
        let mut eligible_cases = 0;

        for case in cases.iter().filter(|c| c.eligible_for_calibration) {
            eligible_cases += 1;
            let bucket = config.cash_bucket(case.conviction.as_ref());
            let stats = &mut buckets[bucket.index()];
            if case.reliable {
                stats.passes += 1;
            } else {
                stats.failures += 1;
            }
        }

        Self {
            buckets,
            eligible_cases,
        }
    }

    pub fn effect(
        &self,
        conviction: Option<&ConvictionSignal>,
        config: &OpenUncertaintyConfig,
    ) -> CashEffect {
        let exposure = conviction
            .map(|s| config.effective_cash_exposure(s))
            .unwrap_or(0.0);
        let bucket = config.cash_bucket(conviction);
        let bucket_stats = self.buckets[bucket.index()];

        if bucket == CashBucket::None {
            return CashEffect {
                eligible: false,
                calibrated: false,
                effective_exposure: exposure,
                bucket,
                sample_count: bucket_stats.total(),
                log_bayes_factor: 0.0,
                log_bayes_factor_std_error: 0.0,
                model_status: CashModelStatus::Ineligible,
            };
        }

        let baseline = self.buckets[CashBucket::None.index()];
        let enough = bucket_stats.total() >= config.min_cash_bucket_outcomes
            && baseline.total() >= config.min_cash_bucket_outcomes;
        if !enough {
            return CashEffect {
                eligible: true,
                calibrated: false,
                effective_exposure: exposure,
                bucket,
                sample_count: bucket_stats.total(),
                log_bayes_factor: 0.0,
                log_bayes_factor_std_error: 0.0,
                model_status: CashModelStatus::InsufficientData,
            };
        }

        // A large marginal association can arise entirely because stronger
        // sources or easier claims select cash. Adding it to source history
        // would count the same information twice. Fail closed until a joint
        // or conditional residual model passes the independent forward gate.
        CashEffect {
            eligible: true,
            calibrated: false,
            effective_exposure: exposure,
            bucket,
            sample_count: bucket_stats.total(),
            log_bayes_factor: 0.0,
            log_bayes_factor_std_error: 0.0,
            model_status: CashModelStatus::DisabledMarginalAssociation,
        }
    }

    /// Diagnostic only. This is an association, not a Bayes factor or a
    /// causal/incremental effect. It never feeds `OpenUncertainty::estimate`.
    pub fn marginal_log_odds(
        &self,
        conviction: Option<&ConvictionSignal>,
        config: &OpenUncertaintyConfig,
    ) -> Option<f64> {
        let bucket = config.cash_bucket(conviction);
        let cash = self.buckets[bucket.index()];
        let baseline = self.buckets[CashBucket::None.index()];
        if bucket == CashBucket::None
            || cash.total() < config.min_cash_bucket_outcomes
            || baseline.total() < config.min_cash_bucket_outcomes
        {
            return None;
        }
        Some(logit(cash.smoothed_probability()) - logit(baseline.smoothed_probability()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CashEffect {
    pub eligible: bool,
    pub calibrated: bool,
    pub effective_exposure: f64,
    pub bucket: CashBucket,
    pub sample_count: u32,
    pub log_bayes_factor: f64,
    pub log_bayes_factor_std_error: f64,
    #[serde(default)]
    pub model_status: CashModelStatus,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CashModelStatus {
    Ineligible,
    InsufficientData,
    #[default]
    DisabledMarginalAssociation,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct SourceHistory {
    pub eligible_random_passes: u32,
    pub eligible_random_failures: u32,
}

impl SourceHistory {
    pub fn total(self) -> u32 {
        self.eligible_random_passes + self.eligible_random_failures
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct EvidenceState {
    /// Non-probabilistic evidence mass supporting the exact claim.
    pub support_mass: f64,
    /// Non-probabilistic evidence mass conflicting with the exact claim.
    pub conflict_mass: f64,
    /// Missing evidence mass declared by the task policy.
    pub missing_mass: f64,
    /// A required task-checklist item is unavailable or unresolved. Neither
    /// source history nor repeated support can authorize a complete answer.
    #[serde(default)]
    pub critical_missing: bool,
    /// Estimated independence of this evidence unit after collapsing known
    /// derivations. A representative underlying event is `1`; another page
    /// that merely reports that same event is `0`. This is deliberately
    /// separate from `dependency_uncertainty`, which describes what remains
    /// unknown about common ancestry.
    #[serde(default = "fully_independent")]
    pub independence: f64,
    pub dependency_uncertainty: f64,
    pub staleness_uncertainty: f64,
    pub distribution_shift: f64,
}

impl Default for EvidenceState {
    fn default() -> Self {
        Self {
            support_mass: 0.0,
            conflict_mass: 0.0,
            missing_mass: 1.0,
            critical_missing: false,
            independence: 1.0,
            dependency_uncertainty: 1.0,
            staleness_uncertainty: 0.0,
            distribution_shift: 1.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimAssessment {
    pub claim_ref: String,
    pub task: String,
    pub horizon: String,
    pub policy_id: String,
    pub source_history: SourceHistory,
    pub evidence: EvidenceState,
    pub conviction: Option<ConvictionSignal>,
    pub deterministic_outcome: Option<DeterministicOutcome>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct UncertaintyVector {
    pub posterior_dispersion: Option<f64>,
    pub ignorance: f64,
    pub conflict: f64,
    pub dependency_uncertainty: f64,
    pub staleness_uncertainty: f64,
    pub distribution_shift: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UncertaintyEstimate {
    pub claim_ref: String,
    pub task: String,
    pub horizon: String,
    pub policy_id: String,
    pub mode: UncertaintyMode,
    pub deterministic_outcome: Option<DeterministicOutcome>,
    pub p_reliable: Option<f64>,
    pub interval_90: Option<(f64, f64)>,
    pub support: f64,
    pub conflict: f64,
    pub ignorance: f64,
    #[serde(default)]
    pub critical_missing: bool,
    /// Estimated independence of the evidence unit represented by this
    /// estimate. It is not a truth probability.
    pub independence: f64,
    pub uncertainty: UncertaintyVector,
    pub cash: CashEffect,
    pub source_outcomes: u32,
}

#[derive(Debug, Clone)]
pub struct OpenUncertainty {
    config: OpenUncertaintyConfig,
    cash_calibration: CashCalibration,
}

impl OpenUncertainty {
    pub fn new(config: OpenUncertaintyConfig, cash_calibration: CashCalibration) -> Self {
        Self {
            config,
            cash_calibration,
        }
    }

    pub fn config(&self) -> &OpenUncertaintyConfig {
        &self.config
    }

    pub fn estimate(&self, assessment: &ClaimAssessment) -> UncertaintyEstimate {
        let source_outcomes = assessment.source_history.total();
        let (support, conflict, ignorance) =
            normalized_evidence(assessment.evidence, source_outcomes, &self.config);
        let cash = self
            .cash_calibration
            .effect(assessment.conviction.as_ref(), &self.config);

        let base_vector = UncertaintyVector {
            posterior_dispersion: None,
            ignorance,
            conflict,
            dependency_uncertainty: clamp01(assessment.evidence.dependency_uncertainty),
            staleness_uncertainty: clamp01(assessment.evidence.staleness_uncertainty),
            distribution_shift: clamp01(assessment.evidence.distribution_shift),
        };

        if let Some(outcome) = assessment.deterministic_outcome {
            return self.output(
                assessment,
                UncertaintyMode::Deterministic,
                Some(outcome),
                None,
                None,
                support,
                conflict,
                ignorance,
                base_vector,
                cash,
            );
        }

        let enough_source_data = source_outcomes >= self.config.min_source_outcomes;
        let in_distribution = assessment.evidence.distribution_shift.is_finite()
            && assessment.evidence.distribution_shift < self.config.ood_fallback_threshold;
        if !enough_source_data || !in_distribution || assessment.evidence.critical_missing {
            return self.output(
                assessment,
                UncertaintyMode::EvidentialOnly,
                None,
                None,
                None,
                support,
                conflict,
                ignorance,
                base_vector,
                cash,
            );
        }

        let alpha = self.config.prior_alpha.max(EPS)
            + assessment.source_history.eligible_random_passes as f64;
        let beta = self.config.prior_beta.max(EPS)
            + assessment.source_history.eligible_random_failures as f64;
        let source_p = alpha / (alpha + beta);
        let source_variance = alpha * beta / ((alpha + beta).powi(2) * (alpha + beta + 1.0));
        let source_logit_se = source_variance.sqrt() / (source_p * (1.0 - source_p)).max(EPS);

        let cash_log_bf = if cash.calibrated {
            cash.log_bayes_factor
        } else {
            0.0
        };
        let cash_se = if cash.calibrated {
            cash.log_bayes_factor_std_error
        } else {
            0.0
        };
        let posterior_logit = logit(source_p) + cash_log_bf;
        let p_reliable = logistic(posterior_logit);

        let base_logit_se = (source_logit_se.powi(2) + cash_se.powi(2)).sqrt();
        let widening = 1.0
            + clamp01(assessment.evidence.dependency_uncertainty)
            + clamp01(assessment.evidence.staleness_uncertainty)
            + clamp01(assessment.evidence.distribution_shift);
        let margin = Z_90 * base_logit_se * widening;
        let lower = logistic(posterior_logit - margin);
        let upper = logistic(posterior_logit + margin);

        let vector = UncertaintyVector {
            posterior_dispersion: Some((upper - lower).clamp(0.0, 1.0)),
            ..base_vector
        };

        self.output(
            assessment,
            UncertaintyMode::Calibrated,
            None,
            Some(p_reliable),
            Some((lower, upper)),
            support,
            conflict,
            ignorance,
            vector,
            cash,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn output(
        &self,
        assessment: &ClaimAssessment,
        mode: UncertaintyMode,
        deterministic_outcome: Option<DeterministicOutcome>,
        p_reliable: Option<f64>,
        interval_90: Option<(f64, f64)>,
        support: f64,
        conflict: f64,
        ignorance: f64,
        uncertainty: UncertaintyVector,
        cash: CashEffect,
    ) -> UncertaintyEstimate {
        UncertaintyEstimate {
            claim_ref: assessment.claim_ref.clone(),
            task: assessment.task.clone(),
            horizon: assessment.horizon.clone(),
            policy_id: assessment.policy_id.clone(),
            mode,
            deterministic_outcome,
            p_reliable,
            interval_90,
            support,
            conflict,
            ignorance,
            critical_missing: assessment.evidence.critical_missing,
            independence: clamp01(assessment.evidence.independence),
            uncertainty,
            cash,
            source_outcomes: assessment.source_history.total(),
        }
    }
}

fn normalized_evidence(
    evidence: EvidenceState,
    source_outcomes: u32,
    config: &OpenUncertaintyConfig,
) -> (f64, f64, f64) {
    let raw_support = finite_nonnegative(evidence.support_mass);
    let raw_conflict = finite_nonnegative(evidence.conflict_mass);
    // Keep at most one unit of observed mass. More documents cannot dilute a
    // fixed missing-checklist requirement. Callers still collapse derivations.
    let scale = raw_support.max(raw_conflict).max(1.0);
    let scaled_support = raw_support / scale;
    let scaled_conflict = raw_conflict / scale;
    let scaled_total = scaled_support + scaled_conflict;
    let observed = (raw_support + raw_conflict).min(1.0);
    let support = if scaled_total > 0.0 {
        observed * scaled_support / scaled_total
    } else {
        0.0
    };
    let conflict = if scaled_total > 0.0 {
        observed * scaled_conflict / scaled_total
    } else {
        0.0
    };
    // Invalid coverage cannot mean no missing evidence.
    let missing = if evidence.missing_mass.is_finite() {
        evidence.missing_mass.max(0.0)
    } else {
        1.0
    };
    let observed_mass = (support + conflict + missing).max(1.0);
    let sample_ignorance = config.min_source_outcomes as f64
        / (source_outcomes as f64 + config.min_source_outcomes.max(1) as f64);
    let effective_missing = missing + observed_mass * sample_ignorance;
    let total = support + conflict + effective_missing;
    if total <= EPS {
        (0.0, 0.0, 1.0)
    } else {
        (support / total, conflict / total, effective_missing / total)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct DecisionPolicy {
    pub ignorance_penalty: f64,
    pub conflict_penalty: f64,
    pub dependency_penalty: f64,
    pub staleness_penalty: f64,
    pub distribution_shift_penalty: f64,
    pub licensed_cost_penalty: f64,
    /// Keeps relevant, uncertain candidates visible under an exploratory policy.
    pub exploration_floor: f64,
}

impl Default for DecisionPolicy {
    fn default() -> Self {
        Self {
            ignorance_penalty: 0.10,
            conflict_penalty: 0.15,
            dependency_penalty: 0.05,
            staleness_penalty: 0.10,
            distribution_shift_penalty: 0.10,
            licensed_cost_penalty: 0.0,
            exploration_floor: 0.10,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DecisionCandidate<T> {
    pub item: T,
    pub retrieval_rank: usize,
    pub base_relevance: f64,
    pub licensed_cost: f64,
    pub estimate: UncertaintyEstimate,
}

#[derive(Debug, Clone)]
pub struct DecisionResult<T> {
    pub item: T,
    pub retrieval_rank: usize,
    pub decision_rank: usize,
    pub base_relevance: f64,
    pub decision_score: f64,
    pub estimate: UncertaintyEstimate,
}

impl DecisionPolicy {
    pub fn score(
        &self,
        base_relevance: f64,
        licensed_cost: f64,
        estimate: &UncertaintyEstimate,
    ) -> f64 {
        let reliance = if estimate.critical_missing {
            self.exploration_floor
        } else {
            match estimate.mode {
                UncertaintyMode::Calibrated => estimate
                    .interval_90
                    .map(|(lower, _)| lower)
                    .unwrap_or(self.exploration_floor),
                UncertaintyMode::Deterministic => match estimate.deterministic_outcome {
                    Some(DeterministicOutcome::Pass) => 1.0,
                    Some(DeterministicOutcome::Fail) => 0.0,
                    _ => self.exploration_floor,
                },
                UncertaintyMode::EvidentialOnly => {
                    self.exploration_floor + (1.0 - self.exploration_floor) * estimate.support
                }
            }
        };

        // Known dependence and unresolved dependence are different. A
        // registry can resolve that two articles derive from one poll; that
        // should lower dependency *uncertainty* while still preventing both
        // articles from buying two units of corroboration. The union-like
        // risk below preserves the old behavior when independence is 1.
        let independence = clamp01(estimate.independence);
        let unresolved = clamp01(estimate.uncertainty.dependency_uncertainty);
        let dependence_risk = 1.0 - independence * (1.0 - unresolved);

        base_relevance.max(0.0) * reliance
            - self.ignorance_penalty * estimate.uncertainty.ignorance
            - self.conflict_penalty * estimate.uncertainty.conflict
            - self.dependency_penalty * dependence_risk
            - self.staleness_penalty * estimate.uncertainty.staleness_uncertainty
            - self.distribution_shift_penalty * estimate.uncertainty.distribution_shift
            - self.licensed_cost_penalty * licensed_cost.max(0.0)
    }

    pub fn rerank<T>(&self, candidates: Vec<DecisionCandidate<T>>) -> Vec<DecisionResult<T>> {
        let mut scored: Vec<(DecisionCandidate<T>, f64)> = candidates
            .into_iter()
            .map(|candidate| {
                let score = self.score(
                    candidate.base_relevance,
                    candidate.licensed_cost,
                    &candidate.estimate,
                );
                (candidate, score)
            })
            .collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        scored
            .into_iter()
            .enumerate()
            .map(|(index, (candidate, decision_score))| DecisionResult {
                item: candidate.item,
                retrieval_rank: candidate.retrieval_rank,
                decision_rank: index + 1,
                base_relevance: candidate.base_relevance,
                decision_score,
                estimate: candidate.estimate,
            })
            .collect()
    }
}

fn fully_independent() -> f64 {
    1.0
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CalibrationMetrics {
    pub count: usize,
    pub brier_score: f64,
    pub log_loss: f64,
    pub expected_calibration_error: f64,
}

pub fn calibration_metrics(predictions: &[(f64, bool)], bins: usize) -> CalibrationMetrics {
    if predictions.is_empty() {
        return CalibrationMetrics {
            count: 0,
            brier_score: 0.0,
            log_loss: 0.0,
            expected_calibration_error: 0.0,
        };
    }

    let mut brier = 0.0;
    let mut log_loss = 0.0;
    let bin_count = bins.max(1);
    let mut bin_sums = vec![(0usize, 0.0f64, 0usize); bin_count];

    for &(raw_p, outcome) in predictions {
        let p = raw_p.clamp(EPS, 1.0 - EPS);
        let y = if outcome { 1.0 } else { 0.0 };
        brier += (p - y).powi(2);
        log_loss += -(y * p.ln() + (1.0 - y) * (1.0 - p).ln());
        let index = ((p * bin_count as f64).floor() as usize).min(bin_count - 1);
        let bin = &mut bin_sums[index];
        bin.0 += 1;
        bin.1 += p;
        bin.2 += outcome as usize;
    }

    let n = predictions.len() as f64;
    let ece = bin_sums
        .into_iter()
        .filter(|(count, _, _)| *count > 0)
        .map(|(count, probability_sum, positives)| {
            let avg_probability = probability_sum / count as f64;
            let observed_rate = positives as f64 / count as f64;
            count as f64 / n * (avg_probability - observed_rate).abs()
        })
        .sum();

    CalibrationMetrics {
        count: predictions.len(),
        brier_score: brier / n,
        log_loss: log_loss / n,
        expected_calibration_error: ece,
    }
}

fn clamp01(value: f64) -> f64 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        1.0
    }
}

fn finite_nonnegative(value: f64) -> f64 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

fn logit(p: f64) -> f64 {
    let p = p.clamp(EPS, 1.0 - EPS);
    (p / (1.0 - p)).ln()
}

fn logistic(x: f64) -> f64 {
    if x >= 0.0 {
        1.0 / (1.0 + (-x).exp())
    } else {
        let exp = x.exp();
        exp / (1.0 + exp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conviction(amount: f64) -> ConvictionSignal {
        ConvictionSignal {
            bond_usd: amount,
            max_loss_fraction: 1.0,
            lock_duration_days: 30.0,
            predicate_coverage: 1.0,
            funding_independence: 1.0,
            unhedged_fraction: 1.0,
            posted_before_outcome: true,
            settlement_grade: SettlementGrade::EventBound,
        }
    }

    fn calibration(
        config: &OpenUncertaintyConfig,
        high_passes: usize,
        high_failures: usize,
    ) -> CashCalibration {
        let mut cases = Vec::new();
        for i in 0..100 {
            cases.push(ResolvedClaim {
                reliable: i < 50,
                conviction: None,
                eligible_for_calibration: true,
            });
        }
        for _ in 0..high_passes {
            cases.push(ResolvedClaim {
                reliable: true,
                conviction: Some(conviction(100_000.0)),
                eligible_for_calibration: true,
            });
        }
        for _ in 0..high_failures {
            cases.push(ResolvedClaim {
                reliable: false,
                conviction: Some(conviction(100_000.0)),
                eligible_for_calibration: true,
            });
        }
        CashCalibration::fit(&cases, config)
    }

    fn assessment(history: SourceHistory, conviction: Option<ConvictionSignal>) -> ClaimAssessment {
        ClaimAssessment {
            claim_ref: "claim:1".to_string(),
            task: "event_forecast".to_string(),
            horizon: "P30D".to_string(),
            policy_id: "test-v1".to_string(),
            source_history: history,
            evidence: EvidenceState {
                support_mass: 2.0,
                conflict_mass: 0.2,
                missing_mass: 0.5,
                critical_missing: false,
                independence: 1.0,
                dependency_uncertainty: 0.1,
                staleness_uncertainty: 0.0,
                distribution_shift: 0.0,
            },
            conviction,
            deterministic_outcome: None,
        }
    }

    #[test]
    fn cash_requires_real_pre_outcome_loss() {
        let config = OpenUncertaintyConfig::default();
        let mut signal = conviction(10_000.0);
        assert!(config.effective_cash_exposure(&signal) > 0.0);

        signal.posted_before_outcome = false;
        assert_eq!(config.effective_cash_exposure(&signal), 0.0);

        signal.posted_before_outcome = true;
        signal.settlement_grade = SettlementGrade::EvidenceOnly;
        assert_eq!(config.effective_cash_exposure(&signal), 0.0);

        signal.settlement_grade = SettlementGrade::EventBound;
        signal.unhedged_fraction = 0.0;
        assert_eq!(config.effective_cash_exposure(&signal), 0.0);
    }

    #[test]
    fn headline_amount_is_capped() {
        let config = OpenUncertaintyConfig::default();
        let at_cap = config.effective_cash_exposure(&conviction(100_000.0));
        let whale = config.effective_cash_exposure(&conviction(1_000_000_000.0));
        assert!((at_cap - whale).abs() < 1e-12);
        assert!((whale - 1.0).abs() < 1e-12);
    }

    #[test]
    fn marginal_cash_association_is_diagnostic_only() {
        let config = OpenUncertaintyConfig::default();
        let informative = calibration(&config, 90, 10);
        let effect = informative.effect(Some(&conviction(100_000.0)), &config);
        assert!(!effect.calibrated);
        assert_eq!(effect.log_bayes_factor, 0.0);
        assert!(
            informative
                .marginal_log_odds(Some(&conviction(100_000.0)), &config)
                .unwrap()
                > 0.0
        );

        let uninformative = calibration(&config, 50, 50);
        let effect = uninformative.effect(Some(&conviction(100_000.0)), &config);
        assert!(effect.log_bayes_factor.abs() < 1e-12);
        assert!(
            uninformative
                .marginal_log_odds(Some(&conviction(100_000.0)), &config)
                .unwrap()
                .abs()
                < 1e-12
        );

        let adverse = calibration(&config, 10, 90);
        let effect = adverse.effect(Some(&conviction(100_000.0)), &config);
        assert_eq!(effect.log_bayes_factor, 0.0);
        assert!(
            adverse
                .marginal_log_odds(Some(&conviction(100_000.0)), &config)
                .unwrap()
                < 0.0
        );
    }

    #[test]
    fn sparse_or_ood_claims_do_not_emit_probability() {
        let config = OpenUncertaintyConfig::default();
        let engine = OpenUncertainty::new(config.clone(), calibration(&config, 90, 10));
        let sparse = assessment(
            SourceHistory {
                eligible_random_passes: 2,
                eligible_random_failures: 1,
            },
            Some(conviction(100_000.0)),
        );
        assert_eq!(
            engine.estimate(&sparse).mode,
            UncertaintyMode::EvidentialOnly
        );

        let mut ood = assessment(
            SourceHistory {
                eligible_random_passes: 30,
                eligible_random_failures: 10,
            },
            Some(conviction(100_000.0)),
        );
        ood.evidence.distribution_shift = 0.95;
        assert_eq!(engine.estimate(&ood).mode, UncertaintyMode::EvidentialOnly);
    }

    #[test]
    fn marginal_cash_never_double_counts_source_history() {
        let config = OpenUncertaintyConfig::default();
        let history = SourceHistory {
            eligible_random_passes: 30,
            eligible_random_failures: 10,
        };
        let informative = OpenUncertainty::new(config.clone(), calibration(&config, 90, 10));
        let no_cash = informative.estimate(&assessment(history, None));
        let with_cash = informative.estimate(&assessment(history, Some(conviction(100_000.0))));
        assert_eq!(with_cash.mode, UncertaintyMode::Calibrated);
        assert!(!with_cash.cash.calibrated);
        assert_eq!(with_cash.p_reliable, no_cash.p_reliable);
        assert_eq!(with_cash.interval_90, no_cash.interval_90);

        let sparse_calibration = CashCalibration::fit(&[], &config);
        let uncalibrated = OpenUncertainty::new(config, sparse_calibration);
        let no_cash = uncalibrated.estimate(&assessment(history, None));
        let with_cash = uncalibrated.estimate(&assessment(history, Some(conviction(100_000.0))));
        assert!(!with_cash.cash.calibrated);
        assert!((with_cash.p_reliable.unwrap() - no_cash.p_reliable.unwrap()).abs() < 1e-12);
    }

    #[test]
    fn whale_cash_cannot_override_strong_negative_history() {
        let config = OpenUncertaintyConfig::default();
        let engine = OpenUncertainty::new(config.clone(), calibration(&config, 99, 1));
        let estimate = engine.estimate(&assessment(
            SourceHistory {
                eligible_random_passes: 2,
                eligible_random_failures: 98,
            },
            Some(conviction(1_000_000_000.0)),
        ));
        assert!(estimate.cash.log_bayes_factor <= config.max_abs_cash_log_bayes_factor);
        assert!(estimate.p_reliable.unwrap() < 0.10);
    }

    #[test]
    fn deterministic_result_is_scoped_and_non_probabilistic() {
        let config = OpenUncertaintyConfig::default();
        let engine = OpenUncertainty::new(config.clone(), calibration(&config, 90, 10));
        let mut input = assessment(SourceHistory::default(), Some(conviction(100_000.0)));
        input.deterministic_outcome = Some(DeterministicOutcome::Pass);
        let estimate = engine.estimate(&input);
        assert_eq!(estimate.mode, UncertaintyMode::Deterministic);
        assert_eq!(
            estimate.deterministic_outcome,
            Some(DeterministicOutcome::Pass)
        );
        assert!(estimate.p_reliable.is_none());
    }

    #[test]
    fn decision_ranking_preserves_retrieval_rank() {
        let config = OpenUncertaintyConfig::default();
        let engine = OpenUncertainty::new(config.clone(), calibration(&config, 90, 10));
        let weak = engine.estimate(&assessment(
            SourceHistory {
                eligible_random_passes: 10,
                eligible_random_failures: 30,
            },
            None,
        ));
        let strong = engine.estimate(&assessment(
            SourceHistory {
                eligible_random_passes: 35,
                eligible_random_failures: 5,
            },
            Some(conviction(100_000.0)),
        ));
        let candidates = vec![
            DecisionCandidate {
                item: "more-relevant-weak",
                retrieval_rank: 1,
                base_relevance: 0.90,
                licensed_cost: 0.0,
                estimate: weak,
            },
            DecisionCandidate {
                item: "slightly-less-relevant-strong",
                retrieval_rank: 2,
                base_relevance: 0.86,
                licensed_cost: 0.0,
                estimate: strong,
            },
        ];
        let ranked = DecisionPolicy::default().rerank(candidates);
        assert_eq!(ranked[0].item, "slightly-less-relevant-strong");
        assert_eq!(ranked[0].retrieval_rank, 2);
        assert_eq!(ranked[0].decision_rank, 1);
    }

    #[test]
    fn known_derivation_is_penalized_even_when_ancestry_is_resolved() {
        let config = OpenUncertaintyConfig::default();
        let engine = OpenUncertainty::new(config.clone(), calibration(&config, 90, 10));
        let independent = engine.estimate(&assessment(SourceHistory::default(), None));
        let mut duplicate_input = assessment(SourceHistory::default(), None);
        duplicate_input.evidence.independence = 0.0;
        duplicate_input.evidence.dependency_uncertainty = 0.0;
        let duplicate = engine.estimate(&duplicate_input);
        let policy = DecisionPolicy::default();
        assert!(policy.score(0.8, 0.0, &independent) > policy.score(0.8, 0.0, &duplicate));
        assert_eq!(duplicate.independence, 0.0);
        assert_eq!(duplicate.uncertainty.dependency_uncertainty, 0.0);
    }

    #[test]
    fn calibration_metrics_reward_better_probabilities() {
        let good = calibration_metrics(&[(0.9, true), (0.8, true), (0.2, false), (0.1, false)], 5);
        let bad = calibration_metrics(&[(0.1, true), (0.2, true), (0.8, false), (0.9, false)], 5);
        assert!(good.brier_score < bad.brier_score);
        assert!(good.log_loss < bad.log_loss);
    }

    #[test]
    fn copied_support_cannot_dilute_a_missing_requirement() {
        let config = OpenUncertaintyConfig::default();
        let engine = OpenUncertainty::new(config.clone(), CashCalibration::fit(&[], &config));
        let mut input = assessment(SourceHistory::default(), None);
        input.evidence.support_mass = 1.0;
        input.evidence.conflict_mass = 0.0;
        input.evidence.missing_mass = 1.0;
        let first = engine.estimate(&input);
        for mass in [10.0, 100.0, 1_000_000.0, f64::MAX] {
            input.evidence.support_mass = mass;
            let repeated = engine.estimate(&input);
            assert!((repeated.ignorance - first.ignorance).abs() < 1e-12);
            assert!(repeated.ignorance.is_finite());
        }
    }

    #[test]
    fn critical_missing_blocks_probability_despite_history_and_cash() {
        let config = OpenUncertaintyConfig::default();
        let engine = OpenUncertainty::new(config.clone(), calibration(&config, 99, 1));
        let mut input = assessment(
            SourceHistory {
                eligible_random_passes: 990,
                eligible_random_failures: 10,
            },
            Some(conviction(100_000.0)),
        );
        let complete = engine.estimate(&input);
        input.evidence.critical_missing = true;
        input.evidence.support_mass = 1_000_000.0;
        let missing = engine.estimate(&input);
        assert_eq!(missing.mode, UncertaintyMode::EvidentialOnly);
        assert!(missing.p_reliable.is_none());
        assert!(missing.critical_missing);
        assert!(
            DecisionPolicy::default().score(0.9, 0.0, &missing)
                < DecisionPolicy::default().score(0.9, 0.0, &complete)
        );
    }

    #[test]
    fn deterministic_pass_remains_scoped_with_critical_missing() {
        let config = OpenUncertaintyConfig::default();
        let engine = OpenUncertainty::new(config.clone(), CashCalibration::fit(&[], &config));
        let mut input = assessment(SourceHistory::default(), None);
        input.evidence.critical_missing = true;
        input.deterministic_outcome = Some(DeterministicOutcome::Pass);
        let estimate = engine.estimate(&input);
        assert_eq!(estimate.mode, UncertaintyMode::Deterministic);
        assert!(estimate.critical_missing);
        assert!(estimate.p_reliable.is_none());
    }

    #[test]
    fn cash_status_distinguishes_disabled_model_from_ineligible_or_sparse() {
        let config = OpenUncertaintyConfig::default();
        let dense = calibration(&config, 90, 10);
        assert_eq!(
            dense.effect(None, &config).model_status,
            CashModelStatus::Ineligible
        );
        let signal = conviction(100_000.0);
        assert_eq!(
            CashCalibration::fit(&[], &config)
                .effect(Some(&signal), &config)
                .model_status,
            CashModelStatus::InsufficientData
        );
        assert_eq!(
            dense.effect(Some(&signal), &config).model_status,
            CashModelStatus::DisabledMarginalAssociation
        );
    }

    #[test]
    fn invalid_missing_mass_never_means_complete_coverage() {
        let config = OpenUncertaintyConfig::default();
        let mut input = EvidenceState::default();
        input.support_mass = 100.0;
        for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            input.missing_mass = invalid;
            let (_, _, ignorance) = normalized_evidence(input, 1000, &config);
            assert!(ignorance.is_finite());
            assert!(ignorance >= 0.5);
        }
    }
}
