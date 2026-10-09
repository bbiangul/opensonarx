//! Local, integer-only economic controls for the proposed stable-unit pilot.
//!
//! All amounts use ONE configured asset's smallest unit (examples use USD
//! millionths). This is not a wallet, durable ledger, oracle, authorization
//! service or deployed contract. Callers must authenticate funding, identities
//! and resolution before invoking these operations. No token bonus, burn,
//! consumer indemnity or cash-confidence effect is implicitly included.

use oi_types::{Error, Result};
use std::collections::HashMap;

fn invalid(message: &str) -> Error {
    Error::Internal(message.into())
}

fn ceil_div(n: u128, d: u128) -> Result<u64> {
    if d == 0 {
        return Err(invalid("no viable denominator"));
    }
    let value = n / d + u128::from(n % d != 0);
    u64::try_from(value).map_err(|_| invalid("amount overflow"))
}

/// Minimum billed-event price, not a demand or cost forecast. Every attempt
/// costs `variable_per_attempt`; fixed costs cover the same volume/window.
/// Processing fees apply to gross billing; margin to revenue after refunds.
/// No burn or subsidy may be used to manufacture a viable quote.
pub fn minimum_service_price(
    variable_per_attempt: u64,
    fixed_per_window: u64,
    attempts_per_window: u64,
    billable_bps: u16,
    refund_bps: u16,
    processing_bps: u16,
    margin_bps: u16,
) -> Result<u64> {
    if attempts_per_window == 0
        || billable_bps == 0
        || [billable_bps, refund_bps, processing_bps, margin_bps]
            .iter()
            .any(|&bps| bps > 10_000)
    {
        return Err(invalid("invalid volume or basis points"));
    }
    let retained = u128::from(10_000 - refund_bps) * u128::from(10_000 - margin_bps);
    let fee = u128::from(processing_bps) * 10_000;
    let contribution = retained
        .checked_sub(fee)
        .filter(|&n| n > 0)
        .ok_or_else(|| invalid("fees/margin consume all revenue"))?;
    let cost = u128::from(variable_per_attempt) * u128::from(attempts_per_window)
        + u128::from(fixed_per_window);
    let numerator = cost
        .checked_mul(1_000_000_000_000)
        .ok_or_else(|| invalid("cost overflow"))?;
    let denominator = u128::from(attempts_per_window)
        .checked_mul(u128::from(billable_bps))
        .and_then(|n| n.checked_mul(contribution))
        .ok_or_else(|| invalid("volume overflow"))?;
    ceil_div(numerator, denominator)
}

/// Fail closed when a participant payout cannot cover declared costs plus a
/// positive surplus. Subsidies/token appreciation are deliberately not inputs.
pub fn validate_participant_payment(
    payment: u64,
    operating: u64,
    capital: u64,
    expected_loss: u64,
    minimum_surplus: u64,
) -> Result<()> {
    let minimum = [operating, capital, expected_loss, minimum_surplus]
        .into_iter()
        .try_fold(0u64, |n, c| n.checked_add(c))
        .ok_or_else(|| invalid("cost overflow"))?;
    if minimum_surplus == 0 || payment < minimum {
        return Err(invalid("participant payout is not sustainable"));
    }
    Ok(())
}

/// Expected net per challenge in asset units, rounded down. Precision is
/// conditional on resolution. These are explicit scenario inputs, not an
/// oracle forecast; production must supply independently measured estimates.
pub fn challenger_net(
    bounty: u64,
    challenge: u64,
    fee: u64,
    investigation: u64,
    capital_cost: u64,
    precision_bps: u16,
    unresolved_bps: u16,
) -> Result<i128> {
    if precision_bps > 10_000 || unresolved_bps > 10_000 {
        return Err(invalid("invalid challenge probabilities"));
    }
    let reward = i128::from(precision_bps) * i128::from(bounty)
        - i128::from(10_000 - precision_bps) * i128::from(challenge);
    let net = reward * i128::from(10_000 - unresolved_bps);
    Ok(net.div_euclid(100_000_000)
        - i128::from(fee)
        - i128::from(investigation)
        - i128::from(capital_cost))
}

