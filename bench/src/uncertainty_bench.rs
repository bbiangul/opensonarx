use oi_index::uncertainty::{
    calibration_metrics, CalibrationMetrics, CashCalibration, ClaimAssessment, ConvictionSignal,
    EvidenceState, OpenUncertainty, OpenUncertaintyConfig, ResolvedClaim, SettlementGrade,
    SourceHistory,
};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::hint::black_box;
use std::io::{BufRead, BufReader};
use std::time::Instant;

#[derive(Debug, Serialize)]
struct CashEffectReport {
    bucket: &'static str,
    sample_count: u32,
    effective_exposure: f64,
    learned_log_bayes_factor: f64,
}

#[derive(Debug, Serialize)]
struct UncertaintyBenchmarkReport {
    experiment: &'static str,
    synthetic_only: bool,
    predictive_thesis_validated: bool,
    cash_model_status: &'static str,
    train_cases: usize,
    eval_cases: usize,
    throughput_claims: usize,
    no_cash: CalibrationMetrics,
    calibrated_cash: CalibrationMetrics,
    brier_improvement_pct: f64,
    log_loss_improvement_pct: f64,
    cash_effects: Vec<CashEffectReport>,
    uninformative_cash_max_abs_log_bf: f64,
    whale_guard_probability: f64,
    total_inference_ms: f64,
    average_inference_ns: f64,
    estimates_per_second: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DatasetSplit {
    Train,
    Evaluation,
}

#[derive(Debug, Clone, Deserialize)]
struct UncertaintyDatasetCase {
    claim_ref: String,
    source_ref: String,
    /// Connected-component block joining every known source/event/funder/derivation link.
    evaluation_cluster_ref: String,
    claim_class: String,
    task: String,
    horizon: String,
    policy_id: String,
    split: DatasetSplit,
    committed_at_unix: i64,
    outcome_at_unix: i64,
    reliable: bool,
    #[serde(default = "default_true")]
    eligible_for_calibration: bool,
    #[serde(default)]
    evidence: EvidenceState,
    #[serde(default)]
    conviction: Option<ConvictionSignal>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Serialize)]
struct DatasetEvaluationReport {
    experiment: &'static str,
    dataset_path: String,
    predictive_thesis_validated: bool,
    cash_model_status: &'static str,
    point_estimate_gate_passed: bool,
    confirmatory_calibration_gate_passed: bool,
    bootstrap_iterations: usize,
    brier_improvement_95_interval_pct: Option<(f64, f64)>,
    claim_class: String,
    task: String,
    horizon: String,
    train_cases: usize,
    evaluation_cases: usize,
    scored_evaluation_cases: usize,
    calibrated_coverage: f64,
    no_cash: CalibrationMetrics,
    calibrated_cash: CalibrationMetrics,
    brier_improvement_pct: f64,
    log_loss_improvement_pct: f64,
    ineligible_cash_positions: usize,
    warnings: Vec<String>,
}

fn conviction_for_bucket(bucket: usize) -> Option<ConvictionSignal> {
    let bond_usd = match bucket {
        0 => return None,
        1 => 100.0,
        2 => 10_000.0,
        _ => 100_000.0,
    };
    Some(ConvictionSignal {
        bond_usd,
        max_loss_fraction: 1.0,
        lock_duration_days: 30.0,
        predicate_coverage: 1.0,
        funding_independence: 1.0,
        unhedged_fraction: 1.0,
        posted_before_outcome: true,
        settlement_grade: SettlementGrade::EventBound,
    })
}

fn generate_resolved_claims(
    count: usize,
    success_rates: [f64; 4],
    seed: u64,
) -> Vec<ResolvedClaim> {
    let mut rng = StdRng::seed_from_u64(seed);
    (0..count)
        .map(|index| {
            let bucket = index % 4;
            ResolvedClaim {
                reliable: rng.gen_bool(success_rates[bucket]),
                conviction: conviction_for_bucket(bucket),
                eligible_for_calibration: true,
            }
        })
        .collect()
}

