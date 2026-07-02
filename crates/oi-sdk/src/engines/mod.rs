pub mod bing;
pub mod brave;
pub mod cache;
pub mod circuit_breaker;
pub mod duckduckgo;
pub mod google;
pub mod mojeek;
pub mod qwant;
pub mod searxng;
pub mod serper;
pub mod stract;
pub mod yep;

use crate::config::{SdkConfig, SearchEngineConfig};
use oi_types::{SearchEngine, SearchParams, SearchResultItem};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

/// Per-engine HTTP timeout for external search requests.
const ENGINE_HTTP_TIMEOUT: Duration = Duration::from_secs(10);

/// Create an HTTP agent with search-appropriate timeouts.
/// All engine adapters should use this instead of the global `ureq::get()`/`ureq::post()`.
pub(crate) fn http_agent() -> ureq::Agent {
    ureq::Agent::new_with_config(
        ureq::config::Config::builder()
            .timeout_global(Some(ENGINE_HTTP_TIMEOUT))
            .build(),
    )
}

// ── Shared HTML utilities ──────────────────────────────────────────────────

/// Minimal URL-encoding for query strings.
pub(crate) fn urlencoded(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push('+'),
            _ => {
                out.push('%');
                out.push(char::from(HEX[(b >> 4) as usize]));
                out.push(char::from(HEX[(b & 0xF) as usize]));
            }
        }
    }
    out
}

pub(crate) const HEX: [u8; 16] = *b"0123456789ABCDEF";

/// Basic percent-decoding.
pub(crate) fn url_decode(s: &str) -> String {
    let mut out = Vec::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2]))
        {
            out.push((hi << 4) | lo);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub(crate) fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(10 + b - b'a'),
        b'A'..=b'F' => Some(10 + b - b'A'),
        _ => None,
    }
}

/// Strip HTML tags from a string.
pub(crate) fn strip_html_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    html_decode(&out)
}

/// Decode common HTML entities.
pub(crate) fn html_decode(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&apos;", "'")
}

/// Extract an attribute value from an HTML tag fragment.
pub(crate) fn extract_attr(tag: &str, attr: &str) -> Option<String> {
    let pattern = format!("{attr}=\"");
    let start = tag.find(&pattern)? + pattern.len();
    let end = tag[start..].find('"')? + start;
    Some(html_decode(&tag[start..end]))
}

/// Extract text content after the first `>` until `</`.
pub(crate) fn extract_tag_text(html: &str) -> String {
    let start = match html.find('>') {
        Some(i) => i + 1,
        None => return String::new(),
    };
    let end = html[start..].find("</").map(|i| i + start).unwrap_or(html.len());
    html[start..end].to_string()
}

// ── Engine orchestration ───────────────────────────────────────────────────

/// Query all engines in parallel, dedup results by URL (priority order wins), cap at `limit`.
pub fn search_with_engines(
    engines: &[Arc<dyn SearchEngine>],
    query: &str,
    limit: usize,
    params: &SearchParams,
) -> Vec<SearchResultItem> {
    let total_start = std::time::Instant::now();

    // Capture engine names before spawning (engine order = priority order)
    let engine_names: Vec<String> = engines.iter().map(|e| e.name().to_string()).collect();

    // Query all engines in parallel using scoped threads
    let mut engine_outputs: Vec<(usize, String, Vec<SearchResultItem>, u128)> =
        std::thread::scope(|s| {
            let handles: Vec<_> = engines
                .iter()
                .enumerate()
                .filter(|(_, e)| e.is_available())
                .map(|(idx, engine)| {
                    let engine = Arc::clone(engine);
                    let q = query.to_string();
                    let p = params.clone();
                    s.spawn(move || {
                        let start = std::time::Instant::now();
                        let results = engine.search_with_params(&q, limit, &p);
                        let ms = start.elapsed().as_millis();
                        (idx, engine.name().to_string(), results, ms)
                    })
                })
                .collect();

            handles
                .into_iter()
                .filter_map(|h| h.join().ok())
                .collect()
        });

    // Sort by original engine index (= priority order)
    engine_outputs.sort_by_key(|(idx, _, _, _)| *idx);

    // Dedup by URL, priority engines win
    let mut results = Vec::new();
    let mut seen_urls = HashSet::new();

    for (_, name, engine_results, ms) in &engine_outputs {
        tracing::info!(
            engine = %name,
            returned = engine_results.len(),
            elapsed_ms = *ms as u64,
            "engine query (parallel)"
        );
    }

    for (_, _, engine_results, _) in engine_outputs {
        for r in engine_results {
            if seen_urls.insert(r.url.clone()) {
                results.push(r);
            }
        }
    }

    tracing::info!(
        total_results = results.len(),
        engines_queried = engine_names.len(),
        total_ms = total_start.elapsed().as_millis() as u64,
        "search_with_engines complete (parallel)"
    );

    results.truncate(limit);
    for (i, r) in results.iter_mut().enumerate() {
        r.rank = (i + 1) as u32;
    }

    results
}

