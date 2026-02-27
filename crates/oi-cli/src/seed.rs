use std::io::Read as _;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use oi_crawl::discovery::parse_sitemap;
use oi_crawl::fetcher::{fetch_bytes, fetch_url};
use oi_crawl::robots::{parse_robots_txt, RobotsChecker};
use oi_crawl::spider::{Spider, SpiderConfig};
use oi_crawl::Distiller;
use oi_sdk::{OpenSonarXClient, SdkConfig};
use oi_types::error::ExitCode;
use oi_types::traits::CrawlPipeline;
use tokio::sync::{RwLock, Semaphore};
use tokio::task::JoinSet;

use crate::SeedCommands;
use super::CliConfig;

/// Entry point called from the `seed` dispatcher in commands.rs.
pub async fn run_seed(config: &CliConfig, action: SeedCommands, verbose: bool) -> ExitCode {
    // Spider-based crawl takes a completely different path
    if let SeedCommands::Crawl {
        url,
        depth,
        max_pages,
        concurrency,
    } = action
    {
        return run_spider_crawl(config, &url, depth, max_pages, concurrency, verbose).await;
    }

    let (urls, concurrency) = match action {
        SeedCommands::Sitemap { url, concurrency } => {
            println!("Fetching sitemap: {url}");
            match urls_from_sitemap(&url).await {
                Ok(urls) => {
                    println!("  Found {} URLs", urls.len());
                    (urls, concurrency)
                }
                Err(e) => {
                    eprintln!("Failed to fetch sitemap: {e}");
                    return ExitCode::UserError;
                }
            }
        }
        SeedCommands::Urls { file, concurrency } => match urls_from_file(&file) {
            Ok(urls) => {
                println!("Loaded {} URLs from {file}", urls.len());
                (urls, concurrency)
            }
            Err(e) => {
                eprintln!("Failed to read URL file: {e}");
                return ExitCode::UserError;
            }
        },
        SeedCommands::Registry { source } => match source {
            crate::RegistrySource::DocsRs { top, concurrency } => {
                println!("Fetching top {top} crates from crates.io...");
                match urls_from_docs_rs(top).await {
                    Ok(urls) => {
                        println!("  Resolved {} docs.rs URLs", urls.len());
                        (urls, concurrency)
                    }
                    Err(e) => {
                        eprintln!("Failed to fetch crates.io registry: {e}");
                        return ExitCode::UserError;
                    }
                }
            }
            crate::RegistrySource::Mdn { concurrency } => {
                println!("Fetching MDN sitemap...");
                match urls_from_mdn().await {
                    Ok(urls) => {
                        println!("  Found {} MDN URLs", urls.len());
                        (urls, concurrency)
                    }
                    Err(e) => {
                        eprintln!("Failed to fetch MDN sitemap: {e}");
                        return ExitCode::UserError;
                    }
                }
            }
        },
        SeedCommands::Crawl { .. } => unreachable!("handled above"),
    };

    if urls.is_empty() {
        eprintln!("No URLs to seed.");
        return ExitCode::UserError;
    }

    crawl_and_ingest(config, urls, concurrency, verbose).await
}

// ── URL resolution ──────────────────────────────────────────────────────────

/// Fetch a sitemap and extract all URLs. Recursively fetches nested sitemaps
/// (one level deep). Supports `.xml.gz` via `fetch_bytes` + GzDecoder.
pub async fn urls_from_sitemap(url: &str) -> oi_types::Result<Vec<String>> {
    let xml = fetch_sitemap_xml(url).await?;
    let entries = parse_sitemap(&xml);

    let mut urls = Vec::new();
    let mut nested_sitemaps = Vec::new();

    for entry in &entries {
        if entry.url.ends_with(".xml") || entry.url.ends_with(".xml.gz") {
            nested_sitemaps.push(entry.url.clone());
        } else {
            urls.push(entry.url.clone());
        }
    }

    // Fetch nested sitemaps (one level)
    for nested_url in &nested_sitemaps {
        match fetch_sitemap_xml(nested_url).await {
            Ok(nested_xml) => {
                let nested_entries = parse_sitemap(&nested_xml);
                urls.extend(nested_entries.into_iter().map(|e| e.url));
            }
            Err(e) => {
                tracing::warn!("Failed to fetch nested sitemap {nested_url}: {e}");
            }
        }
    }

    Ok(urls)
}

/// Fetch sitemap XML, handling both plain .xml and .xml.gz URLs.
async fn fetch_sitemap_xml(url: &str) -> oi_types::Result<String> {
    if url.ends_with(".gz") {
        let bytes = fetch_bytes(url).await?;
        let mut decoder = flate2::read::GzDecoder::new(&bytes[..]);
        let mut xml = String::new();
        decoder
            .read_to_string(&mut xml)
            .map_err(|e| oi_types::Error::Internal(format!("gzip decode failed: {e}")))?;
        Ok(xml)
    } else {
        let result = fetch_url(url).await?;
        Ok(result.html)
    }
}

