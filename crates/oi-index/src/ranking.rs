// Ranking formula (stake-neutral):
//   base_relevance * agent_utility_boost * quality_mult * freshness_mult * dns_mult
//   base_relevance = semantic_weight * semantic + bm25_weight * bm25
//
// Stake and reputation deliberately do NOT enter the ranking score — capital
// must not be able to buy rank. They feed `compute_verdict` (an auditable trust
// label callers filter on via `QueryFilters.min_trust`) instead. Relevance
// decides order; quality, freshness, and DNS verification modulate it.

/// Ordinal rank of a trust verdict, used for `min_trust` filtering.
/// Higher = more trusted; "disputed"/unknown sort below everything.
pub fn trust_rank(verdict: &str) -> u8 {
    match verdict {
        "highly_trusted" => 4,
        "trusted" => 3,
        "neutral" => 2,
        "unverified" => 1,
        _ => 0, // "disputed" or unknown
    }
}

/// Quadratic relevance gate (retained utility; no longer wired into scoring).
///
/// Returns 0.0 when base relevance is at or below 0.6 (stake gets no boost),
/// ramps quadratically to 1.0 at 0.85 (full stake boost).
/// The quadratic curve crushes whale influence at low relevance while being
/// generous to genuinely relevant content. No threshold tuning to specific
/// queries — the shape handles variance in score distributions.
///
/// Key points on the curve:
/// - 0.60: gate=0.00 (zero boost — clearly irrelevant)
/// - 0.65: gate=0.04 (4% — negligible even for whales)
/// - 0.70: gate=0.16 (16%)
/// - 0.75: gate=0.36 (36%)
/// - 0.80: gate=0.64 (64%)
/// - 0.85: gate=1.00 (full boost — clearly relevant)
pub fn relevance_gate(base_score: f64) -> f64 {
    const LOWER: f64 = 0.6;
    const UPPER: f64 = 0.85;
    let t = ((base_score - LOWER) / (UPPER - LOWER)).clamp(0.0, 1.0);
    t * t // quadratic: steep at bottom, gentle at top
}

#[derive(Clone)]
pub struct RankingParams {
    pub semantic_weight: f64,
    pub bm25_weight: f64,
    /// Entity-specific stake in USD (direct entity-level stake)
    pub entity_stake_usd: f64,
    /// Parent domain stake in USD
    pub domain_stake_usd: f64,
    /// Whether the domain is a UGC platform (zero passthrough if true)
    pub is_ugc_platform: bool,
    pub reputation: f64,
    /// Number of feedback votes backing the reputation score
    pub reputation_votes: u32,
    pub agent_utility_boost: f64,
    /// Content quality score (0.0-1.0). None = no data = treated as 0.7 (neutral).
    pub quality_score: Option<f64>,
    /// Spam score (0.0-1.0). None = no data = treated as 0.0 (clean).
    pub spam_score: Option<f64>,
    /// Slop score (0.0-1.0). None = no data = treated as 0.0 (clean).
    pub slop_score: Option<f64>,
    /// Whether domain has verified DNS. None = unknown.
    pub dns_verified: Option<bool>,
    /// Content publish timestamp (Unix seconds). None = unknown.
    pub published_at: Option<i64>,
    /// When the stake was created (Unix seconds). None = unknown.
    pub staked_at: Option<i64>,
}

impl Default for RankingParams {
    fn default() -> Self {
        Self {
            semantic_weight: 1.0,
            bm25_weight: 0.0,
            entity_stake_usd: 0.0,
            domain_stake_usd: 0.0,
            is_ugc_platform: false,
            reputation: 1.0,
            reputation_votes: 0,
            agent_utility_boost: 1.0,
            quality_score: None,
            spam_score: None,
            slop_score: None,
            dns_verified: None,
            published_at: None,
            staked_at: None,
        }
    }
}

/// Compute a human/machine-readable trust verdict from multiple signals.
pub fn compute_verdict(
    stake_usd: f64,
    reputation: f64,
    reputation_votes: u32,
    disputed: bool,
    dns_verified: bool,
) -> &'static str {
    if disputed {
        return "disputed";
    }
    if stake_usd >= 10_000.0 && reputation >= 1.8 && reputation_votes >= 50 && dns_verified {
        return "highly_trusted";
    }
    // "trusted" requires DNS-proven domain ownership: without it, anyone can
    // bond capital to a name they don't control and buy the label.
    if stake_usd >= 1_000.0 && reputation >= 1.3 && reputation_votes >= 10 && dns_verified {
        return "trusted";
    }
    if stake_usd < 1.0 && reputation_votes < 5 {
        return "unverified";
    }
    "neutral"
}