fn assessment(index: usize, bucket: usize) -> ClaimAssessment {
    ClaimAssessment {
        claim_ref: format!("synthetic:{index}"),
        task: "event_forecast".to_string(),
        horizon: "P30D".to_string(),
        policy_id: "synthetic-forward-v1".to_string(),
        source_history: SourceHistory {
            eligible_random_passes: 30,
            eligible_random_failures: 20,
        },
        evidence: EvidenceState {
            support_mass: 1.0,
            conflict_mass: 0.2,
            missing_mass: 0.5,
            critical_missing: false,
            independence: 1.0,
            dependency_uncertainty: 0.05,
            staleness_uncertainty: 0.0,
            distribution_shift: 0.0,
        },
        conviction: conviction_for_bucket(bucket),
        deterministic_outcome: None,
    }
}

fn prediction_set(
    engine: &OpenUncertainty,
    count: usize,
    success_rates: [f64; 4],
    seed: u64,
) -> Vec<(f64, bool)> {
    let mut rng = StdRng::seed_from_u64(seed);
    (0..count)
        .map(|index| {
            let bucket = index % 4;
            let outcome = rng.gen_bool(success_rates[bucket]);
            let estimate = engine.estimate(&assessment(index, bucket));
            (
                estimate.p_reliable.expect("calibrated synthetic case"),
                outcome,
            )
        })
        .collect()
}

fn improvement_pct(baseline: f64, candidate: f64) -> f64 {
    if baseline <= f64::EPSILON {
        0.0
    } else {
        100.0 * (baseline - candidate) / baseline
    }
}

