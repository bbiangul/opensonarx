/// Signal comparison test: shows the impact of quality, freshness, and DNS signals
/// on ranking among equally relevant content.
///
/// Run with: cargo test -p oi-index --test signal_comparison -- --nocapture
use oi_index::ranking::{compute_score, RankingParams};

fn base_params() -> RankingParams {
    RankingParams {
        semantic_weight: 1.0,
        bm25_weight: 0.0,
        ..Default::default()
    }
}

#[test]
fn compare_quality_signals() {
    let sem = 0.85;
    let bm25 = 0.70;

    eprintln!("\n{}", "=".repeat(80));
    eprintln!("  SIGNAL COMPARISON: Quality, Freshness, DNS on equally relevant content");
    eprintln!("  Semantic={sem}, BM25={bm25} (stake/reputation are not rank signals)");
    eprintln!("{}", "=".repeat(80));

    // ── 1. Quality signal comparison ──
    eprintln!("\n--- Quality Signal ---");
    eprintln!("  {:<45} {:>8} {:>8}", "Scenario", "Score", "Δ vs neutral");

    let neutral = compute_score(sem, bm25, &base_params());

    let scenarios = [
        ("No quality data (default, q=0.7)",    None,      None,      None),
        ("High quality (q=1.0, clean)",          Some(1.0), Some(0.0), Some(0.0)),
        ("Medium quality (q=0.5, clean)",        Some(0.5), Some(0.0), Some(0.0)),
        ("Low quality (q=0.2, clean)",           Some(0.2), Some(0.0), Some(0.0)),
        ("High quality + light spam (0.3)",      Some(0.9), Some(0.3), Some(0.0)),
        ("High quality + heavy spam (0.7)",      Some(0.9), Some(0.7), Some(0.0)),
        ("High quality + AI slop (0.5)",         Some(0.9), Some(0.0), Some(0.5)),
        ("Low quality + spam + slop",            Some(0.2), Some(0.5), Some(0.4)),
    ];

    for (label, q, spam, slop) in scenarios {
        let params = RankingParams {
            quality_score: q,
            spam_score: spam,
            slop_score: slop,
            ..base_params()
        };
        let score = compute_score(sem, bm25, &params);
        let delta = ((score / neutral - 1.0) * 100.0).round();
        let sign = if delta >= 0.0 { "+" } else { "" };
        eprintln!("  {:<45} {:>8.4} {:>7}{sign}{}%", label, score, "", delta);
    }

    // ── 2. Freshness signal comparison ──
    eprintln!("\n--- Freshness Signal ---");
    eprintln!("  {:<45} {:>8} {:>8}", "Scenario", "Score", "Δ vs neutral");

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    let freshness_scenarios = [
        ("No publish date (unknown)",       None),
        ("Published 1 hour ago",            Some(now - 3600)),
        ("Published 3 days ago",            Some(now - 86400 * 3)),
        ("Published 2 weeks ago",           Some(now - 86400 * 14)),
        ("Published 2 months ago",          Some(now - 86400 * 60)),
        ("Published 6 months ago",          Some(now - 86400 * 180)),
        ("Published 2 years ago",           Some(now - 86400 * 730)),
    ];

    // Use neutral quality for freshness comparison
    let neutral_q = RankingParams {
        quality_score: Some(2.0 / 3.0),
        spam_score: Some(0.0),
        slop_score: Some(0.0),
        ..base_params()
    };
    let neutral_fresh = compute_score(sem, bm25, &neutral_q);

    for (label, pub_at) in freshness_scenarios {
        let params = RankingParams {
            published_at: pub_at,
            quality_score: Some(2.0 / 3.0),
            spam_score: Some(0.0),
            slop_score: Some(0.0),
            ..base_params()
        };
        let score = compute_score(sem, bm25, &params);
        let delta = ((score / neutral_fresh - 1.0) * 100.0).round();
        let sign = if delta >= 0.0 { "+" } else { "" };
        eprintln!("  {:<45} {:>8.4} {:>7}{sign}{}%", label, score, "", delta);
    }

    // ── 3. DNS verification signal ──
    eprintln!("\n--- DNS Verification Signal ---");
    eprintln!("  {:<45} {:>8} {:>8}", "Scenario", "Score", "Δ vs neutral");

    let dns_scenarios = [
        ("DNS unknown (default)",           None),
        ("DNS verified",                    Some(true)),
        ("DNS NOT verified",                Some(false)),
    ];

    for (label, dns) in dns_scenarios {
        let params = RankingParams {
            dns_verified: dns,
            quality_score: Some(2.0 / 3.0),
            spam_score: Some(0.0),
            slop_score: Some(0.0),
            ..base_params()
        };
        let score = compute_score(sem, bm25, &params);
        let delta = ((score / neutral_fresh - 1.0) * 100.0).round();
        let sign = if delta >= 0.0 { "+" } else { "" };
        eprintln!("  {:<45} {:>8.4} {:>7}{sign}{}%", label, score, "", delta);
    }

    // ── 4. Combined: best vs worst case ──
    eprintln!("\n--- Combined: Best vs Worst among equally relevant results ---");

    let best = RankingParams {
        quality_score: Some(1.0),
        spam_score: Some(0.0),
        slop_score: Some(0.0),
        dns_verified: Some(true),
        published_at: Some(now - 3600), // 1 hour ago
        ..base_params()
    };
    let worst = RankingParams {
        quality_score: Some(0.2),
        spam_score: Some(0.5),
        slop_score: Some(0.4),
        dns_verified: Some(false),
        published_at: Some(now - 86400 * 365), // 1 year ago
        ..base_params()
    };
    let best_score = compute_score(sem, bm25, &best);
    let worst_score = compute_score(sem, bm25, &worst);
    let no_signals = RankingParams {
        quality_score: Some(2.0 / 3.0),
        spam_score: Some(0.0),
        slop_score: Some(0.0),
        ..base_params()
    };
    let neutral_score = compute_score(sem, bm25, &no_signals);

    eprintln!("  {:<45} {:>8.4}", "Best case (high q, fresh, verified)", best_score);
    eprintln!("  {:<45} {:>8.4}", "Neutral (no data)", neutral_score);
    eprintln!("  {:<45} {:>8.4}", "Worst case (low q, spam, old, unverified)", worst_score);
    eprintln!("  {:<45} {:>8.2}x", "Best/Worst ratio", best_score / worst_score);
    eprintln!("  {:<45} {:>8.2}x", "Best/Neutral ratio", best_score / neutral_score);

    eprintln!("\n{}", "=".repeat(80));

    // Sanity: best > neutral > worst
    assert!(best_score > neutral_score, "best should beat neutral");
    assert!(neutral_score > worst_score, "neutral should beat worst");
    // The spread shouldn't be too extreme (< 3x between best and worst)
    assert!(best_score / worst_score < 3.0, "spread should be < 3x");
}

