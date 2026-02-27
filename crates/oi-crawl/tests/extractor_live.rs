//! Live extraction test — fetches real documentation pages and verifies the
//! HTML extractor captures content properly while stripping boilerplate.
//!
//! Run: cargo test -p oi-crawl --test extractor_live -- --nocapture --ignored

use oi_crawl::extractor::html_to_markdown;
use oi_crawl::fetcher::fetch_url;

/// Helper: fetch a page, extract markdown, print stats, return markdown.
async fn fetch_and_extract(url: &str) -> String {
    let result = fetch_url(url).await.unwrap_or_else(|e| panic!("Failed to fetch {url}: {e}"));
    let extracted = html_to_markdown(&result.html);
    let md = &extracted.markdown;

    println!("\n{}", "=".repeat(80));
    println!("URL:   {url}");
    println!("Title: {:?}", extracted.title);
    println!("Chars: {}", md.len());
    println!("Lines: {}", md.lines().count());
    // Print first 500 chars as preview
    let preview: String = md.chars().take(500).collect();
    println!("Preview:\n{preview}");
    println!("---");
    // Print last 300 chars
    let tail: String = md.chars().rev().take(300).collect::<String>().chars().rev().collect();
    println!("Tail:\n{tail}");

    md.clone()
}

#[tokio::test]
#[ignore] // requires network
async fn test_typescript_handbook_generics() {
    // Gatsby-rendered TypeScript handbook page
    let md = fetch_and_extract(
        "https://www.typescriptlang.org/docs/handbook/2/generics.html",
    ).await;

    // Must contain actual content about generics
    let md_lower = md.to_lowercase();
    assert!(md_lower.contains("generic"), "Should contain 'generic' — the page topic");
    assert!(md_lower.contains("type parameter") || md_lower.contains("<t>") || md_lower.contains("identity"),
        "Should contain generic examples like type parameter, <T>, or identity function");

    // Should NOT contain sidebar/nav boilerplate
    assert!(!md.contains("Like") || !md.contains("Dislike"),
        "Should strip like/dislike buttons");

    // Should have substantial content (not just nav fragments)
    assert!(md.len() > 500, "Should extract >500 chars of content, got {}", md.len());

    println!("\n[PASS] TypeScript handbook generics: {} chars extracted", md.len());
}

#[tokio::test]
#[ignore] // requires network
async fn test_typescript_handbook_narrowing() {
    let md = fetch_and_extract(
        "https://www.typescriptlang.org/docs/handbook/2/narrowing.html",
    ).await;

    let md_lower = md.to_lowercase();
    assert!(md_lower.contains("narrowing"), "Should contain 'narrowing'");
    assert!(md_lower.contains("typeof") || md_lower.contains("type guard"),
        "Should contain typeof or type guard content");
    assert!(md.len() > 500, "Should extract >500 chars, got {}", md.len());

    println!("\n[PASS] TypeScript narrowing: {} chars extracted", md.len());
}

#[tokio::test]
#[ignore] // requires network
async fn test_typescript_utility_types() {
    let md = fetch_and_extract(
        "https://www.typescriptlang.org/docs/handbook/utility-types.html",
    ).await;

    let md_lower = md.to_lowercase();
    assert!(md_lower.contains("partial") || md_lower.contains("required") || md_lower.contains("pick"),
        "Should contain utility type names");
    assert!(md.len() > 500, "Should extract >500 chars, got {}", md.len());

    println!("\n[PASS] TypeScript utility types: {} chars extracted", md.len());
}

#[tokio::test]
#[ignore] // requires network
async fn test_rust_book_ownership() {
    // mdBook-rendered Rust Book page
    let md = fetch_and_extract(
        "https://doc.rust-lang.org/book/ch04-01-what-is-ownership.html",
    ).await;

    let md_lower = md.to_lowercase();
    assert!(md_lower.contains("ownership"), "Should contain 'ownership'");
    assert!(md_lower.contains("stack") || md_lower.contains("heap"),
        "Should discuss stack and heap");
    assert!(md.len() > 500, "Should extract >500 chars, got {}", md.len());

    // Should not contain chapter navigation
    assert!(!md.contains("Chapter list"), "Should strip sidebar chapter list");

    println!("\n[PASS] Rust Book ownership: {} chars extracted", md.len());
}

#[tokio::test]
#[ignore] // requires network
async fn test_rust_std_string() {
    // Standard library docs (rustdoc-rendered)
    let md = fetch_and_extract(
        "https://doc.rust-lang.org/std/string/struct.String.html",
    ).await;

    let md_lower = md.to_lowercase();
    assert!(md_lower.contains("string"), "Should contain 'String'");
    assert!(md_lower.contains("from_utf8") || md_lower.contains("utf-8") || md_lower.contains("utf8"),
        "Should mention UTF-8");
    assert!(md.len() > 500, "Should extract >500 chars, got {}", md.len());

    println!("\n[PASS] Rust std String: {} chars extracted", md.len());
}

#[tokio::test]
#[ignore] // requires network
async fn test_react_dev_typescript() {
    // react.dev — modern documentation site
    let md = fetch_and_extract(
        "https://react.dev/learn/typescript",
    ).await;

    let md_lower = md.to_lowercase();
    assert!(md_lower.contains("typescript") || md_lower.contains("type"),
        "Should contain TypeScript content");
    assert!(md.len() > 300, "Should extract >300 chars, got {}", md.len());

    println!("\n[PASS] React TypeScript guide: {} chars extracted", md.len());
}

/// Summary test — fetches all key URLs from the benchmark and reports extraction stats.
#[tokio::test]
#[ignore] // requires network
async fn test_extraction_summary() {
    let urls = vec![
        // TypeScript handbook (Gatsby)
        "https://www.typescriptlang.org/docs/handbook/2/generics.html",
        "https://www.typescriptlang.org/docs/handbook/2/narrowing.html",
        "https://www.typescriptlang.org/docs/handbook/2/everyday-types.html",
        "https://www.typescriptlang.org/docs/handbook/utility-types.html",
        "https://www.typescriptlang.org/docs/handbook/enums.html",
        // Rust docs (mdBook + rustdoc)
        "https://doc.rust-lang.org/book/ch04-01-what-is-ownership.html",
        "https://doc.rust-lang.org/book/ch10-02-traits.html",
        "https://doc.rust-lang.org/std/string/struct.String.html",
        "https://doc.rust-lang.org/std/option/enum.Option.html",
        // React (modern doc site)
        "https://react.dev/learn/typescript",
    ];

    let mut successes = 0;
    let mut failures = 0;

    for url in &urls {
        match fetch_url(url).await {
            Ok(result) => {
                let extracted = html_to_markdown(&result.html);
                let md = &extracted.markdown;
                let chars = md.len();
                let lines = md.lines().count();
                let status = if chars > 500 { "OK" } else { "LOW" };

                if chars > 500 {
                    successes += 1;
                } else {
                    failures += 1;
                }

                println!(
                    "[{status:>3}] {chars:>6} chars | {lines:>4} lines | {url}"
                );
            }
            Err(e) => {
                failures += 1;
                println!("[ERR]                         | {url} — {e}");
            }
        }
    }

    println!("\n=== Extraction Summary ===");
    println!("Total:     {}", urls.len());
    println!("Success:   {successes} (>500 chars extracted)");
    println!("Low/Error: {failures}");
    println!(
        "Rate:      {:.0}%",
        successes as f64 / urls.len() as f64 * 100.0
    );

    assert!(
        successes as f64 / urls.len() as f64 >= 0.7,
        "At least 70% of pages should extract >500 chars of content"
    );
}
