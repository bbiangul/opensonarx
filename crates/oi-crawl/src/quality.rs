use std::collections::HashMap;

/// Quality gate: determines if extracted content is worth indexing.
pub struct QualityCheck {
    pub passed: bool,
    pub reason: Option<String>,
    pub quality_score: f64,
    /// Spam score (0.0-1.0): keyword stuffing, repeated content, caps abuse
    pub spam_score: f64,
    /// Repetitiveness score (0.0-1.0): low vocabulary diversity, uniform sentences
    pub slop_score: f64,
}

/// Quality profile for different content types.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum QualityProfile {
    /// Standard web content: 100 char min, 80% link ratio max
    Standard,
    /// Social media posts: 20 char min, 95% link ratio max
    SocialPost,
    /// Video descriptions: 30 char min, 90% link ratio max
    VideoDescription,
}

impl QualityProfile {
    fn min_chars(self) -> usize {
        match self {
            Self::Standard => 100,
            Self::SocialPost => 20,
            Self::VideoDescription => 30,
        }
    }

    fn max_link_ratio(self) -> f64 {
        match self {
            Self::Standard => 0.8,
            Self::SocialPost => 0.95,
            Self::VideoDescription => 0.90,
        }
    }
}

/// Run quality checks with a specific profile.
pub fn check_quality_with_profile(markdown: &str, profile: QualityProfile) -> QualityCheck {
    let min_chars = profile.min_chars();
    let max_link_ratio = profile.max_link_ratio();

    if markdown.len() < min_chars {
        return QualityCheck {
            passed: false,
            reason: Some(format!(
                "Content too short: {} chars (min {})",
                markdown.len(),
                min_chars
            )),
            quality_score: 0.0,
            spam_score: 0.0,
            slop_score: 0.0,
        };
    }

    // Check link ratio
    let lines: Vec<&str> = markdown.lines().filter(|l| !l.trim().is_empty()).collect();
    if !lines.is_empty() {
        let link_lines = lines
            .iter()
            .filter(|l| l.contains("http://") || l.contains("https://") || l.starts_with("- ["))
            .count();
        let link_ratio = link_lines as f64 / lines.len() as f64;
        if link_ratio > max_link_ratio {
            return QualityCheck {
                passed: false,
                reason: Some(format!(
                    "Too many links: {:.0}% of lines are links (max {:.0}%)",
                    link_ratio * 100.0,
                    max_link_ratio * 100.0
                )),
                quality_score: 0.1,
                spam_score: 0.5,
                slop_score: 0.0,
            };
        }
    }

    let word_count = markdown.split_whitespace().count();
    let quality_score = (word_count as f64 / 500.0).min(1.0);

    let spam_score = compute_spam_score(markdown);
    let slop_score = compute_slop_score(markdown);

    if spam_score > 0.7 {
        return QualityCheck {
            passed: false,
            reason: Some(format!("Spam detected (score: {spam_score:.2})")),
            quality_score,
            spam_score,
            slop_score,
        };
    }

    if slop_score > 0.7 {
        return QualityCheck {
            passed: false,
            reason: Some(format!("Repetitive text (score: {slop_score:.2})")),
            quality_score,
            spam_score,
            slop_score,
        };
    }

    QualityCheck {
        passed: true,
        reason: None,
        quality_score,
        spam_score,
        slop_score,
    }
}

/// Run quality checks on extracted markdown content using the Standard profile.
/// Fails if:
/// - Content is less than 100 characters
/// - More than 80% of lines are links
pub fn check_quality(markdown: &str) -> QualityCheck {
    check_quality_with_profile(markdown, QualityProfile::Standard)
}

/// Compute spam score from cheap heuristics.
/// Signals: keyword density, repeated trigrams, all-caps ratio.
fn compute_spam_score(text: &str) -> f64 {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return 0.0;
    }

    let mut signals = Vec::new();

    // 1. Keyword density: any single word > 10% of total. Words of three
    //    characters or fewer are skipped: function words are short in most
    //    languages, and no vocabulary has to be listed to exclude them.
    let mut lower_freq: HashMap<String, usize> = HashMap::new();
    for w in &words {
        let lw = w.to_lowercase();
        if lw.chars().count() > 3 {
            *lower_freq.entry(lw).or_default() += 1;
        }
    }
    let max_density = lower_freq
        .values()
        .map(|&c| c as f64 / words.len() as f64)
        .fold(0.0f64, f64::max);
    if max_density > 0.10 {
        signals.push(((max_density - 0.10) / 0.40).min(1.0));
    }

    // 2. Repeated trigrams: use ratio-based threshold scaled by document length.
    //    In short docs (100 words), 4+ absolute occurrences is suspicious.
    //    In long docs (10K+ words), common trigrams naturally repeat — use ratio instead.
    if words.len() >= 3 {
        let mut trigrams: HashMap<String, usize> = HashMap::new();
        for window in words.windows(3) {
            let tri = format!(
                "{} {} {}",
                window[0].to_lowercase(),
                window[1].to_lowercase(),
                window[2].to_lowercase()
            );
            *trigrams.entry(tri).or_default() += 1;
        }
        let max_tri = *trigrams.values().max().unwrap_or(&0);
        let total_trigrams = words.len() - 2;
        let max_tri_ratio = max_tri as f64 / total_trigrams as f64;
        // Flag when a single trigram exceeds 2% of all trigrams AND appears 4+ times
        if max_tri > 3 && max_tri_ratio > 0.02 {
            signals.push(((max_tri_ratio - 0.02) / 0.08).min(1.0));
        }
    }

    // 3. All-caps ratio: > 30% uppercase letters
    let alpha_chars: Vec<char> = text.chars().filter(|c| c.is_alphabetic()).collect();
    if !alpha_chars.is_empty() {
        let upper_count = alpha_chars.iter().filter(|c| c.is_uppercase()).count();
        let upper_ratio = upper_count as f64 / alpha_chars.len() as f64;
        if upper_ratio > 0.3 {
            signals.push((upper_ratio - 0.3) / 0.7);
        }
    }

    if signals.is_empty() {
        0.0
    } else {
        signals.iter().sum::<f64>() / signals.len() as f64
    }
}

