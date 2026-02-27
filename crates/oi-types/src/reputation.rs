use serde::{Deserialize, Serialize};

/// Configuration for the reputation scoring system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReputationConfig {
    /// Z-score for Wilson confidence interval (1.96 = 95% CI).
    pub z_score: f64,
    /// Annual decay factor for vote age (0.9 = 10% decay per year).
    pub decay_factor: f64,
    /// Epoch duration in seconds (default: 86400 = 24 hours).
    pub epoch_duration_secs: u64,
    /// Minimum reputation score (floor).
    pub min_reputation: f64,
    /// Maximum reputation score (ceiling).
    pub max_reputation: f64,
}

impl Default for ReputationConfig {
    fn default() -> Self {
        Self {
            z_score: 1.96,
            decay_factor: 0.9,
            epoch_duration_secs: 86400,
            min_reputation: 0.1,
            max_reputation: 2.0,
        }
    }
}

/// A vote weighted by spend and time decay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeightedVote {
    pub signal_id: String,
    pub user_id: String,
    pub domain: String,
    pub useful: bool,
    pub created_at: i64,
    pub signature_verified: bool,
    pub spend_weight: f64,
    pub time_weight: f64,
    pub combined_weight: f64,
}

/// Computed reputation for a domain at a given epoch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainReputation {
    pub domain: String,
    pub wilson_score: f64,
    pub raw_positive_rate: f64,
    pub weighted_positive: f64,
    pub weighted_negative: f64,
    pub total_votes: u64,
    pub epoch: u64,
    pub computed_at: i64,
}

/// Summary of a reputation epoch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReputationEpochSummary {
    pub epoch: u64,
    pub domains_scored: u64,
    pub total_signals_processed: u64,
    pub merkle_root: String,
    pub computed_at: i64,
}

/// Merkle proof for a specific domain's reputation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MerkleProof {
    pub domain: String,
    pub leaf_hash: String,
    pub proof: Vec<MerkleProofNode>,
}

/// A single node in a Merkle proof path.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MerkleProofNode {
    pub hash: String,
    pub position: MerklePosition,
}

/// Position of a sibling in the Merkle tree.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum MerklePosition {
    Left,
    Right,
}
