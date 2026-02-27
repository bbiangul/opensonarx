/// Publisher domains pass through 30% of domain-level stake to entity content.
const PUBLISHER_PASSTHROUGH: f64 = 0.3;

/// Ranking formula:
/// `(0.8 * semantic + 0.2 * BM25) * gated_stake_boost * ReputationMult * AgentUtilityBoost`
///
/// Effective stake = entity_stake + domain_stake * passthrough.
/// - Publisher domains: passthrough = 30%
/// - UGC platforms: passthrough = 0% (domain-level staking gives zero boost)
///
/// The stake boost is gated by a smooth relevance ramp: content must be
/// semantically relevant before stake amplifies its ranking. This prevents
/// pay-to-win where high-stake but irrelevant content outranks relevant results.
///
/// Weights tuned via parameter sweep (sw=0.8 beat sw=0.5 by +4.3% R@10 on Arctic Embed S).
/// Minimum feedback votes required before reputation affects ranking.
/// Below this threshold, reputation is treated as neutral (1.0).
const MIN_VOTES_FOR_REPUTATION: u32 = 5;

/// Quadratic relevance gate for stake boost.
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
    if stake_usd >= 1_000.0 && reputation >= 1.3 && reputation_votes >= 10 {
        return "trusted";
    }
    if stake_usd < 1.0 && reputation_votes < 5 {
        return "unverified";
    }
    "neutral"
}

