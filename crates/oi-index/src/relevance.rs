use oi_types::facts::FactExtractionResult;
use std::collections::HashSet;

/// Public API: split content into sentences with byte offsets.
///
/// Returns `(text, byte_start, byte_end)` tuples. Short fragments (<20 chars)
/// are merged into the preceding sentence.
pub fn split_into_sentences(content: &str) -> Vec<(String, usize, usize)> {
    split_sentences(content)
}

/// Extract relevant sentences using pre-computed semantic embeddings.
///
/// Unlike `extract_relevant` (keyword-based), this scores each sentence by cosine
/// similarity with the query embedding. Entity offsets provide an additive boost.
///
/// The caller is responsible for embedding sentences (via `embed_text_batch`)
/// and passing them in the same order as `split_into_sentences` produces.
///
/// Returns `None` when content is empty, no sentences score above the dynamic
/// threshold, or `sentence_embeddings` is empty.
///
/// The threshold is `max(FLOOR, best_sentence_score * RELATIVE_FACTOR)` — this
/// adapts to each document: sentences must be at least 75% as similar as the best
/// match, with an absolute floor of 0.5 to exclude genuinely off-topic content.
pub fn extract_relevant_semantic(
    content: &str,
    query_embedding: &[f32],
    sentence_embeddings: &[Vec<f32>],
    facts: Option<&FactExtractionResult>,
) -> Option<String> {
    /// Absolute floor: sentences below this are never included.
    const FLOOR: f64 = 0.5;
    /// Relative factor: sentences must score at least this fraction of the best.
    /// 0.70 avoids over-filtering when one sentence dominates (e.g. 0.74 vs 0.55).
    const RELATIVE_FACTOR: f64 = 0.70;

    if content.is_empty() || sentence_embeddings.is_empty() {
        return None;
    }

    let sentences = split_sentences(content);
    if sentences.is_empty() {
        return None;
    }

    // Score all sentences (cosine sim + entity boost)
    let all_scores: Vec<(usize, f64)> = sentences
        .iter()
        .enumerate()
        .map(|(i, (_text, byte_start, byte_end))| {
            let sem_score = sentence_embeddings
                .get(i)
                .map(|emb| cosine_sim(query_embedding, emb).max(0.0) as f64)
                .unwrap_or(0.0);

            // Entity boost: additive +0.2 * confidence for overlapping entities
            let entity_boost = facts
                .map(|f| {
                    f.entities
                        .iter()
                        .filter(|e| e.start < *byte_end && e.end > *byte_start)
                        .map(|e| 0.2 * e.confidence)
                        .sum::<f64>()
                })
                .unwrap_or(0.0);

            (i, sem_score + entity_boost)
        })
        .collect();

    let best_score = all_scores
        .iter()
        .map(|(_, s)| *s)
        .fold(0.0f64, f64::max);

    // Dynamic threshold: relative to best, but never below FLOOR
    let threshold = FLOOR.max(best_score * RELATIVE_FACTOR);

    let mut scored: Vec<(usize, f64)> = all_scores
        .into_iter()
        .filter(|(_, score)| *score >= threshold)
        .collect();

    if scored.is_empty() {
        return None;
    }

    // Top 5 by score
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(5);

    // Re-sort by original position (reading order)
    scored.sort_by_key(|&(i, _)| i);

    let relevant: String = scored
        .iter()
        .map(|&(i, _)| sentences[i].0.as_str())
        .collect::<Vec<_>>()
        .join(" ");

    if relevant.is_empty() { None } else { Some(relevant) }
}

/// Cosine similarity between two vectors (local helper for relevance scoring).
fn cosine_sim(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot / (norm_a * norm_b)
    }
}

