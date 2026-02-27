use oi_facts::MockFactExtractor;
use oi_types::facts::FactExtractionResult;
use oi_types::traits::FactExtractionPipeline;
use oi_types::{ChunkMeta, ContentChunk};
use std::sync::Arc;

#[test]
fn test_mock_fact_extraction_english() {
    let ext = MockFactExtractor::new();
    let text = "Microsoft Corporation was founded by Bill Gates and Paul Allen in Albuquerque, New Mexico.";
    let result = ext.extract(text).unwrap();

    assert!(!result.entities.is_empty(), "should find entities");
    let mentions: Vec<&str> = result.entities.iter().map(|e| e.mention.as_str()).collect();
    // Should detect capitalized sequences
    assert!(
        mentions.iter().any(|m| m.contains("Microsoft")),
        "should find Microsoft, got: {mentions:?}"
    );
    assert!(
        mentions.iter().any(|m| m.contains("Bill Gates")),
        "should find Bill Gates, got: {mentions:?}"
    );
}

#[test]
fn test_fact_extraction_trait_object() {
    // Verify dynamic dispatch through Arc<dyn FactExtractionPipeline>
    let pipeline: Arc<dyn FactExtractionPipeline> = Arc::new(MockFactExtractor::new());
    let result = pipeline
        .extract("Apple was founded by Steve Jobs.")
        .unwrap();
    assert!(!result.entities.is_empty());
    assert_eq!(pipeline.name(), "mock");
}

#[test]
fn test_facts_in_crawl_pipeline_flow() {
    // Simulate extracting facts from CrawlOutput-style chunks
    let ext = MockFactExtractor::new();
    let chunks = vec![
        "Tesla announced a new Gigafactory in Berlin, Germany.",
        "The factory will produce 500,000 vehicles per year.",
        "CEO Elon Musk attended the opening ceremony.",
    ];

    let results: Vec<FactExtractionResult> = chunks
        .iter()
        .map(|c| ext.extract(c).unwrap())
        .collect();

    // Each chunk should produce some extraction result
    assert_eq!(results.len(), 3);
    // Entity-rich chunks should have more entities
    let chunk0_entities = results[0].entities.len();
    assert!(
        chunk0_entities > 0,
        "First chunk (with Tesla, Berlin, Germany) should have entities"
    );
}

#[test]
fn test_batch_extraction_matches_individual() {
    let ext = MockFactExtractor::new();
    let texts = &[
        "Google was founded by Larry Page.",
        "Amazon is headquartered in Seattle.",
        "Apple released the iPhone in 2007.",
    ];

    let batch = ext.extract_batch(texts).unwrap();
    assert_eq!(batch.len(), 3);

    for (i, text) in texts.iter().enumerate() {
        let single = ext.extract(text).unwrap();
        assert_eq!(
            batch[i].entities.len(),
            single.entities.len(),
            "batch[{i}] entity count should match single extraction"
        );
        for (be, se) in batch[i].entities.iter().zip(single.entities.iter()) {
            assert_eq!(be.mention, se.mention);
            assert_eq!(be.entity_type, se.entity_type);
            assert_eq!(be.confidence, se.confidence);
        }
    }
}

#[test]
fn test_facts_json_roundtrip() {
    let ext = MockFactExtractor::new();
    let result = ext
        .extract("Barack Obama was the 44th president of the United States.")
        .unwrap();

    let json = serde_json::to_string(&result).unwrap();
    let back: FactExtractionResult = serde_json::from_str(&json).unwrap();

    assert_eq!(back.entities.len(), result.entities.len());
    assert_eq!(back.facts.len(), result.facts.len());
    for (a, b) in back.entities.iter().zip(result.entities.iter()) {
        assert_eq!(a.mention, b.mention);
        assert_eq!(a.start, b.start);
        assert_eq!(a.end, b.end);
    }
}

#[test]
fn test_facts_attached_to_chunk_meta() {
    let ext = MockFactExtractor::new();
    let text = "SpaceX launched Falcon 9 from Cape Canaveral.";
    let facts = ext.extract(text).unwrap();

    // Simulate attaching facts to ChunkMeta
    let chunk = ContentChunk {
        chunk_id: "test_001".to_string(),
        content: text.to_string(),
        embedding: None,
        seq: 0,
        meta: Some(ChunkMeta {
            url: "https://example.com/spacex".to_string(),
            title: "SpaceX Launch".to_string(),
            domain: "example.com".to_string(),
            content_hash: "abc123".to_string(),
            facts: Some(facts),
            ..Default::default()
        }),
    };

    let meta = chunk.meta.as_ref().unwrap();
    assert!(meta.facts.is_some(), "facts should be attached to ChunkMeta");
    let attached = meta.facts.as_ref().unwrap();
    assert!(!attached.entities.is_empty());

    // JSON roundtrip with facts in ChunkMeta
    let json = serde_json::to_string(&chunk).unwrap();
    let back: ContentChunk = serde_json::from_str(&json).unwrap();
    let back_facts = back.meta.unwrap().facts.unwrap();
    assert_eq!(back_facts.entities.len(), attached.entities.len());
}
