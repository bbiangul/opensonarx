use crate::SignedTicket;
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};

/// Maximum number of (payer, nonce) pairs to track for replay protection.
/// At ~80 bytes per entry, 1M entries ≈ 80 MB — bounded and predictable.
const MAX_SEEN_NONCES: usize = 1_000_000;

/// Thread-safe earnings accumulator for node operators.
pub struct EarningsTracker {
    total_earned_lamports: AtomicU64,
    queries_served: AtomicU64,
    pending_tickets: RwLock<Vec<SignedTicket>>,
    /// Bounded replay detection: tracks (payer_pubkey, nonce) pairs.
    /// When full, the oldest entries are evicted (FIFO).
    seen_nonces: RwLock<NonceSet>,
}

/// Capacity-bounded set with FIFO eviction for replay detection.
struct NonceSet {
    set: HashSet<(String, u64)>,
    order: VecDeque<(String, u64)>,
    capacity: usize,
}

impl NonceSet {
    fn new(capacity: usize) -> Self {
        Self {
            set: HashSet::with_capacity(capacity.min(8192)),
            order: VecDeque::with_capacity(capacity.min(8192)),
            capacity,
        }
    }

    /// Insert a (payer, nonce) pair. Returns `true` if newly inserted (not a replay).
    fn insert(&mut self, key: (String, u64)) -> bool {
        if !self.set.insert(key.clone()) {
            return false; // Already seen — replay
        }

        self.order.push_back(key);

        // Evict oldest entries if over capacity
        while self.order.len() > self.capacity {
            if let Some(old) = self.order.pop_front() {
                self.set.remove(&old);
            }
        }

        true
    }
}

impl EarningsTracker {
    pub fn new() -> Self {
        Self {
            total_earned_lamports: AtomicU64::new(0),
            queries_served: AtomicU64::new(0),
            pending_tickets: RwLock::new(Vec::new()),
            seen_nonces: RwLock::new(NonceSet::new(MAX_SEEN_NONCES)),
        }
    }

    /// Check if a ticket nonce has already been seen (replay detection).
    /// Returns `true` if this is the first time seeing this (payer, nonce) pair.
    /// Evicts oldest entries when the set exceeds capacity.
    pub fn check_and_record_nonce(&self, payer_pubkey: &str, nonce: u64) -> bool {
        let mut seen = self.seen_nonces.write().unwrap();
        seen.insert((payer_pubkey.to_string(), nonce))
    }

    /// Record a ticket from a query payment.
    pub fn record_ticket(&self, ticket: SignedTicket) {
        self.total_earned_lamports
            .fetch_add(ticket.amount_lamports, Ordering::Relaxed);
        self.queries_served.fetch_add(1, Ordering::Relaxed);
        self.pending_tickets.write().unwrap().push(ticket);
    }

    /// Record a query served without a ticket (e.g., fanout from entry node).
    /// Increments `queries_served` for emission credit without recording phantom earnings.
    pub fn record_query_served(&self) {
        self.queries_served.fetch_add(1, Ordering::Relaxed);
    }

    /// Total lamports earned since tracker creation.
    pub fn total_earned(&self) -> u64 {
        self.total_earned_lamports.load(Ordering::Relaxed)
    }

    /// Total queries served since tracker creation.
    pub fn queries_served(&self) -> u64 {
        self.queries_served.load(Ordering::Relaxed)
    }

    /// Drain all pending tickets (e.g., for batch settlement).
    pub fn drain_pending(&self) -> Vec<SignedTicket> {
        let mut tickets = self.pending_tickets.write().unwrap();
        std::mem::take(&mut *tickets)
    }

    /// Create a serializable snapshot of current earnings.
    pub fn snapshot(&self) -> EarningsSnapshot {
        let pending = self.pending_tickets.read().unwrap();
        EarningsSnapshot {
            total_earned_lamports: self.total_earned(),
            queries_served: self.queries_served(),
            pending_ticket_count: pending.len() as u64,
            pending_value_lamports: pending.iter().map(|t| t.amount_lamports).sum(),
        }
    }
}

impl Default for EarningsTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Serializable snapshot of earnings for display/persistence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EarningsSnapshot {
    pub total_earned_lamports: u64,
    pub queries_served: u64,
    pub pending_ticket_count: u64,
    pub pending_value_lamports: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ticket(amount: u64) -> SignedTicket {
        SignedTicket {
            payer_pubkey: "payer123".to_string(),
            payee_pubkey: "payee456".to_string(),
            amount_lamports: amount,
            nonce: 1,
            signature: vec![0u8; 64],
            expiry_slot: 1000,
        }
    }