/// Strictly reject rewards when a modeled coalition can break even or profit.
/// Reserve recoverability and ALL realizable bonuses must be included. This
/// does not establish that ownership, external hedges or benefits were found.
pub fn validate_coalition_budget(
    bond: u64,
    bounty: u64,
    recoverable_reserve_bps: u16,
    all_bonus: u64,
    external_cost: u64,
) -> Result<()> {
    if bounty > bond || recoverable_reserve_bps > 10_000 {
        return Err(invalid("invalid coalition assumptions"));
    }
    let leakage = u128::from(bond - bounty) * u128::from(10_000 - recoverable_reserve_bps);
    let cost = u128::from(external_cost) * 10_000;
    if u128::from(all_bonus) * 10_000 >= leakage + cost {
        return Err(invalid("coalition can break even or profit"));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Upheld,
    Rejected,
    Unresolved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisputePayout {
    pub author: u64,
    pub challenger: u64,
    pub verifier: u64,
    pub restricted_reserve: u64,
}

/// Distribution of already-funded author bond + challenge bond + separate fee.
/// No arbitrator supplied by this helper; an outcome is not proof of truth.
pub fn dispute_payout(
    bond: u64,
    challenge: u64,
    fee: u64,
    bounty: u64,
    outcome: Outcome,
) -> Result<DisputePayout> {
    if bond == 0 || challenge == 0 || bounty > bond || fee > challenge {
        return Err(invalid("invalid dispute funding"));
    }
    bond.checked_add(challenge)
        .and_then(|n| n.checked_add(fee))
        .ok_or_else(|| invalid("escrow overflow"))?;
    Ok(match outcome {
        Outcome::Upheld => DisputePayout {
            author: 0,
            challenger: challenge + bounty,
            verifier: fee,
            restricted_reserve: bond - bounty,
        },
        Outcome::Rejected => DisputePayout {
            author: bond + fee,
            challenger: 0,
            verifier: fee,
            restricted_reserve: challenge - fee,
        },
        Outcome::Unresolved => DisputePayout {
            author: bond,
            challenger: challenge,
            verifier: fee,
            restricted_reserve: 0,
        },
    })
}

#[derive(Debug)]
struct Position {
    cluster: String,
    bond: u64,
    closed: bool,
}

/// In-memory allocation of author collateral. Mutable ownership serializes
/// transitions; production needs durable atomic storage and authenticated
/// funding/cluster attribution. Closed IDs stay consumed to prevent replay.
pub struct CollateralBook {
    available: u64,
    locked: u64,
    forfeited: u64,
    cluster_limit: u64,
    clusters: HashMap<String, u64>,
    positions: HashMap<String, Position>,
}

impl CollateralBook {
    pub fn new(funded_budget: u64, cluster_limit: u64) -> Result<Self> {
        if funded_budget == 0 || cluster_limit == 0 || cluster_limit > funded_budget {
            return Err(invalid("invalid funded exposure limits"));
        }
        Ok(Self {
            available: funded_budget,
            locked: 0,
            forfeited: 0,
            cluster_limit,
            clusters: HashMap::new(),
            positions: HashMap::new(),
        })
    }

    pub fn balances(&self) -> (u64, u64, u64) {
        (self.available, self.locked, self.forfeited)
    }

    pub fn admit(&mut self, id: &str, cluster: &str, bond: u64) -> Result<()> {
        if id.trim().is_empty()
            || cluster.trim().is_empty()
            || bond == 0
            || self.positions.contains_key(id)
            || bond > self.available
        {
            return Err(invalid("invalid, duplicate or unfunded commitment"));
        }
        let exposure = self.clusters.get(cluster).copied().unwrap_or(0);
        let next = exposure
            .checked_add(bond)
            .filter(|&n| n <= self.cluster_limit)
            .ok_or_else(|| invalid("cluster exposure limit"))?;
        // No mutations until every precondition passed.
        self.available -= bond;
        self.locked += bond;
        self.clusters.insert(cluster.into(), next);
        self.positions.insert(
            id.into(),
            Position {
                cluster: cluster.into(),
                bond,
                closed: false,
            },
        );
        Ok(())
    }

    /// Author collateral only. Other dispute inputs must be escrowed separately.
    /// Forfeited funds are NOT added back to available capital or consumer funds.
    pub fn close(&mut self, id: &str, outcome: Outcome) -> Result<()> {
        let position = self
            .positions
            .get_mut(id)
            .ok_or_else(|| invalid("unknown commitment"))?;
        if position.closed {
            return Err(invalid("commitment already closed"));
        }
        if outcome == Outcome::Upheld {
            self.forfeited += position.bond;
        } else {
            self.available += position.bond;
        }
        self.locked -= position.bond;
        *self
            .clusters
            .get_mut(&position.cluster)
            .expect("admitted cluster") -= position.bond;
        position.closed = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_price_covers_volume_billability_and_fees() {
        assert_eq!(
            minimum_service_price(10_700, 10_000_000_000, 1_000_000, 10_000, 0, 0, 3_000).unwrap(),
            29_572
        );
        assert_eq!(
            minimum_service_price(10_700, 10_000_000_000, 1_000_000, 5_000, 0, 0, 3_000).unwrap(),
            59_143
        );
        assert!(
            minimum_service_price(10_700, 10_000_000_000, 10_000, 10_000, 0, 300, 3_000).unwrap()
                > 1_000_000
        );
    }

    #[test]
    fn quote_invalid_and_overflow_are_errors() {
        for bps in [0, 10_001, u16::MAX] {
            assert!(minimum_service_price(1, 1, 1, bps, 0, 0, 0).is_err());
        }
        assert!(minimum_service_price(1, 1, 0, 10_000, 0, 0, 0).is_err());
        assert!(minimum_service_price(1, 1, 1, 10_000, 10_000, 0, 0).is_err());
        assert!(minimum_service_price(u64::MAX, 1, u64::MAX, 10_000, 0, 0, 0).is_err());
    }

    #[test]
    fn rewards_do_not_substitute_for_net_surplus() {
        assert!(validate_participant_payment(2_000, 3_000, 0, 0, 1).is_err());
        assert!(validate_participant_payment(2_000, 1_000, 100, 100, 100).is_ok());
        assert!(validate_participant_payment(1, u64::MAX, 1, 0, 1).is_err());
    }

    #[test]
    fn ambiguity_capital_and_rewards_are_not_ignored() {
        assert_eq!(challenger_net(30, 10, 2, 10, 0, 9_000, 0).unwrap(), 14);
        assert_eq!(challenger_net(30, 10, 2, 10, 0, 9_000, 5_000).unwrap(), 1);
        assert_eq!(challenger_net(30, 10, 2, 10, 0, 9_000, 7_500).unwrap(), -6);
        assert!(challenger_net(30, 10, 2, 10, 2, 9_000, 5_000).unwrap() < 0);
        assert!(challenger_net(30, 10, 2, 10, 0, 10_001, 0).is_err());
        assert!(validate_coalition_budget(100, 30, 0, 0, 10).is_ok());
        assert!(validate_coalition_budget(100, 30, 0, 85, 10).is_err());
        assert!(validate_coalition_budget(100, 30, 8_000, 25, 10).is_err());
        assert!(validate_coalition_budget(100, 30, 10_000, 0, 0).is_err());
    }

    #[test]
    fn failed_transitions_preserve_balances() {
        let mut book = CollateralBook::new(100, 50).unwrap();
        for (id, cluster, bond) in [
            ("", "c", 1),
            ("a", "", 1),
            ("a", "c", 0),
            ("a", "c", 51),
            ("a", "c", u64::MAX),
        ] {
            assert!(book.admit(id, cluster, bond).is_err());
            assert_eq!(book.balances(), (100, 0, 0));
        }
        assert!(book.close("missing", Outcome::Upheld).is_err());
        assert_eq!(book.balances(), (100, 0, 0));
    }

    #[test]
    fn settlement_conserves_small_and_extreme_values() {
        for (bond, challenge, fee, bounty) in
            [(100, 10, 2, 30), (1, 1, 0, 1), (u64::MAX - 3, 2, 1, 50)]
        {
            for outcome in [Outcome::Upheld, Outcome::Rejected, Outcome::Unresolved] {
                let p = dispute_payout(bond, challenge, fee, bounty, outcome).unwrap();
                assert_eq!(
                    p.author + p.challenger + p.verifier + p.restricted_reserve,
                    bond + challenge + fee
                );
            }
        }
        assert!(dispute_payout(u64::MAX, 1, 0, 1, Outcome::Upheld).is_err());
        assert!(dispute_payout(100, 10, 2, 101, Outcome::Upheld).is_err());
    }

    #[test]
    fn workflow_reserves_refunds_and_forfeits_without_replay() {
        let mut book = CollateralBook::new(200, 100).unwrap();
        book.admit("a", "source-1", 100).unwrap();
        assert!(book.admit("b", "source-1", 1).is_err());
        book.admit("b", "source-2", 100).unwrap();
        assert!(book.admit("c", "source-3", 1).is_err());
        book.close("a", Outcome::Unresolved).unwrap();
        assert!(book.admit("a", "source-1", 100).is_err());
        assert!(book.close("a", Outcome::Rejected).is_err());
        book.close("b", Outcome::Upheld).unwrap();
        assert_eq!(book.balances(), (100, 0, 100));
        assert!(book.admit("c", "source-3", 101).is_err());
        assert_eq!(book.balances(), (100, 0, 100));
    }
}
