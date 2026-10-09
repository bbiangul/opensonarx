//! Creator exposure telemetry — not a billing or settlement ledger.
//!
//! Agents answering from indexed content generate no page view, no ad
//! impression, no revenue for the creator whose work supplied the answer.
//! This ledger records local-result exposure with heuristic position weights.
//! It does not prove reading, answer contribution, ownership, licensing,
//! buyer consent or authorized delivery. Its credits have no monetary value
//! and must never be fed directly into settlement. A separate authorized
//! delivery receipt and reconciliation flow are required for payable P0 use.
//!
//! Attribution counts SERVED results (what the agent received), weighted by
//! position: top-3 is a placement signal, not observed consumption.
//! External-engine results are never attributed. Open results and citations
//! do not create a debt to a publisher merely by appearing in this ledger.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

/// Accrued usage for one creator domain.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CreatorUsage {
    /// Total times this domain's content appeared in served results.
    pub served: u64,
    /// Times it appeared in the top 3 (placement, not proven consumption).
    pub top3: u64,
    /// Non-monetary exposure weight; not fee units or a payable balance.
    pub credits: u64,
    /// Whether the domain was stake-backed at last attribution.
    pub stake_backed: bool,
    /// Unix seconds of most recent usage.
    pub last_used: u64,
}

/// Heuristic exposure weights. Neither position establishes consumption.
const CREDITS_TOP3: u64 = 10;
const CREDITS_SERVED: u64 = 2;

#[derive(Debug, Default, Serialize, Deserialize)]
struct LedgerFile {
    domains: HashMap<String, CreatorUsage>,
    /// Total queries that produced at least one attribution.
    queries_attributed: u64,
}

/// Persistent, process-safe (best-effort) usage ledger.
pub struct CreatorLedger {
    path: Option<String>,
    state: Mutex<LedgerFile>,
}

impl CreatorLedger {
    /// Load from `path` (or start empty; None = in-memory only).
    pub fn load(path: Option<String>) -> Self {
        let state = path
            .as_deref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|bytes| serde_json::from_slice::<LedgerFile>(&bytes).ok())
            .unwrap_or_default();
        Self {
            path,
            state: Mutex::new(state),
        }
    }

    /// Attribute one query's served results. `served` is
    /// (domain, rank, stake_backed) for each LOCAL result returned.
    pub fn record_query(&self, served: &[(String, u32, bool)]) {
        if served.is_empty() {
            return;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        state.queries_attributed += 1;
        for (domain, rank, stake_backed) in served {
            if domain.is_empty() {
                continue;
            }
            let entry = state.domains.entry(domain.clone()).or_default();
            entry.served += 1;
            entry.stake_backed = *stake_backed;
            entry.last_used = now;
            if *rank <= 3 {
                entry.top3 += 1;
                entry.credits += CREDITS_TOP3;
            } else {
                entry.credits += CREDITS_SERVED;
            }
        }
        drop(state);
        self.save();
    }

    /// Snapshot of all creator usage, sorted by credits descending.
    pub fn snapshot(&self) -> (u64, Vec<(String, CreatorUsage)>) {
        let Ok(state) = self.state.lock() else {
            return (0, Vec::new());
        };
        let mut rows: Vec<(String, CreatorUsage)> = state
            .domains
            .iter()
            .map(|(d, u)| (d.clone(), u.clone()))
            .collect();
        rows.sort_by(|a, b| b.1.credits.cmp(&a.1.credits));
        (state.queries_attributed, rows)
    }

    fn save(&self) {
        let Some(ref path) = self.path else { return };
        let Ok(state) = self.state.lock() else { return };
        if let Some(parent) = Path::new(path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match serde_json::to_vec_pretty(&*state) {
            Ok(bytes) => {
                if let Err(e) = std::fs::write(path, bytes) {
                    tracing::warn!("failed to persist creator ledger: {e}");
                }
            }
            Err(e) => tracing::warn!("failed to serialize creator ledger: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_attribution_weights_and_persistence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ledger.json").to_string_lossy().to_string();

        let ledger = CreatorLedger::load(Some(path.clone()));
        ledger.record_query(&[
            ("blog.example".to_string(), 1, true),
            ("blog.example".to_string(), 5, true),
            ("other.example".to_string(), 2, false),
        ]);

        let (queries, rows) = ledger.snapshot();
        assert_eq!(queries, 1);
        let blog = rows.iter().find(|(d, _)| d == "blog.example").unwrap();
        assert_eq!(blog.1.served, 2);
        assert_eq!(blog.1.top3, 1);
        assert_eq!(blog.1.credits, CREDITS_TOP3 + CREDITS_SERVED);
        assert!(blog.1.stake_backed);

        // Reload from disk — usage must survive the process.
        let reloaded = CreatorLedger::load(Some(path));
        let (q2, rows2) = reloaded.snapshot();
        assert_eq!(q2, 1);
        assert_eq!(rows2.len(), 2);
    }

    #[test]
    fn test_empty_serve_is_noop_and_memory_only_works() {
        let ledger = CreatorLedger::load(None);
        ledger.record_query(&[]);
        let (q, rows) = ledger.snapshot();
        assert_eq!(q, 0);
        assert!(rows.is_empty());
    }
}
