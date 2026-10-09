use oi_types::{LeaderboardEntry, QueryLeaderboard, Result};
use std::collections::HashMap;
use std::sync::RwLock;

/// In-memory leaderboard for local (web3) nodes.
///
/// Tracks query counts per domain/entity identifier. Counts are lost on restart
/// unless the node persists them externally.
pub struct InMemoryLeaderboard {
    counts: RwLock<HashMap<String, u64>>,
}

impl InMemoryLeaderboard {
    pub fn new() -> Self {
        Self {
            counts: RwLock::new(HashMap::new()),
        }
    }
}

impl Default for InMemoryLeaderboard {
    fn default() -> Self {
        Self::new()
    }
}

impl QueryLeaderboard for InMemoryLeaderboard {
    fn record_query(&self, identifiers: &[&str]) -> Result<()> {
        let mut counts = self.counts.write().unwrap();
        for id in identifiers {
            *counts.entry(id.to_string()).or_insert(0) += 1;
        }
        Ok(())
    }

    fn top_queried(&self, limit: usize) -> Result<Vec<LeaderboardEntry>> {
        let counts = self.counts.read().unwrap();
        let mut entries: Vec<_> = counts.iter().collect();
        entries.sort_by(|a, b| b.1.cmp(a.1));

        Ok(entries
            .into_iter()
            .take(limit)
            .map(|(identifier, &query_count)| {
                let is_entity = identifier.contains('/');
                LeaderboardEntry {
                    identifier: identifier.clone(),
                    query_count,
                    stake_truth: 0, // enriched at display time
                    stake_usd: 0.0,
                    is_entity,
                    content_kind: None,
                }
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_and_query() {
        let lb = InMemoryLeaderboard::new();

        lb.record_query(&["nytimes.com", "bbc.com", "nytimes.com"])
            .unwrap();
        lb.record_query(&["nytimes.com", "cnn.com"]).unwrap();

        let top = lb.top_queried(10).unwrap();
        assert_eq!(top.len(), 3);
        assert_eq!(top[0].identifier, "nytimes.com");
        assert_eq!(top[0].query_count, 3);
        assert!(!top[0].is_entity);
        assert_eq!(top[1].query_count, 1); // bbc or cnn
    }

    #[test]
    fn test_entity_detection() {
        let lb = InMemoryLeaderboard::new();
        lb.record_query(&["youtube.com/c/3blue1brown", "nytimes.com"])
            .unwrap();

        let top = lb.top_queried(10).unwrap();
        let entity = top
            .iter()
            .find(|e| e.identifier == "youtube.com/c/3blue1brown")
            .unwrap();
        assert!(entity.is_entity);

        let domain = top.iter().find(|e| e.identifier == "nytimes.com").unwrap();
        assert!(!domain.is_entity);
    }

    #[test]
    fn test_limit() {
        let lb = InMemoryLeaderboard::new();
        for i in 0..100 {
            lb.record_query(&[&format!("site{i}.com")]).unwrap();
        }

        let top = lb.top_queried(5).unwrap();
        assert_eq!(top.len(), 5);
    }

    #[test]
    fn test_empty() {
        let lb = InMemoryLeaderboard::new();
        let top = lb.top_queried(10).unwrap();
        assert!(top.is_empty());
    }
}
