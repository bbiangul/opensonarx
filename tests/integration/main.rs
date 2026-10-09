use oi_types::traits::{SearchIndex, StakeRegistry, StateChannel};
use oi_types::{ContentChunk, QueryFilters, StakeInfo};

#[test]
fn test_end_to_end_ingest_and_search() {
    // 1. Set up stake registry
    let registry = oi_staking::InMemoryStakeRegistry::new();
    registry
        .add_stake(StakeInfo {
            domain: "example.com".to_string(),
            amount_lamports: 1_000_000,
            amount_usd: 150.0,
            staked_at: 1700000000,
            cooldown_until: None,
            wallet: "wallet123".to_string(),
            dns_verified: true,
            reputation: 0.95,
            entity: None,
            disputed: false,
        })
        .unwrap();

    assert!(registry.is_staked("example.com").unwrap());
    assert!(!registry.is_staked("unknown.com").unwrap());

    // 2. Crawl and process HTML content
    let html = "<html><head><title>Rust Guide</title></head><body>\
        <h1>Learn Rust</h1>\
        <p>Rust is a systems programming language focused on safety and performance. \
        Its ownership model prevents data races at compile time without garbage collection. \
        The borrow checker ensures references are always valid throughout their lifetime. \
        Pattern matching provides exhaustive checks on enum variants for robust error handling. \
        Traits enable polymorphism without inheritance, favoring composition over subclassing. \
        Error handling uses Result and Option types for explicit control flow instead of exceptions. \
        The standard library provides efficient collections like Vec, HashMap, and BTreeMap. \
        Cargo manages dependencies and builds reproducibly across platforms. \
        Clippy catches common mistakes and suggests idiomatic improvements to your code. \
        Rustfmt enforces consistent formatting across projects and teams. \
        Async await syntax enables efficient concurrent programming without callback hell. \
        Zero-cost abstractions let you write high-level code that compiles to optimal machine instructions. \
        The type system catches null pointer errors, buffer overflows, and iterator invalidation at compile time. \
        Crates.io hosts over 100,000 community packages for networking, serialization, and web development. \
        Cross-compilation support enables targeting embedded systems, WebAssembly, and mobile platforms. \
        Procedural macros generate boilerplate code at compile time for serialization and database mappings. \
        Unsafe blocks provide escape hatches for FFI and low-level memory operations when needed. \
        The module system organizes code into clear hierarchies with fine-grained visibility control. \
        Lifetime annotations express relationships between references for the borrow checker to verify. \
        Smart pointers like Box, Rc, and Arc provide heap allocation and reference counting.</p>\
        </body></html>".to_string();
    let crawl_output =
        oi_crawl::pipeline::process_html("https://example.com/rust", &html, 512, 128).unwrap();

    assert!(!crawl_output.chunks.is_empty());
    assert_eq!(crawl_output.metadata.title, Some("Rust Guide".to_string()));

    // 3. Add embeddings to chunks (mock: use position-based vectors)
    let dim = 8;
    let chunks_with_embeddings: Vec<ContentChunk> = crawl_output
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

    // 4. Ingest into hybrid index
    let index = oi_index::HybridIndex::new(dim);
    index.ingest(&chunks_with_embeddings).unwrap();
    assert!(index.doc_count() > 0);

    // 5. Search
    let query_embedding = vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    let results = index
        .query(
            &query_embedding,
            "rust programming",
            &QueryFilters::default(),
            5,
        )
        .unwrap();
    assert!(!results.is_empty());

    // 6. State channel
    let channel = oi_staking::escrow::InMemoryStateChannel::new();
    let ticket = channel.create_ticket(100, "payee_node_abc").unwrap();
    assert_eq!(ticket.amount_lamports, 100);
    let tx_hash = channel.settle_batch(&[ticket]).unwrap();
    assert!(!tx_hash.is_empty());
}

#[test]
fn test_crawl_quality_gate() {
    // Short content should be rejected
    let result = oi_crawl::pipeline::process_html(
        "https://example.com/short",
        "<html><body><p>Hi</p></body></html>",
        512,
        128,
    );
    assert!(result.is_err());

    // Link-heavy content should be rejected
    let links_html = format!(
        "<html><body>{}</body></html>",
        (0..25)
            .map(|i| format!("<p>https://example.com/{i}</p>"))
            .collect::<String>()
    );
    let result =
        oi_crawl::pipeline::process_html("https://example.com/links", &links_html, 512, 128);
    assert!(result.is_err());
}

#[test]
fn test_gossip_message_encoding() {
    let encoded = oi_network::gossip::encode_heartbeat("test-peer", 4, 50000, 10);
    let decoded = oi_network::gossip::decode_gossip(&encoded).unwrap();
    assert!(decoded.payload.is_some());
}

#[test]
fn test_error_codes() {
    use oi_types::ErrorCode;

    // Verify error code round-trip
    let code = ErrorCode::QueryTimeout;
    assert_eq!(code.as_u32(), 1001);
    assert_eq!(ErrorCode::from_u32(1001), Some(ErrorCode::QueryTimeout));
    assert_eq!(format!("{code}"), "E1001");

    // All error code ranges
    assert!(ErrorCode::from_u32(1000).is_some()); // Query
    assert!(ErrorCode::from_u32(2000).is_some()); // Payment
    assert!(ErrorCode::from_u32(3000).is_some()); // Stake
    assert!(ErrorCode::from_u32(4000).is_some()); // Dispute
    assert!(ErrorCode::from_u32(5000).is_some()); // Network
    assert!(ErrorCode::from_u32(6000).is_some()); // Auth
    assert!(ErrorCode::from_u32(9999).is_none()); // Invalid

    // Retryable
    assert!(ErrorCode::QueryTimeout.retryable());
    assert!(ErrorCode::NoPeers.retryable());
    assert!(!ErrorCode::QueryNoResults.retryable());
    assert!(!ErrorCode::DomainNotVerified.retryable());

    // Exit codes
    use oi_types::error::ExitCode;
    assert_eq!(ErrorCode::QueryNoResults.exit_code(), ExitCode::Success);
    assert_eq!(ErrorCode::QueryTimeout.exit_code(), ExitCode::NetworkError);
    assert_eq!(
        ErrorCode::InsufficientBalance.exit_code(),
        ExitCode::InsufficientFunds
    );
    assert_eq!(ErrorCode::InvalidWallet.exit_code(), ExitCode::Unauthorized);
}
