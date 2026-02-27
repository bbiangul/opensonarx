use oi_types::entity::{entity_from_url, EntityRef};
use oi_types::traits::SearchIndex;
use oi_types::{ChunkMeta, ContentChunk, QueryFilters, StakeInfo};
use std::collections::HashMap;
use std::sync::Arc;

// ── Helpers ────────────────────────────────────────────────────────────────

/// HTML with enough content to pass the quality gate.
fn sample_html_with_title(title: &str) -> String {
    format!(
        "<html><head><title>{title}</title></head><body>\
        <h1>{title}</h1>\
        <p>{}</p>\
        </body></html>",
        "This is a substantial paragraph of content that provides enough text \
         to pass quality checks in the crawl pipeline. It discusses various \
         interesting topics and ideas in detail. "
            .repeat(8)
    )
}

/// Create a StakeInfo for an entity or domain key at a given USD value.
fn make_entity_stake(key: &str, usd: f64) -> StakeInfo {
    StakeInfo {
        domain: key.to_string(),
        amount_lamports: (usd * 1_000_000.0) as u64,
        amount_usd: usd,
        staked_at: now_secs() as i64,
        cooldown_until: None,
        wallet: "11111111111111111111111111111111".to_string(),
        dns_verified: false,
        reputation: 1.0,
        entity: if key.contains('/') { Some(key.to_string()) } else { None },
        disputed: false,
    }
}

/// Current unix timestamp in seconds.
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// Build a ContentChunk with entity metadata and a mock embedding.
fn make_entity_chunk(
    id: &str,
    text: &str,
    embedding: Vec<f32>,
    url: &str,
    title: &str,
    entity: Option<&str>,
    content_kind: Option<&str>,
) -> ContentChunk {
    let domain = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .and_then(|s| s.split('/').next())
        .unwrap_or("")
        .to_string();
    ContentChunk {
        chunk_id: id.to_string(),
        content: text.to_string(),
        embedding: Some(embedding),
        seq: 0,
        meta: Some(ChunkMeta {
            url: url.to_string(),
            title: title.to_string(),
            domain,
            content_hash: format!("hash_{id}"),
            crawled_at: Some(now_secs()),
            content_type: Some("text/html".to_string()),
            entity: entity.map(|s| s.to_string()),
            content_kind: content_kind.map(|s| s.to_string()),
            ..Default::default()
        }),
    }
}

// ── Test 1: Crawl → entity extraction → ingest → query ────────────────

#[test]
fn test_entity_crawl_ingest_query_pipeline() {
    // Process HTML from a YouTube channel URL — entity should be auto-extracted
    let html = sample_html_with_title("3Blue1Brown: Linear Algebra");
    let url = "https://youtube.com/c/3blue1brown";
    let output = oi_crawl::pipeline::process_html(url, &html, 512, 128).unwrap();

    // Verify entity was extracted on every chunk
    assert!(!output.chunks.is_empty());
    for chunk in &output.chunks {
        let meta = chunk.meta.as_ref().expect("chunk should have metadata");
        assert_eq!(
            meta.entity.as_deref(),
            Some("youtube.com/c/3blue1brown"),
            "entity should be populated from URL"
        );
    }

    // Also verify via the standalone function
    assert_eq!(
        entity_from_url(url),
        Some("youtube.com/c/3blue1brown".to_string())
    );

    // Ingest into HybridIndex with mock embeddings
    let dim = 8;
    let chunks_with_embeddings: Vec<ContentChunk> = output
        .chunks
        .into_iter()
        .enumerate()
        .map(|(i, mut chunk)| {
            let mut emb = vec![0.0f32; dim];
            emb[i % dim] = 1.0;
            chunk.embedding = Some(emb);
            chunk
        })
        .collect();

    let index = oi_index::HybridIndex::new(dim);
    index.ingest(&chunks_with_embeddings).unwrap();
    assert!(index.doc_count() > 0);

    // Query and verify entity is preserved in results
    let query_emb = vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    let results = index
        .query(&query_emb, "linear algebra", &QueryFilters::default(), 5)
        .unwrap();
    assert!(!results.is_empty());
    for r in &results {
        assert_eq!(r.entity.as_deref(), Some("youtube.com/c/3blue1brown"));
    }
}

// ── Test 2: Entity stake enrichment end-to-end ─────────────────────────

