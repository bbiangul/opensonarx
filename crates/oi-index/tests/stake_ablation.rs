//! Stake-neutrality regression guard (was: the stake-weighting ablation).
//!
//! Background: the ranking formula used to multiply the score by a stake boost,
//! which let capital buy rank (a $1k stake flipped a less-relevant, lower-quality
//! doc to #1 — see this file's git history). The refactor removed stake and
//! reputation from `compute_score`; they now feed `compute_verdict` /
//! `QueryFilters.min_trust` instead.
//!
//! This locks that in: on a controlled 2-document contest — an HONEST doc (more
//! relevant, higher quality) vs an ADVERSARY (less relevant, lower quality) buying
//! stake — the honest doc must ALWAYS win, at any stake level, and the score must
//! not move with stake at all.
//!
//! Run: cargo test -p oi-index --test stake_ablation -- --nocapture
use oi_index::ranking::{compute_score, RankingParams};

const SEM_HONEST: f64 = 0.88; // honest content is genuinely more relevant
const SEM_ADVERSARY: f64 = 0.82; // adversary is *almost* as relevant (worst case)
const BM25: f64 = 0.0; // semantic-only so base == SEM (matches default weights)

/// Honest publisher: highly relevant, high quality, verified — but no stake.
fn honest() -> RankingParams {
    RankingParams {
        semantic_weight: 1.0,
        bm25_weight: 0.0,
        quality_score: Some(0.95),
        spam_score: Some(0.0),
        slop_score: Some(0.0),
        dns_verified: Some(true),
        ..Default::default()
    }
}

/// Adversary: less relevant, mediocre quality — climbs by buying domain stake.
fn adversary(stake_usd: f64) -> RankingParams {
    RankingParams {
        semantic_weight: 1.0,
        bm25_weight: 0.0,
        domain_stake_usd: stake_usd,
        is_ugc_platform: false, // publisher domain → 30% passthrough
        quality_score: Some(0.60),
        spam_score: Some(0.0),
        slop_score: Some(0.0),
        dns_verified: Some(false),
        ..Default::default()
    }
}

/// Model the "stripped" ranking: stake/reputation contribute nothing to order.
/// Equivalent to the refactor that moves stake to an eligibility/label role.
fn strip_stake(mut p: RankingParams) -> RankingParams {
    p.entity_stake_usd = 0.0;
    p.domain_stake_usd = 0.0;
    p.reputation = 1.0;
    p.reputation_votes = 0;
    p.staked_at = None;
    p
}

fn score_full(p: &RankingParams, sem: f64) -> f64 {
    compute_score(sem, BM25, p)
}
fn score_stripped(p: &RankingParams, sem: f64) -> f64 {
    compute_score(sem, BM25, &strip_stake(p.clone()))
}

#[test]
fn stake_and_reputation_never_change_rank() {
    let hon = honest();
    let honest_score = score_full(&hon, SEM_HONEST);

    eprintln!("\n{}", "=".repeat(60));
    eprintln!("  STAKE NEUTRALITY — honest(sem={SEM_HONEST}, q=0.95) vs adversary(sem={SEM_ADVERSARY}, q=0.60)");
    eprintln!("{}", "=".repeat(60));
    eprintln!("  honest score: {honest_score:.4}");
    eprintln!("\n  {:>12} | {:>10} {:>10} {:>9}", "adv stake", "adv score", "hon score", "winner");
    eprintln!("  {}", "-".repeat(47));

    let mut prev_adv: Option<f64> = None;
    for &s in &[0.0, 100.0, 1_000.0, 10_000.0, 50_000.0, 100_000.0, 1_000_000.0] {
        let adv = adversary(s);
        let adv_score = score_full(&adv, SEM_ADVERSARY);
        let winner = if adv_score > honest_score { "ADVERSARY" } else { "honest" };
        eprintln!(
            "  {:>12} | {:>10.4} {:>10.4} {:>9}",
            format!("${}", s as u64),
            adv_score,
            honest_score,
            winner
        );

        // Stake must not move the score at all.
        if let Some(p) = prev_adv {
            assert!((adv_score - p).abs() < 1e-12, "stake changed the rank score at ${}", s as u64);
        }
        prev_adv = Some(adv_score);

        // The more-relevant, higher-quality doc must always win.
        assert!(adv_score < honest_score, "adversary must never outrank via stake (${})", s as u64);

        // full == stripped, since the scorer ignores stake/reputation entirely.
        assert!(
            (adv_score - score_stripped(&adv, SEM_ADVERSARY)).abs() < 1e-12,
            "full and stripped scoring must now be identical"
        );
    }

    // Reputation must not move rank either.
    for rep in [0.1_f64, 0.5, 2.0, 3.0] {
        let mut h = honest();
        h.reputation = rep;
        h.reputation_votes = 100;
        assert!(
            (score_full(&h, SEM_HONEST) - honest_score).abs() < 1e-12,
            "reputation {rep} changed the rank score"
        );
    }

    eprintln!("\n  Stake/reputation never moved the score; relevance + quality decided.");
    eprintln!("{}\n", "=".repeat(60));
}

/// Sanity: removing stake from rank must NOT disturb ordering when nobody is
/// staked (the actual benchmark condition). The 82% corpus has zero stake, so
/// the refactor is provably a no-op there.
#[test]
fn stripping_stake_is_a_noop_when_no_one_is_staked() {
    let a = honest();
    let mut b = honest();
    b.quality_score = Some(0.70); // slightly worse, still no stake anywhere

    let full_order = score_full(&a, SEM_HONEST) > score_full(&b, SEM_ADVERSARY);
    let strip_order = score_stripped(&a, SEM_HONEST) > score_stripped(&b, SEM_ADVERSARY);

    assert_eq!(
        full_order, strip_order,
        "with no stake present, stripped and full rankings must be identical"
    );
    // And the scores themselves are identical, since stake/rep were already neutral.
    assert!((score_full(&a, SEM_HONEST) - score_stripped(&a, SEM_HONEST)).abs() < 1e-12);
}