/// Compute a repetitiveness score from text statistics: low vocabulary
/// diversity and uniform sentence length. It measures form, not origin or
/// meaning: no phrase list, and no claim about whether a model wrote the text.
fn compute_slop_score(text: &str) -> f64 {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() < 20 {
        return 0.0;
    }

    let mut signals = Vec::new();

    // 1. Vocabulary diversity: unique_words / total_words
    let lower_words: Vec<String> = words.iter().map(|w| w.to_lowercase()).collect();
    let unique: std::collections::HashSet<&str> = lower_words.iter().map(|s| s.as_str()).collect();
    let diversity = unique.len() as f64 / lower_words.len() as f64;
    // Very low diversity (< 0.3) is a slop signal
    if diversity < 0.3 {
        signals.push(1.0 - diversity / 0.3);
    }

    // 2. Sentence length uniformity: low variance = robotic text
    let sentences: Vec<&str> = text
        .split(['.', '!', '?'])
        .filter(|s| !s.trim().is_empty())
        .collect();
    if sentences.len() >= 5 {
        let lengths: Vec<f64> = sentences
            .iter()
            .map(|s| s.split_whitespace().count() as f64)
            .collect();
        let mean = lengths.iter().sum::<f64>() / lengths.len() as f64;
        if mean > 0.0 {
            let variance =
                lengths.iter().map(|l| (l - mean).powi(2)).sum::<f64>() / lengths.len() as f64;
            let cv = variance.sqrt() / mean; // coefficient of variation
            // Very low CV (< 0.2) means all sentences same length = robotic
            if cv < 0.2 {
                signals.push(1.0 - cv / 0.2);
            }
        }
    }

    if signals.is_empty() {
        0.0
    } else {
        signals.iter().sum::<f64>() / signals.len() as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_short_content_fails() {
        let result = check_quality("too short");
        assert!(!result.passed);
        assert!(result.reason.unwrap().contains("too short"));
    }

    #[test]
    fn test_link_heavy_fails() {
        let mut content = String::new();
        for i in 0..20 {
            content.push_str(&format!("https://example.com/{i}\n"));
        }
        content.push_str("some actual content here that is not a link\n");
        // 20/21 lines are links = 95% > 80%
        let result = check_quality(&content);
        assert!(!result.passed);
        assert!(result.reason.unwrap().contains("links"));
    }

    #[test]
    fn test_good_content_passes() {
        let content = "Rust is a systems programming language focused on safety and concurrency. \
            Its ownership model prevents data races at compile time. The borrow checker ensures \
            references are always valid. Pattern matching provides exhaustive checks on enum variants. \
            Traits enable polymorphism without inheritance. Error handling uses Result and Option types \
            for explicit control flow. The standard library provides efficient collections like Vec, \
            HashMap, and BTreeMap. Cargo manages dependencies and builds. Clippy catches common mistakes. \
            Rustfmt enforces consistent formatting across projects.";
        let result = check_quality(content);
        assert!(result.passed);
        assert!(result.quality_score > 0.0);
        assert!(result.spam_score < 0.5);
        assert!(result.slop_score < 0.5);
    }

    #[test]
    fn test_quality_score_caps_at_1() {
        let content = "word ".repeat(1000);
        let result = check_quality(&content);
        assert!(result.quality_score <= 1.0);
    }

    // Spam detection tests

    #[test]
    fn test_keyword_stuffing_detected() {
        // 50% keyword density
        let content = "rust rust rust rust rust other words here to pad the content out a bit more so it passes the length check and we can test spam detection properly. rust rust rust rust rust more padding needed here.";
        let score = compute_spam_score(content);
        assert!(score > 0.3, "keyword stuffing not detected: {score}");
    }

    #[test]
    fn test_repeated_trigrams_detected() {
        let content = "buy now cheap buy now cheap buy now cheap buy now cheap buy now cheap buy now cheap buy now cheap and some other words to make it long enough for testing.";
        let score = compute_spam_score(content);
        assert!(score > 0.3, "repeated trigrams not detected: {score}");
    }

    #[test]
    fn test_all_caps_detected() {
        let content = "THIS IS ALL CAPS CONTENT THAT SHOULD BE FLAGGED AS SPAM BECAUSE IT IS VERY LOUD AND OBNOXIOUS AND NOBODY LIKES READING ALL CAPS TEXT ON THE INTERNET";
        let score = compute_spam_score(content);
        assert!(score > 0.3, "all-caps not detected: {score}");
    }

    #[test]
    fn test_clean_content_low_spam() {
        let content = "Rust is a systems programming language focused on safety, speed, and concurrency. It achieves memory safety without garbage collection through its ownership system.";
        let score = compute_spam_score(content);
        assert!(score < 0.2, "clean content has high spam score: {score}");
    }

    // AI slop detection tests

    #[test]
    fn test_stock_phrases_alone_do_not_score() {
        // Wording is not evidence of origin or quality: a text full of stock
        // phrases but otherwise varied is not flagged.
        let content = "In today's rapidly evolving technological landscape, it is important to note that artificial intelligence has become a game-changer. This comprehensive guide will explore the synergy between modern approaches and leverage cutting-edge solutions for a holistic approach to the problem.";
        assert_eq!(compute_slop_score(content), 0.0);
    }

    #[test]
    fn test_function_words_are_not_stuffing_in_any_language() {
        let portuguese = "O banco de dados do sistema de pagamentos do país foi criado pela equipe de engenharia de software da instituição, que cuidou da integração de cada uma das partes de forma gradual ao longo de vários anos de trabalho.";
        assert_eq!(compute_spam_score(portuguese), 0.0);
    }

    #[test]
    fn test_low_vocabulary_diversity() {
        let content = "the the the the the the the the the the the the the the the the the the the the is is is is is is is is is is a a a a a a a a a a";
        let score = compute_slop_score(content);
        assert!(score > 0.3, "low diversity not detected: {score}");
    }

    #[test]
    fn test_clean_content_low_slop() {
        let content = "Rust's ownership model prevents data races at compile time. The borrow checker ensures references are always valid. Pattern matching provides exhaustive checks on enum variants. Traits enable polymorphism without inheritance. Error handling uses Result and Option types for explicit control flow.";
        let score = compute_slop_score(content);
        assert!(score < 0.3, "clean content has high slop score: {score}");
    }

    #[test]
    fn test_spam_content_rejected_by_check_quality() {
        // Keyword stuffing that should trigger spam rejection
        let content = "crypto crypto crypto crypto crypto crypto crypto crypto crypto crypto crypto crypto crypto crypto crypto crypto crypto crypto crypto crypto crypto crypto other word here for padding the content to pass length check minimum requirement.";
        let result = check_quality(content);
        assert!(!result.passed, "spam content should be rejected");
        assert!(result.spam_score > 0.5);
    }

    #[test]
    fn test_quality_check_includes_scores() {
        let content = "This is a normal article about programming in Rust. It covers topics like ownership, borrowing, and lifetimes. The type system prevents many common bugs at compile time.";
        let result = check_quality(content);
        assert!(result.passed);
        // Scores should be populated even for passing content
        assert!(result.spam_score >= 0.0);
        assert!(result.slop_score >= 0.0);
    }

    // Quality profile tests

    #[test]
    fn test_social_post_profile_accepts_short() {
        // 30 chars — passes SocialPost (min 20) but fails Standard (min 100)
        let content = "Rust 2.0 just dropped! Check it out, amazing features.";
        let social = check_quality_with_profile(content, QualityProfile::SocialPost);
        assert!(social.passed, "SocialPost should accept short content");

        let standard = check_quality_with_profile(content, QualityProfile::Standard);
        assert!(!standard.passed, "Standard should reject short content");
    }

    #[test]
    fn test_video_description_profile() {
        let content = "In this video we explore the fundamentals of linear algebra.";
        let result = check_quality_with_profile(content, QualityProfile::VideoDescription);
        assert!(result.passed);
    }

    #[test]
    fn test_social_post_profile_rejects_too_short() {
        let content = "Hi";
        let result = check_quality_with_profile(content, QualityProfile::SocialPost);
        assert!(!result.passed);
    }

    #[test]
    fn test_profile_link_ratio_thresholds() {
        // 90% links: fails Standard (80% max), passes SocialPost (95% max)
        let mut content = String::new();
        for i in 0..9 {
            content.push_str(&format!("https://example.com/{i}\n"));
        }
        content.push_str("Some actual content that is not a link\n");

        let standard = check_quality_with_profile(&content, QualityProfile::Standard);
        assert!(!standard.passed, "Standard should reject 90% links");

        let social = check_quality_with_profile(&content, QualityProfile::SocialPost);
        assert!(social.passed, "SocialPost should accept 90% links");
    }
}