#[test]
fn test_entity_stake_enrichment_e2e() {
    let dim = 8;

    // Register entity stake via InMemoryStakeRegistry
    let registry = oi_staking::InMemoryStakeRegistry::new();
    registry
        .add_stake(make_entity_stake("youtube.com/c/3blue1brown", 1000.0))
        .unwrap();
    let registry = Arc::new(registry);

    let index = oi_index::HybridIndex::new(dim).with_stake_registry(registry);

    let chunks = vec![make_entity_chunk(
        "c1",
        "linear algebra explained visually with animations and examples",
        vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        "https://youtube.com/watch?v=abc",
        "Linear Algebra",
        Some("youtube.com/c/3blue1brown"),
        Some("video"),
    )];
    index.ingest(&chunks).unwrap();

    let results = index
        .query(
            &[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            "linear algebra",
            &QueryFilters::default(),
            5,
        )
        .unwrap();

    assert_eq!(results.len(), 1);
    assert!(results[0].stake_truth > 0, "entity stake should enrich result");
    assert_eq!(results[0].stake_type, "entity");
    assert_eq!(results[0].content_kind.as_deref(), Some("video"));
    assert_eq!(
        results[0].entity.as_deref(),
        Some("youtube.com/c/3blue1brown")
    );
}

// ── Test 3: UGC zero passthrough vs publisher domain passthrough ───────

#[test]
fn test_ugc_vs_publisher_domain_passthrough() {
    let dim = 8;

    let registry = oi_staking::InMemoryStakeRegistry::new();
    // UGC platform: domain-level stake on youtube.com
    registry
        .add_stake(make_entity_stake("youtube.com", 1000.0))
        .unwrap();
    // Publisher domain: domain-level stake on nytimes.com
    registry
        .add_stake(make_entity_stake("nytimes.com", 1000.0))
        .unwrap();
    let registry = Arc::new(registry);

    let index = oi_index::HybridIndex::new(dim).with_stake_registry(registry);

    let chunks = vec![
        make_entity_chunk(
            "yt1",
            "math video content from youtube channel about calculus",
            vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            "https://youtube.com/watch?v=abc",
            "Math Video",
            Some("youtube.com/c/somechannel"),
            Some("video"),
        ),
        make_entity_chunk(
            "nyt1",
            "breaking news article from the new york times reporter",
            vec![0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            "https://nytimes.com/2024/article",
            "Breaking News",
            None,
            None,
        ),
    ];
    index.ingest(&chunks).unwrap();

    // Query for YouTube content
    let yt_results = index
        .query(
            &[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            "math video calculus",
            &QueryFilters::default(),
            1,
        )
        .unwrap();
    assert_eq!(yt_results.len(), 1);
    assert_eq!(
        yt_results[0].stake_truth, 0,
        "UGC domain stake should give zero passthrough"
    );

    // Query for NYTimes content
    let nyt_results = index
        .query(
            &[0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            "breaking news article",
            &QueryFilters::default(),
            1,
        )
        .unwrap();
    assert_eq!(nyt_results.len(), 1);
    assert!(
        nyt_results[0].stake_truth > 0,
        "Publisher domain stake should pass through"
    );
    assert_eq!(nyt_results[0].stake_type, "domain");
}

// ── Test 4: Entity normalization in stake flow ─────────────────────────

#[test]
fn test_entity_normalization_in_stake_flow() {
    let dim = 8;

    // Parse "twitter.com/elonmusk" → normalizes to "x.com/elonmusk"
    let entity = EntityRef::parse("twitter.com/elonmusk").unwrap();
    assert_eq!(entity.canonical(), "x.com/elonmusk");

    // Stake under the normalized key
    let registry = oi_staking::InMemoryStakeRegistry::new();
    registry
        .add_stake(make_entity_stake("x.com/elonmusk", 500.0))
        .unwrap();
    let registry = Arc::new(registry);

    let index = oi_index::HybridIndex::new(dim).with_stake_registry(registry);

    // Content with entity "x.com/elonmusk" (already normalized)
    let chunks = vec![make_entity_chunk(
        "c1",
        "tweet about technology and artificial intelligence discussion",
        vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        "https://x.com/elonmusk/status/123",
        "Tech Tweet",
        Some("x.com/elonmusk"),
        Some("post"),
    )];
    index.ingest(&chunks).unwrap();

    let results = index
        .query(
            &[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            "technology artificial intelligence",
            &QueryFilters::default(),
            5,
        )
        .unwrap();

    assert_eq!(results.len(), 1);
    assert!(
        results[0].stake_truth > 0,
        "normalized entity should match stake"
    );
    assert_eq!(results[0].stake_type, "entity");
}

// ── Test 5: Entity filtering end-to-end ────────────────────────────────

#[test]
fn test_entity_filtering_e2e() {
    let dim = 8;
    let index = oi_index::HybridIndex::new(dim);

    let chunks = vec![
        make_entity_chunk(
            "c1",
            "linear algebra explained visually with animations",
            vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            "https://youtube.com/watch?v=1",
            "3B1B Linear Algebra",
            Some("youtube.com/c/3blue1brown"),
            Some("video"),
        ),
        make_entity_chunk(
            "c2",
            "physics experiments explained visually with demonstrations",
            vec![0.9, 0.1, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            "https://youtube.com/watch?v=2",
            "Veritasium Physics",
            Some("youtube.com/@veritasium"),
            Some("video"),
        ),
    ];
    index.ingest(&chunks).unwrap();

    // Filter for 3blue1brown only
    let filtered = index
        .query(
            &[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            "explained visually",
            &QueryFilters {
                entities: vec!["youtube.com/c/3blue1brown".to_string()],
                ..Default::default()
            },
            5,
        )
        .unwrap();
    assert_eq!(filtered.len(), 1, "filter should return only 3blue1brown");
    assert_eq!(
        filtered[0].entity.as_deref(),
        Some("youtube.com/c/3blue1brown")
    );

    // Without filter: both results
    let unfiltered = index
        .query(
            &[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            "explained visually",
            &QueryFilters::default(),
            5,
        )
        .unwrap();
    assert_eq!(unfiltered.len(), 2, "unfiltered should return both");
}

// ── Test 6: Gossip entity round-trip ───────────────────────────────────

#[test]
fn test_gossip_entity_round_trip() {
    use oi_types::proto::opensonarx::gossip_message::Payload;

    // Encode with entity
    let encoded = oi_network::gossip::encode_content_announcement(
        "sentinel-1",
        "https://youtube.com/watch?v=abc",
        "youtube.com",
        "hash123",
        3,
        "shard-0",
        false,
        "youtube.com/c/3blue1brown",
    );
    let decoded = oi_network::gossip::decode_gossip(&encoded).unwrap();
    match decoded.payload.unwrap() {
        Payload::ContentAnnouncement(ca) => {
            assert_eq!(ca.entity, "youtube.com/c/3blue1brown");
            assert_eq!(ca.domain, "youtube.com");
            assert_eq!(ca.url, "https://youtube.com/watch?v=abc");
        }
        _ => panic!("expected ContentAnnouncement"),
    }

    // Empty entity round-trips correctly
    let encoded_empty = oi_network::gossip::encode_content_announcement(
        "sentinel-2",
        "https://example.com/page",
        "example.com",
        "hash456",
        1,
        "shard-1",
        false,
        "",
    );
    let decoded_empty = oi_network::gossip::decode_gossip(&encoded_empty).unwrap();
    match decoded_empty.payload.unwrap() {
        Payload::ContentAnnouncement(ca) => {
            assert!(ca.entity.is_empty(), "empty entity should round-trip as empty");
        }
        _ => panic!("expected ContentAnnouncement"),
    }
}

// ── Test 7: entities_from_stake_events cross-crate ─────────────────────

#[test]
fn test_entities_from_stake_events_cross_crate() {
    // Feed events with twitter/x aliases, case variations, invalid refs
    let events = vec![
        ("twitter.com/elonmusk".to_string(), 10_000u64),
        ("x.com/ElonMusk".to_string(), 5_000u64),          // alias + case → same canonical
        ("youtube.com/c/3Blue1Brown".to_string(), 20_000),
        ("youtube.com/c/3blue1brown".to_string(), 5_000),   // case variation → merges
        ("".to_string(), 1_000),                             // invalid: empty
        ("no-dot".to_string(), 1_000),                       // invalid: no dot
        ("reddit.com/r/Rust".to_string(), 8_000),
    ];

    let entities = oi_crawl::daemon::entities_from_stake_events(&events);

    // Should have 3 valid entities: x.com/elonmusk, youtube.com/c/3blue1brown, reddit.com/r/rust
    assert_eq!(entities.len(), 3, "invalid refs should be skipped, aliases merged");

    // Build a lookup by canonical key
    let map: HashMap<String, &oi_crawl::daemon::MonitoredEntity> = entities
        .iter()
        .map(|e| (e.entity.canonical(), e))
        .collect();

    // twitter.com → x.com normalization should merge
    let x = map.get("x.com/elonmusk").expect("x.com/elonmusk should exist");
    assert_eq!(x.stake_amount, 15_000, "twitter + x should merge to 15k");
    assert!(x.entity.is_ugc_platform());

    // YouTube case merging
    let yt = map
        .get("youtube.com/c/3blue1brown")
        .expect("youtube channel should exist");
    assert_eq!(yt.stake_amount, 25_000, "case variants should merge to 25k");
    assert!(yt.entity.is_ugc_platform());

    // Reddit normalization
    let reddit = map.get("reddit.com/r/rust").expect("reddit should exist");
    assert_eq!(reddit.stake_amount, 8_000);
    assert!(reddit.entity.is_ugc_platform());
}
