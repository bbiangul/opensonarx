use oi_types::traits::EconomicsEngine;

/// Base annual emission in year 0 (epoch 0): 10 million tokens.
const BASE_EMISSION: u64 = 10_000_000;

/// Seconds in one year (365 days).
const SECONDS_PER_YEAR: u64 = 365 * 24 * 3600;

/// Off-chain economics engine for the $TRUTH token.
///
/// Implements halving emissions, fee burns, revenue sharing, and yield
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
    /// * `_slash_pct` — percentage of domain stake slashed (0–100). Slashing
    ///   is applied lazily on-chain at withdraw time; slashed tokens remain in
    ///   the vault and are NOT paid to the reporter.
    /// * `_domain_stake` — the domain's total stake (unused in payout calc).
    ///
    /// Returns `(reporter_payout, protocol_payout)`.
    ///
    /// If guilty:
    ///   - reporter receives their bond back.
    ///   - domain is blacklisted; stakers lose `slash_pct`% on withdraw (lazy).
    ///   - protocol receives nothing extra.
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
    pub fn compute_emission_shares_with_pct(&self, emission_amount: u64, staker_pct: u8) -> (u64, u64) {
        let staker = (emission_amount as u128 * staker_pct as u128 / 100) as u64;
        let sentinel = emission_amount - staker;
        (staker, sentinel)
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
    /// Year 0 (epoch 0) emits `BASE_EMISSION` (10M tokens).
    /// Each subsequent epoch halves the emission.
    /// At epoch >= 64 the emission is 0 (prevents shift overflow on u64).
    ///
    /// Design note (Issue #17): Bootstrapping inflation is expected —
    /// early epochs emit more to incentivize initial network growth.
    fn compute_emission(&self, epoch: u64, _total_staked: u64) -> u64 {
        if epoch >= 64 {
            return 0;
        }
        BASE_EMISSION >> epoch
    }

    /// Compute total burn from query fees.
    ///
    /// - `burn_pct`% of query fees are burned on-chain during channel settlement.
    /// - Dispute slashes remain in the vault (not burned); they are passed
    ///   through for accounting but do not contribute to on-chain burns.
    fn compute_burn(&self, query_fees: u64, _dispute_slashes: u64, burn_pct: u8) -> u64 {
        (query_fees as u128 * burn_pct as u128 / 100) as u64
    }

    /// Split query revenue into (publisher_share, curator_share, sentinel_share, burned).
    ///
    /// Burn is `burn_pct`% (default 45). The remainder is split:
    /// - Publisher: 35% of total
    /// - Curator:   10% of total
    /// - Sentinel:  10% of total
    /// - Burned:    burn_pct% of total
    ///
    /// When burn_pct changes, the payee portion adjusts accordingly.
    fn compute_revenue_shares(&self, query_revenue: u64, burn_pct: u8) -> (u64, u64, u64, u64) {
        let burned = (query_revenue as u128 * burn_pct as u128 / 100) as u64;
        let remainder = query_revenue - burned;
        // Split non-burn portion: ~63.6% publisher, ~18.2% curator, ~18.2% sentinel
        // of the remainder (which at 45% burn means 35/10/10 of total)
        let publisher = remainder * 35 / 55;
        let curator = remainder * 10 / 55;
        let sentinel = remainder - publisher - curator;
        (publisher, curator, sentinel, burned)
    }

    /// Compute emission split: (staker_portion, sentinel_portion).
    /// `staker_pct` — governable staker emission share (0-100, default 70).
    fn compute_emission_shares(&self, emission_amount: u64, staker_pct: u8) -> (u64, u64) {
        self.compute_emission_shares_with_pct(emission_amount, staker_pct)
    }

    /// Compute the curator yield cap using governable `cap_bps` (basis points).
    /// Default 1500 = 15.00% APY.
    ///
    /// Uses u128 intermediates to avoid overflow.
    fn compute_curator_yield_cap(&self, stake_amount: u64, duration_secs: u64, cap_bps: u64) -> u64 {
        let numerator = stake_amount as u128 * cap_bps as u128 * duration_secs as u128;
        let denominator = 10_000u128 * SECONDS_PER_YEAR as u128;
        (numerator / denominator) as u64
    }

    /// Compute escrow yield at 5% APY.
    ///
    /// Uses u128 intermediates to avoid overflow.
    fn compute_escrow_yield(&self, channel_amount: u64, duration_secs: u64) -> u64 {
        let numerator = channel_amount as u128 * 5 * duration_secs as u128;
        let denominator = 100u128 * SECONDS_PER_YEAR as u128;
        (numerator / denominator) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> TruthEconomics {
        TruthEconomics::new()
    }

    #[test]
    fn test_emission_halving() {
        let e = engine();
        assert_eq!(e.compute_emission(0, 0), 10_000_000);
        assert_eq!(e.compute_emission(1, 0), 5_000_000);
        assert_eq!(e.compute_emission(2, 0), 2_500_000);
        assert_eq!(e.compute_emission(3, 0), 1_250_000);

        // Epoch 63 should still produce a non-zero value (10M >> 63 = 0 actually,
        // but let's verify the boundary precisely).
        // 10_000_000 in binary has ~24 bits, so shifting right by 23 gives 1.
        // At epoch 23: 10_000_000 >> 23 = 1
        assert_eq!(e.compute_emission(23, 0), 1);
        // At epoch 24 and beyond (but < 64) the integer division floors to 0.
        assert_eq!(e.compute_emission(24, 0), 0);

        // Epoch 63: still goes through the shift path, just gives 0.
        assert_eq!(e.compute_emission(63, 0), 0);

        // Epoch 64: explicitly guarded to return 0 (no overflow).
        assert_eq!(e.compute_emission(64, 0), 0);
        assert_eq!(e.compute_emission(100, 0), 0);
    }

    #[test]
    fn test_burn_computation() {
        let e = engine();
        // 45% of 1000 query fees = 450. Dispute slashes are NOT burned (lazy on-chain).
        assert_eq!(e.compute_burn(1000, 200, 45), 450);
        // Edge: zero inputs
        assert_eq!(e.compute_burn(0, 0, 45), 0);
        // Odd query fee: 999 * 45 / 100 = 449 (integer division). Slashes ignored.
        assert_eq!(e.compute_burn(999, 1, 45), 449);
        // Custom burn_pct
        assert_eq!(e.compute_burn(1000, 0, 30), 300);
    }

    #[test]
    fn test_revenue_shares() {
        let e = engine();
        let (pub_share, cur_share, sentinel_share, burned) = e.compute_revenue_shares(1000, 45);
        assert_eq!(burned, 450);
        // Remainder 550 split: publisher 350, curator 100, sentinel 100
        assert_eq!(pub_share, 350);
        assert_eq!(cur_share, 100);
        assert_eq!(sentinel_share, 100);

        // Zero revenue
        let (p, c, s, b) = e.compute_revenue_shares(0, 45);
        assert_eq!((p, c, s, b), (0, 0, 0, 0));

        // Large revenue
        let (p, c, s, b) = e.compute_revenue_shares(1_000_000, 45);
        assert_eq!(b, 450_000);
        assert_eq!(p, 350_000);
        assert_eq!(c, 100_000);
        assert_eq!(s, 100_000);
    }

    #[test]
    fn test_curator_yield_cap() {
        let e = engine();
        // 1M tokens staked for exactly 1 year at 1500 bps (15%) = 150,000
        let one_year = 365 * 24 * 3600u64;
        assert_eq!(e.compute_curator_yield_cap(1_000_000, one_year, 1500), 150_000);

        // Half a year → 75,000
        assert_eq!(e.compute_curator_yield_cap(1_000_000, one_year / 2, 1500), 75_000);

        // Zero duration
        assert_eq!(e.compute_curator_yield_cap(1_000_000, 0, 1500), 0);

        // Zero stake
        assert_eq!(e.compute_curator_yield_cap(0, one_year, 1500), 0);

        // Different cap: 1000 bps (10%)
        assert_eq!(e.compute_curator_yield_cap(1_000_000, one_year, 1000), 100_000);
    }

    #[test]
    fn test_escrow_yield() {
        let e = engine();
        // 1M tokens escrowed for 1 year → 5% = 50,000
        let one_year = 365 * 24 * 3600u64;
        assert_eq!(e.compute_escrow_yield(1_000_000, one_year), 50_000);

        // Half a year → 25,000
        assert_eq!(e.compute_escrow_yield(1_000_000, one_year / 2), 25_000);

        // Zero
        assert_eq!(e.compute_escrow_yield(0, one_year), 0);
        assert_eq!(e.compute_escrow_yield(1_000_000, 0), 0);
    }

    #[test]
    fn test_emission_shares() {
        let e = engine();
        // 70% staker, 30% sentinel (default)
        let (staker, sentinel) = e.compute_emission_shares_with_pct(10_000_000, 70);
        assert_eq!(staker, 7_000_000);
        assert_eq!(sentinel, 3_000_000);

        // Zero emission
        let (s, n) = e.compute_emission_shares_with_pct(0, 70);
        assert_eq!((s, n), (0, 0));

        // Odd amount — sentinel gets remainder
        let (s, n) = e.compute_emission_shares_with_pct(101, 70);
        assert_eq!(s, 70); // 101 * 70 / 100 = 70
        assert_eq!(n, 31); // 101 - 70 = 31

        // Custom pct: 60/40
        let (s, n) = e.compute_emission_shares_with_pct(1_000, 60);
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
