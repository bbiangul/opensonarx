//! Debug test: traces TypeScript search retrieval to diagnose ranking issues.
//!
//! Run: cargo test -p integration --test ts_search_debug -- --nocapture --ignored

use oi_crawl::Distiller;
use oi_index::HybridIndex;
use oi_types::traits::{CrawlPipeline, SearchIndex};
use oi_types::{EmbeddingPipeline, QueryFilters};
use std::sync::Arc;

/// All TypeScript source URLs from the agent-cost benchmark.
const TS_URLS: &[&str] = &[
    "https://www.typescriptlang.org/docs/handbook/2/generics.html",
    "https://www.typescriptlang.org/docs/handbook/2/functions.html",
    "https://www.typescriptlang.org/docs/handbook/2/everyday-types.html",
    "https://www.typescriptlang.org/docs/handbook/2/types-from-types.html",
    "https://www.typescriptlang.org/docs/handbook/2/objects.html",
    "https://www.typescriptlang.org/docs/handbook/declaration-merging.html",
    "https://www.typescriptlang.org/docs/handbook/interfaces.html",
    "https://www.typescriptlang.org/docs/handbook/2/narrowing.html",
    "https://www.typescriptlang.org/docs/handbook/unions-and-intersections.html",
    "https://www.typescriptlang.org/docs/handbook/utility-types.html",
    "https://www.typescriptlang.org/docs/handbook/2/mapped-types.html",
    "https://www.typescriptlang.org/docs/handbook/2/keyof-types.html",
    "https://www.typescriptlang.org/docs/handbook/enums.html",
    "https://www.typescriptlang.org/docs/handbook/basic-types.html",
    "https://www.typescriptlang.org/docs/handbook/release-notes/typescript-1-7.html",
    "https://www.typescriptlang.org/docs/handbook/release-notes/typescript-3-0.html",
    "https://www.typescriptlang.org/docs/handbook/release-notes/typescript-3-5.html",
    "https://www.typescriptlang.org/docs/handbook/release-notes/typescript-4-1.html",
    "https://www.typescriptlang.org/docs/handbook/release-notes/typescript-4-9.html",
    "https://www.typescriptlang.org/docs/handbook/release-notes/typescript-5-0.html",
    "https://www.typescriptlang.org/docs/handbook/type-compatibility.html",
    "https://www.typescriptlang.org/docs/handbook/advanced-types.html",
    "https://www.typescriptlang.org/docs/handbook/2/conditional-types.html",
    "https://www.typescriptlang.org/docs/handbook/decorators.html",
    "https://www.typescriptlang.org/docs/handbook/2/template-literal-types.html",
    "https://www.typescriptlang.org/docs/handbook/2/indexed-access-types.html",
    "https://www.typescriptlang.org/docs/handbook/variable-declarations.html",
    "https://www.typescriptlang.org/docs/handbook/jsx.html",
    "https://www.typescriptlang.org/docs/handbook/namespaces.html",
    "https://www.typescriptlang.org/docs/handbook/namespaces-and-modules.html",
    "https://www.typescriptlang.org/docs/handbook/modules.html",
    "https://www.typescriptlang.org/docs/handbook/compiler-options.html",
    "https://www.typescriptlang.org/docs/handbook/typescript-in-5-minutes-func.html",
    "https://www.typescriptlang.org/docs/handbook/declaration-files/by-example.html",
    "https://www.typescriptlang.org/tsconfig",
    "https://react.dev/learn/typescript",
    "https://react.dev/reference/react/Component",
    "https://react.dev/learn/passing-props-to-a-component",
    "https://javascript.info/async-await",
    "https://javascript.info/promise-basics",
    "https://nodejs.org/api/events.html",
];

/// Failing TypeScript queries from the benchmark.
const FAILING_QUERIES: &[(&str, &[&str])] = &[
    (
        "What are utility types like Partial, Required, and Pick in TypeScript?",
        &["Partial", "Required", "Pick"],
    ),
    (
        "How do you use enums in TypeScript?",
        &["enum", "const enum"],
    ),
    (
        "How do you define an async function in TypeScript?",
        &["async", "await", "Promise"],
    ),
    (
        "What is the unknown type in TypeScript and how does it differ from any?",
        &["unknown", "type guard", "any"],
    ),
    (
        "How do you use type guards in TypeScript?",
        &["typeof", "instanceof", "narrowing"],
    ),
    (
        "How do you create a mapped type in TypeScript?",
        &["in keyof", "mapped", "Record"],
    ),
    (
        "What are conditional types in TypeScript?",
        &["extends", "infer", "conditional"],
    ),
    (
        "How do you use decorators in TypeScript?",
        &["decorator", "@"],
    ),
    (
        "What is the difference between type assertion and type casting in TypeScript?",
        &["as", "assertion"],
    ),
    (
        "How do you use template literal types in TypeScript?",
        &["template literal", "backtick"],
    ),
    (
        "What is the never type in TypeScript?",
        &["never", "exhaustive"],
    ),
    (
        "How do you configure tsconfig.json strict mode?",
        &["strict", "strictNullChecks", "tsconfig"],
    ),
    (
        "What is the keyof operator in TypeScript?",
        &["keyof", "index type"],
    ),
    (
        "How do you use type narrowing with the in operator?",
        &["in", "narrowing", "property check"],
    ),
    (
        "How do you use the Omit utility type in TypeScript?",
        &["Omit", "Exclude"],
    ),
    (
        "How do you write a type-safe event emitter in TypeScript?",
        &["EventEmitter", "generic", "emit"],
    ),
    (
        "What are index signatures in TypeScript?",
        &["index signature", "[key: string]"],
    ),
    (
        "How do you use satisfies operator in TypeScript?",
        &["satisfies", "type check"],
    ),
];