/// Wrap a raw engine with circuit breaker + cache layers.
fn wrap_engine(engine: Arc<dyn SearchEngine>) -> Arc<dyn SearchEngine> {
    // Order: cache → circuit_breaker → inner
    // Cache sits outermost so cache hits don't trigger breaker logic.
    let with_breaker = Arc::new(circuit_breaker::CircuitBreakerEngine::new(engine));
    Arc::new(cache::CachedEngine::new(with_breaker))
}

/// Build the default engine list from SDK config.
///
/// If `search_engines` is configured, uses those (sorted by priority).
/// Falls back to legacy behavior: DDG always, Brave if `brave_api_key` is set.
///
/// Every engine is automatically wrapped with a circuit breaker (exponential
/// backoff on failures) and a response cache (5-minute TTL).
pub fn default_engines(config: &SdkConfig) -> Vec<Arc<dyn SearchEngine>> {
    if let Some(ref engine_configs) = config.search_engines {
        let mut configs: Vec<&SearchEngineConfig> =
            engine_configs.iter().filter(|c| c.enabled).collect();
        configs.sort_by_key(|c| c.priority);

        let mut engines: Vec<Arc<dyn SearchEngine>> = Vec::new();
        for ec in configs {
            if let Some(engine) = build_engine(ec) {
                engines.push(wrap_engine(engine));
            }
        }
        if engines.is_empty() {
            // Fallback: always have at least DDG
            engines.push(wrap_engine(Arc::new(duckduckgo::DuckDuckGo)));
        }
        engines
    } else {
        // Legacy mode: API-backed engines first (Google-quality results,
        // reliable), free DuckDuckGo scrape as the always-on fallback.
        let mut engines: Vec<Arc<dyn SearchEngine>> = Vec::new();
        if let Some(ref key) = config.serper_api_key {
            engines.push(wrap_engine(Arc::new(serper::Serper::new(key.clone()))));
        }
        if let Some(ref key) = config.brave_api_key {
            engines.push(wrap_engine(Arc::new(brave::BraveSearch::new(key.clone()))));
        }
        engines.push(wrap_engine(Arc::new(duckduckgo::DuckDuckGo)));
        engines
    }
}