    #[test]
    fn test_record_ticket() {
        let tracker = EarningsTracker::new();
        tracker.record_ticket(make_ticket(1000));
        tracker.record_ticket(make_ticket(2000));

        assert_eq!(tracker.total_earned(), 3000);
        assert_eq!(tracker.queries_served(), 2);
    }

    #[test]
    fn test_drain_pending() {
        let tracker = EarningsTracker::new();
        tracker.record_ticket(make_ticket(500));
        tracker.record_ticket(make_ticket(700));

        let drained = tracker.drain_pending();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].amount_lamports, 500);
        assert_eq!(drained[1].amount_lamports, 700);

        // Pending should be empty after drain
        let drained2 = tracker.drain_pending();
        assert!(drained2.is_empty());

        // But totals should be preserved
        assert_eq!(tracker.total_earned(), 1200);
        assert_eq!(tracker.queries_served(), 2);
    }

    #[test]
    fn test_snapshot() {
        let tracker = EarningsTracker::new();
        tracker.record_ticket(make_ticket(1000));
        tracker.record_ticket(make_ticket(2000));

        let snap = tracker.snapshot();
        assert_eq!(snap.total_earned_lamports, 3000);
        assert_eq!(snap.queries_served, 2);
        assert_eq!(snap.pending_ticket_count, 2);
        assert_eq!(snap.pending_value_lamports, 3000);
    }

    #[test]
    fn test_empty_tracker() {
        let tracker = EarningsTracker::new();
        assert_eq!(tracker.total_earned(), 0);
        assert_eq!(tracker.queries_served(), 0);

        let snap = tracker.snapshot();
        assert_eq!(snap.total_earned_lamports, 0);
        assert_eq!(snap.pending_ticket_count, 0);
    }

    #[test]
    fn test_record_query_served_without_ticket() {
        let tracker = EarningsTracker::new();
        tracker.record_query_served();
        tracker.record_query_served();
        tracker.record_query_served();

        assert_eq!(tracker.queries_served(), 3);
        assert_eq!(tracker.total_earned(), 0);
        assert!(tracker.drain_pending().is_empty());
    }

    #[test]
    fn test_mixed_ticket_and_ticketless_queries() {
        let tracker = EarningsTracker::new();
        tracker.record_ticket(make_ticket(1000));
        tracker.record_query_served();
        tracker.record_query_served();
        tracker.record_ticket(make_ticket(2000));
        tracker.record_query_served();

        // 2 tickets + 3 ticketless = 5 queries served
        assert_eq!(tracker.queries_served(), 5);
        // Only ticket earnings are recorded
        assert_eq!(tracker.total_earned(), 3000);
        // Only 2 pending tickets
        assert_eq!(tracker.drain_pending().len(), 2);
    }

    #[test]
    fn test_nonce_replay_detection() {
        let tracker = EarningsTracker::new();
        // First time seeing this nonce → accepted
        assert!(tracker.check_and_record_nonce("payer_a", 0));
        // Same nonce from same payer → rejected (replay)
        assert!(!tracker.check_and_record_nonce("payer_a", 0));
        // Same nonce from different payer → accepted
        assert!(tracker.check_and_record_nonce("payer_b", 0));
        // Different nonce from same payer → accepted
        assert!(tracker.check_and_record_nonce("payer_a", 1));
    }

    #[test]
    fn test_nonce_set_eviction() {
        let mut nonce_set = NonceSet::new(3);

        assert!(nonce_set.insert(("a".to_string(), 0)));
        assert!(nonce_set.insert(("b".to_string(), 1)));
        assert!(nonce_set.insert(("c".to_string(), 2)));

        // Set is full (capacity=3). Inserting a 4th evicts the oldest ("a", 0).
        assert!(nonce_set.insert(("d".to_string(), 3)));
        assert_eq!(nonce_set.set.len(), 3);

        // ("a", 0) was evicted, so inserting it again should succeed
        assert!(nonce_set.insert(("a".to_string(), 0)));
        // ("b", 1) was evicted by the above insert
        assert!(nonce_set.insert(("b".to_string(), 1)));
    }

    #[test]
    fn test_nonce_set_duplicate_before_eviction() {
        let mut nonce_set = NonceSet::new(10);
        assert!(nonce_set.insert(("x".to_string(), 5)));
        assert!(!nonce_set.insert(("x".to_string(), 5))); // Duplicate → false
        assert_eq!(nonce_set.set.len(), 1);
    }
}
