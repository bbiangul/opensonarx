use serde::{Deserialize, Serialize};

/// Token emission schedule parameters.
/// Year 1: 10M tokens, halving each year.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmissionSchedule {
    pub epoch: u64,
    pub total_staked: u64,
    pub emission_amount: u64,
}

/// Record of tokens burned from query fees.
/// Dispute slashes remain in the vault (not burned); slashing is applied
/// lazily at withdraw time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BurnEvent {
    pub query_fees_burned: u64,
    pub total_burned: u64,
}

/// Revenue share breakdown for an epoch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevenueShare {
    pub total_revenue: u64,
    pub publisher_share: u64,  // 35%
    pub curator_share: u64,    // 10%
    pub sentinel_share: u64,   // 10%
    pub burned: u64,           // 45%
}

/// Curator yield cap computation result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CuratorYieldCap {
    pub stake_amount: u64,
    pub duration_secs: u64,
    pub max_yield: u64,
    pub apy_pct: f64,
}

/// Escrow yield computation result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscrowYield {
    pub channel_amount: u64,
    pub duration_secs: u64,
    pub yield_amount: u64,
}

/// Governable economic parameters for off-chain use.
/// Mirrors the on-chain ProtocolConfig fields that affect economics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernableParams {
    /// Burn percentage for channel settlement (0-100, default 45)
    pub burn_pct: u8,
    /// Staker share of emissions (0-100, default 70)
    pub staker_emission_pct: u8,
    /// Curator yield cap in basis points (default 1500 = 15.00% APY)
    pub curator_yield_cap_bps: u64,
    /// Hard cap on total minted $TRUTH (u64::MAX = uncapped)
    pub max_total_supply: u64,
    /// Minimum epoch duration in seconds (0 = no constraint)
    pub min_epoch_duration: i64,
    /// Minimum total jury weight for dispute quorum (0 = no quorum)
    pub min_jury_weight: u64,
    /// Governance timelock delay after voting ends (seconds)
    pub execution_delay_secs: i64,
    /// $TRUTH deposit required to create a proposal (0 = free)
    pub proposal_deposit: u64,
}

impl Default for GovernableParams {
    fn default() -> Self {
        Self {
            burn_pct: 45,
            staker_emission_pct: 70,
            curator_yield_cap_bps: 1500,
            max_total_supply: u64::MAX,
            min_epoch_duration: 0,
            min_jury_weight: 0,
            execution_delay_secs: 0,
            proposal_deposit: 0,
        }
    }
}

/// DAO treasury allocation for subsidizing free-tier web2 users.
/// The DAO votes on `budget_truth_per_epoch` — this is how much $TRUTH
/// the whale wallet may spend per epoch on behalf of free-tier users.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FreeTierBudget {
    pub epoch: u64,
    /// Total $TRUTH allocated by DAO governance for this epoch
    pub budget_truth_per_epoch: u64,
    /// Amount already spent (sum of all free-tier query tickets)
    pub spent_truth: u64,
    /// Number of free-tier users active this epoch
    pub active_users: u32,
    /// Cost per query in lamports (dynamically adjusted)
    pub cost_per_query_lamports: u64,
}

impl FreeTierBudget {
    /// Remaining budget for this epoch.
    pub fn remaining(&self) -> u64 {
        self.budget_truth_per_epoch.saturating_sub(self.spent_truth)
    }

    /// Whether the free tier is still funded.
    pub fn is_funded(&self) -> bool {
        self.remaining() >= self.cost_per_query_lamports
    }
}

/// Tracks a single free-tier query charge against the DAO whale wallet.
/// These are the "invisible" $TRUTH payments that happen when a web2
/// user searches via the gateway.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubsidizedQuery {
    pub user_id: String,
    pub query_hash: String,
    /// Lamports charged from whale wallet for this query
    pub cost_lamports: u64,
    pub timestamp: i64,
    /// Node that served the query (payee of the state channel ticket)
    pub node_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_free_tier_budget_remaining() {
        let budget = FreeTierBudget {
            epoch: 1,
            budget_truth_per_epoch: 1_000_000,
            spent_truth: 400_000,
            active_users: 50,
            cost_per_query_lamports: 100,
        };
        assert_eq!(budget.remaining(), 600_000);
        assert!(budget.is_funded());
    }

    #[test]
    fn test_free_tier_budget_exhausted() {
        let budget = FreeTierBudget {
            epoch: 1,
            budget_truth_per_epoch: 1_000,
            spent_truth: 999,
            active_users: 10,
            cost_per_query_lamports: 100,
        };
        assert_eq!(budget.remaining(), 1);
        assert!(!budget.is_funded());
    }

    #[test]
    fn test_free_tier_budget_exact_boundary() {
        let budget = FreeTierBudget {
            epoch: 1,
            budget_truth_per_epoch: 1_000,
            spent_truth: 900,
            active_users: 10,
            cost_per_query_lamports: 100,
        };
        assert_eq!(budget.remaining(), 100);
        assert!(budget.is_funded()); // exactly one query left — should be funded
    }

    #[test]
    fn test_free_tier_budget_saturating() {
        let budget = FreeTierBudget {
            epoch: 1,
            budget_truth_per_epoch: 100,
            spent_truth: 200,
            active_users: 1,
            cost_per_query_lamports: 10,
        };
        assert_eq!(budget.remaining(), 0);
        assert!(!budget.is_funded());
    }
}
