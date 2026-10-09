pub mod bing;
pub mod brave;
pub mod brave_html;
pub mod cache;
pub mod circuit_breaker;
pub mod duckduckgo;
pub mod google;
pub mod mojeek;
pub mod qwant;
pub mod searxng;
pub mod serper;
pub mod stract;
pub mod wikipedia;
pub mod yep;

use crate::config::{SdkConfig, SearchEngineConfig};
use oi_types::{SearchEngine, SearchParams, SearchResultItem};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

/// Per-engine HTTP timeout for external search requests.
// Key-free scrapers are a fallback, not a license to consume the entire
// serving budget. Typical DDG responses are sub-second in the live suite;
// after five seconds a fresh local snapshot is preferable to a 10s tail.
const ENGINE_HTTP_TIMEOUT: Duration = Duration::from_secs(5);
const ENGINE_DNS_TIMEOUT: Duration = Duration::from_millis(1_500);
const ENGINE_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
// Serving deadline is intentionally tighter than the per-worker HTTP budget:
// workers may finish and update their circuit state later, but a live query
// must not spend five seconds waiting for discovery when retained evidence can
// answer or explicitly abstain. Healthy keyless-provider responses in the live
// runs were normally sub-second; 1.75 seconds leaves DNS plus response headroom
// while two bounded diligence passes stay inside the four-second answer SLO.
const ENGINE_ORCHESTRATION_TIMEOUT: Duration = Duration::from_millis(1_750);
// Contracted APIs do not share the keyless HTML response distribution. The
// 1.75s cutoff discarded otherwise successful API responses in paired live
// captures. This bounded profile trades latency for availability; it is not
// a production SLO assertion, and does not relax evidence/relevance gates.
const API_ENGINE_ORCHESTRATION_TIMEOUT: Duration = Duration::from_secs(3);
// The default keyless HTML adapters are fallbacks, not contracted APIs.
// Querying all of them for every request exhausts secondary providers long before
// it is needed. Bing normally responds in ~300 ms in the live matrix; hedge
// after 750 ms or immediately when its surface is shallow/off-topic.
const KEYLESS_PRIMARY_HEDGE_DELAY: Duration = Duration::from_millis(750);
const KEYLESS_SECONDARY_HEDGE_DELAY: Duration = Duration::from_millis(750);

/// Create an HTTP agent with search-appropriate timeouts.
/// All engine adapters should use this instead of the global `ureq::get()`/`ureq::post()`.
pub(crate) fn http_agent() -> ureq::Agent {
    http_agent_status_policy(true)
}

