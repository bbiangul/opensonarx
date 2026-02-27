use oi_types::facts::{ExtractedEntity, Fact, FactExtractionResult};
use oi_types::traits::FactExtractionPipeline;
use oi_types::Result;
use sha2::{Digest, Sha256};

use crate::normalizer;

/// Entity types assigned deterministically via hash of mention.
const ENTITY_TYPES: &[&str] = &[
    "person",
    "organization",
    "location",
    "date",
    "event",
    "product",
    "quantity",
    "money",
];

/// Relation types assigned deterministically via hash of entity pair.
const RELATION_TYPES: &[&str] = &[
    "founded_by",
    "born_in",
    "located_in",
    "capital_of",
    "ceo_of",
    "plays_for",
    "member_of",
    "part_of",
];

/// Mock fact extractor using deterministic rules.
///
/// Finds entities by detecting capitalized word sequences (Latin-script) or
/// runs of CJK characters. Assigns types and confidence via SHA-256 hash
/// for full determinism. Always available — no ONNX dependency.
pub struct MockFactExtractor;

impl MockFactExtractor {
    pub fn new() -> Self {
        Self
    }
}

impl Default for MockFactExtractor {
    fn default() -> Self {
        Self::new()
    }
}

/// Hash a string and return the first 8 bytes as a u64.
fn hash_u64(s: &str) -> u64 {
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    let result = hasher.finalize();
    u64::from_le_bytes(result[..8].try_into().unwrap())
}

/// Detect capitalized word sequences and CJK character runs as entity mentions.
fn detect_mentions(text: &str) -> Vec<(String, usize, usize)> {
    let mut mentions = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        // CJK character run detection
        if is_cjk(chars[i]) {
            let start = i;
            while i < len && is_cjk(chars[i]) {
                i += 1;
            }
            // Only consider runs of 2+ CJK characters
            if i - start >= 2 {
                let mention: String = chars[start..i].iter().collect();
                let byte_start = byte_offset(text, start);
                let byte_end = byte_offset(text, i);
                mentions.push((mention, byte_start, byte_end));
            }
            continue;
        }

        // Latin capitalized word sequences
        if chars[i].is_uppercase() {
            let start = i;
            // Consume the first capitalized word
            while i < len && (chars[i].is_alphanumeric() || chars[i] == '-' || chars[i] == '\'') {
                i += 1;
            }
            // Check for multi-word sequence (e.g., "Bill Gates", "New York City")
            while i < len && chars[i] == ' ' {
                let space_pos = i;
                i += 1;
                if i < len && chars[i].is_uppercase() {
                    // Continue with next capitalized word
                    while i < len
                        && (chars[i].is_alphanumeric() || chars[i] == '-' || chars[i] == '\'')
                    {
                        i += 1;
                    }
                } else {
                    i = space_pos;
                    break;
                }
            }
            let mention: String = chars[start..i].iter().collect();
            // Skip single-char "entities" and common sentence starters
            if mention.len() >= 2 && !is_common_word(&mention) {
                let byte_start = byte_offset(text, start);
                let byte_end = byte_offset(text, i);
                mentions.push((mention, byte_start, byte_end));
            }
        } else {
            i += 1;
        }
    }

    mentions
}

fn is_cjk(c: char) -> bool {
    matches!(c,
        '\u{4E00}'..='\u{9FFF}' |   // CJK Unified
        '\u{3400}'..='\u{4DBF}' |   // CJK Extension A
        '\u{3040}'..='\u{309F}' |   // Hiragana
        '\u{30A0}'..='\u{30FF}' |   // Katakana
        '\u{F900}'..='\u{FAFF}'     // CJK Compatibility
    )
}

/// Convert char index to byte offset.
fn byte_offset(text: &str, char_idx: usize) -> usize {
    text.char_indices()
        .nth(char_idx)
        .map(|(byte_pos, _)| byte_pos)
        .unwrap_or(text.len())
}

/// Filter out common sentence-starting words that aren't entities.
fn is_common_word(s: &str) -> bool {
    matches!(
        s,
        "The" | "This" | "That" | "These" | "Those" | "It" | "In" | "On" | "At" | "By" | "To"
            | "For" | "With" | "From" | "As" | "But" | "Or" | "And" | "If" | "So" | "No"
            | "Not" | "He" | "She" | "We" | "They" | "My" | "Your" | "His" | "Her" | "Its"
            | "Our" | "An" | "All" | "Each" | "Every" | "Some" | "Any" | "El" | "La" | "Le"
            | "Les" | "Der" | "Die" | "Das" | "Ein" | "Eine"
    )
}

