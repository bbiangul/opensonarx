//! Debug test: inspects what the HTML extractor produces from real TypeScript pages.
//!
//! Run: cargo test -p oi-integration-tests --test ts_extractor_debug -- --nocapture --ignored

use oi_crawl::extractor::html_to_markdown;

/// Fetch HTML from a URL using ureq.
fn fetch_html(url: &str) -> Option<String> {
    let agent = ureq::Agent::new_with_config(
        ureq::config::Config::builder()
            .timeout_global(Some(std::time::Duration::from_secs(15)))
            .build(),
    );
    match agent.get(url).call() {
        Ok(mut resp) => {
            let buf = resp.body_mut().read_to_vec().ok()?;
            Some(String::from_utf8_lossy(&buf).to_string())
        }
        Err(e) => {
            eprintln!("[FETCH FAIL] {url}: {e}");
            None
        }
    }
}

/// Pages that the benchmark's failing TS queries depend on.
/// Each entry: (url, expected_keywords)
const CRITICAL_TS_PAGES: &[(&str, &[&str])] = &[
    (
        "https://www.typescriptlang.org/docs/handbook/2/narrowing.html",
        &["typeof", "instanceof", "narrowing", "type guard", "discriminated"],
    ),
    (
        "https://www.typescriptlang.org/docs/handbook/enums.html",
        &["enum", "const enum", "reverse mapping"],
    ),
    (
        "https://www.typescriptlang.org/docs/handbook/2/mapped-types.html",
        &["in keyof", "mapped", "Record"],
    ),
    (
        "https://www.typescriptlang.org/docs/handbook/decorators.html",
        &["decorator", "experimentalDecorators", "@"],
    ),
    (
        "https://www.typescriptlang.org/docs/handbook/2/keyof-types.html",
        &["keyof", "index type"],
    ),
    (
        "https://www.typescriptlang.org/docs/handbook/2/template-literal-types.html",
        &["template literal", "backtick"],
    ),
    (
        "https://www.typescriptlang.org/docs/handbook/utility-types.html",
        &["Partial", "Required", "Pick", "Omit", "Record"],
    ),
    (
        "https://www.typescriptlang.org/docs/handbook/2/conditional-types.html",
        &["extends", "infer", "conditional"],
    ),
    (
        "https://www.typescriptlang.org/docs/handbook/namespaces.html",
        &["namespace"],
    ),
    (
        "https://www.typescriptlang.org/docs/handbook/2/objects.html",
        &["interface", "type"],
    ),
    (
        "https://www.typescriptlang.org/docs/handbook/interfaces.html",
        &["interface", "extends", "declaration merging"],
    ),
    (
        "https://www.typescriptlang.org/docs/handbook/release-notes/typescript-4-9.html",
        &["satisfies"],
    ),
];

#[test]
#[ignore]
fn debug_ts_extraction() {
    println!("\n=== TypeScript Page Extraction Debug ===\n");

    let mut total_pages = 0;
    let mut pages_with_content = 0;
    let mut keyword_hits = 0;
    let mut keyword_total = 0;

    for (url, keywords) in CRITICAL_TS_PAGES {
        total_pages += 1;
        print!("[FETCHING] {url} ... ");

        let html = match fetch_html(url) {
            Some(h) => h,
            None => {
                println!("FAILED");
                keyword_total += keywords.len();
                continue;
            }
        };

        let extracted = html_to_markdown(&html);
        let md = &extracted.markdown;
        let md_lower = md.to_lowercase();

        if md.is_empty() {
            println!("EMPTY EXTRACTION ({} bytes HTML)", html.len());
            keyword_total += keywords.len();
            continue;
        }

        pages_with_content += 1;
        println!("OK ({} chars extracted from {} bytes HTML)", md.len(), html.len());

        // Check each keyword
        let mut found = Vec::new();
        let mut missing = Vec::new();
        for kw in *keywords {
            keyword_total += 1;
            if md_lower.contains(&kw.to_lowercase()) {
                found.push(*kw);
                keyword_hits += 1;
            } else {
                missing.push(*kw);
            }
        }

        if !missing.is_empty() {
            println!("  Keywords FOUND: {:?}", found);
            println!("  Keywords MISSING: {:?}", missing);
            // Show first 500 chars to understand what WAS extracted
            let preview: String = md.chars().take(500).collect();
            println!("  Content preview: {}", preview.replace('\n', " "));
        } else {
            println!("  All keywords found: {:?}", found);
        }
        println!();
    }

    println!("\n=== Summary ===");
    println!("Pages fetched: {total_pages}");
    println!("Pages with content: {pages_with_content}");
    println!(
        "Keyword coverage: {keyword_hits}/{keyword_total} ({:.0}%)",
        keyword_hits as f64 / keyword_total as f64 * 100.0
    );
    println!();
}