/// Build a single engine from its config entry.
///
/// Key-free engine status (probed 2026-07-01): duckduckgo WORKS (the only
/// reliable key-free engine); google/bing scrapers return silently empty
/// (bot walls); qwant 403 (DataDome); stract 404 (service appears shut
/// down); yep 501. mojeek requires a paid key for its JSON API. Persistent
/// failures trip the per-engine circuit breaker, so dead engines self-mute
/// — but production deployments should configure at least one API-backed
/// engine (brave/serper) rather than lean on scrapers.
fn build_engine(ec: &SearchEngineConfig) -> Option<Arc<dyn SearchEngine>> {
    match ec.name.to_lowercase().as_str() {
        "duckduckgo" | "ddg" => Some(Arc::new(duckduckgo::DuckDuckGo)),
        "brave" => {
            let key = ec.api_key.as_ref()?;
            Some(Arc::new(brave::BraveSearch::new(key.clone())))
        }
        "mojeek" => Some(Arc::new(mojeek::Mojeek::new(ec.api_key.clone()))),
        "searxng" | "searx" => {
            let url = ec.api_key.as_ref()?; // api_key holds instance URL for SearXNG
            Some(Arc::new(searxng::SearXNG::new(url.clone())))
        }
        "google" => Some(Arc::new(google::GoogleScraper)),
        "bing" => Some(Arc::new(bing::BingScraper)),
        "qwant" => Some(Arc::new(qwant::Qwant)),
        "yep" => Some(Arc::new(yep::Yep)),
        "stract" => Some(Arc::new(stract::Stract)),
        "serper" => {
            let key = ec.api_key.as_ref()?;
            Some(Arc::new(serper::Serper::new(key.clone())))
        }
        _ => {
            tracing::warn!("Unknown search engine: {}", ec.name);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_urlencoded() {
        assert_eq!(urlencoded("hello world"), "hello+world");
        assert_eq!(urlencoded("rust+lang"), "rust%2Blang");
        assert_eq!(urlencoded("a b&c"), "a+b%26c");
    }

    #[test]
    fn test_url_decode() {
        assert_eq!(url_decode("hello%20world"), "hello world");
        assert_eq!(url_decode("https%3A%2F%2Fexample.com"), "https://example.com");
    }

    #[test]
    fn test_strip_html_tags() {
        assert_eq!(strip_html_tags("<b>bold</b> text"), "bold text");
        assert_eq!(strip_html_tags("no tags"), "no tags");
        assert_eq!(strip_html_tags("<a href=\"x\">link</a>"), "link");
    }

    #[test]
    fn test_html_decode() {
        assert_eq!(html_decode("a &amp; b"), "a & b");
        assert_eq!(html_decode("&lt;tag&gt;"), "<tag>");
    }

    #[test]
    fn test_search_with_engines_dedup() {
        struct FakeEngine {
            name: &'static str,
            results: Vec<SearchResultItem>,
        }
        impl SearchEngine for FakeEngine {
            fn name(&self) -> &str { self.name }
            fn search(&self, _query: &str, _limit: usize) -> Vec<SearchResultItem> {
                self.results.clone()
            }
        }

        fn make_result(url: &str, source: &str) -> SearchResultItem {
            SearchResultItem {
                result_id: format!("{source}_1"),
                rank: 1,
                url: url.to_string(),
                title: "Test".to_string(),
                snippet: String::new(),
                content_md: String::new(),
                semantic_score: 0.0,
                bm25_score: 0.0,
                final_score: 0.0,
                stake_truth: 0,
                stake_usd: 0,
                stake_type: "none".to_string(),
                reputation: 0.0,
                content_hash: String::new(),
                extraction_quality: "external".to_string(),
                source: source.to_string(),
                entity: None,
                content_kind: None,
                crawled_at: None,
                published_at: None,
                trust_verdict: "external".to_string(),
                reputation_votes: 0,
                disputed: false,
                facts: None,
                content_relevant: None,
                domain_first_seen_at: None,
                domain_url_count: None,
                domain_reputation_score: None,
                domain_total_votes: None,
            }
        }

        let engines: Vec<Arc<dyn SearchEngine>> = vec![
            Arc::new(FakeEngine {
                name: "engine_a",
                results: vec![
                    make_result("https://example.com", "a"),
                    make_result("https://unique-a.com", "a"),
                ],
            }),
            Arc::new(FakeEngine {
                name: "engine_b",
                results: vec![
                    make_result("https://example.com", "b"), // dupe
                    make_result("https://unique-b.com", "b"),
                ],
            }),
        ];

        let results = search_with_engines(&engines, "test", 10, &SearchParams::default());
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].url, "https://example.com");
        assert_eq!(results[1].url, "https://unique-a.com");
        assert_eq!(results[2].url, "https://unique-b.com");
        // Ranks should be re-assigned
        assert_eq!(results[0].rank, 1);
        assert_eq!(results[1].rank, 2);
        assert_eq!(results[2].rank, 3);
    }

    #[test]
    fn test_default_engines_legacy_mode() {
        let config = SdkConfig::default();
        let engines = default_engines(&config);
        assert_eq!(engines.len(), 1);
        assert_eq!(engines[0].name(), "duckduckgo");
    }

    #[test]
    fn test_default_engines_with_brave_key() {
        // API-backed engines come first; free DDG scrape is the fallback.
        let config = SdkConfig {
            brave_api_key: Some("test_key".to_string()),
            ..SdkConfig::default()
        };
        let engines = default_engines(&config);
        assert_eq!(engines.len(), 2);
        assert_eq!(engines[0].name(), "brave");
        assert_eq!(engines[1].name(), "duckduckgo");
    }

    #[test]
    fn test_default_engines_with_serper_key() {
        let config = SdkConfig {
            serper_api_key: Some("test_key".to_string()),
            brave_api_key: Some("test_key".to_string()),
            ..SdkConfig::default()
        };
        let engines = default_engines(&config);
        assert_eq!(engines.len(), 3);
        assert_eq!(engines[0].name(), "serper");
        assert_eq!(engines[1].name(), "brave");
        assert_eq!(engines[2].name(), "duckduckgo");
    }

    #[test]
    fn test_default_engines_configured() {
        let config = SdkConfig {
            search_engines: Some(vec![
                SearchEngineConfig {
                    name: "qwant".to_string(),
                    api_key: None,
                    enabled: true,
                    priority: 2,
                },
                SearchEngineConfig {
                    name: "duckduckgo".to_string(),
                    api_key: None,
                    enabled: true,
                    priority: 1,
                },
                SearchEngineConfig {
                    name: "stract".to_string(),
                    api_key: None,
                    enabled: false,
                    priority: 3,
                },
            ]),
            ..SdkConfig::default()
        };
        let engines = default_engines(&config);
        assert_eq!(engines.len(), 2); // stract disabled
        assert_eq!(engines[0].name(), "duckduckgo"); // priority 1
        assert_eq!(engines[1].name(), "qwant"); // priority 2
    }
}
