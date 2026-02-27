use oi_crawl::quality::check_quality;

use crate::dataset::{BenchmarkCase, Difficulty};

/// Run quality gate on adversarial benchmark cases, return rejection rate.
pub fn evaluate_quality_gate(cases: &[BenchmarkCase]) -> QualityGateResult {
    let adversarial: Vec<&BenchmarkCase> = cases
        .iter()
        .filter(|c| c.difficulty == Difficulty::Adversarial)
        .collect();

    if adversarial.is_empty() {
        return QualityGateResult {
            total: 0,
            rejected: 0,
            rejection_rate: 0.0,
            false_negatives: vec![],
        };
    }

    let mut rejected = 0;
    let mut false_negatives = Vec::new();

    for case in &adversarial {
        for doc in &case.relevant_docs {
            let check = check_quality(&doc.content);
            if !check.passed {
                rejected += 1;
            } else {
                false_negatives.push(case.id.clone());
            }
        }
    }

    let total = adversarial.len();
    QualityGateResult {
        total,
        rejected,
        rejection_rate: rejected as f64 / total as f64,
        false_negatives,
    }
}

#[allow(dead_code)]
pub struct QualityGateResult {
    pub total: usize,
    pub rejected: usize,
    pub rejection_rate: f64,
    /// Case IDs that should have been rejected but weren't
    pub false_negatives: Vec<String>,
}

/// Run quality gate on clean content cases, ensure low false positive rate.
pub fn evaluate_false_positive_rate(cases: &[BenchmarkCase]) -> f64 {
    let clean: Vec<&BenchmarkCase> = cases
        .iter()
        .filter(|c| c.difficulty != Difficulty::Adversarial)
        .collect();

    if clean.is_empty() {
        return 0.0;
    }

    let mut false_positives = 0;
    let mut total = 0;

    for case in &clean {
        for doc in &case.relevant_docs {
            total += 1;
            let check = check_quality(&doc.content);
            if !check.passed {
                false_positives += 1;
            }
        }
    }

    if total == 0 {
        0.0
    } else {
        false_positives as f64 / total as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generator::generate_text_dataset;

    #[test]
    fn test_adversarial_detection() {
        let ds = generate_text_dataset(100);
        let result = evaluate_quality_gate(&ds.cases);
        // Heuristic-only gate catches the most egregious spam/slop
        // Semantic filtering (via real embeddings) handles the rest
        assert!(
            result.rejection_rate > 0.2,
            "rejection rate too low: {:.2}%",
            result.rejection_rate * 100.0
        );
    }

    #[test]
    fn test_clean_content_passes() {
        let ds = generate_text_dataset(100);
        let fpr = evaluate_false_positive_rate(&ds.cases);
        assert!(
            fpr < 0.2,
            "false positive rate too high: {:.2}%",
            fpr * 100.0
        );
    }
}