fn get_embedder() -> Arc<dyn EmbeddingPipeline> {
    Arc::new(oi_embed::MiniLmEmbedder::new().expect("MiniLM pipeline should load"))
}

#[tokio::test]
#[ignore]
async fn debug_ts_ingestion() {
    let embedder = get_embedder();
    let distiller = Distiller::new().with_embedder(embedder.clone());

    println!("\n=== PHASE 1: Ingestion ===\n");

    let mut total_chunks = 0usize;
    let mut url_chunks: Vec<(String, usize, usize)> = Vec::new(); // (url, chunks, chars)

    for url in TS_URLS {
        match distiller.crawl(url).await {
            Ok(output) => {
                let n = output.chunks.len();
                let total_chars: usize = output.chunks.iter().map(|c| c.content.len()).sum();
                total_chunks += n;
                url_chunks.push((url.to_string(), n, total_chars));
                println!("[OK ] {:>3} chunks | {:>6} chars | {url}", n, total_chars);
            }
            Err(e) => {
                url_chunks.push((url.to_string(), 0, 0));
                println!("[ERR]   0 chunks |      0 chars | {url} — {e}");
            }
        }
    }

    println!("\nTotal: {total_chunks} chunks from {} URLs", TS_URLS.len());
    let failed = url_chunks.iter().filter(|(_, n, _)| *n == 0).count();
    println!("Failed: {failed} URLs");

    // Check that key content pages have chunks
    let key_pages = [
        "utility-types",
        "enums",
        "narrowing",
        "mapped-types",
        "keyof-types",
        "conditional-types",
        "decorators",
        "template-literal",
    ];
    println!("\n--- Key pages ---");
    for page in &key_pages {
        let entry = url_chunks.iter().find(|(u, _, _)| u.contains(page));
        match entry {
            Some((url, n, chars)) => {
                let status = if *n > 0 { "OK " } else { "MISSING" };
                println!("[{status}] {n} chunks, {chars} chars — {url}");
            }
            None => println!("[???] No URL matching '{page}'"),
        }
    }
}

#[tokio::test]
#[ignore]
async fn debug_ts_search_ranking() {
    let embedder = get_embedder();
    let index = HybridIndex::new(embedder.dimension());
    let distiller = Distiller::new().with_embedder(embedder.clone());

    println!("\n=== Ingesting TypeScript pages ===\n");

    let mut total_chunks = 0usize;
    for url in TS_URLS {
        match distiller.crawl(url).await {
            Ok(output) => {
                let n = output.chunks.len();
                if let Err(e) = index.ingest(&output.chunks) {
                    println!("[ERR] ingest fail: {url} — {e}");
                } else {
                    total_chunks += n;
                }
            }
            Err(e) => {
                println!("[ERR] crawl fail: {url} — {e}");
            }
        }
    }
    println!("Indexed {total_chunks} chunks\n");

    println!("=== PHASE 2: Search Results for Failing Queries ===\n");

    let filters = QueryFilters::default();

    for (query, expected_keywords) in FAILING_QUERIES {
        let embedding = embedder.embed_query(query).expect("embed should work");
        let results = index
            .query(&embedding, query, &filters, 5)
            .expect("search should work");

        println!("─── Q: {query}");
        println!("    Expected: {expected_keywords:?}");
        println!("    Results ({}):", results.len());

        let mut has_relevant = false;
        for (i, r) in results.iter().enumerate() {
            let preview: String = r.content_md.chars().take(120).collect();
            let preview_clean = preview.replace('\n', " ");

            // Check if this result contains any expected keyword
            let content_lower = r.content_md.to_lowercase();
            let title_lower = r.title.to_lowercase();
            let kw_found: Vec<&&str> = expected_keywords
                .iter()
                .filter(|kw| {
                    content_lower.contains(&kw.to_lowercase())
                        || title_lower.contains(&kw.to_lowercase())
                })
                .collect();

            let relevance_tag = if kw_found.is_empty() { "MISS" } else { "HIT " };
            if !kw_found.is_empty() {
                has_relevant = true;
            }

            println!(
                "    [{relevance_tag}] #{} score={:.4} | {} | {preview_clean}...",
                i + 1,
                r.final_score,
                r.url,
            );
            if !kw_found.is_empty() {
                println!("           keywords found: {kw_found:?}");
            }
        }

        if !has_relevant {
            println!("    >>> NO RELEVANT RESULTS IN TOP 5 <<<");
        }
        println!();
    }
}
