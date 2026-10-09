use oi_types::MetricsSnapshot;
/// Prometheus-compatible metrics collector for the OpenSonarX node.
///
/// All counters use lock-free atomics for concurrent recording from
/// query/ingest hot paths.
use oi_types::traits::MetricsCollector;
use std::sync::atomic::{AtomicU64, Ordering};

pub struct IndexMetrics {
    queries_total: AtomicU64,
    /// Latency sum stored as the bits of an f64, updated via CAS loop.
    queries_latency_sum_ms: AtomicU64,
    ingest_total: AtomicU64,
    docs_stored: AtomicU64,
    peers_connected: AtomicU64,
    errors_total: AtomicU64,
}

impl IndexMetrics {
    pub fn new() -> Self {
        Self {
            queries_total: AtomicU64::new(0),
            queries_latency_sum_ms: AtomicU64::new(
                0u64.to_be_bytes().iter().copied().fold(0u64, |acc, _| acc),
            ), // 0.0 as f64 bits
            ingest_total: AtomicU64::new(0),
            docs_stored: AtomicU64::new(0),
            peers_connected: AtomicU64::new(0),
            errors_total: AtomicU64::new(0),
        }
    }

    /// Set the number of stored documents (gauge-style).
    pub fn set_docs_stored(&self, count: u64) {
        self.docs_stored.store(count, Ordering::Relaxed);
    }

    /// Set the number of connected peers (gauge-style).
    pub fn set_peers_connected(&self, count: u64) {
        self.peers_connected.store(count, Ordering::Relaxed);
    }

    /// Render metrics in Prometheus text exposition format.
    pub fn render_prometheus(&self) -> String {
        let snap = self.snapshot();
        let mut out = String::new();

        out.push_str("# HELP oi_queries_total Total queries served\n");
        out.push_str("# TYPE oi_queries_total counter\n");
        out.push_str(&format!("oi_queries_total {}\n", snap.queries_total));

        out.push_str("# HELP oi_queries_latency_sum_ms Sum of query latencies in milliseconds\n");
        out.push_str("# TYPE oi_queries_latency_sum_ms counter\n");
        out.push_str(&format!(
            "oi_queries_latency_sum_ms {}\n",
            snap.queries_latency_sum_ms
        ));

        out.push_str("# HELP oi_ingest_total Total documents ingested\n");
        out.push_str("# TYPE oi_ingest_total counter\n");
        out.push_str(&format!("oi_ingest_total {}\n", snap.ingest_total));

        out.push_str("# HELP oi_docs_stored Current number of documents stored\n");
        out.push_str("# TYPE oi_docs_stored gauge\n");
        out.push_str(&format!("oi_docs_stored {}\n", snap.docs_stored));

        out.push_str("# HELP oi_peers_connected Current number of connected peers\n");
        out.push_str("# TYPE oi_peers_connected gauge\n");
        out.push_str(&format!("oi_peers_connected {}\n", snap.peers_connected));

        out.push_str("# HELP oi_errors_total Total errors encountered\n");
        out.push_str("# TYPE oi_errors_total counter\n");
        out.push_str(&format!("oi_errors_total {}\n", snap.errors_total));

        out
    }

    /// Render a JSON health status string.
    pub fn render_health(&self) -> String {
        let snap = self.snapshot();
        let avg_latency = if snap.queries_total > 0 {
            snap.queries_latency_sum_ms / snap.queries_total as f64
        } else {
            0.0
        };
        format!(
            r#"{{"healthy":true,"queries_total":{},"ingest_total":{},"docs_stored":{},"peers_connected":{},"errors_total":{},"avg_latency_ms":{:.2}}}"#,
            snap.queries_total,
            snap.ingest_total,
            snap.docs_stored,
            snap.peers_connected,
            snap.errors_total,
            avg_latency,
        )
    }

    /// Atomically add a f64 value to the latency sum using a CAS loop.
    fn add_latency(&self, val: f64) {
        loop {
            let current_bits = self.queries_latency_sum_ms.load(Ordering::Relaxed);
            let current = f64::from_bits(current_bits);
            let new = current + val;
            let new_bits = new.to_bits();
            if self
                .queries_latency_sum_ms
                .compare_exchange_weak(current_bits, new_bits, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                break;
            }
        }
    }
}

impl Default for IndexMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsCollector for IndexMetrics {
    fn record_query(&self, latency_ms: f64) {
        self.queries_total.fetch_add(1, Ordering::Relaxed);
        self.add_latency(latency_ms);
    }

    fn record_ingest(&self, doc_count: u64) {
        self.ingest_total.fetch_add(doc_count, Ordering::Relaxed);
        self.docs_stored.fetch_add(doc_count, Ordering::Relaxed);
    }

    fn record_error(&self) {
        self.errors_total.fetch_add(1, Ordering::Relaxed);
    }

    fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            queries_total: self.queries_total.load(Ordering::Relaxed),
            queries_latency_sum_ms: f64::from_bits(
                self.queries_latency_sum_ms.load(Ordering::Relaxed),
            ),
            ingest_total: self.ingest_total.load(Ordering::Relaxed),
            docs_stored: self.docs_stored.load(Ordering::Relaxed),
            peers_connected: self.peers_connected.load(Ordering::Relaxed),
            errors_total: self.errors_total.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_query() {
        let m = IndexMetrics::new();
        m.record_query(12.5);
        m.record_query(7.5);

        let snap = m.snapshot();
        assert_eq!(snap.queries_total, 2);
        assert!((snap.queries_latency_sum_ms - 20.0).abs() < 0.001);
    }

    #[test]
    fn test_record_ingest() {
        let m = IndexMetrics::new();
        m.record_ingest(10);
        m.record_ingest(5);

        let snap = m.snapshot();
        assert_eq!(snap.ingest_total, 15);
        assert_eq!(snap.docs_stored, 15);
    }

    #[test]
    fn test_prometheus_format() {
        let m = IndexMetrics::new();
        m.record_query(5.0);
        m.record_ingest(3);
        m.record_error();

        let output = m.render_prometheus();
        assert!(output.contains("oi_queries_total 1"));
        assert!(output.contains("oi_ingest_total 3"));
        assert!(output.contains("oi_errors_total 1"));
        assert!(output.contains("oi_docs_stored 3"));
        assert!(output.contains("# HELP oi_queries_total"));
        assert!(output.contains("# TYPE oi_queries_total counter"));
    }

    #[test]
    fn test_snapshot() {
        let m = IndexMetrics::new();

        // Initially everything is zero
        let snap = m.snapshot();
        assert_eq!(snap.queries_total, 0);
        assert_eq!(snap.ingest_total, 0);
        assert_eq!(snap.docs_stored, 0);
        assert_eq!(snap.errors_total, 0);
        assert!((snap.queries_latency_sum_ms - 0.0).abs() < 0.001);

        // After some operations
        m.record_query(10.0);
        m.record_ingest(100);
        m.set_peers_connected(5);

        let snap = m.snapshot();
        assert_eq!(snap.queries_total, 1);
        assert_eq!(snap.ingest_total, 100);
        assert_eq!(snap.docs_stored, 100);
        assert_eq!(snap.peers_connected, 5);
        assert!((snap.queries_latency_sum_ms - 10.0).abs() < 0.001);
    }
}