pub fn run_uncertainty_benchmark(
    train_cases: usize,
    eval_cases: usize,
    throughput_claims: usize,
    output: &str,
) {
    let config = OpenUncertaintyConfig::default();
    let informative_rates = [0.60, 0.65, 0.75, 0.85];
    let train = generate_resolved_claims(train_cases, informative_rates, 0x51A7_2026);
    let cash_calibration = CashCalibration::fit(&train, &config);
    let cash_engine = OpenUncertainty::new(config.clone(), cash_calibration.clone());
    let no_cash_engine = OpenUncertainty::new(config.clone(), CashCalibration::fit(&[], &config));

    // Forward split uses a distinct deterministic seed.
    let no_cash_predictions =
        prediction_set(&no_cash_engine, eval_cases, informative_rates, 0xF04D_2026);
    let cash_predictions = prediction_set(&cash_engine, eval_cases, informative_rates, 0xF04D_2026);
    let no_cash = calibration_metrics(&no_cash_predictions, 10);
    let calibrated_cash = calibration_metrics(&cash_predictions, 10);

    let cash_effects = (1..=3)
        .map(|bucket| {
            let signal = conviction_for_bucket(bucket).expect("cash bucket");
            let effect = cash_calibration.effect(Some(&signal), &config);
            CashEffectReport {
                bucket: match bucket {
                    1 => "low",
                    2 => "medium",
                    _ => "high",
                },
                sample_count: effect.sample_count,
                effective_exposure: effect.effective_exposure,
                learned_log_bayes_factor: effect.log_bayes_factor,
            }
        })
        .collect();

    // Negative control: cash is deliberately uninformative in every bucket.
    let uninformative_train =
        generate_resolved_claims(train_cases, [0.65, 0.65, 0.65, 0.65], 0xC0DE_2026);
    let uninformative = CashCalibration::fit(&uninformative_train, &config);
    let uninformative_cash_max_abs_log_bf = (1..=3)
        .map(|bucket| {
            let signal = conviction_for_bucket(bucket).expect("cash bucket");
            uninformative
                .effect(Some(&signal), &config)
                .log_bayes_factor
                .abs()
        })
        .fold(0.0, f64::max);

    // Adversarial guard: enormous cash cannot overwrite a 2/98 source record.
    let mut whale = conviction_for_bucket(3).expect("high bucket");
    whale.bond_usd = 1_000_000_000.0;
    let mut whale_assessment = assessment(0, 3);
    whale_assessment.source_history = SourceHistory {
        eligible_random_passes: 2,
        eligible_random_failures: 98,
    };
    whale_assessment.conviction = Some(whale);
    let whale_guard_probability = cash_engine
        .estimate(&whale_assessment)
        .p_reliable
        .expect("calibrated whale guard");

    // Hot-path throughput, including allocation of the public estimate object.
    let template = assessment(42, 3);
    let started = Instant::now();
    for _ in 0..throughput_claims {
        black_box(cash_engine.estimate(black_box(&template)));
    }
    let elapsed = started.elapsed();
    let total_inference_ms = elapsed.as_secs_f64() * 1_000.0;
    let average_inference_ns = if throughput_claims == 0 {
        0.0
    } else {
        elapsed.as_nanos() as f64 / throughput_claims as f64
    };
    let estimates_per_second = if elapsed.as_secs_f64() <= f64::EPSILON {
        0.0
    } else {
        throughput_claims as f64 / elapsed.as_secs_f64()
    };

    let report = UncertaintyBenchmarkReport {
        experiment: "openuncertainty-mvp1-synthetic-ablation",
        synthetic_only: true,
        predictive_thesis_validated: false,
        cash_model_status: "disabled_marginal_association_not_incremental_evidence",
        train_cases,
        eval_cases,
        throughput_claims,
        brier_improvement_pct: improvement_pct(no_cash.brier_score, calibrated_cash.brier_score),
        log_loss_improvement_pct: improvement_pct(no_cash.log_loss, calibrated_cash.log_loss),
        no_cash,
        calibrated_cash,
        cash_effects,
        uninformative_cash_max_abs_log_bf,
        whale_guard_probability,
        total_inference_ms,
        average_inference_ns,
        estimates_per_second,
    };

    if output == "json" {
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
        return;
    }

    println!("\n=== OpenUncertainty MVP-1 — synthetic cash ablation ===");
    println!(
        "train={} eval={} throughput={}",
        report.train_cases, report.eval_cases, report.throughput_claims
    );
    println!("Synthetic mechanism test only; predictive thesis validated: NO\n");
    println!(
        "{:<24} {:>10} {:>10} {:>10}",
        "Model", "Brier", "Log loss", "ECE"
    );
    println!(
        "{:<24} {:>10.6} {:>10.6} {:>10.6}",
        "No cash",
        report.no_cash.brier_score,
        report.no_cash.log_loss,
        report.no_cash.expected_calibration_error
    );
    println!(
        "{:<24} {:>10.6} {:>10.6} {:>10.6}",
        "Calibrated cash",
        report.calibrated_cash.brier_score,
        report.calibrated_cash.log_loss,
        report.calibrated_cash.expected_calibration_error
    );
    println!(
        "Brier improvement: {:.2}% | log-loss improvement: {:.2}%",
        report.brier_improvement_pct, report.log_loss_improvement_pct
    );
    println!(
        "Negative-control max |cash log BF|: {:.4}",
        report.uninformative_cash_max_abs_log_bf
    );
    println!(
        "Whale guard p(reliable) after 2/98 history: {:.4}",
        report.whale_guard_probability
    );
    println!(
        "Inference: {:.1} ns/claim | {:.0} claims/s | {:.2} ms total",
        report.average_inference_ns, report.estimates_per_second, report.total_inference_ms
    );
}

fn read_jsonl(path: &str) -> Result<Vec<UncertaintyDatasetCase>, String> {
    let file = File::open(path).map_err(|error| format!("cannot open {path}: {error}"))?;
    let reader = BufReader::new(file);
    let mut cases = Vec::new();
    for (index, line) in reader.lines().enumerate() {
        let line_number = index + 1;
        let line = line.map_err(|error| format!("cannot read {path}:{line_number}: {error}"))?;
        if line.trim().is_empty() {
            continue;
        }
        let case = serde_json::from_str(&line)
            .map_err(|error| format!("invalid JSON at {path}:{line_number}: {error}"))?;
        cases.push(case);
    }
    Ok(cases)
}

