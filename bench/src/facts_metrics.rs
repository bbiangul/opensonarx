use std::collections::HashMap;

/// NER evaluation metrics (exact and partial span matching).
#[derive(Debug, Clone, Default)]
pub struct NerMetrics {
    pub exact_precision: f64,
    pub exact_recall: f64,
    pub exact_f1: f64,
    pub partial_precision: f64,
    pub partial_recall: f64,
    pub partial_f1: f64,
}

/// Relation extraction evaluation metrics.
#[derive(Debug, Clone, Default)]
pub struct RelationMetrics {
    pub precision: f64,
    pub recall: f64,
    pub f1: f64,
}

/// Aggregate benchmark result across languages and entity types.
#[derive(Debug, Clone)]
pub struct FactsBenchmarkResult {
    pub overall_ner: NerMetrics,
    pub overall_rel: RelationMetrics,
    pub per_language: HashMap<String, (NerMetrics, RelationMetrics)>,
    pub per_entity_type: HashMap<String, NerMetrics>,
    pub avg_extraction_time_ms: f64,
}

/// A predicted or gold entity span for evaluation.
#[derive(Debug, Clone)]
pub struct EntitySpan {
    pub start: usize,
    pub end: usize,
    pub entity_type: String,
}

/// A predicted or gold relation for evaluation.
#[derive(Debug, Clone)]
pub struct RelationSpan {
    pub subject: String,
    pub predicate: String,
    pub object: String,
}

fn f1_score(precision: f64, recall: f64) -> f64 {
    if precision + recall == 0.0 {
        0.0
    } else {
        2.0 * precision * recall / (precision + recall)
    }
}

/// Compute NER metrics: exact and partial span + type matching.
pub fn evaluate_ner(predicted: &[EntitySpan], gold: &[EntitySpan]) -> NerMetrics {
    if predicted.is_empty() && gold.is_empty() {
        return NerMetrics {
            exact_precision: 1.0,
            exact_recall: 1.0,
            exact_f1: 1.0,
            partial_precision: 1.0,
            partial_recall: 1.0,
            partial_f1: 1.0,
        };
    }

    // Exact match: same start, end, type
    let exact_matches = predicted
        .iter()
        .filter(|p| {
            gold.iter()
                .any(|g| p.start == g.start && p.end == g.end && p.entity_type == g.entity_type)
        })
        .count();

    let exact_precision = if predicted.is_empty() {
        0.0
    } else {
        exact_matches as f64 / predicted.len() as f64
    };
    let exact_recall = if gold.is_empty() {
        0.0
    } else {
        exact_matches as f64 / gold.len() as f64
    };

    // Partial match: overlapping span + same type
    let partial_matches = predicted
        .iter()
        .filter(|p| {
            gold.iter()
                .any(|g| p.entity_type == g.entity_type && spans_overlap(p, g))
        })
        .count();

    let partial_precision = if predicted.is_empty() {
        0.0
    } else {
        partial_matches as f64 / predicted.len() as f64
    };
    let partial_recall = if gold.is_empty() {
        0.0
    } else {
        // Count how many gold entities have a partial match
        let gold_matched = gold
            .iter()
            .filter(|g| {
                predicted
                    .iter()
                    .any(|p| p.entity_type == g.entity_type && spans_overlap(p, g))
            })
            .count();
        gold_matched as f64 / gold.len() as f64
    };

    NerMetrics {
        exact_precision,
        exact_recall,
        exact_f1: f1_score(exact_precision, exact_recall),
        partial_precision,
        partial_recall,
        partial_f1: f1_score(partial_precision, partial_recall),
    }
}

fn spans_overlap(a: &EntitySpan, b: &EntitySpan) -> bool {
    a.start < b.end && b.start < a.end
}

/// Compute relation extraction metrics.
/// Matches are based on (subject_normalized, predicate, object_normalized).
pub fn evaluate_relations(predicted: &[RelationSpan], gold: &[RelationSpan]) -> RelationMetrics {
    if predicted.is_empty() && gold.is_empty() {
        return RelationMetrics {
            precision: 1.0,
            recall: 1.0,
            f1: 1.0,
        };
    }

    let matches = predicted
        .iter()
        .filter(|p| {
            gold.iter().any(|g| {
                p.subject == g.subject && p.predicate == g.predicate && p.object == g.object
            })
        })
        .count();

    let precision = if predicted.is_empty() {
        0.0
    } else {
        matches as f64 / predicted.len() as f64
    };
    let recall = if gold.is_empty() {
        0.0
    } else {
        matches as f64 / gold.len() as f64
    };

    RelationMetrics {
        precision,
        recall,
        f1: f1_score(precision, recall),
    }
}