/// Extract only the sentences from `content` that are relevant to `query`,
/// using keyword matching and (optionally) entity byte offsets from fact extraction.
///
/// Returns `None` when content is empty, no sentences score above threshold,
/// or there are no keywords and no facts to score against.
pub fn extract_relevant(
    content: &str,
    query: &str,
    facts: Option<&FactExtractionResult>,
) -> Option<String> {
    if content.is_empty() {
        return None;
    }

    let keywords = tokenize_query(query);
    let has_facts = facts.map(|f| !f.entities.is_empty()).unwrap_or(false);

    if keywords.is_empty() && !has_facts {
        return None;
    }

    let sentences = split_sentences(content);
    if sentences.is_empty() {
        return None;
    }

    let mut scored: Vec<(usize, f64)> = sentences
        .iter()
        .enumerate()
        .filter_map(|(i, (text, byte_start, byte_end))| {
            let score = score_sentence(text, *byte_start, *byte_end, &keywords, facts);
            if score >= 0.5 {
                Some((i, score))
            } else {
                None
            }
        })
        .collect();

    if scored.is_empty() {
        return None;
    }

    // Top 5 by score
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(5);

    // Re-sort by original position (reading order)
    scored.sort_by_key(|&(i, _)| i);

    let relevant: String = scored
        .iter()
        .map(|&(i, _)| sentences[i].0.as_str())
        .collect::<Vec<_>>()
        .join(" ");

    if relevant.is_empty() { None } else { Some(relevant) }
}

/// Split content into sentences, tracking byte offsets into the original `content`.
/// Splits on sentence-ending punctuation (.!?) followed by whitespace or EOF,
/// and on double-newlines (paragraph breaks). Single newlines do NOT split.
/// Triple-backtick fenced code blocks are kept atomic (no splitting inside them).
/// Merges fragments shorter than 20 chars: headings (`#`) merge forward into the
/// next sentence, other short fragments merge backward into the previous sentence.
fn split_sentences(content: &str) -> Vec<(String, usize, usize)> {
    let mut sentences: Vec<(String, usize, usize)> = Vec::new();
    let mut current_start: usize = 0; // byte offset into content
    let mut current = String::new();
    let chars: Vec<char> = content.chars().collect();
    let mut byte_pos: usize = 0; // running byte offset
    let mut i = 0;
    let mut in_code_block = false;

    while i < chars.len() {
        let ch = chars[i];
        let ch_len = ch.len_utf8();

        // Toggle code-block state on ``` fences; consume all 3 backticks together
        if ch == '`' && i + 2 < chars.len() && chars[i + 1] == '`' && chars[i + 2] == '`' {
            in_code_block = !in_code_block;
            current.push(ch);
            byte_pos += ch_len;
            current.push(chars[i + 1]);
            byte_pos += chars[i + 1].len_utf8();
            current.push(chars[i + 2]);
            byte_pos += chars[i + 2].len_utf8();
            i += 3;
            continue;
        }

        current.push(ch);
        byte_pos += ch_len;

        // Only check sentence boundaries when NOT inside a code block
        if !in_code_block {
            let is_sentence_end = (ch == '.' || ch == '!' || ch == '?')
                && (i + 1 >= chars.len() || chars[i + 1].is_whitespace());

            // Only break on double-newline (paragraph boundary), not single newlines
            let is_para_break = ch == '\n'
                && i + 1 < chars.len()
                && chars[i + 1] == '\n';

            if is_sentence_end || is_para_break {
                let trimmed = current.trim().to_string();
                if !trimmed.is_empty() {
                    sentences.push((trimmed, current_start, byte_pos));
                }
                // Skip whitespace after sentence end
                i += 1;
                while i < chars.len() && chars[i].is_whitespace() {
                    byte_pos += chars[i].len_utf8();
                    i += 1;
                }
                current_start = byte_pos;
                current.clear();
                continue;
            }
        }

        i += 1;
    }

    // Remainder
    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        sentences.push((trimmed, current_start, content.len()));
    }

    // Merge short fragments (<20 chars):
    // - Heading fragments (start with '#') merge FORWARD into the next sentence
    // - Other short fragments merge BACKWARD into the previous sentence
    let sentences_count = sentences.len();
    let mut merged: Vec<(String, usize, usize)> = Vec::new();
    for (idx, (text, start, end)) in sentences.into_iter().enumerate() {
        if text.len() < 20 {
            if text.starts_with('#') && idx + 1 < sentences_count {
                // Heading fragment with more sentences to come: push for forward merge
                merged.push((text, start, end));
            } else if !merged.is_empty() {
                // Non-heading short fragment (or last heading): merge into previous
                let last = merged.last_mut().unwrap();
                last.0.push(' ');
                last.0.push_str(&text);
                last.2 = end;
            } else {
                merged.push((text, start, end));
            }
        } else {
            // Normal-length sentence: if previous is a heading fragment, prepend it
            if let Some(prev) = merged.last()
                && prev.0.starts_with('#')
                && prev.0.len() < 20
            {
                let prev = merged.pop().unwrap();
                let combined = format!("{} {}", prev.0, text);
                merged.push((combined, prev.1, end));
            } else {
                merged.push((text, start, end));
            }
        }
    }

    merged
}