fn validate_dataset(cases: &[UncertaintyDatasetCase]) -> Result<(String, String, String), String> {
    if cases.is_empty() {
        return Err("dataset is empty".to_string());
    }

    let mut ids = HashSet::new();
    let mut scopes = HashSet::new();
    let mut train_count = 0;
    let mut evaluation_count = 0;
    let mut max_train_outcome = i64::MIN;
    let mut min_evaluation_commitment = i64::MAX;

    for case in cases {
        if case.claim_ref.trim().is_empty()
            || case.source_ref.trim().is_empty()
            || case.evaluation_cluster_ref.trim().is_empty()
        {
            return Err(
                "claim_ref, source_ref, and evaluation_cluster_ref must be non-empty".to_string(),
            );
        }
        if !ids.insert(case.claim_ref.as_str()) {
            return Err(format!("duplicate claim_ref: {}", case.claim_ref));
        }
        if case.committed_at_unix >= case.outcome_at_unix {
            return Err(format!(
                "claim {} must be committed before its outcome",
                case.claim_ref
            ));
        }
        scopes.insert((
            case.claim_class.as_str(),
            case.task.as_str(),
            case.horizon.as_str(),
        ));
        match case.split {
            DatasetSplit::Train => {
                train_count += 1;
                max_train_outcome = max_train_outcome.max(case.outcome_at_unix);
            }
            DatasetSplit::Evaluation => {
                evaluation_count += 1;
                min_evaluation_commitment = min_evaluation_commitment.min(case.committed_at_unix);
            }
        }
    }

    if scopes.len() != 1 {
        return Err(
            "MVP-1 dataset evaluation requires exactly one claim_class/task/horizon scope"
                .to_string(),
        );
    }
    if train_count == 0 || evaluation_count == 0 {
        return Err("dataset requires both train and evaluation cases".to_string());
    }
    if min_evaluation_commitment <= max_train_outcome {
        return Err(format!(
            "forward-time leakage: earliest evaluation commitment ({min_evaluation_commitment}) must be after latest training outcome ({max_train_outcome})"
        ));
    }

    let (claim_class, task, horizon) = scopes.into_iter().next().unwrap();
    Ok((
        claim_class.to_string(),
        task.to_string(),
        horizon.to_string(),
    ))
}

fn cluster_bootstrap_brier_improvement(
    predictions: &[(f64, f64, bool, String)],
    iterations: usize,
) -> Option<(f64, f64)> {
    if iterations == 0 {
        return None;
    }
    let mut grouped: HashMap<&str, Vec<(f64, f64, bool)>> = HashMap::new();
    for (no_cash, cash, outcome, cluster) in predictions {
        grouped
            .entry(cluster.as_str())
            .or_default()
            .push((*no_cash, *cash, *outcome));
    }
    let clusters: Vec<&Vec<(f64, f64, bool)>> = grouped.values().collect();
    if clusters.len() < 2 {
        return None;
    }

    let mut rng = StdRng::seed_from_u64(0x0B00_57A9);
    let mut improvements = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let mut baseline_error = 0.0;
        let mut cash_error = 0.0;
        let mut count = 0usize;
        for _ in 0..clusters.len() {
            let cluster = clusters[rng.gen_range(0..clusters.len())];
            for &(no_cash, cash, outcome) in cluster {
                let y = if outcome { 1.0 } else { 0.0 };
                baseline_error += (no_cash - y).powi(2);
                cash_error += (cash - y).powi(2);
                count += 1;
            }
        }
        if count > 0 {
            improvements.push(improvement_pct(
                baseline_error / count as f64,
                cash_error / count as f64,
            ));
        }
    }
    if improvements.is_empty() {
        return None;
    }
    improvements.sort_by(f64::total_cmp);
    let lower_index = ((improvements.len() - 1) as f64 * 0.025).floor() as usize;
    let upper_index = ((improvements.len() - 1) as f64 * 0.975).ceil() as usize;
    Some((
        improvements[lower_index],
        improvements[upper_index.min(improvements.len() - 1)],
    ))
}

