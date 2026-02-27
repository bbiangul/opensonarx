use oi_facts::MockFactExtractor;
use oi_types::traits::FactExtractionPipeline;

fn assert_no_panic_and_valid(ext: &MockFactExtractor, text: &str, label: &str) {
    let result = ext.extract(text).unwrap();
    assert!(
        result.extraction_time_ms >= 0.0,
        "{label}: extraction_time_ms should be non-negative"
    );
    for entity in &result.entities {
        assert!(
            entity.start < entity.end,
            "{label}: entity '{}' has invalid offsets ({}, {})",
            entity.mention,
            entity.start,
            entity.end
        );
        assert!(
            entity.end <= text.len(),
            "{label}: entity '{}' end {} exceeds text len {}",
            entity.mention,
            entity.end,
            text.len()
        );
        assert!(
            entity.confidence >= 0.0 && entity.confidence <= 1.0,
            "{label}: entity '{}' confidence {} out of range",
            entity.mention,
            entity.confidence
        );
    }
}

#[test]
fn test_ner_english() {
    let ext = MockFactExtractor::new();
    let text = "Microsoft was founded by Bill Gates and Paul Allen in Albuquerque, New Mexico.";
    let result = ext.extract(text).unwrap();
    let mentions: Vec<&str> = result.entities.iter().map(|e| e.mention.as_str()).collect();
    assert!(
        mentions.iter().any(|m| m.contains("Microsoft")),
        "should find Microsoft, got: {mentions:?}"
    );
    assert!(
        mentions.iter().any(|m| m.contains("Bill Gates")),
        "should find Bill Gates, got: {mentions:?}"
    );
    assert!(
        mentions.iter().any(|m| m.contains("Paul Allen")),
        "should find Paul Allen, got: {mentions:?}"
    );
    assert_no_panic_and_valid(&ext, text, "en");
}

#[test]
fn test_ner_portuguese() {
    let ext = MockFactExtractor::new();
    let text = "O presidente Lula visitou São Paulo e Rio de Janeiro para discutir investimentos.";
    let result = ext.extract(text).unwrap();
    let mentions: Vec<&str> = result.entities.iter().map(|e| e.mention.as_str()).collect();
    assert!(
        mentions.iter().any(|m| m.contains("Lula")),
        "should find Lula, got: {mentions:?}"
    );
    assert!(
        mentions.iter().any(|m| m.contains("São Paulo") || m.contains("Paulo")),
        "should find São Paulo, got: {mentions:?}"
    );
    assert_no_panic_and_valid(&ext, text, "pt");
}

#[test]
fn test_ner_spanish() {
    let ext = MockFactExtractor::new();
    let text = "Felipe VI inauguró la nueva terminal del aeropuerto de Madrid junto con Pedro Sánchez.";
    let result = ext.extract(text).unwrap();
    let mentions: Vec<&str> = result.entities.iter().map(|e| e.mention.as_str()).collect();
    assert!(
        mentions.iter().any(|m| m.contains("Felipe")),
        "should find Felipe, got: {mentions:?}"
    );
    assert!(
        mentions.iter().any(|m| m.contains("Madrid")),
        "should find Madrid, got: {mentions:?}"
    );
    assert_no_panic_and_valid(&ext, text, "es");
}

#[test]
fn test_ner_german() {
    let ext = MockFactExtractor::new();
    let text = "Bundeskanzler Olaf Scholz traf sich mit Emmanuel Macron in Berlin.";
    let result = ext.extract(text).unwrap();
    let mentions: Vec<&str> = result.entities.iter().map(|e| e.mention.as_str()).collect();
    assert!(
        mentions.iter().any(|m| m.contains("Olaf Scholz")),
        "should find Olaf Scholz, got: {mentions:?}"
    );
    assert!(
        mentions.iter().any(|m| m.contains("Berlin")),
        "should find Berlin, got: {mentions:?}"
    );
    assert_no_panic_and_valid(&ext, text, "de");
}

#[test]
fn test_ner_french() {
    let ext = MockFactExtractor::new();
    let text = "Le président Emmanuel Macron a reçu Angela Merkel à Paris.";
    let result = ext.extract(text).unwrap();
    let mentions: Vec<&str> = result.entities.iter().map(|e| e.mention.as_str()).collect();
    assert!(
        mentions.iter().any(|m| m.contains("Emmanuel Macron")),
        "should find Emmanuel Macron, got: {mentions:?}"
    );
    assert!(
        mentions.iter().any(|m| m.contains("Paris")),
        "should find Paris, got: {mentions:?}"
    );
    assert_no_panic_and_valid(&ext, text, "fr");
}

#[test]
fn test_ner_chinese() {
    let ext = MockFactExtractor::new();
    let text = "习近平主席在北京会见了法国总统马克龙。";
    let result = ext.extract(text).unwrap();
    // Mock uses CJK character detection — should find some entities
    assert!(
        !result.entities.is_empty(),
        "should find CJK entities in Chinese text"
    );
    assert_no_panic_and_valid(&ext, text, "zh");
}

#[test]
fn test_ner_japanese() {
    let ext = MockFactExtractor::new();
    let text = "岸田首相は東京で会議を開催した。";
    let result = ext.extract(text).unwrap();
    assert!(
        !result.entities.is_empty(),
        "should find entities in Japanese text"
    );
    assert_no_panic_and_valid(&ext, text, "ja");
}

#[test]
fn test_multilingual_no_panic() {
    let ext = MockFactExtractor::new();
    let texts = &[
        ("en", "The United Nations General Assembly met in New York."),
        ("pt", "O Flamengo venceu o Palmeiras no Maracanã."),
        ("es", "El Real Madrid ganó la Liga de Campeones."),
        ("de", "Bayern München besiegte Borussia Dortmund in der Allianz Arena."),
        ("fr", "Kylian Mbappé a marqué un but au Parc des Princes."),
        ("zh", "腾讯公司总部位于深圳。"),
        ("ja", "トヨタ自動車は愛知県豊田市に本社を置く。"),
    ];

    for (lang, text) in texts {
        let result = ext.extract(text);
        assert!(
            result.is_ok(),
            "Extraction should not panic for language {lang}"
        );
        let r = result.unwrap();
        assert!(
            r.extraction_time_ms >= 0.0,
            "{lang}: negative extraction time"
        );
    }
}

#[test]
fn test_fact_density_varies_by_content() {
    let ext = MockFactExtractor::new();

    // Entity-rich text
    let rich = ext
        .extract(
            "Microsoft Corporation was founded by Bill Gates and Paul Allen \
             in Albuquerque, New Mexico on April 4, 1975. Later they moved to Redmond, Washington.",
        )
        .unwrap();

    // Entity-poor (SEO filler)
    let poor = ext
        .extract(
            "this is a really great amazing wonderful fantastic incredible piece of content \
             that you should definitely read because it is so very useful and helpful and great",
        )
        .unwrap();

    assert!(
        rich.entities.len() > poor.entities.len(),
        "Entity-rich text ({}) should have more entities than SEO filler ({})",
        rich.entities.len(),
        poor.entities.len()
    );
}
