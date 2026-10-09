//! Circuit breaker wrapper for search engines.
//!
//! Tracks per-engine failures and applies exponential backoff to prevent
//! hammering a downed engine. State transitions:
//!
//! ```text
//! Closed (healthy) → Open (tripped, all calls blocked)
//!                         ↓ after cooldown
//!                   HalfOpen (one probe allowed)
//!                     ↙          ↘
//!              Closed (OK)    Open (fail again)
//! ```

use oi_types::{SearchEngine, SearchParams, SearchResultItem};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

/// Circuit breaker state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Normal operation — all calls pass through.
    Closed,
    /// Engine is failing — all calls are blocked until cooldown expires.
    Open,
    /// Cooldown expired — one probe call is allowed to test recovery.
    HalfOpen,
}

struct BreakerState {
    state: State,
    consecutive_failures: u32,
    last_failure: Option<Instant>,
    cooldown: Duration,
    in_flight: usize,
}

impl BreakerState {
    fn new(initial_cooldown: Duration) -> Self {
        Self {
            state: State::Closed,
            consecutive_failures: 0,
            last_failure: None,
            cooldown: initial_cooldown,
            in_flight: 0,
        }
    }
}

/// Circuit breaker configuration.
pub struct CircuitBreakerConfig {
    /// Number of consecutive failures before opening the circuit.
    pub failure_threshold: u32,
    /// Initial cooldown before trying a half-open probe.
    pub initial_cooldown: Duration,
    /// Maximum cooldown after repeated failures (caps exponential backoff).
    pub max_cooldown: Duration,
    /// Maximum simultaneous calls while the engine is healthy. This bounds
    /// detached resolver/HTTP workers without dropping all concurrent healthy
    /// queries behind a provider-wide single-flight lock.
    pub max_in_flight: usize,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 3,
            initial_cooldown: Duration::from_secs(30),
            max_cooldown: Duration::from_secs(300), // 5 minutes
            max_in_flight: 4,
        }
    }
}

/// Wraps a `SearchEngine` with circuit breaker protection.
///
/// When the inner engine fails repeatedly, the circuit opens and subsequent
/// calls return empty results immediately (no HTTP call). After a cooldown
/// period, one probe call is allowed. If it succeeds, the circuit closes.
/// If it fails again, the cooldown doubles (exponential backoff, capped).
pub struct CircuitBreakerEngine {
    inner: Arc<dyn SearchEngine>,
    state: RwLock<BreakerState>,
    config: CircuitBreakerConfig,
}

impl CircuitBreakerEngine {
    pub fn new(inner: Arc<dyn SearchEngine>) -> Self {
        let config = CircuitBreakerConfig::default();
        let state = RwLock::new(BreakerState::new(config.initial_cooldown));
        Self {
            inner,
            state,
            config,
        }
    }

    pub fn with_config(inner: Arc<dyn SearchEngine>, config: CircuitBreakerConfig) -> Self {
        let state = RwLock::new(BreakerState::new(config.initial_cooldown));
        Self {
            inner,
            state,
            config,
        }
    }

    /// Check if a call should proceed. Returns true if allowed, false if blocked.
    fn should_allow(&self) -> bool {
        let mut state = self.state.write().unwrap();

        match state.state {
            State::Closed => {
                if state.in_flight >= self.config.max_in_flight.max(1) {
                    false
                } else {
                    state.in_flight += 1;
                    true
                }
            }
            State::Open => {
                // Check if cooldown has elapsed → transition to HalfOpen
                if let Some(last_failure) = state.last_failure
                    && last_failure.elapsed() >= state.cooldown
                    && state.in_flight == 0
                {
                    state.state = State::HalfOpen;
                    state.in_flight = 1;
                    tracing::info!(
                        engine = %self.inner.name(),
                        "circuit breaker: half-open, allowing probe"
                    );
                    true
                } else {
                    false
                }
            }
            State::HalfOpen => {
                // Already in half-open — only one probe at a time.
                // Additional calls during the probe are blocked.
                false
            }
        }
    }