/// Tokenize query into distinct lowercase keywords, stripping punctuation, skipping <2 chars.
fn tokenize_query(query: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    query
        .split_whitespace()
        .map(|w| w.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase())
        .filter(|w| w.len() >= 2 && seen.insert(w.clone()))
        .collect()
}

/// Score a sentence based on whole-word keyword matches and entity overlaps.
/// `byte_start`/`byte_end` are byte offsets into the original content string,
/// matching the byte offsets stored in `ExtractedEntity.start`/`.end`.
fn score_sentence(
    text: &str,
    byte_start: usize,
    byte_end: usize,
    keywords: &[String],
    facts: Option<&FactExtractionResult>,
) -> f64 {
    let text_lower = text.to_lowercase();

    // Keyword score: count of distinct query keywords found as whole words
    let keyword_score = keywords
        .iter()
        .filter(|kw| contains_word(&text_lower, kw))
        .count() as f64;

    // Entity score: sum of confidence for entities whose [start, end) overlaps this sentence
    let entity_score = facts
        .map(|f| {
            f.entities
                .iter()
                .filter(|e| e.start < byte_end && e.end > byte_start)
                .map(|e| e.confidence)
                .sum::<f64>()
        })
        .unwrap_or(0.0);

    entity_score * 2.0 + keyword_score
}