pub fn run_uncertainty_dataset(
    path: &str,
    output: &str,
    bootstrap_iterations: usize,
) -> Result<(), String> {
    let cases = read_jsonl(path)?;
    let (claim_class, task, horizon) = validate_dataset(&cases)?;
    let config = OpenUncertaintyConfig::default();

    let train: Vec<ResolvedClaim> = cases
        .iter()
        .filter(|case| case.split == DatasetSplit::Train)
        .map(|case| ResolvedClaim {
            reliable: case.reliable,
            conviction: case.conviction.clone(),
            eligible_for_calibration: case.eligible_for_calibration,
        })
        .collect();
    let cash_calibration = CashCalibration::fit(&train, &config);
    let cash_engine = OpenUncertainty::new(config.clone(), cash_calibration);
    let no_cash_engine = OpenUncertainty::new(config.clone(), CashCalibration::fit(&[], &config));

    let mut histories: HashMap<&str, SourceHistory> = HashMap::new();
    for case in cases
        .iter()
        .filter(|case| case.split == DatasetSplit::Train && case.eligible_for_calibration)
    {
        let history = histories.entry(case.source_ref.as_str()).or_default();
        if case.reliable {
            history.eligible_random_passes += 1;
        } else {
            history.eligible_random_failures += 1;
        }
    }

    let evaluation: Vec<&UncertaintyDatasetCase> = cases
        .iter()
        .filter(|case| case.split == DatasetSplit::Evaluation)
        .collect();
    let mut no_cash_predictions = Vec::new();
    let mut cash_predictions = Vec::new();
    let mut paired_predictions = Vec::new();
    let mut ineligible_cash_positions = 0;

    for case in evaluation
        .iter()
        .copied()
        .filter(|case| case.eligible_for_calibration)
    {
        if case
            .conviction
            .as_ref()
            .is_some_and(|signal| !signal.is_eligible())
        {
            ineligible_cash_positions += 1;
        }
        let assessment = ClaimAssessment {
            claim_ref: case.claim_ref.clone(),
            task: case.task.clone(),
            horizon: case.horizon.clone(),
            policy_id: case.policy_id.clone(),
            source_history: histories
                .get(case.source_ref.as_str())
                .copied()
                .unwrap_or_default(),
            evidence: case.evidence,
            conviction: case.conviction.clone(),
            deterministic_outcome: None,
        };
        let baseline = no_cash_engine.estimate(&assessment);
        let candidate = cash_engine.estimate(&assessment);
        if let (Some(no_cash_p), Some(cash_p)) = (baseline.p_reliable, candidate.p_reliable) {
            no_cash_predictions.push((no_cash_p, case.reliable));
            cash_predictions.push((cash_p, case.reliable));
            paired_predictions.push((
                no_cash_p,
                cash_p,
                case.reliable,
                case.evaluation_cluster_ref.clone(),
            ));
        }
    }

    let no_cash = calibration_metrics(&no_cash_predictions, 10);
    let calibrated_cash = calibration_metrics(&cash_predictions, 10);
    let brier_improvement_pct = improvement_pct(no_cash.brier_score, calibrated_cash.brier_score);
    let log_loss_improvement_pct = improvement_pct(no_cash.log_loss, calibrated_cash.log_loss);
    let brier_improvement_95_interval_pct =
        cluster_bootstrap_brier_improvement(&paired_predictions, bootstrap_iterations);
    let eligible_evaluation_count = evaluation
        .iter()
        .filter(|case| case.eligible_for_calibration)
        .count();
    let calibrated_coverage = if eligible_evaluation_count == 0 {
        0.0
    } else {
        cash_predictions.len() as f64 / eligible_evaluation_count as f64
    };

    let mut warnings = vec![
        "Cash contribution is disabled: marginal bucket rates are not conditional on source history/evidence. Legacy calibrated_cash fields now carry the same source-only predictions; this is not a cash-model validation.".to_string(),
    ];
    if cash_predictions.len() < 100 {
        warnings
            .push("fewer than 100 evaluation cases emitted calibrated probabilities".to_string());
    }
    if calibrated_coverage < 0.5 {
        warnings.push("calibrated coverage is below 50%; inspect source cold start".to_string());
    }
    if ineligible_cash_positions > 0 {
        warnings.push(format!(
            "{ineligible_cash_positions} evaluation cash positions were ineligible and received zero exposure"
        ));
    }
    if brier_improvement_95_interval_pct.is_none() {
        warnings.push(
            "Brier confidence interval unavailable; need at least two evaluation clusters and one bootstrap iteration"
                .to_string(),
        );
    }

    // The bootstrap block is supplied by the immutable cohort manifest and must be
    // the connected component of every known source/event/funder/derivation link.
    let point_estimate_gate_passed = cash_predictions.len() >= 100
        && brier_improvement_pct >= 3.0
        && calibrated_cash.log_loss < no_cash.log_loss
        && calibrated_cash.expected_calibration_error <= no_cash.expected_calibration_error + 0.01;
    let confirmatory_calibration_gate_passed = point_estimate_gate_passed
        && brier_improvement_95_interval_pct.is_some_and(|(lower, _)| lower > 0.0);
    let report = DatasetEvaluationReport {
        experiment: "openuncertainty-mvp1-forward-dataset",
        dataset_path: path.to_string(),
        predictive_thesis_validated: false,
        cash_model_status: "disabled_marginal_association_not_incremental_evidence",
        point_estimate_gate_passed,
        confirmatory_calibration_gate_passed,
        bootstrap_iterations,
        brier_improvement_95_interval_pct,
        claim_class,
        task,
        horizon,
        train_cases: train.len(),
        evaluation_cases: evaluation.len(),
        scored_evaluation_cases: cash_predictions.len(),
        calibrated_coverage,
        no_cash,
        calibrated_cash,
        brier_improvement_pct,
        log_loss_improvement_pct,
        ineligible_cash_positions,
        warnings,
    };

    if output == "json" {
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
    } else {
        println!("\n=== OpenUncertainty MVP-1 — forward dataset evaluation ===");
        println!("dataset: {}", report.dataset_path);
        println!(
            "scope: {}/{}/{} | train={} eval={} scored={} coverage={:.1}%",
            report.claim_class,
            report.task,
            report.horizon,
            report.train_cases,
            report.evaluation_cases,
            report.scored_evaluation_cases,
            100.0 * report.calibrated_coverage
        );
        println!(
            "no-cash Brier={:.6} log-loss={:.6} ECE={:.6}",
            report.no_cash.brier_score,
            report.no_cash.log_loss,
            report.no_cash.expected_calibration_error
        );
        println!(
            "with-cash Brier={:.6} log-loss={:.6} ECE={:.6}",
            report.calibrated_cash.brier_score,
            report.calibrated_cash.log_loss,
            report.calibrated_cash.expected_calibration_error
        );
        println!(
            "Brier improvement={:.2}% | log-loss improvement={:.2}%",
            report.brier_improvement_pct, report.log_loss_improvement_pct
        );
        println!(
            "Point screen={} | block-bootstrap calibration gate={} | thesis validated: NO",
            report.point_estimate_gate_passed, report.confirmatory_calibration_gate_passed
        );
        if let Some((lower, upper)) = report.brier_improvement_95_interval_pct {
            println!("Brier improvement 95% cluster interval: [{lower:.2}%, {upper:.2}%]");
        }
        for warning in &report.warnings {
            println!("WARNING: {warning}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cluster_bootstrap_detects_consistent_brier_improvement() {
        let predictions: Vec<(f64, f64, bool, String)> = (0..200)
            .map(|index| {
                let outcome = index % 2 == 0;
                let no_cash = if outcome { 0.6 } else { 0.4 };
                let cash = if outcome { 0.9 } else { 0.1 };
                (no_cash, cash, outcome, format!("cluster:{index}"))
            })
            .collect();
        let (lower, upper) =
            cluster_bootstrap_brier_improvement(&predictions, 500).expect("interval");
        assert!(lower > 0.0);
        assert!(upper >= lower);
    }

    #[test]
    fn cluster_bootstrap_requires_independent_blocks() {
        let predictions = vec![(0.6, 0.7, true, "one-cluster".to_string())];
        assert!(cluster_bootstrap_brier_improvement(&predictions, 100).is_none());
    }
}
