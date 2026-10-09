use oi_types::traits::EconomicsEngine;
use oi_types::{Error, Result};

fn validate_percentages(first: u8, second: u8) -> Result<()> {
    if u16::from(first) + u16::from(second) > 100 {
        return Err(Error::Internal("economic percentages exceed 100".into()));
    }
    Ok(())
}

/// Base annual emission in year 0 (epoch 0): 10 million tokens.
const BASE_EMISSION: u64 = 10_000_000;

/// Seconds in one year (365 days).
const SECONDS_PER_YEAR: u64 = 365 * 24 * 3600;

/// Off-chain economics engine for the $TRUTH token.
///
/// Implements decaying emissions, fee burns, revenue sharing, and yield
/// calculations used by validators and the CLI to preview economics
/// without hitting the chain.
pub struct TruthEconomics;

impl TruthEconomics {
    pub fn new() -> Self {
        Self
    }

    /// Compute dispute payouts after a vote resolves.
    ///
    /// * `dispute_bond` — the bond the reporter posted to open the dispute.
    /// * `votes_guilty` — total voting weight that voted guilty.
    /// * `votes_innocent` — total voting weight that voted innocent.
    /// * `_slash_pct` — percentage of domain stake slashed (0–100). Slashing is
    ///   applied lazily on-chain at withdraw time (see stake.rs): of the slashed
    ///   amount, 50% is burned and 50% is retained in the vault as protocol
    ///   revenue. None of it is paid to the reporter. This function only computes
    ///   the bond payout, not the slash distribution.
    /// * `_domain_stake` — the domain's total stake (unused in payout calc).
    ///
    /// Returns `(reporter_payout, protocol_payout)` — the bond settlement only.
    ///
    /// If guilty:
    ///   - reporter receives their full bond back.
    ///   - domain is blacklisted; stakers lose `slash_pct`% on withdraw (lazy),
    ///     of which 50% is burned and 50% retained as protocol revenue.
    ///   - protocol receives nothing extra from the bond.
    ///
    /// If innocent (or tied):
    ///   - reporter's bond is forfeited to the protocol.
    ///   - reporter receives nothing.
    pub fn compute_dispute_payouts(
        &self,
        dispute_bond: u64,
        votes_guilty: u64,
        votes_innocent: u64,
        _slash_pct: u64,
        _domain_stake: u64,
    ) -> (u64, u64) {
        if votes_guilty > votes_innocent {
            // Guilty — reporter gets bond back. Slash is applied lazily on-chain.
            (dispute_bond, 0)
        } else {
            // Innocent or tied — bond forfeited to protocol.
            (0, dispute_bond)
        }
    }

    /// Compute the emission split between stakers and sentinels.
    ///
    /// On-chain, `staker_pct`% of emissions go to staker rewards (publishers/curators)
    /// and the remainder to sentinel rewards.
    ///
    /// Design note (Issue #5): Publishers and curators earn the same emission rate
    /// per token staked (shared reward index). Differentiation is via query revenue
    /// (future Model C) and the curator yield cap.
    ///
    /// Returns `(staker_portion, sentinel_portion)`.
    pub fn compute_emission_shares_with_pct(
        &self,
        emission_amount: u64,
        staker_pct: u8,
    ) -> Result<(u64, u64)> {
        validate_percentages(staker_pct, 0)?;
        let staker = (emission_amount as u128 * staker_pct as u128 / 100) as u64;
        let sentinel = emission_amount - staker;
        Ok((staker, sentinel))
    }
}

impl Default for TruthEconomics {
    fn default() -> Self {
        Self::new()
    }
}

impl EconomicsEngine for TruthEconomics {
    /// Compute token emission for a given epoch.
    ///
    /// Mirrors on-chain `AdvanceEpoch` (programs/.../processor/reward.rs):
    /// epoch 0 emits `BASE_EMISSION` (10M tokens), and each subsequent epoch
    /// applies a 1.5% geometric decay as iterated integer `* 985 / 1000`, so
    /// off-chain previews match on-chain rounding exactly. The schedule spans
    /// ~3 years with 7-day epochs and converges to 0.
    ///
    /// Design note (Issue #17): Bootstrapping inflation is expected —
    /// early epochs emit more to incentivize initial network growth.
    fn compute_emission(&self, epoch: u64, _total_staked: u64) -> u64 {
        let mut emission = BASE_EMISSION;
        for _ in 0..epoch {
            emission = emission * 985 / 1000;
            if emission == 0 {
                break;
            }
        }
        emission
    }