pub fn compute_score(semantic_score: f64, bm25_score: f64, params: &RankingParams) -> f64 {
    let base = params.semantic_weight * semantic_score + params.bm25_weight * bm25_score;

    // Content quality multiplier: high quality → up to 1.2x, low quality → down to 0.6x.
    // Clamp inputs to [0, 1] to prevent gaming via out-of-range values.
    let quality = params.quality_score.unwrap_or(0.7).clamp(0.0, 1.0);
    let spam = params.spam_score.unwrap_or(0.0).clamp(0.0, 1.0);
    let slop = params.slop_score.unwrap_or(0.0).clamp(0.0, 1.0);
    let quality_mult = (0.6 + 0.6 * quality) * (1.0 - 0.3 * spam) * (1.0 - 0.2 * slop);

    // Smooth freshness decay: exponential curve instead of cliff bands.
    // Day 0 → +15%, Day 7 → +12%, Day 30 → +5.5%, Day 90 → +0.7%, Day 365 → ~0%.
    let freshness_mult = if let Some(pub_ts) = params.published_at {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let age_days = ((now - pub_ts) as f64 / 86400.0).max(0.0);
        1.0 + 0.15 * (-age_days / 30.0).exp()
    } else {
        1.0 // unknown = neutral
    };

    // DNS verification signal
    let dns_mult = match params.dns_verified {
        Some(true) => 1.05,  // 5% boost for verified domains
        Some(false) => 0.95, // 5% penalty for known-unverified
        None => 1.0,         // unknown = neutral
    };

    // Stake and reputation are intentionally NOT factored in here. They are
    // trust signals (see `compute_verdict` / `QueryFilters.min_trust`), not
    // ranking signals — capital must not buy rank. Relevance decides order.
    base * params.agent_utility_boost * quality_mult * freshness_mult * dns_mult
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Neutral quality multiplier when no quality data is provided (all None):
    /// quality=0.7, spam=0.0, slop=0.0 → (0.6 + 0.6*0.7) * 1.0 * 1.0 = 1.02
    const NEUTRAL_Q: f64 = 1.02;

    /// Helper: params with quality signals explicitly neutral (quality_mult = 1.0).
    fn neutral_quality_params() -> RankingParams {
        RankingParams {
            quality_score: Some(2.0 / 3.0), // (0.6 + 0.6*2/3) = 1.0
            spam_score: Some(0.0),
            slop_score: Some(0.0),
            ..Default::default()
        }
    }

    // ── Base relevance ──────────────────────────────────────────────────

    #[test]
    fn test_base_relevance_only() {
        let params = neutral_quality_params();
        let score = compute_score(0.8, 0.6, &params);
        let expected = 1.0 * 0.8 + 0.0 * 0.6; // semantic-only weights by default
        assert!((score - expected).abs() < 1e-10);
    }

    #[test]
    fn test_weighted_base() {
        let params = RankingParams {
            semantic_weight: 0.5,
            bm25_weight: 0.5,
            ..neutral_quality_params()
        };
        let score = compute_score(0.9, 0.7, &params);
        let expected = 0.5 * 0.9 + 0.5 * 0.7; // 0.8
        assert!((score - expected).abs() < 1e-10);
    }

    // ── Stake / reputation are NOT ranking signals ──────────────────────

    #[test]
    fn test_stake_never_affects_score() {
        let baseline = compute_score(0.8, 0.0, &neutral_quality_params());
        for stake in [1.0, 100.0, 1_024.0, 50_000.0, 1_000_000_000.0] {
            let p = RankingParams { entity_stake_usd: stake, ..neutral_quality_params() };
            assert!((compute_score(0.8, 0.0, &p) - baseline).abs() < 1e-12,
                "entity stake ${stake} must not change the score");
            let p = RankingParams { domain_stake_usd: stake, is_ugc_platform: false, ..neutral_quality_params() };
            assert!((compute_score(0.8, 0.0, &p) - baseline).abs() < 1e-12,
                "publisher domain stake ${stake} must not change the score");
            let p = RankingParams { domain_stake_usd: stake, is_ugc_platform: true, ..neutral_quality_params() };
            assert!((compute_score(0.8, 0.0, &p) - baseline).abs() < 1e-12,
                "UGC domain stake ${stake} must not change the score");
        }
    }

    #[test]
    fn test_reputation_never_affects_score() {
        let baseline = compute_score(0.8, 0.0, &neutral_quality_params());
        for rep in [0.0_f64, 0.5, 1.0, 2.0, 3.0] {
            for votes in [0u32, 5, 100] {
                let p = RankingParams { reputation: rep, reputation_votes: votes, ..neutral_quality_params() };
                assert!((compute_score(0.8, 0.0, &p) - baseline).abs() < 1e-12,
                    "reputation {rep} (votes={votes}) must not change the score");
            }
        }
    }

    #[test]
    fn test_stake_age_never_affects_score() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
        let baseline = compute_score(0.8, 0.0, &neutral_quality_params());
        for age_days in [0i64, 365, 365 * 5] {
            let p = RankingParams {
                entity_stake_usd: 1024.0,
                staked_at: Some(now - age_days * 86400),
                ..neutral_quality_params()
            };
            assert!((compute_score(0.8, 0.0, &p) - baseline).abs() < 1e-12,
                "stake age must not change the score");
        }
    }

    #[test]
    fn test_zero_stake_no_boost() {
        let params = neutral_quality_params();
        let score = compute_score(1.0, 1.0, &params);
        assert!((score - 1.0).abs() < 1e-10);
    }

    // ── Quality multiplier ──────────────────────────────────────────────

    #[test]
    fn test_quality_mult_high_quality() {
        let params = RankingParams {
            quality_score: Some(1.0),
            spam_score: Some(0.0),
            slop_score: Some(0.0),
            ..Default::default()
        };
        let score = compute_score(1.0, 1.0, &params);
        let base = 1.0;
        let quality_mult = 0.6 + 0.6 * 1.0; // 1.2
        assert!((score - base * quality_mult).abs() < 1e-10);
    }

    #[test]
    fn test_quality_mult_low_quality_high_spam() {
        let params = RankingParams {
            quality_score: Some(0.2),
            spam_score: Some(0.8),
            slop_score: Some(0.5),
            ..Default::default()
        };
        let score = compute_score(1.0, 1.0, &params);
        let quality_mult = (0.6 + 0.6 * 0.2) * (1.0 - 0.3 * 0.8) * (1.0 - 0.2 * 0.5);
        assert!((score - quality_mult).abs() < 1e-10);
    }

    #[test]
    fn test_neutral_quality_mult_with_defaults() {
        let params = RankingParams::default();
        let score = compute_score(1.0, 1.0, &params);
        assert!((score - NEUTRAL_Q).abs() < 1e-10, "neutral quality_mult should be {NEUTRAL_Q}");
    }

    // ── DNS verification ────────────────────────────────────────────────

    #[test]
    fn test_dns_verified_boost() {
        let params = RankingParams { dns_verified: Some(true), ..neutral_quality_params() };
        let score = compute_score(1.0, 1.0, &params);
        assert!((score - 1.05).abs() < 1e-10);
    }

    #[test]
    fn test_dns_unverified_penalty() {
        let params = RankingParams { dns_verified: Some(false), ..neutral_quality_params() };
        let score = compute_score(1.0, 1.0, &params);
        assert!((score - 0.95).abs() < 1e-10);
    }

    // ── Freshness ───────────────────────────────────────────────────────

    #[test]
    fn test_freshness_boost_recent() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
        let params = RankingParams { published_at: Some(now - 3600), ..neutral_quality_params() };
        let score = compute_score(1.0, 1.0, &params);
        assert!(score > 1.14 && score < 1.16, "recent content ~15% boost, got {score}");
    }

    #[test]
    fn test_freshness_smooth_decay_30days() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
        let params = RankingParams { published_at: Some(now - 86400 * 30), ..neutral_quality_params() };
        let score = compute_score(1.0, 1.0, &params);
        assert!(score > 1.04 && score < 1.07, "30-day content ~5.5% boost, got {score}");
    }

    #[test]
    fn test_freshness_no_boost_old() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
        let params = RankingParams { published_at: Some(now - 86400 * 365), ..neutral_quality_params() };
        let score = compute_score(1.0, 1.0, &params);
        assert!(score > 0.99 && score < 1.01, "1-year content ~0% boost, got {score}");
    }

    // ── Full formula proves stake/reputation are ignored ────────────────

    #[test]
    fn test_full_formula_with_all_signals() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
        let params = RankingParams {
            semantic_weight: 0.8,
            bm25_weight: 0.2,
            // High stake + bad reputation on purpose — must NOT move the score.
            entity_stake_usd: 1024.0,
            reputation: 0.5,
            reputation_votes: 100,
            quality_score: Some(0.8),
            spam_score: Some(0.1),
            slop_score: Some(0.05),
            dns_verified: Some(true),
            published_at: Some(now - 86400 * 10),
            ..Default::default()
        };
        let score = compute_score(0.9, 0.7, &params);
        let base = 0.8 * 0.9 + 0.2 * 0.7; // 0.86
        let quality_mult = (0.6 + 0.6 * 0.8) * (1.0 - 0.3 * 0.1) * (1.0 - 0.2 * 0.05);
        let freshness_mult = 1.0 + 0.15 * (-10.0 / 30.0_f64).exp();
        let dns_mult = 1.05;
        let expected = base * quality_mult * freshness_mult * dns_mult; // no stake, no reputation
        assert!((score - expected).abs() < 1e-10, "got {score}, expected {expected}");
    }

    // ── Trust verdict (feeds min_trust filter, not the score) ───────────

    #[test]
    fn test_verdict_disputed() {
        assert_eq!(compute_verdict(100_000.0, 2.0, 100, true, true), "disputed");
    }

    #[test]
    fn test_verdict_highly_trusted() {
        assert_eq!(compute_verdict(10_000.0, 1.8, 50, false, true), "highly_trusted");
    }

    #[test]
    fn test_verdict_trusted() {
        assert_eq!(compute_verdict(1_000.0, 1.3, 10, false, true), "trusted");
    }

    #[test]
    fn test_verdict_trusted_requires_dns_proof() {
        // Same stake/reputation/votes WITHOUT domain-ownership proof caps at
        // neutral — capital bonded to a name you don't control buys no label.
        assert_eq!(compute_verdict(1_000.0, 1.3, 10, false, false), "neutral");
    }

    #[test]
    fn test_verdict_capital_alone_buys_nothing_above_neutral() {
        // Adversarial: a whale with $1M staked, DNS-verified, but no earned
        // reputation/votes — money cannot shortcut community validation.
        assert_eq!(compute_verdict(1_000_000.0, 1.0, 0, false, true), "neutral");
    }

    #[test]
    fn test_verdict_disputed_overrides_everything() {
        // Adversarial: an actively disputed domain keeps the disputed label
        // no matter how much capital/reputation it holds.
        assert_eq!(compute_verdict(1_000_000.0, 3.0, 999, true, true), "disputed");
    }

    #[test]
    fn test_verdict_reputation_without_stake_stays_neutral() {
        // Adversarial: reputation farming without bonded capital cannot reach
        // trusted — there must be something at risk to slash.
        assert_eq!(compute_verdict(0.0, 2.0, 100, false, true), "neutral");
    }

    #[test]
    fn test_verdict_unverified() {
        assert_eq!(compute_verdict(0.0, 1.0, 0, false, false), "unverified");
    }

    #[test]
    fn test_verdict_neutral() {
        assert_eq!(compute_verdict(500.0, 1.2, 8, false, false), "neutral");
    }

    #[test]
    fn test_trust_rank_ordering() {
        assert!(trust_rank("highly_trusted") > trust_rank("trusted"));
        assert!(trust_rank("trusted") > trust_rank("neutral"));
        assert!(trust_rank("neutral") > trust_rank("unverified"));
        assert!(trust_rank("unverified") > trust_rank("disputed"));
        assert_eq!(trust_rank("disputed"), 0);
        assert_eq!(trust_rank("anything-unknown"), 0);
    }

    // ── Relevance gate (retained utility) ───────────────────────────────

    #[test]
    fn test_relevance_gate_below_lower() {
        assert!((relevance_gate(0.5) - 0.0).abs() < 1e-10);
        assert!((relevance_gate(0.3) - 0.0).abs() < 1e-10);
        assert!((relevance_gate(0.6) - 0.0).abs() < 1e-10);
    }

    #[test]
    fn test_relevance_gate_above_upper() {
        assert!((relevance_gate(0.85) - 1.0).abs() < 1e-10);
        assert!((relevance_gate(0.9) - 1.0).abs() < 1e-10);
        assert!((relevance_gate(1.0) - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_relevance_gate_quadratic_shape() {
        assert!((relevance_gate(0.65) - 0.04).abs() < 1e-10);
        assert!((relevance_gate(0.725) - 0.25).abs() < 1e-10);
        assert!((relevance_gate(0.80) - 0.64).abs() < 1e-10);
    }
}