impl FactExtractionPipeline for MockFactExtractor {
    fn extract(&self, text: &str) -> Result<FactExtractionResult> {
        let start = std::time::Instant::now();

        let raw_mentions = detect_mentions(text);

        let entities: Vec<ExtractedEntity> = raw_mentions
            .iter()
            .map(|(mention, start, end)| {
                let h = hash_u64(mention);
                let type_idx = (h % ENTITY_TYPES.len() as u64) as usize;
                let entity_type = ENTITY_TYPES[type_idx].to_string();
                let confidence = (h % 1000) as f64 / 1000.0;
                let normalized = normalizer::normalize_entity(mention, &entity_type);
                ExtractedEntity {
                    mention: mention.clone(),
                    normalized,
                    entity_type,
                    start: *start,
                    end: *end,
                    confidence,
                }
            })
            .collect();

        // Create relation triplets between consecutive entity pairs
        let mut facts = Vec::new();
        for pair in entities.windows(2) {
            let key = format!("{}:{}", pair[0].mention, pair[1].mention);
            let h = hash_u64(&key);
            let rel_idx = (h % RELATION_TYPES.len() as u64) as usize;
            let confidence = (h % 1000) as f64 / 1000.0;
            facts.push(Fact {
                subject: pair[0].clone(),
                predicate: RELATION_TYPES[rel_idx].to_string(),
                object: pair[1].clone(),
                confidence,
            });
        }

        let elapsed = start.elapsed().as_secs_f64() * 1000.0;

        Ok(FactExtractionResult {
            entities,
            facts,
            language: None,
            extraction_time_ms: elapsed,
        })
    }

    fn supported_entity_types(&self) -> &[&str] {
        ENTITY_TYPES
    }

    fn supported_relation_types(&self) -> &[&str] {
        RELATION_TYPES
    }

    fn name(&self) -> &str {
        "mock"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deterministic() {
        let ext = MockFactExtractor::new();
        let r1 = ext.extract("Microsoft was founded by Bill Gates.").unwrap();
        let r2 = ext.extract("Microsoft was founded by Bill Gates.").unwrap();
        assert_eq!(r1.entities.len(), r2.entities.len());
        for (a, b) in r1.entities.iter().zip(r2.entities.iter()) {
            assert_eq!(a.mention, b.mention);
            assert_eq!(a.entity_type, b.entity_type);
            assert_eq!(a.confidence, b.confidence);
        }
    }

    #[test]
    fn test_empty_text() {
        let ext = MockFactExtractor::new();
        let result = ext.extract("").unwrap();
        assert!(result.entities.is_empty());
        assert!(result.facts.is_empty());
    }

    #[test]
    fn test_entities_found() {
        let ext = MockFactExtractor::new();
        let result = ext
            .extract("Barack Obama was born in Honolulu, Hawaii.")
            .unwrap();
        assert!(result.entities.len() >= 2); // At least Barack Obama and Honolulu
        let mentions: Vec<&str> = result.entities.iter().map(|e| e.mention.as_str()).collect();
        assert!(mentions.iter().any(|m| m.contains("Barack Obama")));
    }

    #[test]
    fn test_facts_from_pairs() {
        let ext = MockFactExtractor::new();
        let result = ext
            .extract("Apple Inc was founded by Steve Jobs in Cupertino.")
            .unwrap();
        // Multiple entities means there should be facts between consecutive pairs
        if result.entities.len() >= 2 {
            assert!(!result.facts.is_empty());
            for fact in &result.facts {
                assert!(!fact.predicate.is_empty());
            }
        }
    }

    #[test]
    fn test_confidence_range() {
        let ext = MockFactExtractor::new();
        let result = ext
            .extract("Google was founded by Larry Page and Sergey Brin at Stanford University.")
            .unwrap();
        for entity in &result.entities {
            assert!(entity.confidence >= 0.0);
            assert!(entity.confidence <= 1.0);
        }
        for fact in &result.facts {
            assert!(fact.confidence >= 0.0);
            assert!(fact.confidence <= 1.0);
        }
    }

    #[test]
    fn test_offset_validity() {
        let text = "Microsoft Corporation headquarters in Redmond, Washington.";
        let ext = MockFactExtractor::new();
        let result = ext.extract(text).unwrap();
        for entity in &result.entities {
            assert!(entity.start < entity.end, "start must be < end");
            assert!(entity.end <= text.len(), "end must be <= text length");
            // Verify the mention matches what's at the offset
            let slice = &text[entity.start..entity.end];
            assert_eq!(slice, entity.mention, "offset must match mention text");
        }
    }

    #[test]
    fn test_name() {
        let ext = MockFactExtractor::new();
        assert_eq!(ext.name(), "mock");
    }

    #[test]
    fn test_batch_consistency() {
        let ext = MockFactExtractor::new();
        let texts = &[
            "Apple Inc was founded by Steve Jobs.",
            "Google is headquartered in Mountain View.",
        ];
        let batch = ext.extract_batch(texts).unwrap();
        assert_eq!(batch.len(), 2);
        for (i, text) in texts.iter().enumerate() {
            let single = ext.extract(text).unwrap();
            assert_eq!(batch[i].entities.len(), single.entities.len());
        }
    }

    #[test]
    fn test_supported_types() {
        let ext = MockFactExtractor::new();
        assert!(!ext.supported_entity_types().is_empty());
        assert!(!ext.supported_relation_types().is_empty());
        assert!(ext.supported_entity_types().contains(&"person"));
        assert!(ext.supported_relation_types().contains(&"founded_by"));
    }
}