/// Check if `haystack` contains `needle` as a whole word (not a substring).
/// Word boundaries are non-alphanumeric characters or string edges.
fn contains_word(haystack: &str, needle: &str) -> bool {
    let mut start = 0;
    while let Some(pos) = haystack[start..].find(needle) {
        let abs_pos = start + pos;
        let before_ok = abs_pos == 0
            || !haystack.as_bytes()[abs_pos - 1].is_ascii_alphanumeric();
        let after_pos = abs_pos + needle.len();
        let after_ok = after_pos >= haystack.len()
            || !haystack.as_bytes()[after_pos].is_ascii_alphanumeric();
        if before_ok && after_ok {
            return true;
        }
        start = abs_pos + needle.len();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use oi_types::facts::{ExtractedEntity, FactExtractionResult};

    #[test]
    fn test_empty_content_returns_none() {
        assert!(extract_relevant("", "test query", None).is_none());
    }

    #[test]
    fn test_no_keywords_no_facts_returns_none() {
        let content = "This is some content. It has multiple sentences.";
        assert!(extract_relevant(content, "", None).is_none());
    }

    #[test]
    fn test_keyword_only_selects_matching() {
        let content = "Rust is a systems programming language. \
                        Python is great for data science. \
                        Rust provides memory safety without garbage collection. \
                        JavaScript runs in the browser.";
        let result = extract_relevant(content, "rust programming", None);
        assert!(result.is_some());
        let text = result.unwrap();
        assert!(text.contains("Rust is a systems programming language"));
        assert!(text.contains("Rust provides memory safety"));
        // Should NOT contain unrelated sentences
        assert!(!text.contains("JavaScript runs"));
    }

    #[test]
    fn test_entity_boosted_sentences_rank_higher() {
        let content = "Barack Obama was president. The weather is nice today. Obama signed the act.";
        let facts = FactExtractionResult {
            entities: vec![ExtractedEntity {
                mention: "Barack Obama".to_string(),
                normalized: "barack obama".to_string(),
                entity_type: "person".to_string(),
                start: 0,
                end: 12,
                confidence: 0.95,
            }],
            facts: vec![],
            language: None,
            extraction_time_ms: 0.0,
        };
        let result = extract_relevant(content, "obama", Some(&facts));
        assert!(result.is_some());
        let text = result.unwrap();
        // First sentence should be included (entity overlap + keyword)
        assert!(text.contains("Barack Obama was president"));
    }

    #[test]
    fn test_no_matches_returns_none() {
        let content = "The weather is sunny. Birds are singing. Trees are green.";
        assert!(extract_relevant(content, "quantum computing", None).is_none());
    }

    #[test]
    fn test_non_ascii_content() {
        let content = "Rust é uma linguagem de programação. Python é popular no Brasil. Rust oferece segurança de memória.";
        let result = extract_relevant(content, "rust programação", None);
        assert!(result.is_some());
        let text = result.unwrap();
        assert!(text.contains("Rust"));
    }

    #[test]
    fn test_max_five_sentences() {
        let content = "Rust is great for systems programming. \
                        Rust provides memory safety guarantees. \
                        Rust has a strong type system built in. \
                        Rust compiles to native code efficiently. \
                        Rust has excellent error handling patterns. \
                        Rust supports async await for concurrency. \
                        Rust has a growing ecosystem of crates.";
        let result = extract_relevant(content, "rust", None);
        assert!(result.is_some());
        let text = result.unwrap();
        // Count sentences by splitting on period-space pattern
        let sentences: Vec<&str> = text.split(". ")
            .filter(|s| !s.is_empty())
            .collect();
        assert!(sentences.len() <= 5, "got {} sentences: {:?}", sentences.len(), sentences);
    }

    #[test]
    fn test_reading_order_preserved() {
        let content = "First about rust. Second unrelated. Third about rust programming. Fourth about rust safety.";
        let result = extract_relevant(content, "rust programming", None);
        assert!(result.is_some());
        let text = result.unwrap();
        // "First" should come before "Third" in the output
        if text.contains("First") && text.contains("Third") {
            let first_pos = text.find("First").unwrap();
            let third_pos = text.find("Third").unwrap();
            assert!(first_pos < third_pos);
        }
    }

    #[test]
    fn test_tokenize_query() {
        let tokens = tokenize_query("What is Rust?");
        assert_eq!(tokens, vec!["what", "is", "rust"]);
    }

    #[test]
    fn test_tokenize_query_strips_short() {
        let tokens = tokenize_query("a Rust b");
        assert_eq!(tokens, vec!["rust"]);
    }

    #[test]
    fn test_tokenize_query_deduplicates() {
        let tokens = tokenize_query("rust Rust RUST");
        assert_eq!(tokens, vec!["rust"]);
    }

    #[test]
    fn test_whole_word_matching() {
        // "art" should NOT match "started" as a substring
        let content = "She started painting early in the morning every single day. \
                        Art is her greatest passion in life and career. \
                        The gallery started showing new and innovative works.";
        let result = extract_relevant(content, "art", None);
        assert!(result.is_some());
        let text = result.unwrap();
        assert!(text.contains("Art is her greatest passion"));
        // "started" sentences should not match "art" as a substring
        assert!(!text.contains("started painting"));
        assert!(!text.contains("gallery started"));
    }

    #[test]
    fn test_single_newline_does_not_split() {
        // Single newline in markdown should NOT be a sentence boundary
        let content = "This is a long sentence that\ncontinues on the next line about rust.";
        let result = extract_relevant(content, "rust", None);
        assert!(result.is_some());
        let text = result.unwrap();
        // Should be one sentence, not split at \n
        assert!(text.contains("continues on the next line"));
    }

    #[test]
    fn test_double_newline_splits() {
        let content = "First paragraph about rust.\n\nSecond paragraph about python.";
        let result = extract_relevant(content, "rust", None);
        assert!(result.is_some());
        let text = result.unwrap();
        assert!(text.contains("rust"));
        assert!(!text.contains("python"));
    }

    #[test]
    fn test_split_sentences_byte_offsets() {
        // Sentences must be >= 20 chars to avoid merge
        let content = "Hello world and everyone. Goodbye world and everyone.";
        let sentences = split_sentences(content);
        assert_eq!(sentences.len(), 2);
        // "Hello world and everyone." = 25 bytes, starts at 0
        assert_eq!(sentences[0].1, 0);
        assert_eq!(sentences[0].2, 25);
        // "Goodbye world and everyone." starts at 26
        assert_eq!(sentences[1].1, 26);
        assert_eq!(sentences[1].2, content.len());
    }

    #[test]
    fn test_split_sentences_byte_offsets_multibyte() {
        // Non-ASCII: verify byte offsets with multi-byte chars
        // "Café is really quite great." — 'é' is 2 bytes
        let content = "Café is really quite great. Tea is also really quite good.";
        let sentences = split_sentences(content);
        assert_eq!(sentences.len(), 2);
        // "Café is really quite great." = 28 bytes ('é' = 2 bytes, rest ASCII)
        assert_eq!(sentences[0].1, 0);
        assert_eq!(sentences[0].2, 28);
        // Next sentence starts at byte 29
        assert_eq!(sentences[1].1, 29);
    }

    #[test]
    fn test_entity_offset_overlap_with_sentences() {
        // Verify entity byte offsets correctly overlap sentence byte ranges
        let content = "Microsoft was founded in 1975. Apple was founded in 1976.";
        let facts = FactExtractionResult {
            entities: vec![
                ExtractedEntity {
                    mention: "Microsoft".to_string(),
                    normalized: "microsoft".to_string(),
                    entity_type: "organization".to_string(),
                    start: 0,   // byte offset of "Microsoft" in content
                    end: 9,
                    confidence: 0.95,
                },
                ExtractedEntity {
                    mention: "Apple".to_string(),
                    normalized: "apple".to_string(),
                    entity_type: "organization".to_string(),
                    start: 31,  // byte offset of "Apple" in content
                    end: 36,
                    confidence: 0.90,
                },
            ],
            facts: vec![],
            language: None,
            extraction_time_ms: 0.0,
        };
        // Query "microsoft" should select first sentence (entity boost + keyword)
        let result = extract_relevant(content, "microsoft", Some(&facts));
        assert!(result.is_some());
        let text = result.unwrap();
        assert!(text.contains("Microsoft was founded"));
    }

    #[test]
    fn test_contains_word_boundary() {
        assert!(contains_word("hello world", "hello"));
        assert!(contains_word("hello world", "world"));
        assert!(!contains_word("helloworld", "hello"));
        assert!(contains_word("the art of code", "art"));
        assert!(!contains_word("started early", "art"));
        assert!(!contains_word("smart move", "art"));
    }

    // ── Semantic extraction tests ─────────────────────────────────────

    #[test]
    fn test_split_into_sentences_public_api() {
        let content = "Hello world and everyone. Goodbye world and everyone.";
        let sentences = split_into_sentences(content);
        assert_eq!(sentences.len(), 2);
        assert_eq!(sentences[0].0, "Hello world and everyone.");
        assert_eq!(sentences[1].0, "Goodbye world and everyone.");
    }

    #[test]
    fn test_semantic_empty_content() {
        let qe = vec![1.0f32, 0.0, 0.0];
        assert!(extract_relevant_semantic("", &qe, &[], None).is_none());
    }

    #[test]
    fn test_semantic_empty_embeddings() {
        let qe = vec![1.0f32, 0.0, 0.0];
        assert!(extract_relevant_semantic("Some content here.", &qe, &[], None).is_none());
    }

    #[test]
    fn test_semantic_selects_similar_sentences() {
        // Sentence 0: high similarity to query
        // Sentence 1: orthogonal (low similarity)
        // Sentence 2: moderate similarity
        let content = "Rust ensures memory safety well. \
                        The weather is nice today indeed. \
                        Rust prevents dangling pointers and double frees.";
        let query_emb = vec![1.0f32, 0.0, 0.0];
        let sentence_embs = vec![
            vec![0.9, 0.1, 0.0],   // high sim with query
            vec![0.0, 0.0, 1.0],   // orthogonal
            vec![0.8, 0.2, 0.0],   // moderate sim
        ];
        let result = extract_relevant_semantic(content, &query_emb, &sentence_embs, None);
        assert!(result.is_some());
        let text = result.unwrap();
        assert!(text.contains("memory safety"));
        assert!(text.contains("dangling pointers"));
        assert!(!text.contains("weather"));
    }

    #[test]
    fn test_semantic_threshold_filters_low_scores() {
        let content = "Sentence about nothing in particular here. Another irrelevant sentence about stuff.";
        let query_emb = vec![1.0f32, 0.0, 0.0];
        // Both sentences have very low similarity
        let sentence_embs = vec![
            vec![0.1, 0.1, 0.9],
            vec![0.0, 0.1, 0.9],
        ];
        let result = extract_relevant_semantic(content, &query_emb, &sentence_embs, None);
        assert!(result.is_none());
    }

    #[test]
    fn test_semantic_entity_boost() {
        let content = "Barack Obama signed the Affordable Care Act. The weather is nice.";
        let query_emb = vec![1.0f32, 0.0, 0.0];
        // cos([1,0,0], [a,b,c]) = a/sqrt(a^2+b^2+c^2)
        // With a=0.44, b=0.8, c=0.4: sim = 0.44/sqrt(0.9936) ≈ 0.441
        let sentence_embs = vec![
            vec![0.44, 0.8, 0.4],  // cosine sim ≈ 0.44, below 0.5 floor
            vec![0.0, 0.0, 1.0],   // orthogonal
        ];
        let facts = FactExtractionResult {
            entities: vec![ExtractedEntity {
                mention: "Barack Obama".to_string(),
                normalized: "barack obama".to_string(),
                entity_type: "person".to_string(),
                start: 0,
                end: 12,
                confidence: 0.95,
            }],
            facts: vec![],
            language: None,
            extraction_time_ms: 0.0,
        };
        // With entity boost: 0.44 + 0.2*0.95 = 0.63
        // best_score = 0.63, threshold = max(0.5, 0.63*0.75) = max(0.5, 0.47) = 0.5
        // 0.63 >= 0.5 → passes
        let result = extract_relevant_semantic(content, &query_emb, &sentence_embs, Some(&facts));
        assert!(result.is_some());
        let text = result.unwrap();
        assert!(text.contains("Barack Obama"));
    }

    #[test]
    fn test_semantic_max_five_and_reading_order() {
        // 7 sentences, all with high similarity
        let content = "First sentence about rust and memory. \
                        Second sentence about rust safety here. \
                        Third sentence about rust ownership model. \
                        Fourth sentence about rust concurrency. \
                        Fifth sentence about rust performance. \
                        Sixth sentence about rust ecosystem. \
                        Seventh sentence about rust tooling.";
        let query_emb = vec![1.0f32, 0.0, 0.0];
        let sentence_embs: Vec<Vec<f32>> = (0..7)
            .map(|i| vec![0.9 - i as f32 * 0.05, 0.1, 0.0])
            .collect();
        let result = extract_relevant_semantic(content, &query_emb, &sentence_embs, None);
        assert!(result.is_some());
        let text = result.unwrap();
        // Count sentences
        let count = text.split(". ").filter(|s| !s.is_empty()).count();
        assert!(count <= 5, "got {count} sentences");
        // First should come before Fifth
        if text.contains("First") && text.contains("Fifth") {
            assert!(text.find("First").unwrap() < text.find("Fifth").unwrap());
        }
    }

    #[test]
    fn test_cosine_sim_helper() {
        let a = vec![1.0f32, 0.0, 0.0];
        let b = vec![1.0f32, 0.0, 0.0];
        assert!((cosine_sim(&a, &b) - 1.0).abs() < 1e-6);

        let c = vec![0.0f32, 1.0, 0.0];
        assert!(cosine_sim(&a, &c).abs() < 1e-6);

        let zero = vec![0.0f32, 0.0, 0.0];
        assert_eq!(cosine_sim(&a, &zero), 0.0);
    }

    // ── Code block-aware splitting tests ────────────────────────────────

    #[test]
    fn test_split_sentences_code_block_preserved() {
        // Periods inside ``` fenced code blocks must NOT trigger sentence splits
        let content = "Create the file:\n```\napi . logger . info ( \"Hello!\" );\n```\nThe gateway discovers your plugin automatically.";
        let sentences = split_sentences(content);
        // "api", "logger", "info" must all be in the same sentence
        let code_sentences: Vec<_> = sentences
            .iter()
            .filter(|(t, _, _)| t.contains("api") && t.contains("logger") && t.contains("info"))
            .collect();
        assert_eq!(
            code_sentences.len(),
            1,
            "Code block content should stay in one sentence, got: {sentences:?}"
        );
    }

    #[test]
    fn test_split_sentences_multiple_code_blocks() {
        let content = "First install the dependency.\n```\nnpm install foo . bar\n```\nThen configure it properly.\n```\nconfig . set ( \"key\" , \"value\" );\n```\nYou are all done with the setup.";
        let sentences = split_sentences(content);
        // Each code block stays within its surrounding sentence
        let npm_sentences: Vec<_> = sentences
            .iter()
            .filter(|(t, _, _)| t.contains("npm install foo"))
            .collect();
        assert_eq!(
            npm_sentences.len(),
            1,
            "First code block should be in one sentence, got: {sentences:?}"
        );
        let config_sentences: Vec<_> = sentences
            .iter()
            .filter(|(t, _, _)| t.contains("config . set"))
            .collect();
        assert_eq!(
            config_sentences.len(),
            1,
            "Second code block should be in one sentence, got: {sentences:?}"
        );
    }

    #[test]
    fn test_heading_merges_forward() {
        // Short heading "## Resources" should merge into the NEXT sentence, not the previous one
        let content =
            "Previous section is done.\n\n## Resources\n\nHere are some useful links for further reading.";
        let sentences = split_sentences(content);
        // Find the sentence containing "Resources"
        let heading_sentence = sentences.iter().find(|(t, _, _)| t.contains("Resources"));
        assert!(
            heading_sentence.is_some(),
            "Should find sentence with Resources, got: {sentences:?}"
        );
        let (text, _, _) = heading_sentence.unwrap();
        // "Resources" should be combined with "Here are some useful links"
        assert!(
            text.contains("Resources") && text.contains("useful links"),
            "Heading should merge forward into next sentence, got: {text:?}"
        );
        // "Resources" should NOT be in the previous sentence
        if let Some((prev_text, _, _)) = sentences.iter().find(|(t, _, _)| t.contains("Previous"))
        {
            assert!(
                !prev_text.contains("Resources"),
                "Heading should not merge backward, got: {prev_text:?}"
            );
        }
    }

}