    /// Record a successful call.
    fn record_success(&self) {
        let mut state = self.state.write().unwrap();
        if state.state != State::Closed {
            tracing::info!(engine = %self.inner.name(), "circuit breaker: closed (recovered)");
        }
        state.state = State::Closed;
        state.in_flight = state.in_flight.saturating_sub(1);
        state.consecutive_failures = 0;
        state.cooldown = self.config.initial_cooldown;
    }

    /// Record a failed call.
    fn record_failure(&self) {
        let mut state = self.state.write().unwrap();
        state.consecutive_failures += 1;
        state.last_failure = Some(Instant::now());
        state.in_flight = state.in_flight.saturating_sub(1);

        if state.state == State::HalfOpen {
            // Probe failed — re-open with doubled cooldown
            state.state = State::Open;
            state.cooldown = (state.cooldown * 2).min(self.config.max_cooldown);
            tracing::warn!(
                engine = %self.inner.name(),
                cooldown_secs = state.cooldown.as_secs(),
                "circuit breaker: probe failed, re-opened with increased cooldown"
            );
        } else if state.consecutive_failures >= self.config.failure_threshold {
            state.state = State::Open;
            tracing::warn!(
                engine = %self.inner.name(),
                failures = state.consecutive_failures,
                cooldown_secs = state.cooldown.as_secs(),
                "circuit breaker: opened"
            );
        }
    }

    /// Execute a search through the circuit breaker.
    fn execute<F>(&self, f: F) -> Vec<SearchResultItem>
    where
        F: FnOnce() -> std::result::Result<Vec<SearchResultItem>, ()>,
    {
        // Apply short backpressure when a healthy keyless provider reaches
        // its concurrency cap. Returning an immediate empty result here made
        // load look indistinguishable from “the web has no answer”. Four
        // workers at normal sub-second latency drain a rewrite burst well
        // inside the outer two-second orchestration deadline.
        const CAPACITY_WAIT: Duration = Duration::from_millis(1_500);
        const CAPACITY_POLL: Duration = Duration::from_millis(5);
        let wait_started = Instant::now();
        loop {
            if self.should_allow() {
                break;
            }
            let waiting_for_capacity = self
                .state
                .read()
                .map(|state| {
                    state.state == State::Closed
                        && state.in_flight >= self.config.max_in_flight.max(1)
                })
                .unwrap_or(false);
            if !waiting_for_capacity || wait_started.elapsed() >= CAPACITY_WAIT {
                tracing::debug!(
                    engine = %self.inner.name(),
                    "circuit breaker: call blocked (open or capacity deadline)"
                );
                return Vec::new();
            }
            std::thread::sleep(CAPACITY_POLL);
        }

        match f() {
            Ok(results) => {
                // Zero matching documents is a healthy search response. Only
                // an adapter-reported transport/parser error may trip the
                // circuit; cardinality is not a health signal.
                self.record_success();
                results
            }
            Err(()) => {
                self.record_failure();
                Vec::new()
            }
        }
    }
}

impl SearchEngine for CircuitBreakerEngine {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn search(&self, query: &str, limit: usize) -> Vec<SearchResultItem> {
        self.execute(|| self.inner.search_checked(query, limit))
    }

    fn search_with_params(
        &self,
        query: &str,
        limit: usize,
        params: &SearchParams,
    ) -> Vec<SearchResultItem> {
        self.execute(|| self.inner.search_with_params_checked(query, limit, params))
    }

    fn requires_api_key(&self) -> bool {
        self.inner.requires_api_key()
    }