pub fn compute_score(semantic_score: f64, bm25_score: f64, params: &RankingParams) -> f64 {
    let base = params.semantic_weight * semantic_score + params.bm25_weight * bm25_score;

    // Compute effective stake: entity stake + domain passthrough
    let domain_passthrough = if params.is_ugc_platform {
        0.0
    } else {
        PUBLISHER_PASSTHROUGH
    };
    let effective_stake = params.entity_stake_usd + params.domain_stake_usd * domain_passthrough;

    // Stake age decay: 5% per year — dormant stakes lose influence over time
    let stake_age_mult = if let Some(staked_at) = params.staked_at {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let age_years = (now - staked_at) as f64 / (365.25 * 86400.0);
        0.95_f64.powf(age_years.max(0.0))
    } else {
        1.0
    };

    // Only apply stake boost for stakes >= $1 (log2 of values < 1 is negative)
    // Gate by relevance: irrelevant content doesn't benefit from staking.
    let raw_stake_boost = if effective_stake >= 1.0 {
        ((1.0 + 0.1 * effective_stake.log2()) * stake_age_mult).min(3.0)
    } else {
        1.0
    };
    let gate = relevance_gate(base);
    let stake_boost = 1.0 + (raw_stake_boost - 1.0) * gate;

    // Content quality multiplier: high quality → up to 1.2x, low quality → down to 0.6x
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

    // Require minimum votes before reputation departs from neutral.
    // Raw score is still returned to clients for transparency.
    let effective_rep = if params.reputation_votes >= MIN_VOTES_FOR_REPUTATION {
        params.reputation
    } else {
        1.0
    };
    let reputation = effective_rep.clamp(0.0, 3.0);

    base * stake_boost * reputation * params.agent_utility_boost
        * quality_mult * freshness_mult * dns_mult
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Neutral quality multiplier when no quality data is provided (all None):
    /// quality=0.7, spam=0.0, slop=0.0 → (0.6 + 0.6*0.7) * 1.0 * 1.0 = 1.02
    const NEUTRAL_Q: f64 = 1.02;

    /// Helper: params with quality signals explicitly neutral (quality_mult = 1.0)
    /// and sufficient reputation votes so reputation isn't forced to 1.0.
    fn neutral_quality_params() -> RankingParams {
        RankingParams {
            // quality_score=2/3 → (0.6 + 0.6*2/3) = 1.0; spam=0, slop=0
            quality_score: Some(2.0 / 3.0),
            spam_score: Some(0.0),
            slop_score: Some(0.0),
            reputation_votes: MIN_VOTES_FOR_REPUTATION, // enough to use actual reputation
            ..Default::default()
        }
    }

    #[test]
    fn test_no_stake_no_boost() {
        let params = neutral_quality_params();
        let score = compute_score(0.8, 0.6, &params);
        let expected = 1.0 * 0.8 + 0.0 * 0.6; // 0.8 (semantic only)
        assert!((score - expected).abs() < 1e-10);
    }

    #[test]
    fn test_entity_stake_boost() {
        let params = RankingParams {
            entity_stake_usd: 1024.0, // log2(1024) = 10
            ..neutral_quality_params()
        };
        let score = compute_score(0.8, 0.6, &params);
        let base = 1.0 * 0.8; // 0.8 (semantic only)
        let raw_boost = 1.0 + 0.1 * 10.0;   // 2.0
        let gate = relevance_gate(base);      // (0.8-0.6)/0.25 = 0.8
        let gated_boost = 1.0 + (raw_boost - 1.0) * gate;
        assert!((score - base * gated_boost).abs() < 1e-10);
    }

    #[test]
    fn test_domain_stake_publisher_passthrough() {
        // Publisher domain: 30% passthrough
        let params = RankingParams {
            domain_stake_usd: 1000.0,
            is_ugc_platform: false,
            ..neutral_quality_params()
        };
        let score = compute_score(0.8, 0.6, &params);
        let base = 1.0 * 0.8; // 0.8 (semantic only)
        let effective: f64 = 1000.0 * 0.3; // $300
        let raw_boost = 1.0 + 0.1 * effective.log2();
        let gate = relevance_gate(base);
        let gated_boost = 1.0 + (raw_boost - 1.0) * gate;
        assert!((score - base * gated_boost).abs() < 1e-10);
    }

    #[test]
    fn test_domain_stake_ugc_zero_passthrough() {
        // UGC platform: zero passthrough
        let params = RankingParams {
            domain_stake_usd: 1000.0,
            is_ugc_platform: true,
            ..neutral_quality_params()
        };
        let score = compute_score(0.8, 0.6, &params);
        let base = 1.0 * 0.8; // 0.8 (semantic only)
        // effective = 0 + 1000 * 0 = 0, no boost
        assert!((score - base).abs() < 1e-10, "UGC domain stake should not boost");
    }

    #[test]
    fn test_entity_stake_on_ugc_platform() {
        // Entity-level stake on UGC platform gets full boost (gated by relevance)
        let params = RankingParams {
            entity_stake_usd: 1024.0,
            domain_stake_usd: 5000.0, // ignored for UGC
            is_ugc_platform: true,
            ..neutral_quality_params()
        };
        let score = compute_score(0.8, 0.6, &params);
        let base = 1.0 * 0.8; // 0.8 (semantic only)
        let raw_boost = 1.0 + 0.1 * 1024.0_f64.log2(); // entity stake only
        let gate = relevance_gate(base);
        let gated_boost = 1.0 + (raw_boost - 1.0) * gate;
        assert!((score - base * gated_boost).abs() < 1e-10);
    }

    #[test]
    fn test_reputation_multiplier() {
        let params = RankingParams {
            reputation: 0.5,
            ..neutral_quality_params()
        };
        let score = compute_score(1.0, 1.0, &params);
        let base = 1.0 * 1.0; // 1.0 (semantic only)
        let expected = base * 0.5; // stake_boost=1.0, rep=0.5
        assert!((score - expected).abs() < 1e-10);
    }

    #[test]
    fn test_full_formula() {
        let params = RankingParams {
            semantic_weight: 0.5,
            bm25_weight: 0.5,
            entity_stake_usd: 256.0,  // log2(256) = 8
            reputation: 0.9,
            agent_utility_boost: 1.2,
            ..neutral_quality_params()
        };
        let score = compute_score(0.9, 0.7, &params);
        let base = 0.5 * 0.9 + 0.5 * 0.7; // 0.8
        let raw_boost = 1.0 + 0.1 * 8.0;   // 1.8
        let gate = relevance_gate(base);     // (0.8-0.6)/0.25 = 0.8
        let gated_boost = 1.0 + (raw_boost - 1.0) * gate;
        let expected = base * gated_boost * 0.9 * 1.2;
        assert!((score - expected).abs() < 1e-10);
    }

    #[test]
    fn test_small_stake_no_negative_boost() {
        let params = RankingParams {
            entity_stake_usd: 0.5,
            ..neutral_quality_params()
        };
        let score = compute_score(0.8, 0.6, &params);
        let base = 1.0 * 0.8; // 0.8 (semantic only)
        assert!((score - base).abs() < 1e-10, "small stake should not reduce score");
    }

    #[test]
    fn test_zero_stake_no_boost() {
        let params = neutral_quality_params();
        let score = compute_score(1.0, 1.0, &params);
        let expected = 1.0 * 1.0; // 1.0 (semantic only)
        assert!((score - expected).abs() < 1e-10);
    }

    #[test]
    fn test_stake_boost_capped() {
        // Use high base score (1.0) so relevance gate is fully open
        let params = RankingParams {
            entity_stake_usd: 1_000_000_000.0,
            ..neutral_quality_params()
        };
        let score = compute_score(1.0, 1.0, &params);
        let base = 1.0 * 1.0; // 1.0 (semantic only)
        let expected = base * 3.0; // capped at 3.0, gate=1.0 at base=1.0
        assert!((score - expected).abs() < 1e-10, "stake boost should be capped at 3.0");
    }

    #[test]
    fn test_backward_compat_domain_only() {
        // Domain-only staking (no entity) works as before for publisher domains
        let params = RankingParams {
            domain_stake_usd: 1024.0,
            is_ugc_platform: false,
            ..neutral_quality_params()
        };
        let score = compute_score(0.8, 0.6, &params);
        let base = 1.0 * 0.8; // 0.8 (semantic only)
        let effective: f64 = 1024.0 * 0.3; // 30% passthrough
        let raw_boost = 1.0 + 0.1 * effective.log2();
        let gate = relevance_gate(base);
        let gated_boost = 1.0 + (raw_boost - 1.0) * gate;
        assert!((score - base * gated_boost).abs() < 1e-10);
    }

    // ── New quality/freshness/DNS signal tests ──

    #[test]
    fn test_quality_mult_high_quality() {
        let params = RankingParams {
            quality_score: Some(1.0),
            spam_score: Some(0.0),
            slop_score: Some(0.0),
            ..Default::default()
        };
        let score = compute_score(1.0, 1.0, &params);
        let base = 0.8 + 0.2; // 1.0
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
        let base = 1.0;
        let quality_mult = (0.6 + 0.6 * 0.2) * (1.0 - 0.3 * 0.8) * (1.0 - 0.2 * 0.5);
        assert!((score - base * quality_mult).abs() < 1e-10);
    }

    #[test]
    fn test_neutral_quality_mult_with_defaults() {
        // Default params: quality_score=None → 0.7, spam/slop=None → 0.0
        let params = RankingParams::default();
        let score = compute_score(1.0, 1.0, &params);
        let base = 1.0;
        assert!((score - base * NEUTRAL_Q).abs() < 1e-10, "neutral quality_mult should be {NEUTRAL_Q}");
    }

    #[test]
    fn test_dns_verified_boost() {
        let params = RankingParams {
            dns_verified: Some(true),
            ..neutral_quality_params()
        };
        let score = compute_score(1.0, 1.0, &params);
        let expected = 1.0 * 1.05; // 5% boost
        assert!((score - expected).abs() < 1e-10);
    }

    #[test]
    fn test_dns_unverified_penalty() {
        let params = RankingParams {
            dns_verified: Some(false),
            ..neutral_quality_params()
        };
        let score = compute_score(1.0, 1.0, &params);
        let expected = 1.0 * 0.95; // 5% penalty
        assert!((score - expected).abs() < 1e-10);
    }

    #[test]
    fn test_freshness_boost_recent() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let params = RankingParams {
            published_at: Some(now - 3600), // 1 hour ago
            ..neutral_quality_params()
        };
        let score = compute_score(1.0, 1.0, &params);
        let base = 1.0;
        // Smooth decay: ~0 days old → 1.0 + 0.15 * exp(0) ≈ 1.15
        assert!(score > base * 1.14 && score < base * 1.16, "recent content should get ~15% boost, got {score}");
    }

    #[test]
    fn test_freshness_smooth_decay_30days() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let params = RankingParams {
            published_at: Some(now - 86400 * 30), // 30 days ago
            ..neutral_quality_params()
        };
        let score = compute_score(1.0, 1.0, &params);
        let base = 1.0;
        // Smooth: 1.0 + 0.15 * exp(-1) ≈ 1.055
        assert!(score > base * 1.04 && score < base * 1.07, "30-day content should get ~5.5% boost, got {score}");
    }

    #[test]
    fn test_freshness_no_boost_old() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let params = RankingParams {
            published_at: Some(now - 86400 * 365), // 1 year ago
            ..neutral_quality_params()
        };
        let score = compute_score(1.0, 1.0, &params);
        let base = 1.0;
        // Smooth: 1.0 + 0.15 * exp(-365/30) ≈ 1.0000...
        assert!(score > base * 0.99 && score < base * 1.01, "1-year old content should get ~0% boost, got {score}");
    }

    #[test]
    fn test_full_formula_with_all_signals() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let params = RankingParams {
            semantic_weight: 0.8,
            bm25_weight: 0.2,
            entity_stake_usd: 1024.0,
            reputation: 0.9,
            reputation_votes: 10, // enough to count
            agent_utility_boost: 1.0,
            quality_score: Some(0.8),
            spam_score: Some(0.1),
            slop_score: Some(0.05),
            dns_verified: Some(true),
            published_at: Some(now - 86400 * 10), // 10 days ago
            domain_stake_usd: 0.0,
            is_ugc_platform: false,
            staked_at: None,
        };
        let score = compute_score(0.9, 0.7, &params);
        let base = 0.8 * 0.9 + 0.2 * 0.7; // 0.86
        let stake_boost = 1.0 + 0.1 * 1024.0_f64.log2(); // 2.0
        let quality_mult = (0.6 + 0.6 * 0.8) * (1.0 - 0.3 * 0.1) * (1.0 - 0.2 * 0.05);
        let freshness_mult = 1.0 + 0.15 * (-10.0 / 30.0_f64).exp(); // smooth decay at 10 days
        let dns_mult = 1.05;
        let expected = base * stake_boost * 0.9 * 1.0 * quality_mult * freshness_mult * dns_mult;
        assert!((score - expected).abs() < 1e-10, "got {score}, expected {expected}");
    }

    // ── Trust verdict tests ──

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
        assert_eq!(compute_verdict(1_000.0, 1.3, 10, false, false), "trusted");
    }

    #[test]
    fn test_verdict_unverified() {
        assert_eq!(compute_verdict(0.0, 1.0, 0, false, false), "unverified");
    }

    #[test]
    fn test_verdict_neutral() {
        assert_eq!(compute_verdict(500.0, 1.2, 8, false, false), "neutral");
    }

    // ── Min vote threshold tests ──

    #[test]
    fn test_reputation_ignored_below_min_votes() {
        // Bad reputation (0.5) with only 2 votes → treated as neutral (1.0)
        let params = RankingParams {
            reputation: 0.5,
            reputation_votes: 2, // below threshold
            ..neutral_quality_params()
        };
        let score = compute_score(1.0, 1.0, &params);
        let base = 1.0;
        // With reputation forced to 1.0, score == base
        assert!((score - base).abs() < 1e-10, "low-vote reputation should be neutral, got {score}");
    }

    #[test]
    fn test_reputation_applied_above_min_votes() {
        let params = RankingParams {
            reputation: 0.5,
            reputation_votes: MIN_VOTES_FOR_REPUTATION,
            ..neutral_quality_params()
        };
        let score = compute_score(1.0, 1.0, &params);
        let base = 1.0;
        let expected = base * 0.5;
        assert!((score - expected).abs() < 1e-10, "above-threshold reputation should apply, got {score}");
    }

    // ── Stake age decay tests ──

    #[test]
    fn test_stake_age_decay_recent() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let params = RankingParams {
            entity_stake_usd: 1024.0,
            staked_at: Some(now - 86400), // 1 day ago
            ..neutral_quality_params()
        };
        let score = compute_score(1.0, 1.0, &params);
        let base = 1.0;
        let boost_no_decay = 1.0 + 0.1 * 1024.0_f64.log2(); // 2.0
        // Recent stake: nearly full boost
        assert!(score > base * boost_no_decay * 0.99, "recent stake should have near-full boost, got {score}");
    }

    #[test]
    fn test_stake_age_decay_5years() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let params = RankingParams {
            entity_stake_usd: 1024.0,
            staked_at: Some(now - (5.0 * 365.25 * 86400.0) as i64), // 5 years ago
            ..neutral_quality_params()
        };
        let score = compute_score(1.0, 1.0, &params);
        let base = 1.0;
        let boost_no_decay = 1.0 + 0.1 * 1024.0_f64.log2(); // 2.0
        let age_decay = 0.95_f64.powi(5); // ~0.774
        let expected = base * ((boost_no_decay - 1.0) * age_decay + 1.0); // boost part decays
        // The actual formula: (1 + 0.1*log2(1024)) * 0.95^5 = 2.0 * 0.774 = 1.548
        let expected_exact = base * (boost_no_decay * age_decay);
        assert!((score - expected_exact).abs() < 0.01, "5yr stake should decay to ~{expected_exact:.3}, got {score}");
    }

    // ── Relevance gate tests ──

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
        // Quadratic: steep at bottom, gentle at top
        // At 0.65: t=0.2, gate=0.04
        assert!((relevance_gate(0.65) - 0.04).abs() < 1e-10);
        // At 0.725 (linear midpoint): t=0.5, gate=0.25 (NOT 0.5)
        assert!((relevance_gate(0.725) - 0.25).abs() < 1e-10);
        // At 0.80: t=0.8, gate=0.64
        assert!((relevance_gate(0.80) - 0.64).abs() < 1e-10);
    }

    #[test]
    fn test_relevance_gate_blocks_irrelevant_stake() {
        // Low relevance (base=0.5) + high stake → stake boost should be zero
        let params = RankingParams {
            entity_stake_usd: 50_000.0,
            ..neutral_quality_params()
        };
        let score = compute_score(0.5, 0.5, &params);
        let base = 1.0 * 0.5; // 0.5 (semantic only)
        // gate=0 at base=0.5 (below 0.6), so no stake boost
        assert!((score - base).abs() < 1e-10, "irrelevant content should get no stake boost, got {score}");
    }

    #[test]
    fn test_relevance_gate_whale_at_low_relevance() {
        // $50k whale at base=0.65 → quadratic gate gives only 4% of boost
        let params = RankingParams {
            entity_stake_usd: 50_000.0, // raw boost ≈ 2.56
            ..neutral_quality_params()
        };
        let score = compute_score(0.65, 0.0, &params);
        let base = 1.0 * 0.65; // 0.65 (semantic only)
        let raw_boost = (1.0 + 0.1 * 50_000.0_f64.log2()).min(3.0);
        let gate = relevance_gate(base); // t=0.2, gate=0.04
        let gated = 1.0 + (raw_boost - 1.0) * gate;
        // Whale gets negligible boost — score barely above base
        assert!((score - base * gated).abs() < 1e-10);
        assert!(gated < 1.1, "whale at low relevance should get <10% boost, got {:.1}%", (gated - 1.0) * 100.0);
    }

    #[test]
    fn test_relevance_gate_full_boost_high_relevance() {
        // High relevance (base=1.0) + stake → full boost
        let params = RankingParams {
            entity_stake_usd: 1024.0,
            ..neutral_quality_params()
        };
        let score = compute_score(1.0, 1.0, &params);
        let base = 1.0;
        let boost = 1.0 + 0.1 * 1024.0_f64.log2(); // 2.0
        assert!((score - base * boost).abs() < 1e-10, "highly relevant content should get full boost, got {score}");
    }
}
