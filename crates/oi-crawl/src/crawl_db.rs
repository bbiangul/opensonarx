//! Lightweight SQLite wrapper for frontier persistence.

use rusqlite::Connection;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::Duration;

use crate::frontier::FrontierEntry;

/// SQLite-backed persistence for the crawl frontier.
pub struct CrawlDb {
    conn: Mutex<Connection>,
}

impl CrawlDb {
    /// Open (or create) a SQLite database at the given path with WAL mode.
    pub fn open(path: &str) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(|e| format!("SQLite open error: {e}"))?;
        let db = Self {
            conn: Mutex::new(conn),
        };
        db.init_schema()?;
        Ok(db)
    }

    /// Create an in-memory database (for tests).
    pub fn open_in_memory() -> Self {
        let conn = Connection::open_in_memory().expect("in-memory SQLite");
        let db = Self {
            conn: Mutex::new(conn),
        };
        db.init_schema().expect("schema init");
        db
    }

    fn init_schema(&self) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL;")
            .map_err(|e| format!("WAL pragma: {e}"))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS frontier_urls (
                url TEXT PRIMARY KEY,
                domain TEXT NOT NULL,
                priority INTEGER NOT NULL,
                domain_stake INTEGER NOT NULL,
                added_at_secs INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS frontier_seen (url TEXT PRIMARY KEY);
            CREATE TABLE IF NOT EXISTS domain_state (
                domain TEXT PRIMARY KEY,
                delay_ms INTEGER NOT NULL DEFAULT 2000,
                last_crawl_secs INTEGER NOT NULL DEFAULT 0,
                crawl_count INTEGER NOT NULL DEFAULT 0
            );",
        )
        .map_err(|e| format!("Schema init: {e}"))?;
        Ok(())
    }

    /// Persist the full frontier state (destructive replace).
    pub fn save_frontier(
        &self,
        entries: &[FrontierEntry],
        seen: &HashSet<String>,
        domain_last_crawl: &HashMap<String, i64>,
        domain_delays: &HashMap<String, Duration>,
        domain_crawl_counts: &HashMap<String, usize>,
    ) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("tx begin: {e}"))?;

        tx.execute("DELETE FROM frontier_urls", [])
            .map_err(|e| format!("clear urls: {e}"))?;
        tx.execute("DELETE FROM frontier_seen", [])
            .map_err(|e| format!("clear seen: {e}"))?;
        tx.execute("DELETE FROM domain_state", [])
            .map_err(|e| format!("clear domain_state: {e}"))?;

        for entry in entries {
            tx.execute(
                "INSERT INTO frontier_urls (url, domain, priority, domain_stake, added_at_secs) VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![entry.url, entry.domain, entry.priority as i64, entry.domain_stake as i64, entry.added_at],
            )
            .map_err(|e| format!("insert url: {e}"))?;
        }

        for url in seen {
            tx.execute("INSERT INTO frontier_seen (url) VALUES (?1)", [url])
                .map_err(|e| format!("insert seen: {e}"))?;
        }

        // Merge domain_last_crawl, domain_delays, and domain_crawl_counts
        let mut all_domains: HashSet<&str> = HashSet::new();
        for d in domain_last_crawl.keys() {
            all_domains.insert(d.as_str());
        }
        for d in domain_delays.keys() {
            all_domains.insert(d.as_str());
        }
        for d in domain_crawl_counts.keys() {
            all_domains.insert(d.as_str());
        }

        for domain in all_domains {
            let delay_ms = domain_delays
                .get(domain)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(2000);
            let last_crawl = domain_last_crawl.get(domain).copied().unwrap_or(0);
            let crawl_count = domain_crawl_counts.get(domain).copied().unwrap_or(0) as i64;
            tx.execute(
                "INSERT INTO domain_state (domain, delay_ms, last_crawl_secs, crawl_count) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![domain, delay_ms, last_crawl, crawl_count],
            )
            .map_err(|e| format!("insert domain_state: {e}"))?;
        }

        tx.commit().map_err(|e| format!("tx commit: {e}"))?;
        Ok(())
    }

    /// Load the full frontier state from the database.
    #[allow(clippy::type_complexity)]
    pub fn load_frontier(
        &self,
    ) -> Result<
        (
            Vec<FrontierEntry>,
            HashSet<String>,
            HashMap<String, i64>,
            HashMap<String, Duration>,
            HashMap<String, usize>,
        ),
        String,
    > {
        let conn = self.conn.lock().unwrap();

        // Load URLs
        let mut stmt = conn
            .prepare("SELECT url, domain, priority, domain_stake, added_at_secs FROM frontier_urls")
            .map_err(|e| format!("prepare urls: {e}"))?;
        let entries: Vec<FrontierEntry> = stmt
            .query_map([], |row| {
                Ok(FrontierEntry {
                    url: row.get(0)?,
                    domain: row.get(1)?,
                    priority: row.get::<_, i64>(2)? as u64,
                    domain_stake: row.get::<_, i64>(3)? as u64,
                    added_at: row.get(4)?,
                })
            })
            .map_err(|e| format!("query urls: {e}"))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| format!("collect urls: {e}"))?;

        // Load seen
        let mut stmt = conn
            .prepare("SELECT url FROM frontier_seen")
            .map_err(|e| format!("prepare seen: {e}"))?;
        let seen: HashSet<String> = stmt
            .query_map([], |row| row.get(0))
            .map_err(|e| format!("query seen: {e}"))?
            .collect::<std::result::Result<HashSet<_>, _>>()
            .map_err(|e| format!("collect seen: {e}"))?;

        // Load domain state
        let mut stmt = conn
            .prepare("SELECT domain, delay_ms, last_crawl_secs, crawl_count FROM domain_state")
            .map_err(|e| format!("prepare domain_state: {e}"))?;
        let mut domain_last_crawl = HashMap::new();
        let mut domain_delays = HashMap::new();
        let mut domain_crawl_counts = HashMap::new();

        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })
            .map_err(|e| format!("query domain_state: {e}"))?;

        for row in rows {
            let (domain, delay_ms, last_crawl, crawl_count) =
                row.map_err(|e| format!("domain_state row: {e}"))?;
            if last_crawl > 0 {
                domain_last_crawl.insert(domain.clone(), last_crawl);
            }
            domain_delays.insert(domain.clone(), Duration::from_millis(delay_ms as u64));
            if crawl_count > 0 {
                domain_crawl_counts.insert(domain, crawl_count as usize);
            }
        }

        Ok((
            entries,
            seen,
            domain_last_crawl,
            domain_delays,
            domain_crawl_counts,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_checkpoint_and_restore() {
        let db = CrawlDb::open_in_memory();

        let entries = vec![
            FrontierEntry {
                url: "https://a.com/1".to_string(),
                domain: "a.com".to_string(),
                priority: 100,
                domain_stake: 5_000_000,
                added_at: 1700000000,
            },
            FrontierEntry {
                url: "https://b.com/1".to_string(),
                domain: "b.com".to_string(),
                priority: 50,
                domain_stake: 1_000_000,
                added_at: 1700000001,
            },
        ];
        let mut seen = HashSet::new();
        seen.insert("https://a.com/1".to_string());
        seen.insert("https://b.com/1".to_string());
        seen.insert("https://c.com/old".to_string());
        let domain_last_crawl = HashMap::new();
        let domain_delays = HashMap::new();
        let domain_crawl_counts = HashMap::new();

        db.save_frontier(
            &entries,
            &seen,
            &domain_last_crawl,
            &domain_delays,
            &domain_crawl_counts,
        )
        .unwrap();

        let (loaded_entries, loaded_seen, _, _, _) = db.load_frontier().unwrap();
        assert_eq!(loaded_entries.len(), 2);
        assert_eq!(loaded_seen.len(), 3);
        assert!(loaded_seen.contains("https://c.com/old"));
    }

    #[test]
    fn test_seen_set_persists() {
        let db = CrawlDb::open_in_memory();

        let mut seen = HashSet::new();
        seen.insert("https://example.com/page".to_string());
        db.save_frontier(
            &[],
            &seen,
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
        )
        .unwrap();

        let (_, loaded_seen, _, _, _) = db.load_frontier().unwrap();
        assert!(loaded_seen.contains("https://example.com/page"));
    }

    #[test]
    fn test_domain_delays_persist() {
        let db = CrawlDb::open_in_memory();

        let mut delays = HashMap::new();
        delays.insert("slow.com".to_string(), Duration::from_secs(10));
        delays.insert("fast.com".to_string(), Duration::from_millis(500));

        db.save_frontier(
            &[],
            &HashSet::new(),
            &HashMap::new(),
            &delays,
            &HashMap::new(),
        )
        .unwrap();

        let (_, _, _, loaded_delays, _) = db.load_frontier().unwrap();
        assert_eq!(loaded_delays["slow.com"], Duration::from_secs(10));
        assert_eq!(loaded_delays["fast.com"], Duration::from_millis(500));
    }
}