    fn is_available(&self) -> bool {
        // If circuit is open, report as unavailable so orchestrator can skip
        let state = self.state.read().unwrap();
        if state.state == State::Open
            && let Some(last_failure) = state.last_failure
            && last_failure.elapsed() < state.cooldown
        {
            return false;
        }
        self.inner.is_available()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    struct FailingEngine {
        should_fail: AtomicBool,
        call_count: AtomicUsize,
    }

    struct SlowEngine;

    struct SlowHealthyEngine;

    impl SearchEngine for SlowEngine {
        fn name(&self) -> &str {
            "slow"
        }

        fn search(&self, _query: &str, _limit: usize) -> Vec<SearchResultItem> {
            std::thread::sleep(Duration::from_millis(100));
            Vec::new()
        }
    }

    impl SearchEngine for SlowHealthyEngine {
        fn name(&self) -> &str {
            "slow-healthy"
        }

        fn search(&self, _query: &str, _limit: usize) -> Vec<SearchResultItem> {
            std::thread::sleep(Duration::from_millis(50));
            vec![successful_result()]
        }
    }

    fn successful_result() -> SearchResultItem {
        SearchResultItem {
            result_id: "r1".to_string(),
            rank: 1,
            url: "https://example.com".to_string(),
            title: "Example".to_string(),
            snippet: "A snippet".to_string(),
            content_md: String::new(),
            semantic_score: 0.0,
            bm25_score: 0.0,
            final_score: 0.0,
            stake_truth: 0,
            stake_usd: 0,
            stake_type: "none".to_string(),
            reputation: 0.0,
            content_hash: String::new(),
            extraction_quality: "external".to_string(),
            source: "failing".to_string(),
            entity: None,
            content_kind: None,
            crawled_at: None,
            published_at: None,
            trust_verdict: "external".to_string(),
            reputation_votes: 0,
            disputed: false,
            facts: None,
            content_relevant: None,
            domain_first_seen_at: None,
            domain_url_count: None,
            domain_reputation_score: None,
            domain_total_votes: None,
            decision: None,
        }
    }

    impl FailingEngine {
        fn new(should_fail: bool) -> Self {
            Self {
                should_fail: AtomicBool::new(should_fail),
                call_count: AtomicUsize::new(0),
            }
        }

        fn set_fail(&self, fail: bool) {
            self.should_fail.store(fail, Ordering::SeqCst);
        }
    }

    impl SearchEngine for FailingEngine {
        fn name(&self) -> &str {
            "failing"
        }

        fn search(&self, _query: &str, _limit: usize) -> Vec<SearchResultItem> {
            self.search_checked(_query, _limit).unwrap_or_default()
        }

        fn search_checked(
            &self,
            _query: &str,
            _limit: usize,
        ) -> std::result::Result<Vec<SearchResultItem>, ()> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            if self.should_fail.load(Ordering::SeqCst) {
                Err(())
            } else {
                Ok(vec![successful_result()])
            }
        }
    }

    #[test]
    fn test_healthy_engine_passes_through() {
        let inner = Arc::new(FailingEngine::new(false));
        let cb = CircuitBreakerEngine::new(inner.clone());

        let results = cb.search("test", 10);
        assert_eq!(results.len(), 1);
        assert_eq!(inner.call_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_healthy_engine_serves_multiple_concurrent_queries() {
        let breaker = Arc::new(CircuitBreakerEngine::new(Arc::new(SlowHealthyEngine)));
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let workers: Vec<_> = (0..2)
            .map(|index| {
                let breaker = Arc::clone(&breaker);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    breaker.search(&format!("query-{index}"), 5)
                })
            })
            .collect();
        assert!(
            workers
                .into_iter()
                .all(|worker| !worker.join().unwrap().is_empty())
        );
    }

    #[test]
    fn test_circuit_opens_after_threshold() {
        let inner = Arc::new(FailingEngine::new(true));
        let config = CircuitBreakerConfig {
            failure_threshold: 3,
            initial_cooldown: Duration::from_secs(60),
            max_cooldown: Duration::from_secs(300),
            max_in_flight: 4,
        };
        let cb = CircuitBreakerEngine::with_config(inner.clone(), config);

        // 3 failures should open the circuit
        cb.search("test", 10);
        cb.search("test", 10);
        cb.search("test", 10);
        assert_eq!(inner.call_count.load(Ordering::SeqCst), 3);

        // 4th call should be blocked (circuit open)
        let results = cb.search("test", 10);
        assert!(results.is_empty());
        assert_eq!(inner.call_count.load(Ordering::SeqCst), 3); // no new call
    }