/// Read URLs from a text file (one per line, skip empty lines and `#` comments).
pub fn urls_from_file(path: &str) -> oi_types::Result<Vec<String>> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| oi_types::Error::Internal(format!("cannot read {path}: {e}")))?;

    let urls: Vec<String> = content
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.to_string())
        .collect();

    Ok(urls)
}

/// Fetch the top N crates from crates.io and build docs.rs entry URLs.
pub async fn urls_from_docs_rs(top_n: usize) -> oi_types::Result<Vec<String>> {
    let mut crate_names = Vec::new();
    let per_page = 100;
    let pages = top_n.div_ceil(per_page);

    for page in 1..=pages {
        let api_url = format!(
            "https://crates.io/api/v1/crates?page={page}&per_page={per_page}&sort=downloads"
        );
        let result = fetch_url(&api_url).await?;

        let json: serde_json::Value = serde_json::from_str(&result.html).map_err(|e| {
            oi_types::Error::Internal(format!("failed to parse crates.io response: {e}"))
        })?;

        if let Some(crates) = json["crates"].as_array() {
            for krate in crates {
                if let Some(name) = krate["id"].as_str() {
                    crate_names.push(name.to_string());
                    if crate_names.len() >= top_n {
                        break;
                    }
                }
            }
        }

        if crate_names.len() >= top_n {
            break;
        }

        // Rate-limit: 500ms between API pages
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    crate_names.truncate(top_n);

    let urls: Vec<String> = crate_names
        .iter()
        .map(|name| format!("https://docs.rs/crate/{name}/latest"))
        .collect();

    Ok(urls)
}

/// Fetch the MDN English sitemap and extract all documentation URLs.
pub async fn urls_from_mdn() -> oi_types::Result<Vec<String>> {
    let sitemap_url = "https://developer.mozilla.org/sitemaps/en-us/sitemap.xml.gz";
    urls_from_sitemap(sitemap_url).await
}

// ── Core pipeline ───────────────────────────────────────────────────────────

/// Crawl a batch of URLs and ingest them into the local index.
async fn crawl_and_ingest(
    _config: &CliConfig,
    urls: Vec<String>,
    concurrency: usize,
    verbose: bool,
) -> ExitCode {
    // Set up local index path
    let index_path = match dirs::home_dir() {
        Some(h) => h.join(".opensonarx").join("index"),
        None => {
            eprintln!("Cannot determine home directory");
            return ExitCode::UserError;
        }
    };
    let _ = std::fs::create_dir_all(&index_path);

    let sdk_config = SdkConfig {
        local_index_path: Some(index_path.to_str().unwrap_or_default().to_string()),
        ..Default::default()
    };
    let client = OpenSonarXClient::new(sdk_config);

    let distiller = Distiller::new().with_embedder(client.embedder_arc());

    let robots = Arc::new(RwLock::new(RobotsChecker::new()));
    let semaphore = Arc::new(Semaphore::new(concurrency));

    // Atomic progress counters
    let total = urls.len();
    let indexed = Arc::new(AtomicUsize::new(0));
    let failed = Arc::new(AtomicUsize::new(0));
    let robots_skipped = Arc::new(AtomicUsize::new(0));
    let processed = Arc::new(AtomicUsize::new(0));

    // Progress reporting task
    let indexed_prog = indexed.clone();
    let failed_prog = failed.clone();
    let robots_prog = robots_skipped.clone();
    let processed_prog = processed.clone();
    let progress_handle = tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            let p = processed_prog.load(Ordering::Relaxed);
            let i = indexed_prog.load(Ordering::Relaxed);
            let f = failed_prog.load(Ordering::Relaxed);
            let r = robots_prog.load(Ordering::Relaxed);
            eprintln!(
                "[progress] {p}/{total} processed — {i} indexed, {f} failed, {r} robots-skipped"
            );
            if p >= total {
                break;
            }
        }
    });

    let distiller = Arc::new(distiller);
    let client = Arc::new(client);
    let save_interval = 500;

    let mut join_set = JoinSet::new();

    for url in urls {
        let permit = semaphore.clone().acquire_owned().await.unwrap();
        let distiller = distiller.clone();
        let client = client.clone();
        let robots = robots.clone();
        let indexed = indexed.clone();
        let failed = failed.clone();
        let robots_skipped = robots_skipped.clone();
        let processed = processed.clone();

        join_set.spawn(async move {
            let _permit = permit;

            // Check robots.txt
            if let Some(domain) = extract_domain(&url) {
                let needs_fetch = {
                    let checker = robots.read().await;
                    !checker.has_rules(&domain)
                };
                if needs_fetch {
                    let robots_url = format!("https://{domain}/robots.txt");
                    if let Ok(result) = fetch_url(&robots_url).await {
                        let rules = parse_robots_txt(&result.html);
                        let mut checker = robots.write().await;
                        checker.add_rules(&domain, rules);
                    }
                }

                let checker = robots.read().await;
                if !checker.is_allowed(&url) {
                    robots_skipped.fetch_add(1, Ordering::Relaxed);
                    processed.fetch_add(1, Ordering::Relaxed);
                    if verbose {
                        eprintln!("[SKIP] robots.txt blocked: {url}");
                    }
                    return;
                }
            }

            // Crawl
            match distiller.crawl(&url).await {
                Ok(output) => {
                    if output.chunks.is_empty() {
                        failed.fetch_add(1, Ordering::Relaxed);
                        if verbose {
                            eprintln!("[SKIP] no content: {url}");
                        }
                    } else {
                        match client.ingest(&output.chunks) {
                            Ok(()) => {
                                let count = indexed.fetch_add(1, Ordering::Relaxed) + 1;
                                if verbose {
                                    eprintln!(
                                        "[OK] {} — {} chunks",
                                        url,
                                        output.chunks.len()
                                    );
                                }
                                // Periodic save
                                if count.is_multiple_of(save_interval)
                                    && let Ok(saved) = client.save_index()
                                    && saved && verbose
                                {
                                    eprintln!("[SAVE] index saved at {count} docs");
                                }
                            }
                            Err(e) => {
                                failed.fetch_add(1, Ordering::Relaxed);
                                if verbose {
                                    eprintln!("[ERR] ingest {url}: {e}");
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    failed.fetch_add(1, Ordering::Relaxed);
                    if verbose {
                        eprintln!("[ERR] crawl {url}: {e}");
                    }
                }
            }

            processed.fetch_add(1, Ordering::Relaxed);
        });
    }

    // Wait for all tasks
    while join_set.join_next().await.is_some() {}

    // Cancel progress reporter
    progress_handle.abort();

    // Final save
    match client.save_index() {
        Ok(true) => println!("Index saved to {}", index_path.display()),
        Ok(false) => eprintln!("Warning: no storage configured, index not persisted"),
        Err(e) => eprintln!("Failed to save index: {e}"),
    }

    // Summary
    let i = indexed.load(Ordering::Relaxed);
    let f = failed.load(Ordering::Relaxed);
    let r = robots_skipped.load(Ordering::Relaxed);
    println!("\nSeed complete:");
    println!("  Total URLs:      {total}");
    println!("  Indexed:         {i}");
    println!("  Failed:          {f}");
    println!("  Robots-skipped:  {r}");
    println!("  Docs in index:   {}", client.index().doc_count());

    ExitCode::Success
}

// ── Spider-based crawl ──────────────────────────────────────────────────────

/// Crawl a site by following links from a start URL using the Spider.
async fn run_spider_crawl(
    _config: &CliConfig,
    url: &str,
    depth: usize,
    max_pages: usize,
    concurrency: usize,
    verbose: bool,
) -> ExitCode {
    println!("Spidering {url} (depth={depth}, max_pages={max_pages}, concurrency={concurrency})");

    let index_path = match dirs::home_dir() {
        Some(h) => h.join(".opensonarx").join("index"),
        None => {
            eprintln!("Cannot determine home directory");
            return ExitCode::UserError;
        }
    };
    let _ = std::fs::create_dir_all(&index_path);

    let sdk_config = SdkConfig {
        local_index_path: Some(index_path.to_str().unwrap_or_default().to_string()),
        ..Default::default()
    };
    let client = Arc::new(OpenSonarXClient::new(sdk_config));

    let distiller = Arc::new(Distiller::new().with_embedder(client.embedder_arc()));

    let spider_config = SpiderConfig {
        max_depth: depth,
        max_pages,
        concurrency,
        respect_robots: true,
        same_domain_only: true,
    };
    let spider = Spider::new(spider_config, distiller);

    let indexed = Arc::new(AtomicUsize::new(0));
    let ingest_failed = Arc::new(AtomicUsize::new(0));
    let client_cb = client.clone();
    let indexed_cb = indexed.clone();
    let ingest_failed_cb = ingest_failed.clone();
    let verbose_cb = verbose;

    let result = spider
        .crawl(vec![url.to_string()], move |output| {
            if output.chunks.is_empty() {
                return;
            }
            let url = output.url.clone();
            match client_cb.ingest(&output.chunks) {
                Ok(()) => {
                    indexed_cb.fetch_add(1, Ordering::Relaxed);
                    if verbose_cb {
                        eprintln!("[OK] {} — {} chunks", url, output.chunks.len());
                    }
                }
                Err(e) => {
                    ingest_failed_cb.fetch_add(1, Ordering::Relaxed);
                    if verbose_cb {
                        eprintln!("[ERR] ingest {url}: {e}");
                    }
                }
            }
        })
        .await;

    // Save index
    match client.save_index() {
        Ok(true) => println!("Index saved to {}", index_path.display()),
        Ok(false) => eprintln!("Warning: no storage configured, index not persisted"),
        Err(e) => eprintln!("Failed to save index: {e}"),
    }

    let i = indexed.load(Ordering::Relaxed);
    let if_ = ingest_failed.load(Ordering::Relaxed);
    println!("\nSpider complete:");
    println!("  URLs discovered: {}", result.urls_discovered);
    println!("  Pages crawled:   {}", result.pages_crawled);
    println!("  Pages failed:    {}", result.pages_failed);
    println!("  Robots-skipped:  {}", result.pages_skipped_robots);
    println!("  Indexed:         {i}");
    if if_ > 0 {
        println!("  Ingest failed:   {if_}");
    }
    println!("  Docs in index:   {}", client.index().doc_count());

    ExitCode::Success
}

/// Extract domain from a URL string.
fn extract_domain(url: &str) -> Option<String> {
    let after_scheme = url.split("//").nth(1)?;
    let host = after_scheme.split('/').next()?;
    // Strip port if present
    let domain = host.split(':').next()?;
    if domain.is_empty() {
        None
    } else {
        Some(domain.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_domain() {
        assert_eq!(
            extract_domain("https://docs.rs/tokio/latest/tokio/"),
            Some("docs.rs".to_string())
        );
        assert_eq!(
            extract_domain("https://developer.mozilla.org/en-US/docs/Web"),
            Some("developer.mozilla.org".to_string())
        );
        assert_eq!(extract_domain("not-a-url"), None);
    }

    #[test]
    fn test_urls_from_file() {
        let dir = std::env::temp_dir();
        let path = dir.join("oi-seed-test-urls.txt");
        std::fs::write(
            &path,
            "# comment line\nhttps://example.com\n\nhttps://docs.rs\n# another comment\n",
        )
        .unwrap();

        let urls = urls_from_file(path.to_str().unwrap()).unwrap();
        assert_eq!(urls, vec!["https://example.com", "https://docs.rs"]);

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn test_urls_from_file_empty() {
        let dir = std::env::temp_dir();
        let path = dir.join("oi-seed-test-empty.txt");
        std::fs::write(&path, "# only comments\n\n").unwrap();

        let urls = urls_from_file(path.to_str().unwrap()).unwrap();
        assert!(urls.is_empty());

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn test_urls_from_file_not_found() {
        let result = urls_from_file("/nonexistent/path.txt");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_urls_from_sitemap_parses_xml() {
        // Test the parsing portion by directly calling parse_sitemap
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
            <urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
                <url><loc>https://example.com/page1</loc></url>
                <url><loc>https://example.com/page2</loc></url>
            </urlset>"#;
        let entries = parse_sitemap(xml);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].url, "https://example.com/page1");
        assert_eq!(entries[1].url, "https://example.com/page2");
    }

    #[tokio::test]
    async fn test_fetch_sitemap_xml_gz_decode() {
        // Create a gzipped sitemap in memory and verify GzDecoder works
        use flate2::write::GzEncoder;
        use std::io::Write;

        let xml = r#"<?xml version="1.0"?><urlset><url><loc>https://test.com/a</loc></url></urlset>"#;
        let mut encoder = GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(xml.as_bytes()).unwrap();
        let compressed = encoder.finish().unwrap();

        // Verify GzDecoder round-trips correctly
        let mut decoder = flate2::read::GzDecoder::new(&compressed[..]);
        let mut decoded = String::new();
        decoder.read_to_string(&mut decoded).unwrap();
        assert_eq!(decoded, xml);
    }

    #[test]
    fn test_sitemap_separates_nested_from_urls() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
            <sitemapindex xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
                <sitemap><loc>https://example.com/sitemap1.xml</loc></sitemap>
                <sitemap><loc>https://example.com/sitemap2.xml.gz</loc></sitemap>
            </sitemapindex>"#;
        let entries = parse_sitemap(xml);

        let nested: Vec<_> = entries
            .iter()
            .filter(|e| e.url.ends_with(".xml") || e.url.ends_with(".xml.gz"))
            .collect();
        let regular: Vec<_> = entries
            .iter()
            .filter(|e| !e.url.ends_with(".xml") && !e.url.ends_with(".xml.gz"))
            .collect();

        assert_eq!(nested.len(), 2);
        assert!(regular.is_empty());
    }
}