/// Per-document result for aggregation.
pub struct DocResult {
    pub language: String,
    pub ner: NerMetrics,
    pub rel: RelationMetrics,
    pub entity_type_ner: HashMap<String, NerMetrics>,
    pub extraction_time_ms: f64,
}

/// Aggregate per-document results into a FactsBenchmarkResult.
pub fn aggregate_results(results: &[DocResult]) -> FactsBenchmarkResult {
    if results.is_empty() {
        return FactsBenchmarkResult {
            overall_ner: NerMetrics::default(),
            overall_rel: RelationMetrics::default(),
            per_language: HashMap::new(),
            per_entity_type: HashMap::new(),
            avg_extraction_time_ms: 0.0,
        };
    }

    let n = results.len() as f64;

    // Macro average NER
    let overall_ner = NerMetrics {
        exact_precision: results.iter().map(|r| r.ner.exact_precision).sum::<f64>() / n,
        exact_recall: results.iter().map(|r| r.ner.exact_recall).sum::<f64>() / n,
        exact_f1: results.iter().map(|r| r.ner.exact_f1).sum::<f64>() / n,
        partial_precision: results.iter().map(|r| r.ner.partial_precision).sum::<f64>() / n,
        partial_recall: results.iter().map(|r| r.ner.partial_recall).sum::<f64>() / n,
        partial_f1: results.iter().map(|r| r.ner.partial_f1).sum::<f64>() / n,
    };

    let overall_rel = RelationMetrics {
        precision: results.iter().map(|r| r.rel.precision).sum::<f64>() / n,
        recall: results.iter().map(|r| r.rel.recall).sum::<f64>() / n,
        f1: results.iter().map(|r| r.rel.f1).sum::<f64>() / n,
    };

    // Per-language
    let mut per_language: HashMap<String, Vec<&DocResult>> = HashMap::new();
    for r in results {
        per_language.entry(r.language.clone()).or_default().push(r);
    }
    let per_language = per_language
        .into_iter()
        .map(|(lang, docs)| {
            let ln = docs.len() as f64;
            let ner = NerMetrics {
                exact_precision: docs.iter().map(|r| r.ner.exact_precision).sum::<f64>() / ln,
                exact_recall: docs.iter().map(|r| r.ner.exact_recall).sum::<f64>() / ln,
                exact_f1: docs.iter().map(|r| r.ner.exact_f1).sum::<f64>() / ln,
                partial_precision: docs.iter().map(|r| r.ner.partial_precision).sum::<f64>() / ln,
                partial_recall: docs.iter().map(|r| r.ner.partial_recall).sum::<f64>() / ln,
                partial_f1: docs.iter().map(|r| r.ner.partial_f1).sum::<f64>() / ln,
            };
            let rel = RelationMetrics {
                precision: docs.iter().map(|r| r.rel.precision).sum::<f64>() / ln,
                recall: docs.iter().map(|r| r.rel.recall).sum::<f64>() / ln,
                f1: docs.iter().map(|r| r.rel.f1).sum::<f64>() / ln,
            };
            (lang, (ner, rel))
        })
        .collect();

    // Per entity type
    let mut type_results: HashMap<String, Vec<&NerMetrics>> = HashMap::new();
    for r in results {
        for (etype, metrics) in &r.entity_type_ner {
            type_results.entry(etype.clone()).or_default().push(metrics);
        }
    }
    let per_entity_type = type_results
        .into_iter()
        .map(|(etype, metrics)| {
            let tn = metrics.len() as f64;
            let ner = NerMetrics {
                exact_precision: metrics.iter().map(|m| m.exact_precision).sum::<f64>() / tn,
                exact_recall: metrics.iter().map(|m| m.exact_recall).sum::<f64>() / tn,
                exact_f1: metrics.iter().map(|m| m.exact_f1).sum::<f64>() / tn,
                partial_precision: metrics.iter().map(|m| m.partial_precision).sum::<f64>() / tn,
                partial_recall: metrics.iter().map(|m| m.partial_recall).sum::<f64>() / tn,
                partial_f1: metrics.iter().map(|m| m.partial_f1).sum::<f64>() / tn,
            };
            (etype, ner)
        })
        .collect();

    let avg_extraction_time_ms = results.iter().map(|r| r.extraction_time_ms).sum::<f64>() / n;

    FactsBenchmarkResult {
        overall_ner,
        overall_rel,
        per_language,
        per_entity_type,
        avg_extraction_time_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_span(start: usize, end: usize, entity_type: &str) -> EntitySpan {
        EntitySpan {
            start,
            end,
            entity_type: entity_type.to_string(),
        }
    }

    fn make_rel(subject: &str, predicate: &str, object: &str) -> RelationSpan {
        RelationSpan {
            subject: subject.to_string(),
            predicate: predicate.to_string(),
            object: object.to_string(),
        }
    }

    #[test]
    fn test_perfect_ner() {
        let gold = vec![make_span(0, 5, "person"), make_span(10, 20, "location")];
        let pred = vec![make_span(0, 5, "person"), make_span(10, 20, "location")];
        let m = evaluate_ner(&pred, &gold);
        assert_eq!(m.exact_precision, 1.0);
        assert_eq!(m.exact_recall, 1.0);
        assert_eq!(m.exact_f1, 1.0);
    }

    #[test]
    fn test_partial_ner() {
        let gold = vec![make_span(0, 10, "person")];
        // Predicted span overlaps but isn't exact
        let pred = vec![make_span(2, 10, "person")];
        let m = evaluate_ner(&pred, &gold);
        assert_eq!(m.exact_precision, 0.0); // Not exact match
        assert_eq!(m.partial_precision, 1.0); // Overlapping match
    }

    #[test]
    fn test_no_match_ner() {
        let gold = vec![make_span(0, 5, "person")];
        let pred = vec![make_span(10, 20, "location")];
        let m = evaluate_ner(&pred, &gold);
        assert_eq!(m.exact_precision, 0.0);
        assert_eq!(m.exact_recall, 0.0);
        assert_eq!(m.exact_f1, 0.0);
    }

    #[test]
    fn test_partial_overlap_different_type() {
        let gold = vec![make_span(0, 10, "person")];
        let pred = vec![make_span(5, 15, "organization")]; // Overlaps but wrong type
        let m = evaluate_ner(&pred, &gold);
        assert_eq!(m.partial_precision, 0.0);
        assert_eq!(m.partial_recall, 0.0);
    }

    #[test]
    fn test_empty_predictions() {
        let gold = vec![make_span(0, 5, "person")];
        let pred = vec![];
        let m = evaluate_ner(&pred, &gold);
        assert_eq!(m.exact_precision, 0.0);
        assert_eq!(m.exact_recall, 0.0);
    }

    #[test]
    fn test_empty_gold() {
        let gold = vec![];
        let pred = vec![make_span(0, 5, "person")];
        let m = evaluate_ner(&pred, &gold);
        assert_eq!(m.exact_recall, 0.0);
        assert_eq!(m.exact_precision, 0.0); // 0 matches / 1 pred
    }

    #[test]
    fn test_perfect_relations() {
        let gold = vec![make_rel("microsoft", "founded_by", "bill gates")];
        let pred = vec![make_rel("microsoft", "founded_by", "bill gates")];
        let m = evaluate_relations(&pred, &gold);
        assert_eq!(m.precision, 1.0);
        assert_eq!(m.recall, 1.0);
        assert_eq!(m.f1, 1.0);
    }

    #[test]
    fn test_no_relation_match() {
        let gold = vec![make_rel("microsoft", "founded_by", "bill gates")];
        let pred = vec![make_rel("apple", "ceo_of", "tim cook")];
        let m = evaluate_relations(&pred, &gold);
        assert_eq!(m.precision, 0.0);
        assert_eq!(m.recall, 0.0);
    }

    #[test]
    fn test_partial_relations() {
        let gold = vec![
            make_rel("microsoft", "founded_by", "bill gates"),
            make_rel("apple", "ceo_of", "tim cook"),
        ];
        let pred = vec![
            make_rel("microsoft", "founded_by", "bill gates"),
            make_rel("google", "hq_in", "mountain view"),
        ];
        let m = evaluate_relations(&pred, &gold);
        assert_eq!(m.precision, 0.5); // 1/2 predictions correct
        assert_eq!(m.recall, 0.5); // 1/2 gold found
    }

    #[test]
    fn test_f1_symmetry() {
        // F1 should be the same regardless of argument order for same P/R
        let p = 0.8;
        let r = 0.6;
        let f1a = f1_score(p, r);
        let f1b = f1_score(r, p);
        // F1 is symmetric in p and r: 2*p*r/(p+r) = 2*r*p/(r+p)
        assert!((f1a - f1b).abs() < 1e-10);
    }
}
