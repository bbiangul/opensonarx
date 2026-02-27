use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A named entity extracted from text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedEntity {
    /// Surface form as it appears in text: "Barack Obama"
    pub mention: String,
    /// Canonical/normalized form: "barack obama"
    pub normalized: String,
    /// Entity type: "person", "organization", "location", "date", etc.
    pub entity_type: String,
    /// Character offset start (inclusive)
    pub start: usize,
    /// Character offset end (exclusive)
    pub end: usize,
    /// Confidence score 0.0-1.0
    pub confidence: f64,
}

/// A structured fact triplet: (subject, predicate, object).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fact {
    /// The subject entity of this fact
    pub subject: ExtractedEntity,
    /// The relation type: "founded_by", "born_in", "capital_of", etc.
    pub predicate: String,
    /// The object entity of this fact
    pub object: ExtractedEntity,
    /// Confidence score 0.0-1.0
    pub confidence: f64,
}

/// Result of running fact extraction on a piece of text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactExtractionResult {
    /// All entities found in the text
    pub entities: Vec<ExtractedEntity>,
    /// Structured fact triplets
    pub facts: Vec<Fact>,
    /// Detected language (ISO 639-1)
    pub language: Option<String>,
    /// Time taken for extraction in milliseconds
    pub extraction_time_ms: f64,
}

impl Default for FactExtractionResult {
    fn default() -> Self {
        Self {
            entities: Vec::new(),
            facts: Vec::new(),
            language: None,
            extraction_time_ms: 0.0,
        }
    }
}

impl FactExtractionResult {
    /// Return entities with confidence above the given threshold.
    pub fn entities_above(&self, threshold: f64) -> Vec<&ExtractedEntity> {
        self.entities
            .iter()
            .filter(|e| e.confidence >= threshold)
            .collect()
    }

    /// Return facts with confidence above the given threshold.
    pub fn facts_above(&self, threshold: f64) -> Vec<&Fact> {
        self.facts
            .iter()
            .filter(|f| f.confidence >= threshold)
            .collect()
    }

    /// Count entities grouped by type.
    pub fn entity_type_count(&self) -> HashMap<String, usize> {
        let mut counts = HashMap::new();
        for e in &self.entities {
            *counts.entry(e.entity_type.clone()).or_insert(0) += 1;
        }
        counts
    }

    /// Compute fact density: facts per word.
    /// Returns 0.0 if word_count is 0.
    pub fn fact_density(&self, word_count: usize) -> f64 {
        if word_count == 0 {
            return 0.0;
        }
        self.facts.len() as f64 / word_count as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_entity(mention: &str, entity_type: &str, confidence: f64) -> ExtractedEntity {
        ExtractedEntity {
            mention: mention.to_string(),
            normalized: mention.to_lowercase(),
            entity_type: entity_type.to_string(),
            start: 0,
            end: mention.len(),
            confidence,
        }
    }

    fn sample_result() -> FactExtractionResult {
        let e1 = sample_entity("Microsoft", "organization", 0.95);
        let e2 = sample_entity("Bill Gates", "person", 0.90);
        let e3 = sample_entity("Albuquerque", "location", 0.40);
        FactExtractionResult {
            entities: vec![e1.clone(), e2.clone(), e3],
            facts: vec![Fact {
                subject: e1,
                predicate: "founded_by".to_string(),
                object: e2,
                confidence: 0.85,
            }],
            language: Some("en".to_string()),
            extraction_time_ms: 12.5,
        }
    }

    #[test]
    fn test_serde_roundtrip() {
        let result = sample_result();
        let json = serde_json::to_string(&result).unwrap();
        let back: FactExtractionResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back.entities.len(), 3);
        assert_eq!(back.facts.len(), 1);
        assert_eq!(back.language, Some("en".to_string()));
    }

    #[test]
    fn test_entities_above_threshold() {
        let result = sample_result();
        let high = result.entities_above(0.80);
        assert_eq!(high.len(), 2); // Microsoft(0.95) + Bill Gates(0.90)
        let very_high = result.entities_above(0.95);
        assert_eq!(very_high.len(), 1); // Microsoft only
    }

    #[test]
    fn test_facts_above_threshold() {
        let result = sample_result();
        let above = result.facts_above(0.80);
        assert_eq!(above.len(), 1);
        let none = result.facts_above(0.99);
        assert_eq!(none.len(), 0);
    }

    #[test]
    fn test_default() {
        let result = FactExtractionResult::default();
        assert!(result.entities.is_empty());
        assert!(result.facts.is_empty());
        assert!(result.language.is_none());
        assert_eq!(result.extraction_time_ms, 0.0);
    }

    #[test]
    fn test_entity_type_count() {
        let result = sample_result();
        let counts = result.entity_type_count();
        assert_eq!(counts.get("organization"), Some(&1));
        assert_eq!(counts.get("person"), Some(&1));
        assert_eq!(counts.get("location"), Some(&1));
    }

    #[test]
    fn test_fact_density() {
        let result = sample_result();
        let density = result.fact_density(100);
        assert!((density - 0.01).abs() < 1e-9); // 1 fact / 100 words
    }

    #[test]
    fn test_fact_density_zero_words() {
        let result = sample_result();
        assert_eq!(result.fact_density(0), 0.0);
    }

    #[test]
    fn test_entity_offsets() {
        let entity = sample_entity("Barack Obama", "person", 0.99);
        assert_eq!(entity.start, 0);
        assert_eq!(entity.end, 12);
        assert!(entity.end > entity.start);
    }
}