#[test]
fn compare_signal_vs_stake_dominance() {
    eprintln!("\n{}", "=".repeat(80));
    eprintln!("  SIGNAL vs STAKE: stake is decoupled from rank — quality/freshness decide");
    eprintln!("{}", "=".repeat(80));

    let sem = 0.85;
    let bm25 = 0.70;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    // Low-stake ($10) but excellent quality, fresh, verified
    let low_stake_good = RankingParams {
        domain_stake_usd: 10.0,
        quality_score: Some(1.0),
        spam_score: Some(0.0),
        slop_score: Some(0.0),
        dns_verified: Some(true),
        published_at: Some(now - 3600),
        ..Default::default()
    };

    // High-stake ($10000) but poor quality, old
    let high_stake_bad = RankingParams {
        domain_stake_usd: 10_000.0,
        quality_score: Some(0.3),
        spam_score: Some(0.4),
        slop_score: Some(0.3),
        dns_verified: Some(false),
        published_at: Some(now - 86400 * 365),
        ..Default::default()
    };

    // High-stake ($10000) with good quality (the expected winner)
    let high_stake_good = RankingParams {
        domain_stake_usd: 10_000.0,
        quality_score: Some(0.9),
        spam_score: Some(0.0),
        slop_score: Some(0.0),
        dns_verified: Some(true),
        published_at: Some(now - 86400 * 5),
        ..Default::default()
    };

    let s1 = compute_score(sem, bm25, &low_stake_good);
    let s2 = compute_score(sem, bm25, &high_stake_bad);
    let s3 = compute_score(sem, bm25, &high_stake_good);

    eprintln!("\n  {:<50} {:>8}", "Scenario", "Score");
    eprintln!("  {}", "-".repeat(60));
    eprintln!("  {:<50} {:>8.4}", "Low stake ($10) + excellent quality + fresh", s1);
    eprintln!("  {:<50} {:>8.4}", "High stake ($10K) + poor quality + old", s2);
    eprintln!("  {:<50} {:>8.4}", "High stake ($10K) + good quality + fresh", s3);
    eprintln!();

    eprintln!("  Key insight: stake does NOT enter the ranking score.");
    eprintln!("  The lower-staked ($10) but higher-quality, fresher result wins:");
    eprintln!("    s1 (low stake, excellent, fresh) = {s1:.4}");
    eprintln!("    s3 (high stake, good)            = {s3:.4}");
    eprintln!("    s2 (high stake, poor)            = {s2:.4}");

    eprintln!("\n{}", "=".repeat(80));

    // Stake is decoupled from rank: quality + freshness decide the order.
    assert!(s1 > s3, "excellent quality + freshest must beat high-stake merely-good (stake is not a rank signal)");
    assert!(s3 > s2, "good quality must beat poor quality + spam");
    assert!(s1 > s2, "excellent quality must beat poor quality");
}