    #[test]
    fn test_circuit_reports_unavailable_when_open() {
        let inner = Arc::new(FailingEngine::new(true));
        let config = CircuitBreakerConfig {
            failure_threshold: 2,
            initial_cooldown: Duration::from_secs(60),
            max_cooldown: Duration::from_secs(300),
            max_in_flight: 4,
        };
        let cb = CircuitBreakerEngine::with_config(inner, config);

        assert!(cb.is_available());

        // Trip the breaker
        cb.search("test", 10);
        cb.search("test", 10);

        assert!(!cb.is_available());
    }

    #[test]
    fn test_legitimate_empty_results_do_not_open_the_circuit() {
        let cb = CircuitBreakerEngine::with_config(
            Arc::new(SlowEngine),
            CircuitBreakerConfig {
                failure_threshold: 1,
                initial_cooldown: Duration::from_secs(60),
                max_cooldown: Duration::from_secs(300),
                max_in_flight: 4,
            },
        );
        assert!(cb.search("no matching documents", 10).is_empty());
        assert!(cb.is_available());
        assert!(cb.search("still healthy", 10).is_empty());
        assert!(cb.is_available());
    }

    #[test]
    fn test_half_open_recovery() {
        let inner = Arc::new(FailingEngine::new(true));
        let config = CircuitBreakerConfig {
            failure_threshold: 2,
            initial_cooldown: Duration::from_millis(0), // instant cooldown for test
            max_cooldown: Duration::from_secs(300),
            max_in_flight: 4,
        };
        let cb = CircuitBreakerEngine::with_config(inner.clone(), config);

        // Trip the breaker
        cb.search("test", 10);
        cb.search("test", 10);
        assert_eq!(inner.call_count.load(Ordering::SeqCst), 2);

        // Cooldown=0ms, so next call should go through as half-open probe
        inner.set_fail(false);
        let results = cb.search("test", 10);
        assert_eq!(results.len(), 1);
        assert_eq!(inner.call_count.load(Ordering::SeqCst), 3);

        // Circuit should be closed now — next call succeeds normally
        let results = cb.search("test", 10);
        assert_eq!(results.len(), 1);
        assert_eq!(inner.call_count.load(Ordering::SeqCst), 4);
    }

    #[test]
    fn test_name_delegates() {
        let inner = Arc::new(FailingEngine::new(false));
        let cb = CircuitBreakerEngine::new(inner);
        assert_eq!(cb.name(), "failing");
    }

    #[test]
    fn test_configured_in_flight_cap_applies_backpressure() {
        let breaker = Arc::new(CircuitBreakerEngine::with_config(
            Arc::new(SlowEngine),
            CircuitBreakerConfig {
                failure_threshold: 3,
                initial_cooldown: Duration::from_secs(30),
                max_cooldown: Duration::from_secs(300),
                max_in_flight: 1,
            },
        ));
        let worker = {
            let breaker = Arc::clone(&breaker);
            std::thread::spawn(move || breaker.search("first", 5))
        };
        std::thread::sleep(Duration::from_millis(10));
        assert!(breaker.is_available());
        let started = Instant::now();
        assert!(breaker.search("second", 5).is_empty());
        assert!(started.elapsed() >= Duration::from_millis(50));
        assert!(started.elapsed() < Duration::from_millis(250));
        worker.join().unwrap();
    }

    #[test]
    fn test_success_resets_failure_count() {
        let inner = Arc::new(FailingEngine::new(true));
        let config = CircuitBreakerConfig {
            failure_threshold: 3,
            initial_cooldown: Duration::from_secs(60),
            max_cooldown: Duration::from_secs(300),
            max_in_flight: 4,
        };
        let cb = CircuitBreakerEngine::with_config(inner.clone(), config);

        // 2 failures (below threshold)
        cb.search("test", 10);
        cb.search("test", 10);

        // 1 success — should reset counter
        inner.set_fail(false);
        cb.search("test", 10);

        // 2 more failures — still below threshold (counter was reset)
        inner.set_fail(true);
        cb.search("test", 10);
        cb.search("test", 10);

        // Circuit should still be closed (only 2 consecutive failures)
        assert!(cb.is_available());
    }
}