    /// Compute total burn from query fees.
    ///
    /// - `burn_pct`% of query fees are burned on-chain during channel settlement
    ///   (default 10).
    /// - Dispute slashes remain in the vault (not burned at settlement); they are
    ///   passed through for accounting but do not contribute to on-chain query burns.
    fn compute_burn(&self, query_fees: u64, _dispute_slashes: u64, burn_pct: u8) -> Result<u64> {
        validate_percentages(burn_pct, 0)?;
        Ok((query_fees as u128 * burn_pct as u128 / 100) as u64)
    }

    /// Split channel revenue into (publisher, curator, sentinel, burned).
    ///
    /// Mirrors on-chain `SettleChannel` (processor/channel.rs):
    /// - Burned:    `burn_pct`% of revenue (default 10)
    /// - Sentinel:  `sentinel_fee_pct`% of revenue (default 5)
    /// - Publisher: the remainder, paid to the payee (default 85%)
    /// - Curator:   always 0 — curators earn via emissions, not channel passthrough
    ///
    /// Integer-division remainder accrues to the publisher to avoid token loss,
    /// matching the on-chain `payee_amount = actual_spent - burn - sentinel_fee`.
    fn compute_revenue_shares(
        &self,
        query_revenue: u64,
        burn_pct: u8,
        sentinel_fee_pct: u8,
    ) -> Result<(u64, u64, u64, u64)> {
        validate_percentages(burn_pct, sentinel_fee_pct)?;
        let burned = (query_revenue as u128 * burn_pct as u128 / 100) as u64;
        let sentinel = (query_revenue as u128 * sentinel_fee_pct as u128 / 100) as u64;
        let publisher = query_revenue - burned - sentinel;
        let curator = 0;
        Ok((publisher, curator, sentinel, burned))
    }

    /// Compute emission split: (staker_portion, sentinel_portion).
    /// `staker_pct` — governable staker emission share (0-100, default 70).
    fn compute_emission_shares(&self, emission_amount: u64, staker_pct: u8) -> Result<(u64, u64)> {
        self.compute_emission_shares_with_pct(emission_amount, staker_pct)
    }

    /// Compute the curator yield cap using governable `cap_bps` (basis points).
    /// Default 1500 = 15.00% APY.
    ///
    /// Uses u128 intermediates to avoid overflow.
    fn compute_curator_yield_cap(
        &self,
        stake_amount: u64,
        duration_secs: u64,
        cap_bps: u64,
    ) -> Result<u64> {
        let numerator = (stake_amount as u128)
            .checked_mul(cap_bps as u128)
            .and_then(|n| n.checked_mul(duration_secs as u128))
            .ok_or_else(|| Error::Internal("yield numerator overflow".into()))?;
        let denominator = 10_000u128 * SECONDS_PER_YEAR as u128;
        u64::try_from(numerator / denominator)
            .map_err(|_| Error::Internal("yield amount overflow".into()))
    }