/// Inspect provider status bodies safely without changing other adapters.
pub(crate) fn http_agent_status_policy(status_as_error: bool) -> ureq::Agent {
    ureq::Agent::new_with_config(
        ureq::config::Config::builder()
            .http_status_as_error(status_as_error)
            .timeout_global(Some(ENGINE_HTTP_TIMEOUT))
            // Keep explicit phase budgets in addition to the global budget.
            // Some platform resolvers otherwise dominate the serving tail
            // when DNS is degraded before an HTTP connection exists.
            .timeout_resolve(Some(ENGINE_DNS_TIMEOUT))
            .timeout_connect(Some(ENGINE_CONNECT_TIMEOUT))
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
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
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
    // Search providers frequently HTML-escape every non-ASCII character in
    // titles and snippets (for example `computa&#231;&#227;o`). Leaving numeric
    // entities encoded is not merely cosmetic: query-relative lexical gates
    // then see `computa`, `231`, and `227` instead of `computacao`, and can
    // discard an otherwise healthy Portuguese result set. Decode bounded
    // named and numeric entities before any semantic/provider sanity check.
    let mut decoded = String::with_capacity(s.len());
    let mut remaining = s;
    while let Some(entity_start) = remaining.find('&') {
        decoded.push_str(&remaining[..entity_start]);
        let entity_tail = &remaining[entity_start + 1..];
        let Some(entity_end) = entity_tail.find(';').filter(|end| *end <= 12) else {
            decoded.push('&');
            remaining = entity_tail;
            continue;
        };
        let entity = &entity_tail[..entity_end];
        let value = match entity {
            "amp" => Some('&'),
            "apos" => Some('\''),
            "gt" => Some('>'),
            "lt" => Some('<'),
            "quot" => Some('"'),
            _ => entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                .and_then(|digits| u32::from_str_radix(digits, 16).ok())
                .or_else(|| {
                    entity
                        .strip_prefix('#')
                        .and_then(|digits| digits.parse::<u32>().ok())
                })
                .and_then(char::from_u32),
        };
        if let Some(value) = value {
            decoded.push(value);
        } else {
            decoded.push('&');
            decoded.push_str(entity);
            decoded.push(';');
        }
        remaining = &entity_tail[entity_end + 1..];
    }
    decoded.push_str(remaining);
    decoded
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
    let end = html[start..]
        .find("</")
        .map(|i| i + start)
        .unwrap_or(html.len());
    html[start..end].to_string()
}

/// Return a byte end no farther than `max_bytes` from `start`, rounded down to
/// a UTF-8 character boundary. Provider HTML is Unicode and arbitrary byte
/// truncation can otherwise panic inside a request worker.
pub(crate) fn bounded_byte_end(text: &str, start: usize, max_bytes: usize) -> usize {
    let mut end = start.saturating_add(max_bytes).min(text.len());
    while end > start && !text.is_char_boundary(end) {
        end -= 1;
    }
    end
}

/// Remove provider and campaign identifiers that do not change the target
/// resource. Cleaning happens before cross-provider deduplication and before a
/// discovery lead can enter the crawl queue, so tracking variants neither
/// pollute displayed destinations nor create duplicate retained documents.
fn strip_tracking_params(raw_url: &str) -> String {
    let Ok(mut parsed) = url::Url::parse(raw_url) else {
        return raw_url.to_string();
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return raw_url.to_string();
    }
    let retained = parsed
        .query_pairs()
        .filter(|(key, _)| {
            let key = key.to_ascii_lowercase();
            !key.starts_with("utm_")
                && !matches!(
                    key.as_str(),
                    "dclid" | "fbclid" | "gclid" | "mc_cid" | "mc_eid" | "msclkid" | "msockid"
                )
        })
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    let original_count = parsed.query_pairs().count();
    if retained.len() == original_count {
        return parsed.to_string();
    }
    if retained.is_empty() {
        parsed.set_query(None);
    } else {
        parsed.query_pairs_mut().clear().extend_pairs(retained);
    }
    parsed.to_string()
}

/// Minimal lexical sanity check for a raw provider result surface.
///
/// This is not the final relevance ranker. It detects a more basic adapter
/// failure: a non-empty parsed page whose entire result set is unrelated to
/// the requested topic (for example, adult URLs returned for an Anthropic
/// founders query after a provider markup change). The query-intent and
/// semantic layers still decide individual candidates after this batch check.
pub fn provider_surface_topic_relevant(query: &str, result: &SearchResultItem) -> bool {
    use crate::query_intent::{
        QueryIntent, classify_query, entity_words, factual_query_terms, relevance_terms,
    };

    const GENERIC_PREDICATES: &[&str] = &[
        "created", "founded", "latest", "news", "noticia", "noticias", "hoje", "today",
    ];
    const QUERY_SCAFFOLD: &[&str] = &[
        "a",
        "an",
        "and",
        "advantage",
        "advantages",
        "are",
        "benefit",
        "benefits",
        "can",
        "could",
        "da",
        "de",
        "did",
        "do",
        "does",
        "e",
        "how",
        "keep",
        "like",
        "look",
        "o",
        "of",
        "on",
        "should",
        "the",
        "what",
        "when",
        "where",
        "which",
        "who",
        "why",
        "with",
        "would",
    ];
    let intent = classify_query(query);
    if intent == QueryIntent::Tool
        && !crate::query_intent::intent_adjustment(query, result)
            .signals
            .contains(&"functional_tool_surface")
    {
        // A page about Python is not necessarily an online Python compiler.
        // The default waterfall must measure usable tool depth, otherwise a
        // topical but non-functional Bing batch suppresses the DDG fallback.
        return false;
    }
    let mut topic_terms = match intent {
        QueryIntent::Navigational
        | QueryIntent::Transactional
        | QueryIntent::Tool
        | QueryIntent::DueDiligence => entity_words(query, intent),
        QueryIntent::Factual => factual_query_terms(query),
        QueryIntent::CurrentEvents | QueryIntent::Exploratory => relevance_terms(query),
    };
    topic_terms.retain(|term| {
        !GENERIC_PREDICATES.contains(&term.as_str()) && !QUERY_SCAFFOLD.contains(&term.as_str())
    });
    topic_terms.sort();
    topic_terms.dedup();
    if topic_terms.is_empty() {
        return true;
    }

    let surface_terms = relevance_terms(&format!(
        "{} {} {}",
        result.title, result.snippet, result.url
    ));
    let matches = topic_terms
        .iter()
        .filter(|topic| {
            surface_terms.iter().any(|surface| {
                topic == &surface
                    || (topic.len() >= 5
                        && surface.len() >= 5
                        && (topic.starts_with(surface.as_str())
                            || surface.starts_with(topic.as_str())))
            })
        })
        .count();
    // Current-event rewrites often contain a full name, office, jurisdiction,
    // and year. A legitimate headline commonly uses only a surname plus the
    // office (for example, "Brazil's President Lula"), so requiring half of
    // every rewrite token can discard an otherwise healthy provider batch.
    // This is only a batch-level parser sanity check: the original-query
    // scope, freshness, crawl, and evidence gates still evaluate every item.
    let required = match intent {
        QueryIntent::CurrentEvents => topic_terms.len().div_ceil(2).clamp(1, 2),
        // This function is a batch-level parser/challenge-page sanity check,
        // not the relevance ranker. Natural answer pages often mention the
        // subject without repeating question scaffolding or the requested
        // predicate in their public title/snippet. One discriminative subject
        // match is enough to preserve the batch for the later semantic,
        // intent, freshness, crawl, and evidence gates.
        QueryIntent::Exploratory => 1,
        QueryIntent::Navigational
        | QueryIntent::Transactional
        | QueryIntent::Tool
        | QueryIntent::DueDiligence
        | QueryIntent::Factual => topic_terms.len().div_ceil(2).max(1),
    };
    matches >= required
}

// ── Engine orchestration ───────────────────────────────────────────────────

/// Query all engines in parallel, dedup results by URL (priority order wins), cap at `limit`.
pub fn search_with_engines(
    engines: &[Arc<dyn SearchEngine>],
    query: &str,
    limit: usize,
    params: &SearchParams,
) -> Vec<SearchResultItem> {
    let timeout = if engines
        .iter()
        .any(|engine| engine.requires_api_key() && engine.is_available())
    {
        API_ENGINE_ORCHESTRATION_TIMEOUT
    } else {
        ENGINE_ORCHESTRATION_TIMEOUT
    };
    search_with_engines_timeout(engines, query, limit, params, timeout)
}

fn search_with_engines_timeout(
    engines: &[Arc<dyn SearchEngine>],
    query: &str,
    limit: usize,
    params: &SearchParams,
    timeout: Duration,
) -> Vec<SearchResultItem> {
    let total_start = std::time::Instant::now();

    // Capture engine names before spawning (engine order = priority order).
    let engine_names: Vec<String> = engines.iter().map(|e| e.name().to_string()).collect();

    // Detached workers let the serving request honor a hard deadline even
    // when a platform DNS resolver ignores the HTTP client's timeout. The
    // per-engine circuit breaker enforces a small in-flight cap, preventing
    // timed-out resolver threads from multiplying under load without turning
    // a healthy provider into a single-request bottleneck.
    let available = engines
        .iter()
        .enumerate()
        .filter(|(_, engine)| engine.is_available())
        .map(|(idx, engine)| (idx, Arc::clone(engine)))
        .collect::<Vec<_>>();
    let default_keyless_waterfall = available.len() >= 2
        && available[0].1.name() == "bing"
        && !available[0].1.requires_api_key()
        && matches!(available[1].1.name(), "brave-html" | "duckduckgo")
        && available
            .iter()
            .skip(1)
            .all(|(_, engine)| !engine.requires_api_key());

    let (sender, receiver) = std::sync::mpsc::channel();
    let spawn_engine = |idx: usize, engine: Arc<dyn SearchEngine>| {
        let sender = sender.clone();
        let q = query.to_string();
        let p = params.clone();
        std::thread::spawn(move || {
            let start = std::time::Instant::now();
            let results = engine.search_with_params(&q, limit, &p);
            let ms = start.elapsed().as_millis();
            let _ = sender.send((idx, engine.name().to_string(), results, ms));
        });
    };

    let mut expected = 0;
    let mut engine_outputs: Vec<(usize, String, Vec<SearchResultItem>, u128)> = Vec::new();
    let mut waterfall_satisfied = false;
    if default_keyless_waterfall {
        let (idx, engine) = &available[0];
        spawn_engine(*idx, Arc::clone(engine));
        expected = 1;

        // A healthy, deep primary surface is enough for discovery. Provider
        // diversity is not source independence: final claims still require
        // fetched pages from distinct source families. Preserve the second
        // keyless adapter for actual availability/relevance fallback.
        let hedge_deadline = std::time::Instant::now() + KEYLESS_PRIMARY_HEDGE_DELAY.min(timeout);
        let primary_healthy = match receiver
            .recv_timeout(hedge_deadline.saturating_duration_since(std::time::Instant::now()))
        {
            Ok(mut output) => {
                sanitize_provider_batch(query, &mut output.2, &output.1);
                let required_topic_hits = limit.min(3).max(1);
                let healthy = output
                    .2
                    .iter()
                    .filter(|result| provider_surface_topic_relevant(query, result))
                    .count()
                    >= required_topic_hits;
                engine_outputs.push(output);
                healthy
            }
            Err(_) => false,
        };
        if !primary_healthy {
            // Give the higher-priority fallback one bounded opportunity before
            // touching the tertiary provider. This prevents a healthy Brave
            // response from needlessly consuming DDG's much tighter bot-wall
            // budget, while a 750 ms hedge still protects the serving tail
            // when Brave is queued or rate-limited.
            let (secondary_idx, secondary) = &available[1];
            spawn_engine(*secondary_idx, Arc::clone(secondary));
            expected += 1;
            let secondary_deadline =
                std::time::Instant::now() + KEYLESS_SECONDARY_HEDGE_DELAY.min(timeout);
            while std::time::Instant::now() < secondary_deadline && engine_outputs.len() < expected
            {
                match receiver.recv_timeout(
                    secondary_deadline.saturating_duration_since(std::time::Instant::now()),
                ) {
                    Ok(mut output) => {
                        sanitize_provider_batch(query, &mut output.2, &output.1);
                        let healthy = provider_batch_is_deep_topical(query, &output.2, limit);
                        engine_outputs.push(output);
                        if healthy {
                            waterfall_satisfied = true;
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            if !waterfall_satisfied {
                for (idx, engine) in available.iter().skip(2) {
                    spawn_engine(*idx, Arc::clone(engine));
                    expected += 1;
                }
            }
        }
    } else {
        for (idx, engine) in &available {
            spawn_engine(*idx, Arc::clone(engine));
            expected += 1;
        }
    }
    drop(sender);

    let deadline = total_start + timeout;
    while engine_outputs.len() < expected && !waterfall_satisfied {
        let now = std::time::Instant::now();
        if now >= deadline {
            break;
        }
        match receiver.recv_timeout(deadline.saturating_duration_since(now)) {
            Ok(mut output) => {
                if default_keyless_waterfall {
                    sanitize_provider_batch(query, &mut output.2, &output.1);
                    if provider_batch_is_deep_topical(query, &output.2, limit) {
                        waterfall_satisfied = true;
                    }
                }
                engine_outputs.push(output);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    if engine_outputs.len() < expected && !waterfall_satisfied {
        tracing::warn!(
            completed = engine_outputs.len(),
            expected,
            timeout_ms = timeout.as_millis() as u64,
            "external search deadline reached"
        );
    }

    // Sort by original engine index (= priority order)
    engine_outputs.sort_by_key(|(idx, _, _, _)| *idx);

    // A parser can return syntactically valid HTTP URLs from the wrong part of
    // a bot wall or malformed SERP. Treat an entirely off-topic batch as empty
    // before interleaving/crawling; one topical hit is enough to preserve the
    // whole batch for the richer downstream semantic and intent gates.
    for (_, engine, results, _) in &mut engine_outputs {
        sanitize_provider_batch(query, results, engine);
    }

    // Interleave by provider rank before truncation. Appending all results
    // from the first-priority engine first lets it consume `limit` by itself,
    // accidentally erasing provider diversity even when other engines
    // succeeded. Priority still wins URL duplicates at the same rank.
    let mut results = Vec::new();
    let mut seen_urls = HashSet::new();

    for (_, name, engine_results, ms) in &engine_outputs {
        tracing::info!(
            engine = %name,
            returned = engine_results.len(),
            elapsed_ms = *ms as u64,
            "engine query (parallel)"
        );
        for result in engine_results.iter().take(5) {
            tracing::debug!(
                engine = %name,
                query = %query,
                rank = result.rank,
                url = %result.url,
                "engine candidate"
            );
        }
    }

    let max_depth = engine_outputs
        .iter()
        .map(|(_, _, engine_results, _)| engine_results.len())
        .max()
        .unwrap_or(0);
    'ranks: for rank in 0..max_depth {
        for (_, _, engine_results, _) in &engine_outputs {
            if let Some(result) = engine_results.get(rank) {
                let mut result = result.clone();
                result.url = strip_tracking_params(&result.url);
                if !seen_urls.insert(result.url.clone()) {
                    continue;
                }
                results.push(result);
                if results.len() == limit {
                    break 'ranks;
                }
            }
        }
    }

    tracing::info!(
        total_results = results.len(),
        engines_configured = engine_names.len(),
        engines_invoked = expected,
        keyless_waterfall = default_keyless_waterfall,
        total_ms = total_start.elapsed().as_millis() as u64,
        "search_with_engines complete"
    );

    results.truncate(limit);
    for (i, r) in results.iter_mut().enumerate() {
        r.rank = (i + 1) as u32;
    }

    results
}

fn provider_batch_is_deep_topical(query: &str, results: &[SearchResultItem], limit: usize) -> bool {
    let required_topic_hits = limit.min(3).max(1);
    results
        .iter()
        .filter(|result| provider_surface_topic_relevant(query, result))
        .count()
        >= required_topic_hits
}

fn sanitize_provider_batch(query: &str, results: &mut Vec<SearchResultItem>, engine: &str) {
    if results.is_empty()
        || results
            .iter()
            .any(|result| provider_surface_topic_relevant(query, result))
    {
        return;
    }
    if should_warn_semantic_poison(engine) {
        tracing::warn!(
            engine,
            query,
            parsed_results = results.len(),
            "discarding semantically poisoned provider batch"
        );
    } else {
        tracing::debug!(
            engine,
            query,
            parsed_results = results.len(),
            "discarding semantically poisoned provider batch"
        );
    }
    results.clear();
}

fn should_warn_semantic_poison(engine: &str) -> bool {
    const WARNING_INTERVAL: Duration = Duration::from_secs(60);
    static LAST_WARNING: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, std::time::Instant>>,
    > = std::sync::OnceLock::new();
    let warnings = LAST_WARNING.get_or_init(Default::default);
    let Ok(mut warnings) = warnings.lock() else {
        return true;
    };
    let now = std::time::Instant::now();
    let should_warn = warnings
        .get(engine)
        .is_none_or(|last| now.duration_since(*last) >= WARNING_INTERVAL);
    if should_warn {
        warnings.insert(engine.to_string(), now);
    }
    should_warn
}

/// Wrap a raw engine with circuit breaker + cache layers.
fn wrap_engine(
    engine: Arc<dyn SearchEngine>,
    persistent_cache_dir: Option<&std::path::Path>,
) -> Arc<dyn SearchEngine> {
    // The raw adapter preserves typed transport/parser failure for the breaker;
    // the cache stays outermost and never stores the resulting empty vector.
    // Legitimate zero-result responses remain healthy and are also left
    // uncached so a later broader rewrite can recover recall.
    let engine_name = engine.name().to_ascii_lowercase();
    // HTML/keyless endpoints are materially more sensitive to short bursts
    // than contracted JSON APIs. Two in-flight calls still overlap network
    // latency, while the breaker's bounded backpressure serializes the tail of
    // a three-rewrite workload instead of provoking a provider-wide bot wall.
    let max_in_flight = if engine.requires_api_key() {
        4
    } else if engine.name() == "brave-html" {
        // The public Brave surface passed sustained serial probes but returned
        // 429 when two fallback rewrites overlapped in the concurrent E2E
        // suite. Serialize this best-effort adapter; the outer waterfall still
        // runs DDG in parallel and preserves its two-second serving deadline.
        1
    } else {
        2
    };
    let protected = Arc::new(circuit_breaker::CircuitBreakerEngine::with_config(
        engine,
        circuit_breaker::CircuitBreakerConfig {
            max_in_flight,
            ..Default::default()
        },
    ));
    if let Some(cache_dir) = persistent_cache_dir {
        let safe_name = engine_name
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || character == '-' {
                    character
                } else {
                    '_'
                }
            })
            .collect::<String>();
        Arc::new(cache::CachedEngine::with_persistence(
            protected,
            Duration::from_secs(300),
            cache_dir.join(format!("{safe_name}.json")),
        ))
    } else {
        Arc::new(cache::CachedEngine::new(protected))
    }
}

/// Build the default engine list from SDK config.
///
/// If `search_engines` is configured, uses those (sorted by priority).
/// Falls back to legacy behavior: Bing, Brave HTML, and DDG keyless fallbacks,
/// with contracted Brave or Serper APIs ahead of them when keys are configured.
///
/// Every engine is automatically wrapped with a circuit breaker (exponential
/// backoff on failures) and a response cache (5-minute TTL).
pub fn default_engines(config: &SdkConfig) -> Vec<Arc<dyn SearchEngine>> {
    let persistent_cache_dir = config
        .local_index_path
        .as_deref()
        .map(std::path::Path::new)
        .map(|path| path.join("discovery-cache"));
    if let Some(ref engine_configs) = config.search_engines {
        let mut configs: Vec<&SearchEngineConfig> =
            engine_configs.iter().filter(|c| c.enabled).collect();
        configs.sort_by_key(|c| c.priority);

        let mut engines: Vec<Arc<dyn SearchEngine>> = Vec::new();
        for ec in configs {
            if let Some(engine) = build_engine(ec) {
                engines.push(wrap_engine(engine, persistent_cache_dir.as_deref()));
            }
        }
        if engines.is_empty() {
            // Fallback: always have at least DDG
            engines.push(wrap_engine(
                Arc::new(duckduckgo::DuckDuckGo),
                persistent_cache_dir.as_deref(),
            ));
        }
        engines
    } else {
        // Legacy mode: API-backed engines first, then independent keyless
        // fallbacks. Each fallback has its own breaker, so a bot wall or HTML
        // change in one provider does not suppress the other.
        let mut engines: Vec<Arc<dyn SearchEngine>> = Vec::new();
        if let Some(ref key) = config.serper_api_key {
            engines.push(wrap_engine(
                Arc::new(serper::Serper::new(key.clone())),
                persistent_cache_dir.as_deref(),
            ));
        }
        if let Some(ref key) = config.brave_api_key {
            engines.push(wrap_engine(
                Arc::new(brave::BraveSearch::new(key.clone())),
                persistent_cache_dir.as_deref(),
            ));
        }
        engines.push(wrap_engine(
            Arc::new(bing::BingScraper),
            persistent_cache_dir.as_deref(),
        ));
        engines.push(wrap_engine(
            Arc::new(brave_html::BraveHtml),
            persistent_cache_dir.as_deref(),
        ));
        engines.push(wrap_engine(
            Arc::new(duckduckgo::DuckDuckGo),
            persistent_cache_dir.as_deref(),
        ));
        engines
    }
}

/// Build the bounded encyclopedia-reference fallback separately from general
/// web discovery. Keeping it out of `default_engines` prevents one corpus
/// from masquerading as a redundant web provider in health/readiness probes.
pub(crate) fn factual_reference_engines(config: &SdkConfig) -> Vec<Arc<dyn SearchEngine>> {
    let persistent_cache_dir = config
        .local_index_path
        .as_deref()
        .map(std::path::Path::new)
        .map(|path| path.join("discovery-cache"));
    vec![wrap_engine(
        Arc::new(wikipedia::WikipediaSearch),
        persistent_cache_dir.as_deref(),
    )]
}

/// Build a single engine from its config entry.
///
/// Key-free engine status is inherently volatile. Bing, Brave HTML, and
/// DuckDuckGo are the current defaults behind independent circuit breakers;
/// Google/Qwant/Stract/Yep remain opt-in because recent probes returned bot
/// walls, empty results, or unavailable services. Mojeek requires a paid key. Persistent
/// failures trip the per-engine circuit breaker, so dead engines self-mute
/// — but production deployments should configure at least one API-backed
/// engine (brave/serper) rather than lean on scrapers.
fn build_engine(ec: &SearchEngineConfig) -> Option<Arc<dyn SearchEngine>> {
    match ec.name.to_lowercase().as_str() {
        "duckduckgo" | "ddg" => Some(Arc::new(duckduckgo::DuckDuckGo)),
        "brave-html" | "brave_html" => Some(Arc::new(brave_html::BraveHtml)),
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
            decision: None,
        }
    }

    #[test]
    fn test_urlencoded() {
        assert_eq!(urlencoded("hello world"), "hello+world");
        assert_eq!(urlencoded("rust+lang"), "rust%2Blang");
        assert_eq!(urlencoded("a b&c"), "a+b%26c");
    }

    #[test]
    fn test_url_decode() {
        assert_eq!(url_decode("hello%20world"), "hello world");
        assert_eq!(
            url_decode("https%3A%2F%2Fexample.com"),
            "https://example.com"
        );
    }

    #[test]
    fn tracking_params_are_removed_without_dropping_functional_params() {
        assert_eq!(
            strip_tracking_params(
                "https://nubank.com.br/nu/conta?msockid=abc&utm_source=bing&lang=pt-BR"
            ),
            "https://nubank.com.br/nu/conta?lang=pt-BR"
        );
        assert_eq!(
            strip_tracking_params("https://example.com/search?q=pix&page=2"),
            "https://example.com/search?q=pix&page=2"
        );
        assert_eq!(
            strip_tracking_params("https://example.com?utm_source=bing"),
            "https://example.com/"
        );
        assert_eq!(strip_tracking_params("not a url"), "not a url");
    }

    #[test]
    fn provider_surface_sanity_rejects_unrelated_but_valid_urls() {
        let poisoned = make_result("https://www.xnxx.com/video-example", "bing");
        assert!(!provider_surface_topic_relevant(
            "quem fundou a Anthropic?",
            &poisoned
        ));

        let mut relevant = make_result("https://example.com/anthropic-founders", "bing");
        relevant.title = "Who founded Anthropic?".to_string();
        assert!(provider_surface_topic_relevant(
            "quem fundou a Anthropic?",
            &relevant
        ));
    }

    #[test]
    fn provider_surface_sanity_keeps_navigation_and_tool_topics() {
        let mut claude = make_result("https://claude.ai/", "bing");
        claude.title = "Claude".to_string();
        assert!(provider_surface_topic_relevant("claude", &claude));

        let mut compiler = make_result("https://www.onlinegdb.com/online_python_compiler", "bing");
        compiler.title = "Online Python Compiler".to_string();
        assert!(provider_surface_topic_relevant(
            "python online compiler",
            &compiler
        ));

        let mut tutorial = make_result("https://docs.python.org/3/tutorial/", "bing");
        tutorial.title = "The Python Tutorial".to_string();
        assert!(!provider_surface_topic_relevant(
            "python online compiler",
            &tutorial
        ));
    }

    #[test]
    fn provider_surface_sanity_accepts_current_office_alias_headline() {
        let mut result = make_result("https://example.com/brazil-president", "bing");
        result.title = "Brazil's President Lula opens campaign".to_string();
        assert!(provider_surface_topic_relevant(
            "\"Luiz Inácio Lula da Silva\" \"atual presidente\" Brasil",
            &result
        ));

        let unrelated = make_result("https://example.com/football", "bing");
        assert!(!provider_surface_topic_relevant(
            "\"Luiz Inácio Lula da Silva\" \"atual presidente\" Brasil",
            &unrelated
        ));
    }

    #[test]
    fn provider_surface_sanity_preserves_natural_answer_surfaces() {
        let mut medical = make_result("https://health.example/melatonin", "serper");
        medical.title = "Melatonin and heart rate".to_string();
        assert!(provider_surface_topic_relevant(
            "effects of melatonin on pulse",
            &medical
        ));

        let mut anatomy = make_result("https://health.example/respiratory", "serper");
        anatomy.title = "Respiratory system anatomy and function".to_string();
        assert!(provider_surface_topic_relevant(
            "what body system does the respiratory system interact with",
            &anatomy
        ));

        let unrelated = make_result("https://unrelated.example/celebrity-gossip", "serper");
        assert!(!provider_surface_topic_relevant(
            "effects of melatonin on pulse",
            &unrelated
        ));

        let mut generic_benefits = make_result("https://public.example/general-assistance", "bing");
        generic_benefits.title = "Government benefits and assistance".to_string();
        assert!(!provider_surface_topic_relevant(
            "benefits of moringa and ginger",
            &generic_benefits
        ));

        let mut topical_benefits = make_result("https://health.example/moringa", "serper");
        topical_benefits.title = "Moringa and ginger: evidence and safety".to_string();
        assert!(provider_surface_topic_relevant(
            "benefits of moringa and ginger",
            &topical_benefits
        ));
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
        assert_eq!(
            html_decode("computa&#231;&#227;o qu&#226;ntica"),
            "computação quântica"
        );
        assert_eq!(html_decode("caf&#xE9; &unknown;"), "café &unknown;");
    }

    #[test]
    fn test_search_with_engines_dedup() {
        struct FakeEngine {
            name: &'static str,
            results: Vec<SearchResultItem>,
        }
        impl SearchEngine for FakeEngine {
            fn name(&self) -> &str {
                self.name
            }
            fn search(&self, _query: &str, _limit: usize) -> Vec<SearchResultItem> {
                self.results.clone()
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
                    make_result("https://example.com?utm_source=engine-b", "b"), // dupe
                    make_result("https://unique-b.com", "b"),
                ],
            }),
        ];

        let results = search_with_engines(&engines, "test", 10, &SearchParams::default());
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].url, "https://example.com/");
        assert_eq!(results[1].url, "https://unique-a.com/");
        assert_eq!(results[2].url, "https://unique-b.com/");
        // Ranks should be re-assigned
        assert_eq!(results[0].rank, 1);
        assert_eq!(results[1].rank, 2);
        assert_eq!(results[2].rank, 3);
    }

    #[test]
    fn default_keyless_waterfall_preserves_secondary_when_primary_is_deep() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct CountingEngine {
            name: &'static str,
            calls: Arc<AtomicUsize>,
            results: Vec<SearchResultItem>,
        }
        impl SearchEngine for CountingEngine {
            fn name(&self) -> &str {
                self.name
            }
            fn search(&self, _query: &str, _limit: usize) -> Vec<SearchResultItem> {
                self.calls.fetch_add(1, Ordering::SeqCst);
                self.results.clone()
            }
        }

        let bing_calls = Arc::new(AtomicUsize::new(0));
        let ddg_calls = Arc::new(AtomicUsize::new(0));
        let topical = |url: &str, source: &str| {
            let mut result = make_result(url, source);
            result.title = "Quantum computing overview".to_string();
            result
        };
        let engines: Vec<Arc<dyn SearchEngine>> = vec![
            Arc::new(CountingEngine {
                name: "bing",
                calls: Arc::clone(&bing_calls),
                results: vec![
                    topical("https://one.test/quantum", "bing"),
                    topical("https://two.test/quantum", "bing"),
                    topical("https://three.test/quantum", "bing"),
                ],
            }),
            Arc::new(CountingEngine {
                name: "duckduckgo",
                calls: Arc::clone(&ddg_calls),
                results: vec![topical("https://fallback.test/quantum", "duckduckgo")],
            }),
        ];

        let results =
            search_with_engines(&engines, "quantum computing", 5, &SearchParams::default());
        assert_eq!(results.len(), 3);
        assert_eq!(bing_calls.load(Ordering::SeqCst), 1);
        assert_eq!(ddg_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn default_keyless_waterfall_calls_secondary_for_poisoned_primary() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct CountingEngine {
            name: &'static str,
            calls: Arc<AtomicUsize>,
            results: Vec<SearchResultItem>,
        }
        impl SearchEngine for CountingEngine {
            fn name(&self) -> &str {
                self.name
            }
            fn search(&self, _query: &str, _limit: usize) -> Vec<SearchResultItem> {
                self.calls.fetch_add(1, Ordering::SeqCst);
                self.results.clone()
            }
        }

        let bing_calls = Arc::new(AtomicUsize::new(0));
        let ddg_calls = Arc::new(AtomicUsize::new(0));
        let mut poisoned = make_result("https://dictionary.test/best", "bing");
        poisoned.title = "Meaning of best".to_string();
        let topical = |url: &str| {
            let mut result = make_result(url, "duckduckgo");
            result.title = "Best notebook for programming 2026".to_string();
            result
        };
        let engines: Vec<Arc<dyn SearchEngine>> = vec![
            Arc::new(CountingEngine {
                name: "bing",
                calls: Arc::clone(&bing_calls),
                results: vec![poisoned],
            }),
            Arc::new(CountingEngine {
                name: "duckduckgo",
                calls: Arc::clone(&ddg_calls),
                results: vec![
                    topical("https://one.test/notebook"),
                    topical("https://two.test/notebook"),
                    topical("https://three.test/notebook"),
                ],
            }),
        ];

        let results = search_with_engines(
            &engines,
            "melhor notebook para programar em 2026",
            5,
            &SearchParams::default(),
        );
        assert_eq!(results.len(), 3);
        assert!(results.iter().all(|result| result.source == "duckduckgo"));
        assert_eq!(bing_calls.load(Ordering::SeqCst), 1);
        assert_eq!(ddg_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn default_keyless_waterfall_preserves_tertiary_when_secondary_is_deep() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct CountingEngine {
            name: &'static str,
            calls: Arc<AtomicUsize>,
            results: Vec<SearchResultItem>,
        }
        impl SearchEngine for CountingEngine {
            fn name(&self) -> &str {
                self.name
            }
            fn search(&self, _query: &str, _limit: usize) -> Vec<SearchResultItem> {
                self.calls.fetch_add(1, Ordering::SeqCst);
                self.results.clone()
            }
        }

        let bing_calls = Arc::new(AtomicUsize::new(0));
        let brave_calls = Arc::new(AtomicUsize::new(0));
        let ddg_calls = Arc::new(AtomicUsize::new(0));
        let mut poisoned = make_result("https://dictionary.test/best", "bing");
        poisoned.title = "Meaning of best".to_string();
        let topical = |url: &str, source: &str| {
            let mut result = make_result(url, source);
            result.title = "Best notebook for programming 2026".to_string();
            result
        };
        let engines: Vec<Arc<dyn SearchEngine>> = vec![
            Arc::new(CountingEngine {
                name: "bing",
                calls: Arc::clone(&bing_calls),
                results: vec![poisoned],
            }),
            Arc::new(CountingEngine {
                name: "brave-html",
                calls: Arc::clone(&brave_calls),
                results: vec![
                    topical("https://one.test/notebook", "brave-html"),
                    topical("https://two.test/notebook", "brave-html"),
                    topical("https://three.test/notebook", "brave-html"),
                ],
            }),
            Arc::new(CountingEngine {
                name: "duckduckgo",
                calls: Arc::clone(&ddg_calls),
                results: vec![topical("https://tertiary.test/notebook", "duckduckgo")],
            }),
        ];

        let results = search_with_engines(
            &engines,
            "melhor notebook para programar em 2026",
            5,
            &SearchParams::default(),
        );
        assert_eq!(results.len(), 3);
        assert!(results.iter().all(|result| result.source == "brave-html"));
        assert_eq!(bing_calls.load(Ordering::SeqCst), 1);
        assert_eq!(brave_calls.load(Ordering::SeqCst), 1);
        assert_eq!(ddg_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn test_engine_orchestration_interleaves_before_limit() {
        struct FakeEngine {
            name: &'static str,
            prefix: &'static str,
        }
        impl SearchEngine for FakeEngine {
            fn name(&self) -> &str {
                self.name
            }
            fn search(&self, _query: &str, limit: usize) -> Vec<SearchResultItem> {
                (0..limit)
                    .map(|rank| {
                        make_result(&format!("https://{}-{rank}.test", self.prefix), self.name)
                    })
                    .collect()
            }
        }

        let engines: Vec<Arc<dyn SearchEngine>> = vec![
            Arc::new(FakeEngine {
                name: "engine_a",
                prefix: "a",
            }),
            Arc::new(FakeEngine {
                name: "engine_b",
                prefix: "b",
            }),
        ];
        let results = search_with_engines(&engines, "test", 4, &SearchParams::default());
        assert_eq!(
            results
                .iter()
                .map(|result| result.source.as_str())
                .collect::<Vec<_>>(),
            vec!["engine_a", "engine_b", "engine_a", "engine_b"]
        );
    }

    #[test]
    fn test_engine_orchestration_honors_hard_deadline() {
        struct SlowEngine;
        impl SearchEngine for SlowEngine {
            fn name(&self) -> &str {
                "slow"
            }

            fn search(&self, _query: &str, _limit: usize) -> Vec<SearchResultItem> {
                std::thread::sleep(Duration::from_millis(200));
                Vec::new()
            }
        }

        let engines: Vec<Arc<dyn SearchEngine>> = vec![Arc::new(SlowEngine)];
        let started = std::time::Instant::now();
        let results = search_with_engines_timeout(
            &engines,
            "test",
            5,
            &SearchParams::default(),
            Duration::from_millis(20),
        );
        assert!(results.is_empty());
        assert!(started.elapsed() < Duration::from_millis(100));
    }

    #[test]
    fn test_default_engines_legacy_mode() {
        let config = SdkConfig::default();
        let engines = default_engines(&config);
        assert_eq!(engines.len(), 3);
        assert_eq!(engines[0].name(), "bing");
        assert_eq!(engines[1].name(), "brave-html");
        assert_eq!(engines[2].name(), "duckduckgo");
    }

    #[test]
    fn test_default_engines_with_brave_key() {
        // API-backed engines come first; free DDG scrape is the fallback.
        let config = SdkConfig {
            brave_api_key: Some("test_key".to_string()),
            ..SdkConfig::default()
        };
        let engines = default_engines(&config);
        assert_eq!(engines.len(), 4);
        assert_eq!(engines[0].name(), "brave");
        assert_eq!(engines[1].name(), "bing");
        assert_eq!(engines[2].name(), "brave-html");
        assert_eq!(engines[3].name(), "duckduckgo");
    }

    #[test]
    fn test_default_engines_with_serper_key() {
        let config = SdkConfig {
            serper_api_key: Some("test_key".to_string()),
            brave_api_key: Some("test_key".to_string()),
            ..SdkConfig::default()
        };
        let engines = default_engines(&config);
        assert_eq!(engines.len(), 5);
        assert_eq!(engines[0].name(), "serper");
        assert_eq!(engines[1].name(), "brave");
        assert_eq!(engines[2].name(), "bing");
        assert_eq!(engines[3].name(), "brave-html");
        assert_eq!(engines[4].name(), "duckduckgo");
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
