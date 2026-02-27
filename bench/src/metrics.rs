/// Recall@K: fraction of relevant docs found in top-K results.
pub fn recall_at_k(relevant_ids: &[&str], retrieved_ids: &[&str], k: usize) -> f64 {
    if relevant_ids.is_empty() {
        return 0.0;
    }
    let top_k: Vec<&str> = retrieved_ids.iter().take(k).copied().collect();
    let found = relevant_ids
        .iter()
        .filter(|r| top_k.contains(r))
        .count();
    found as f64 / relevant_ids.len() as f64
}

/// Mean Reciprocal Rank: 1/rank of first relevant result.
pub fn mrr(relevant_ids: &[&str], retrieved_ids: &[&str]) -> f64 {
    for (i, id) in retrieved_ids.iter().enumerate() {
        if relevant_ids.contains(id) {
            return 1.0 / (i + 1) as f64;
        }
    }
    0.0
}

/// nDCG@K: normalized discounted cumulative gain.
/// `relevance_map` maps doc_id -> graded relevance (0-3).
pub fn ndcg_at_k(
    relevance_map: &[(&str, u8)],
    retrieved_ids: &[&str],
    k: usize,
) -> f64 {
    let get_rel = |id: &str| -> f64 {
        relevance_map
            .iter()
            .find(|(doc_id, _)| *doc_id == id)
            .map(|(_, r)| *r as f64)
            .unwrap_or(0.0)
    };

    // DCG@K
    let dcg: f64 = retrieved_ids
        .iter()
        .take(k)
        .enumerate()
        .map(|(i, id)| {
            let rel = get_rel(id);
            (2.0f64.powf(rel) - 1.0) / (i as f64 + 2.0).log2()
        })
        .sum();

    // Ideal DCG@K: sort by relevance descending
    let mut ideal_rels: Vec<f64> = relevance_map.iter().map(|(_, r)| *r as f64).collect();
    ideal_rels.sort_by(|a, b| b.partial_cmp(a).unwrap());

    let idcg: f64 = ideal_rels
        .iter()
        .take(k)
        .enumerate()
        .map(|(i, &rel)| (2.0f64.powf(rel) - 1.0) / (i as f64 + 2.0).log2())
        .sum();

    if idcg == 0.0 {
        0.0
    } else {
        dcg / idcg
    }
}

/// Adversarial rejection rate: fraction of adversarial docs correctly rejected.
#[allow(dead_code)]
pub fn adversarial_rejection_rate(total: usize, rejected: usize) -> f64 {
    if total == 0 {
        return 0.0;
    }
    rejected as f64 / total as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recall_perfect() {
        let relevant = vec!["a", "b", "c"];
        let retrieved = vec!["a", "b", "c", "d", "e"];
        assert_eq!(recall_at_k(&relevant, &retrieved, 5), 1.0);
    }

    #[test]
    fn test_recall_partial() {
        let relevant = vec!["a", "b", "c"];
        let retrieved = vec!["a", "d", "e", "b", "f"];
        assert!((recall_at_k(&relevant, &retrieved, 3) - 1.0 / 3.0).abs() < 0.01);
    }

    #[test]
    fn test_recall_at_1() {
        let relevant = vec!["a"];
        let retrieved = vec!["b", "a", "c"];
        assert_eq!(recall_at_k(&relevant, &retrieved, 1), 0.0);
        assert_eq!(recall_at_k(&relevant, &retrieved, 2), 1.0);
    }

    #[test]
    fn test_recall_empty_relevant() {
        assert_eq!(recall_at_k(&[], &["a", "b"], 5), 0.0);
    }

    #[test]
    fn test_mrr_first() {
        let relevant = vec!["a"];
        let retrieved = vec!["a", "b", "c"];
        assert_eq!(mrr(&relevant, &retrieved), 1.0);
    }

    #[test]
    fn test_mrr_third() {
        let relevant = vec!["c"];
        let retrieved = vec!["a", "b", "c"];
        assert!((mrr(&relevant, &retrieved) - 1.0 / 3.0).abs() < 0.001);
    }

    #[test]
    fn test_mrr_not_found() {
        let relevant = vec!["x"];
        let retrieved = vec!["a", "b", "c"];
        assert_eq!(mrr(&relevant, &retrieved), 0.0);
    }

    #[test]
    fn test_ndcg_perfect() {
        let rels = vec![("a", 3u8), ("b", 2), ("c", 1)];
        let retrieved = vec!["a", "b", "c"];
        let score = ndcg_at_k(&rels, &retrieved, 3);
        assert!((score - 1.0).abs() < 0.001, "perfect ranking should give nDCG=1.0, got {score}");
    }

    #[test]
    fn test_ndcg_reversed() {
        let rels = vec![("a", 3u8), ("b", 2), ("c", 1)];
        let retrieved = vec!["c", "b", "a"]; // worst order
        let score = ndcg_at_k(&rels, &retrieved, 3);
        assert!(score < 1.0, "reversed ranking should be < 1.0");
        assert!(score > 0.0, "reversed ranking should be > 0.0");
    }

    #[test]
    fn test_ndcg_no_relevant() {
        let rels: Vec<(&str, u8)> = vec![("a", 0), ("b", 0)];
        let retrieved = vec!["a", "b"];
        let score = ndcg_at_k(&rels, &retrieved, 2);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_adversarial_rejection() {
        assert_eq!(adversarial_rejection_rate(100, 95), 0.95);
        assert_eq!(adversarial_rejection_rate(0, 0), 0.0);
    }
}