    /// Compute escrow yield at 5% APY.
    ///
    /// Uses u128 intermediates to avoid overflow.
    fn compute_escrow_yield(&self, channel_amount: u64, duration_secs: u64) -> Result<u64> {
        let numerator = (channel_amount as u128)
            .checked_mul(5)
            .and_then(|n| n.checked_mul(duration_secs as u128))
            .ok_or_else(|| Error::Internal("yield numerator overflow".into()))?;
        let denominator = 100u128 * SECONDS_PER_YEAR as u128;
        u64::try_from(numerator / denominator)
            .map_err(|_| Error::Internal("yield amount overflow".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> TruthEconomics {
        TruthEconomics::new()
    }

    #[test]
    fn test_invalid_allocations_fail_closed_even_for_zero_revenue() {
        let e = engine();
        for amount in [0, 1, 99, 100, u64::MAX] {
            assert!(e.compute_revenue_shares(amount, 100, 1).is_err());
            assert!(e.compute_revenue_shares(amount, 255, 255).is_err());
            assert!(e.compute_burn(amount, 0, 101).is_err());
            assert!(e.compute_emission_shares(amount, 101).is_err());
        }
    }

    #[test]
    fn test_all_valid_percentage_splits_conserve_integer_units() {
        let e = engine();
        for amount in [0, 1, 99, 100, 101, u64::MAX] {
            for burn in 0..=100 {
                for sentinel in 0..=100 - burn {
                    let (p, c, s, b) = e.compute_revenue_shares(amount, burn, sentinel).unwrap();
                    assert_eq!(
                        p as u128 + c as u128 + s as u128 + b as u128,
                        amount as u128
                    );
                }
            }
        }
    }

    #[test]
    fn test_emission_decay() {
        let e = engine();
        // 1.5% geometric decay via iterated integer `* 985 / 1000`, matching
        // on-chain AdvanceEpoch exactly.
        assert_eq!(e.compute_emission(0, 0), 10_000_000);
        assert_eq!(e.compute_emission(1, 0), 9_850_000);
        assert_eq!(e.compute_emission(2, 0), 9_702_250);
        assert_eq!(e.compute_emission(3, 0), 9_556_716);

        // ~Year 1 (epoch 52) stays healthy, near the documented ~4.557M.
        assert!((4_400_000..=4_600_000).contains(&e.compute_emission(52, 0)));

        // Monotonically decreasing.
        assert!(e.compute_emission(10, 0) < e.compute_emission(9, 0));

        // Converges to zero for large epochs (no overflow, no panic).
        assert_eq!(e.compute_emission(2000, 0), 0);
    }

    #[test]
    fn test_burn_computation() {
        let e = engine();
        // Default 10% of 1000 query fees = 100. Dispute slashes are NOT burned (lazy on-chain).
        assert_eq!(e.compute_burn(1000, 200, 10).unwrap(), 100);
        // Edge: zero inputs
        assert_eq!(e.compute_burn(0, 0, 10).unwrap(), 0);
        // Odd query fee: 999 * 10 / 100 = 99 (integer division). Slashes ignored.
        assert_eq!(e.compute_burn(999, 1, 10).unwrap(), 99);
        // Custom burn_pct
        assert_eq!(e.compute_burn(1000, 0, 30).unwrap(), 300);
    }

    #[test]
    fn test_revenue_shares() {
        let e = engine();
        // Default chain split: 10% burn, 5% sentinel, 85% payee, 0 curator.
        let (pub_share, cur_share, sentinel_share, burned) =
            e.compute_revenue_shares(1000, 10, 5).unwrap();
        assert_eq!(burned, 100);
        assert_eq!(sentinel_share, 50);
        assert_eq!(pub_share, 850);
        assert_eq!(cur_share, 0);
        // Shares always reconstruct the full revenue (no token loss).
        assert_eq!(pub_share + cur_share + sentinel_share + burned, 1000);

        // Zero revenue
        let (p, c, s, b) = e.compute_revenue_shares(0, 10, 5).unwrap();
        assert_eq!((p, c, s, b), (0, 0, 0, 0));

        // Large revenue
        let (p, c, s, b) = e.compute_revenue_shares(1_000_000, 10, 5).unwrap();
        assert_eq!(b, 100_000);
        assert_eq!(s, 50_000);
        assert_eq!(p, 850_000);
        assert_eq!(c, 0);
    }

    #[test]
    fn test_curator_yield_cap() {
        let e = engine();
        // 1M tokens staked for exactly 1 year at 1500 bps (15%) = 150,000
        let one_year = 365 * 24 * 3600u64;
        assert_eq!(
            e.compute_curator_yield_cap(1_000_000, one_year, 1500)
                .unwrap(),
            150_000
        );

        // Half a year → 75,000
        assert_eq!(
            e.compute_curator_yield_cap(1_000_000, one_year / 2, 1500)
                .unwrap(),
            75_000
        );

        // Zero duration
        assert_eq!(e.compute_curator_yield_cap(1_000_000, 0, 1500).unwrap(), 0);

        // Zero stake
        assert_eq!(e.compute_curator_yield_cap(0, one_year, 1500).unwrap(), 0);

        // Different cap: 1000 bps (10%)
        assert_eq!(
            e.compute_curator_yield_cap(1_000_000, one_year, 1000)
                .unwrap(),
            100_000
        );
    }

    #[test]
    fn test_escrow_yield() {
        let e = engine();
        // 1M tokens escrowed for 1 year → 5% = 50,000
        let one_year = 365 * 24 * 3600u64;
        assert_eq!(e.compute_escrow_yield(1_000_000, one_year).unwrap(), 50_000);

        // Half a year → 25,000
        assert_eq!(
            e.compute_escrow_yield(1_000_000, one_year / 2).unwrap(),
            25_000
        );

        // Zero
        assert_eq!(e.compute_escrow_yield(0, one_year).unwrap(), 0);
        assert_eq!(e.compute_escrow_yield(1_000_000, 0).unwrap(), 0);
    }

    #[test]
    fn test_yield_overflow_is_not_truncated_or_wrapped() {
        let e = engine();
        assert!(
            e.compute_curator_yield_cap(u64::MAX, u64::MAX, u64::MAX)
                .is_err()
        );
        assert!(
            e.compute_curator_yield_cap(u64::MAX, SECONDS_PER_YEAR * 2, 10_000)
                .is_err()
        );
        assert!(e.compute_escrow_yield(u64::MAX, u64::MAX).is_err());
        assert_eq!(
            e.compute_curator_yield_cap(u64::MAX, SECONDS_PER_YEAR, 10_000)
                .unwrap(),
            u64::MAX
        );
    }

    #[test]
    fn test_emission_shares() {
        let e = engine();
        // 70% staker, 30% sentinel (default)
        let (staker, sentinel) = e.compute_emission_shares_with_pct(10_000_000, 70).unwrap();
        assert_eq!(staker, 7_000_000);
        assert_eq!(sentinel, 3_000_000);

        // Zero emission
        let (s, n) = e.compute_emission_shares_with_pct(0, 70).unwrap();
        assert_eq!((s, n), (0, 0));

        // Odd amount — sentinel gets remainder
        let (s, n) = e.compute_emission_shares_with_pct(101, 70).unwrap();
        assert_eq!(s, 70); // 101 * 70 / 100 = 70
        assert_eq!(n, 31); // 101 - 70 = 31

        // Custom pct: 60/40
        let (s, n) = e.compute_emission_shares_with_pct(1_000, 60).unwrap();
        assert_eq!(s, 600);
        assert_eq!(n, 400);
    }

    #[test]
    fn test_dispute_payouts_guilty() {
        let e = engine();
        // Reporter posted 500 bond, guilty wins 7-3, slash 10% of 100,000 stake.
        // Reporter only gets bond back. Slash is applied lazily on-chain at withdraw.
        let (reporter, protocol) = e.compute_dispute_payouts(500, 7, 3, 10, 100_000);
        assert_eq!(reporter, 500);
        assert_eq!(protocol, 0);
    }

    #[test]
    fn test_dispute_payouts_innocent() {
        let e = engine();
        // Innocent wins 6-4, bond forfeited
        let (reporter, protocol) = e.compute_dispute_payouts(500, 4, 6, 10, 100_000);
        assert_eq!(reporter, 0);
        assert_eq!(protocol, 500);
    }

    #[test]
    fn test_dispute_payouts_tied() {
        let e = engine();
        // Tied vote: innocent side wins (guilty must be strictly greater)
        let (reporter, protocol) = e.compute_dispute_payouts(500, 5, 5, 10, 100_000);
        assert_eq!(reporter, 0);
        assert_eq!(protocol, 500);
    }
}
