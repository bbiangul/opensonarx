use oi_crawl::extractor::html_to_markdown;
use oi_crawl::Distiller;
#[cfg(feature = "reranker")]
use oi_embed::OnnxReranker;
use oi_index::{HybridConfig, HybridIndex};
use oi_types::traits::{CrawlPipeline, SearchIndex};
#[cfg(feature = "reranker")]
use oi_types::Reranker;
use oi_types::{EmbeddingPipeline, QueryFilters};
use serde::Serialize;
use std::sync::Arc;
use std::time::Instant;

// ── Cost constants (USD per token / per call) ──────────────────────────────

const GEMINI_INPUT_COST_PER_TOKEN: f64 = 0.30 / 1_000_000.0; // $0.30 per 1M tokens
const GEMINI_OUTPUT_COST_PER_TOKEN: f64 = 2.50 / 1_000_000.0; // $2.50 per 1M tokens
const SERPER_COST_PER_SEARCH: f64 = 0.001; // ~$1 per 1K searches
const EXA_COST_PER_SEARCH: f64 = 0.005; // $5 per 1K searches (includes results)
const TAVILY_COST_PER_SEARCH: f64 = 0.008; // 1 credit = $0.008 per basic search

// ── Data structures ────────────────────────────────────────────────────────

struct AgentQuestion {
    id: String,
    language: String,
    question: String,
    expected_keywords: Vec<String>,
    source_urls: Vec<String>,
}

struct SearchContext {
    url: String,
    title: String,
    content: String,
}

#[derive(Default, Clone)]
struct LatencyBreakdown {
    search_ms: f64,
    fetch_ms: f64,
    llm_ms: f64,
    total_ms: f64,
}

#[derive(Default, Clone)]
#[allow(dead_code)]
struct TokenBreakdown {
    input: usize,
    output: usize,
    total: usize,
}

#[derive(Default, Clone)]
#[allow(dead_code)]
struct CostBreakdown {
    llm_input: f64,
    llm_output: f64,
    search_api: f64,
    judge: f64,
    total: f64,
}

#[allow(dead_code)]
struct QuestionResult {
    question_id: String,
    backend: String,
    question_text: String,
    expected_keywords: Vec<String>,
    answer: String,
    correct: bool,
    latency: LatencyBreakdown,
    tokens: TokenBreakdown,
    cost: CostBreakdown,
    context_results: usize,
    context_chars: usize,
    fetches: usize,
    error: Option<String>,
}

#[derive(Default)]
struct BackendAggregate {
    backend: String,
    questions_total: usize,
    questions_correct: usize,
    accuracy: f64,
    avg_latency: LatencyBreakdown,
    p50_latency_ms: f64,
    p95_latency_ms: f64,
    avg_tokens: TokenBreakdown,
    total_tokens: TokenBreakdown,
    total_fetches: usize,
    avg_fetches: f64,
    total_llm_cost: f64,
    total_search_cost: f64,
    total_judge_cost: f64,
    total_overall_cost: f64,
    cost_per_correct_answer: f64,
}

// ── Gemini API client ──────────────────────────────────────────────────────

fn call_gemini(
    api_key: &str,
    prompt: &str,
    model: &str,
) -> Result<(String, TokenBreakdown), String> {
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        model, api_key
    );

    let payload = serde_json::json!({
        "contents": [{"parts": [{"text": prompt}]}]
    });
    let json_bytes = payload.to_string();

    let agent = ureq::Agent::new_with_config(
        ureq::config::Config::builder()
            .timeout_global(Some(std::time::Duration::from_secs(30)))
            .build(),
    );

    let mut body = None;
    let mut last_err = String::new();
    for attempt in 0..=3 {
        if attempt > 0 {
            let delay = std::time::Duration::from_secs(5 * attempt as u64);
            tracing::info!(
                "Gemini 429, backing off {:?} (attempt {}/3)",
                delay,
                attempt
            );
            std::thread::sleep(delay);
        }
        match agent
            .post(&url)
            .header("Content-Type", "application/json")
            .send(json_bytes.as_bytes())
        {
            Ok(mut resp) => {
                body = Some(
                    resp.body_mut()
                        .read_to_vec()
                        .map_err(|e| format!("Gemini read error: {e}"))?,
                );
                break;
            }
            Err(e) => {
                let err_str = e.to_string();
                if err_str.contains("429") && attempt < 3 {
                    last_err = format!("Gemini retry {attempt} failed: {e}");
                    continue;
                }
                return Err(format!("Gemini request failed: {e}"));
            }
        }
    }
    let body = body.ok_or(last_err)?;

    let json: serde_json::Value =
        serde_json::from_slice(&body).map_err(|e| format!("Gemini JSON parse error: {e}"))?;

    let answer = json
        .pointer("/candidates/0/content/parts/0/text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let input_tokens = json
        .pointer("/usageMetadata/promptTokenCount")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    let output_tokens = json
        .pointer("/usageMetadata/candidatesTokenCount")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;

    Ok((
        answer,
        TokenBreakdown {
            input: input_tokens,
            output: output_tokens,
            total: input_tokens + output_tokens,
        },
    ))
}

// ── Serper search + URL fetch ──────────────────────────────────────────────

/// Raw Serper result before URL fetching.
struct SerperHit {
    url: String,
    title: String,
    snippet: String,
}

/// Search Serper API — returns raw hits (no URL fetching yet).
fn search_serper_raw(api_key: &str, query: &str, top_k: usize) -> Vec<SerperHit> {
    let payload = serde_json::json!({
        "q": query,
        "num": top_k.min(100)
    });
    let json_bytes = payload.to_string();

    let agent = ureq::Agent::new_with_config(
        ureq::config::Config::builder()
            .timeout_global(Some(std::time::Duration::from_secs(10)))
            .build(),
    );

    let body = match agent
        .post("https://google.serper.dev/search")
        .header("X-API-KEY", api_key)
        .header("Content-Type", "application/json")
        .send(json_bytes.as_bytes())
    {
        Ok(mut resp) => match resp.body_mut().read_to_vec() {
            Ok(buf) => buf,
            Err(e) => {
                tracing::warn!("Serper response read failed: {e}");
                return Vec::new();
            }
        },
        Err(e) => {
            tracing::warn!("Serper search failed: {e}");
            return Vec::new();
        }
    };

    let json: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let Some(organic) = json.get("organic").and_then(|r| r.as_array()) else {
        return Vec::new();
    };

    organic
        .iter()
        .take(top_k)
        .map(|r| SerperHit {
            url: r
                .get("link")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            title: r
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            snippet: r
                .get("snippet")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        })
        .collect()
}

/// Fetch full page content for each Serper hit, falling back to snippet.
/// Returns (contexts, num_fetches_attempted).
fn fetch_serper_contents(hits: Vec<SerperHit>) -> (Vec<SearchContext>, usize) {
    let num_fetches = hits.len();
    let contexts = hits
        .into_iter()
        .map(|hit| {
            let content = fetch_url_content(&hit.url).unwrap_or(hit.snippet);
            SearchContext {
                url: hit.url,
                title: hit.title,
                content,
            }
        })
        .collect();
    (contexts, num_fetches)
}

fn fetch_url_content(url: &str) -> Option<String> {
    let agent = ureq::Agent::new_with_config(
        ureq::config::Config::builder()
            .timeout_global(Some(std::time::Duration::from_secs(5)))
            .build(),
    );

    let html_bytes = agent.get(url).call().ok()?.body_mut().read_to_vec().ok()?;

    let html = String::from_utf8_lossy(&html_bytes);
    let extracted = html_to_markdown(&html);
    let md = extracted.markdown;

    // Truncate to ~4000 chars to keep context reasonable
    if md.len() > 4000 {
        Some(md[..4000].to_string())
    } else {
        Some(md)
    }
}

// ── Exa API client ─────────────────────────────────────────────────────────

fn search_exa(api_key: &str, query: &str, top_k: usize) -> Vec<SearchContext> {
    let payload = serde_json::json!({
        "query": query,
        "type": "neural",
        "numResults": top_k,
        "contents": { "text": true }
    });
    let json_bytes = payload.to_string();

    let agent = ureq::Agent::new_with_config(
        ureq::config::Config::builder()
            .timeout_global(Some(std::time::Duration::from_secs(15)))
            .build(),
    );

    let body = match agent
        .post("https://api.exa.ai/search")
        .header("Authorization", &format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .send(json_bytes.as_bytes())
    {
        Ok(mut resp) => match resp.body_mut().read_to_vec() {
            Ok(buf) => buf,
            Err(e) => {
                tracing::warn!("Exa response read failed: {e}");
                return Vec::new();
            }
        },
        Err(e) => {
            tracing::warn!("Exa search failed: {e}");
            return Vec::new();
        }
    };

    let json: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let Some(results) = json.get("results").and_then(|r| r.as_array()) else {
        return Vec::new();
    };

    results
        .iter()
        .take(top_k)
        .map(|r| SearchContext {
            url: r
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            title: r
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            content: r
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        })
        .collect()
}

// ── Tavily API client ──────────────────────────────────────────────────────

fn search_tavily(api_key: &str, query: &str, top_k: usize) -> Vec<SearchContext> {
    let payload = serde_json::json!({
        "query": query,
        "max_results": top_k,
        "include_raw_content": false,
        "search_depth": "basic"
    });
    let json_bytes = payload.to_string();

    let agent = ureq::Agent::new_with_config(
        ureq::config::Config::builder()
            .timeout_global(Some(std::time::Duration::from_secs(15)))
            .build(),
    );

    let body = match agent
        .post("https://api.tavily.com/search")
        .header("Authorization", &format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .send(json_bytes.as_bytes())
    {
        Ok(mut resp) => match resp.body_mut().read_to_vec() {
            Ok(buf) => buf,
            Err(e) => {
                tracing::warn!("Tavily response read failed: {e}");
                return Vec::new();
            }
        },
        Err(e) => {
            tracing::warn!("Tavily search failed: {e}");
            return Vec::new();
        }
    };

    let json: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let Some(results) = json.get("results").and_then(|r| r.as_array()) else {
        return Vec::new();
    };

    results
        .iter()
        .take(top_k)
        .map(|r| SearchContext {
            url: r
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            title: r
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            content: r
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        })
        .collect()
}

// ── OpenIndex search ───────────────────────────────────────────────────────

fn search_openindex(
    index: &HybridIndex,
    embedder: &dyn EmbeddingPipeline,
    query: &str,
    top_k: usize,
) -> Vec<SearchContext> {
    let embedding = match embedder.embed_query(query) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("Embed failed: {e}");
            return Vec::new();
        }
    };

    let filters = QueryFilters::default();
    match index.query(&embedding, query, &filters, top_k) {
        Ok(results) => results
            .into_iter()
            .map(|r| SearchContext {
                url: r.url,
                title: r.title,
                content: r.content_md,
            })
            .collect(),
        Err(e) => {
            tracing::warn!("OpenIndex query failed: {e}");
            Vec::new()
        }
    }
}

// ── Agent prompt builder ───────────────────────────────────────────────────

fn build_agent_prompt(question: &str, contexts: &[SearchContext]) -> String {
    let mut prompt = String::from(
        "You are a programming assistant. Answer the following question \
         using ONLY the provided search results. Be concise and accurate.\n\n\
         ## Search Results\n",
    );

    for (i, ctx) in contexts.iter().enumerate() {
        prompt.push_str(&format!(
            "\n[{}] {}\nURL: {}\n{}\n",
            i + 1,
            ctx.title,
            ctx.url,
            ctx.content,
        ));
    }

    prompt.push_str(&format!("\n## Question\n{question}\n"));
    prompt
}

// ── LLM Judge ──────────────────────────────────────────────────────────────

struct JudgeResult {
    correct: bool,
    judge_tokens: TokenBreakdown,
}

fn llm_judge(
    api_key: &str,
    model: &str,
    question: &str,
    answer: &str,
    expected_keywords: &[String],
) -> JudgeResult {
    if answer.is_empty() {
        return JudgeResult {
            correct: false,
            judge_tokens: TokenBreakdown::default(),
        };
    }

    let keywords_hint = expected_keywords.join(", ");
    let prompt = format!(
        r#"You are an evaluation judge. Determine if the ANSWER correctly addresses the QUESTION.

## QUESTION
{question}

## ANSWER
{answer}

## EXPECTED CONCEPTS
The answer should cover concepts related to: {keywords_hint}

## INSTRUCTIONS
- The answer does NOT need to use the exact expected keywords — it just needs to be factually correct and address the question.
- An answer that is correct but uses different terminology should be judged as CORRECT.
- An answer that is vague, wrong, or doesn't address the question should be judged as INCORRECT.
- Respond with ONLY "CORRECT" or "INCORRECT" — nothing else."#
    );

    match call_gemini(api_key, &prompt, model) {
        Ok((response, tokens)) => {
            let correct = response.trim().to_uppercase().contains("CORRECT")
                && !response.trim().to_uppercase().starts_with("INCORRECT");
            JudgeResult {
                correct,
                judge_tokens: tokens,
            }
        }
        Err(e) => {
            tracing::warn!("LLM judge error: {e} — falling back to keyword match");
            let lower = answer.to_lowercase();
            let correct = expected_keywords
                .iter()
                .any(|kw| lower.contains(&kw.to_lowercase()));
            JudgeResult {
                correct,
                judge_tokens: TokenBreakdown::default(),
            }
        }
    }
}

// ── Question dataset ───────────────────────────────────────────────────────

fn build_questions() -> Vec<AgentQuestion> {
    vec![
        // ── Rust questions (25) ──
        q(
            "r01",
            "rust",
            "How do you convert a Vec<u8> to a String in Rust?",
            &["from_utf8"],
            &[
                "https://doc.rust-lang.org/std/string/struct.String.html",
                "https://doc.rust-lang.org/std/str/fn.from_utf8.html",
                "https://doc.rust-lang.org/rust-by-example/std/str.html",
                "https://doc.rust-lang.org/std/string/struct.String.html#method.from_utf8",
            ],
        ),
        q(
            "r02",
            "rust",
            "What is the difference between &str and String in Rust?",
            &["owned", "borrowed", "heap"],
            &[
                "https://doc.rust-lang.org/book/ch04-01-what-is-ownership.html",
                "https://doc.rust-lang.org/book/ch04-03-slices.html",
                "https://doc.rust-lang.org/std/string/struct.String.html",
                "https://doc.rust-lang.org/rust-by-example/std/str.html",
                "https://doc.rust-lang.org/std/primitive.str.html",
            ],
        ),
        q(
            "r03",
            "rust",
            "How do you handle errors with the Result type in Rust?",
            &["Ok", "Err", "unwrap", "match"],
            &[
                "https://doc.rust-lang.org/book/ch09-02-recoverable-errors-with-result.html",
                "https://doc.rust-lang.org/std/result/enum.Result.html",
                "https://doc.rust-lang.org/rust-by-example/error/result.html",
                "https://doc.rust-lang.org/book/ch09-00-error-handling.html",
            ],
        ),
        q(
            "r04",
            "rust",
            "How do you create a new thread in Rust?",
            &["thread::spawn", "spawn"],
            &[
                "https://doc.rust-lang.org/book/ch16-01-threads.html",
                "https://doc.rust-lang.org/std/thread/fn.spawn.html",
                "https://doc.rust-lang.org/rust-by-example/std_misc/threads.html",
                "https://doc.rust-lang.org/std/thread/index.html",
            ],
        ),
        q(
            "r05",
            "rust",
            "What is the purpose of the Option type in Rust?",
            &["Some", "None", "null"],
            &[
                "https://doc.rust-lang.org/std/option/enum.Option.html",
                "https://doc.rust-lang.org/book/ch06-01-defining-an-enum.html",
                "https://doc.rust-lang.org/rust-by-example/std/option.html",
                "https://doc.rust-lang.org/book/ch06-02-match.html",
            ],
        ),
        q(
            "r06",
            "rust",
            "How do you implement a trait for a struct in Rust?",
            &["impl", "for"],
            &[
                "https://doc.rust-lang.org/book/ch10-02-traits.html",
                "https://doc.rust-lang.org/rust-by-example/trait.html",
                "https://doc.rust-lang.org/rust-by-example/trait/impl_trait.html",
                "https://doc.rust-lang.org/reference/items/implementations.html",
            ],
        ),
        q(
            "r07",
            "rust",
            "How do you read a file to a string in Rust?",
            &["read_to_string", "fs::read"],
            &[
                "https://doc.rust-lang.org/std/fs/fn.read_to_string.html",
                "https://doc.rust-lang.org/rust-by-example/std_misc/file/read_lines.html",
                "https://doc.rust-lang.org/rust-by-example/std_misc/file/open.html",
                "https://doc.rust-lang.org/std/fs/struct.File.html",
            ],
        ),
        q(
            "r08",
            "rust",
            "What is the borrow checker in Rust?",
            &["ownership", "reference", "lifetime", "borrow"],
            &[
                "https://doc.rust-lang.org/book/ch04-02-references-and-borrowing.html",
                "https://doc.rust-lang.org/book/ch04-01-what-is-ownership.html",
                "https://doc.rust-lang.org/rust-by-example/scope/borrow.html",
                "https://doc.rust-lang.org/book/ch04-03-slices.html",
                "https://doc.rust-lang.org/rust-by-example/scope/borrow/mut.html",
                "https://doc.rust-lang.org/reference/expressions.html",
            ],
        ),
        q(
            "r09",
            "rust",
            "How do you use pattern matching in Rust?",
            &["match", "arm", "pattern"],
            &[
                "https://doc.rust-lang.org/book/ch06-02-match.html",
                "https://doc.rust-lang.org/book/ch18-03-pattern-syntax.html",
                "https://doc.rust-lang.org/rust-by-example/flow_control/match.html",
                "https://doc.rust-lang.org/rust-by-example/flow_control/match/destructuring.html",
                "https://doc.rust-lang.org/reference/expressions/match-expr.html",
            ],
        ),
        q(
            "r10",
            "rust",
            "How do you create a HashMap in Rust?",
            &["HashMap::new", "insert", "collections"],
            &[
                "https://doc.rust-lang.org/std/collections/struct.HashMap.html",
                "https://doc.rust-lang.org/book/ch08-03-hash-maps.html",
                "https://doc.rust-lang.org/rust-by-example/std/hash.html",
                "https://doc.rust-lang.org/std/collections/index.html",
            ],
        ),
        q(
            "r11",
            "rust",
            "How do you write unit tests in Rust?",
            &["#[test]", "assert", "cfg(test)"],
            &[
                "https://doc.rust-lang.org/book/ch11-01-writing-tests.html",
                "https://doc.rust-lang.org/rust-by-example/testing/unit_testing.html",
                "https://doc.rust-lang.org/book/ch11-02-running-tests.html",
                "https://doc.rust-lang.org/rust-by-example/testing.html",
            ],
        ),
        q(
            "r12",
            "rust",
            "What are lifetimes in Rust and when do you need them?",
            &["'a", "lifetime", "reference"],
            &[
                "https://doc.rust-lang.org/book/ch10-03-lifetime-syntax.html",
                "https://doc.rust-lang.org/rust-by-example/scope/lifetime.html",
                "https://doc.rust-lang.org/rust-by-example/scope/lifetime/explicit.html",
                "https://doc.rust-lang.org/reference/lifetime-elision.html",
            ],
        ),
        q(
            "r13",
            "rust",
            "How do you use closures in Rust?",
            &["closure", "Fn", "FnMut", "FnOnce", "||"],
            &[
                "https://doc.rust-lang.org/book/ch13-01-closures.html",
                "https://doc.rust-lang.org/rust-by-example/fn/closures.html",
                "https://doc.rust-lang.org/rust-by-example/fn/closures/capture.html",
                "https://doc.rust-lang.org/std/ops/trait.Fn.html",
            ],
        ),
        q(
            "r14",
            "rust",
            "How do you iterate over a vector in Rust?",
            &["iter", "for", "into_iter"],
            &[
                "https://doc.rust-lang.org/std/vec/struct.Vec.html",
                "https://doc.rust-lang.org/book/ch13-02-iterators.html",
                "https://doc.rust-lang.org/rust-by-example/flow_control/for.html",
                "https://doc.rust-lang.org/std/iter/trait.Iterator.html",
            ],
        ),
        q(
            "r15",
            "rust",
            "What is the difference between clone and copy in Rust?",
            &["Clone", "Copy", "stack", "heap", "bitwise"],
            &[
                "https://doc.rust-lang.org/std/marker/trait.Copy.html",
                "https://doc.rust-lang.org/std/clone/trait.Clone.html",
                "https://doc.rust-lang.org/book/ch04-01-what-is-ownership.html",
                "https://doc.rust-lang.org/rust-by-example/trait/clone.html",
            ],
        ),
        q(
            "r16",
            "rust",
            "How do you parse a string to an integer in Rust?",
            &["parse", "from_str", "i32", "unwrap"],
            &[
                "https://doc.rust-lang.org/std/primitive.str.html#method.parse",
                "https://doc.rust-lang.org/rust-by-example/types/cast.html",
                "https://doc.rust-lang.org/std/str/trait.FromStr.html",
                "https://doc.rust-lang.org/rust-by-example/conversion/string.html",
                "https://doc.rust-lang.org/rust-by-example/conversion/from_into.html",
            ],
        ),
        q(
            "r17",
            "rust",
            "How do you use async/await in Rust?",
            &["async", "await", "Future", "tokio"],
            &[
                "https://doc.rust-lang.org/book/ch17-01-futures-and-syntax.html",
                "https://doc.rust-lang.org/std/future/trait.Future.html",
                "https://docs.rs/tokio/latest/tokio/",
                "https://rust-lang.github.io/async-book/01_getting_started/04_async_await_primer.html",
                "https://doc.rust-lang.org/book/ch17-02-concurrency-with-async.html",
            ],
        ),
        q(
            "r18",
            "rust",
            "How do you define and use enums in Rust?",
            &["enum", "variant", "match"],
            &[
                "https://doc.rust-lang.org/book/ch06-01-defining-an-enum.html",
                "https://doc.rust-lang.org/rust-by-example/custom_types/enum.html",
                "https://doc.rust-lang.org/book/ch06-02-match.html",
                "https://doc.rust-lang.org/reference/items/enumerations.html",
            ],
        ),
        q(
            "r19",
            "rust",
            "What is the derive macro in Rust?",
            &["derive", "Debug", "Clone", "Serialize"],
            &[
                "https://doc.rust-lang.org/reference/attributes/derive.html",
                "https://doc.rust-lang.org/rust-by-example/trait/derive.html",
                "https://doc.rust-lang.org/book/appendix-03-derivable-traits.html",
                "https://doc.rust-lang.org/rust-by-example/hello/print.html",
            ],
        ),
        q(
            "r20",
            "rust",
            "How do you use channels for message passing in Rust?",
            &["mpsc", "channel", "send", "recv"],
            &[
                "https://doc.rust-lang.org/book/ch16-02-message-passing.html",
                "https://doc.rust-lang.org/std/sync/mpsc/fn.channel.html",
                "https://doc.rust-lang.org/std/sync/mpsc/index.html",
                "https://doc.rust-lang.org/rust-by-example/std_misc/channels.html",
            ],
        ),
        q(
            "r21",
            "rust",
            "How do you use serde to serialize a struct to JSON in Rust?",
            &["serde_json", "Serialize", "to_string"],
            &[
                "https://docs.rs/serde_json/latest/serde_json/",
                "https://serde.rs/derive.html",
                "https://serde.rs/",
                "https://docs.rs/serde/latest/serde/",
                "https://serde.rs/data-model.html",
                "https://serde.rs/json.html",
                "https://docs.rs/serde_json/latest/serde_json/fn.to_string.html",
            ],
        ),
        q(
            "r22",
            "rust",
            "How do you handle multiple error types in Rust?",
            &["Box<dyn Error>", "From", "thiserror", "anyhow"],
            &[
                "https://doc.rust-lang.org/book/ch09-02-recoverable-errors-with-result.html",
                "https://doc.rust-lang.org/rust-by-example/error/multiple_error_types.html",
                "https://docs.rs/thiserror/latest/thiserror/",
                "https://docs.rs/anyhow/latest/anyhow/",
                "https://doc.rust-lang.org/rust-by-example/error/multiple_error_types/boxing_errors.html",
            ],
        ),
        q(
            "r23",
            "rust",
            "What is the difference between iter, into_iter, and iter_mut in Rust?",
            &["iter", "into_iter", "iter_mut", "reference", "owned"],
            &[
                "https://doc.rust-lang.org/std/iter/trait.IntoIterator.html",
                "https://doc.rust-lang.org/book/ch13-02-iterators.html",
                "https://doc.rust-lang.org/std/iter/trait.Iterator.html",
                "https://doc.rust-lang.org/std/vec/struct.Vec.html#method.iter",
            ],
        ),
        q(
            "r24",
            "rust",
            "How do you use smart pointers like Rc and Arc in Rust?",
            &["Rc", "Arc", "reference count", "shared ownership"],
            &[
                "https://doc.rust-lang.org/book/ch15-04-rc.html",
                "https://doc.rust-lang.org/std/sync/struct.Arc.html",
                "https://doc.rust-lang.org/std/rc/struct.Rc.html",
                "https://doc.rust-lang.org/book/ch15-00-smart-pointers.html",
                "https://doc.rust-lang.org/rust-by-example/std/arc.html",
            ],
        ),
        q(
            "r25",
            "rust",
            "How do you create a REST API with Axum in Rust?",
            &["axum", "Router", "handler", "get", "post"],
            &[
                "https://docs.rs/axum/latest/axum/",
                "https://docs.rs/axum/latest/axum/routing/struct.Router.html",
                "https://docs.rs/axum/latest/axum/extract/index.html",
                "https://docs.rs/axum/latest/axum/response/index.html",
            ],
        ),
        // ── TypeScript questions (25) ──
        q(
            "t01",
            "typescript",
            "How do you define a generic function in TypeScript?",
            &["<T>", "type parameter", "generic"],
            &[
                "https://www.typescriptlang.org/docs/handbook/2/generics.html",
                "https://www.typescriptlang.org/docs/handbook/2/functions.html",
            ],
        ),
        q(
            "t02",
            "typescript",
            "What is the difference between interface and type in TypeScript?",
            &["extends", "declaration merging", "intersection"],
            &[
                "https://www.typescriptlang.org/docs/handbook/2/everyday-types.html",
                "https://www.typescriptlang.org/docs/handbook/2/types-from-types.html",
                "https://www.typescriptlang.org/docs/handbook/2/objects.html",
                "https://www.typescriptlang.org/docs/handbook/declaration-merging.html",
                "https://www.typescriptlang.org/docs/handbook/interfaces.html",
            ],
        ),
        q(
            "t03",
            "typescript",
            "How do you use union types in TypeScript?",
            &["|", "union", "narrow"],
            &[
                "https://www.typescriptlang.org/docs/handbook/2/narrowing.html",
                "https://www.typescriptlang.org/docs/handbook/2/everyday-types.html",
                "https://www.typescriptlang.org/docs/handbook/unions-and-intersections.html",
            ],
        ),
        q(
            "t04",
            "typescript",
            "What are utility types like Partial, Required, and Pick in TypeScript?",
            &["Partial", "Required", "Pick"],
            &[
                "https://www.typescriptlang.org/docs/handbook/utility-types.html",
                "https://www.typescriptlang.org/docs/handbook/2/mapped-types.html",
                "https://www.typescriptlang.org/docs/handbook/2/keyof-types.html",
            ],
        ),
        q(
            "t05",
            "typescript",
            "How do you use enums in TypeScript?",
            &["enum", "const enum", "reverse mapping"],
            &[
                "https://www.typescriptlang.org/docs/handbook/enums.html",
                "https://www.typescriptlang.org/docs/handbook/2/everyday-types.html",
                "https://www.typescriptlang.org/docs/handbook/basic-types.html",
            ],
        ),
        q(
            "t06",
            "typescript",
            "How do you define an async function in TypeScript?",
            &["async", "await", "Promise"],
            &[
                "https://www.typescriptlang.org/docs/handbook/release-notes/typescript-1-7.html",
                "https://javascript.info/async-await",
                "https://www.typescriptlang.org/docs/handbook/2/everyday-types.html",
                "https://javascript.info/promise-basics",
                "https://www.typescriptlang.org/docs/handbook/2/functions.html",
            ],
        ),
        q(
            "t07",
            "typescript",
            "What is the unknown type in TypeScript and how does it differ from any?",
            &["unknown", "type guard", "narrowing", "any"],
            &[
                "https://www.typescriptlang.org/docs/handbook/2/functions.html",
                "https://www.typescriptlang.org/docs/handbook/release-notes/typescript-3-0.html",
                "https://www.typescriptlang.org/docs/handbook/2/narrowing.html",
                "https://www.typescriptlang.org/docs/handbook/type-compatibility.html",
                "https://www.typescriptlang.org/docs/handbook/basic-types.html",
                "https://www.typescriptlang.org/docs/handbook/2/types-from-types.html",
            ],
        ),
        q(
            "t08",
            "typescript",
            "How do you use type guards in TypeScript?",
            &["typeof", "instanceof", "is", "narrowing"],
            &[
                "https://www.typescriptlang.org/docs/handbook/2/narrowing.html",
                "https://www.typescriptlang.org/docs/handbook/advanced-types.html",
            ],
        ),
        q(
            "t09",
            "typescript",
            "How do you create a mapped type in TypeScript?",
            &["in keyof", "mapped", "Record"],
            &[
                "https://www.typescriptlang.org/docs/handbook/2/mapped-types.html",
                "https://www.typescriptlang.org/docs/handbook/utility-types.html",
                "https://www.typescriptlang.org/docs/handbook/2/generics.html",
                "https://www.typescriptlang.org/docs/handbook/2/keyof-types.html",
            ],
        ),
        q(
            "t10",
            "typescript",
            "What are conditional types in TypeScript?",
            &["extends", "infer", "conditional", "ternary"],
            &[
                "https://www.typescriptlang.org/docs/handbook/2/conditional-types.html",
                "https://www.typescriptlang.org/docs/handbook/2/types-from-types.html",
                "https://www.typescriptlang.org/docs/handbook/2/generics.html",
                "https://www.typescriptlang.org/docs/handbook/advanced-types.html",
            ],
        ),
        q(
            "t11",
            "typescript",
            "How do you use decorators in TypeScript?",
            &["decorator", "@", "experimentalDecorators"],
            &[
                "https://www.typescriptlang.org/docs/handbook/decorators.html",
                "https://www.typescriptlang.org/docs/handbook/release-notes/typescript-5-0.html",
            ],
        ),
        q(
            "t12",
            "typescript",
            "What is the difference between type assertion and type casting in TypeScript?",
            &["as", "angle bracket", "assertion"],
            &[
                "https://www.typescriptlang.org/docs/handbook/2/everyday-types.html",
                "https://www.typescriptlang.org/docs/handbook/advanced-types.html",
                "https://www.typescriptlang.org/docs/handbook/basic-types.html",
                "https://www.typescriptlang.org/docs/handbook/jsx.html",
            ],
        ),
        q(
            "t13",
            "typescript",
            "How do you define a tuple type in TypeScript?",
            &["tuple", "fixed length", "[string, number]"],
            &[
                "https://www.typescriptlang.org/docs/handbook/2/objects.html",
                "https://www.typescriptlang.org/docs/handbook/variable-declarations.html",
                "https://www.typescriptlang.org/docs/handbook/basic-types.html",
                "https://www.typescriptlang.org/docs/handbook/2/everyday-types.html",
                "https://www.typescriptlang.org/docs/handbook/release-notes/typescript-3-0.html",
            ],
        ),
        q(
            "t14",
            "typescript",
            "How do you use template literal types in TypeScript?",
            &["template literal", "backtick", "`${"],
            &[
                "https://www.typescriptlang.org/docs/handbook/2/template-literal-types.html",
                "https://www.typescriptlang.org/docs/handbook/release-notes/typescript-4-1.html",
            ],
        ),
        q(
            "t15",
            "typescript",
            "What is the never type in TypeScript?",
            &["never", "exhaustive", "unreachable"],
            &[
                "https://www.typescriptlang.org/docs/handbook/2/narrowing.html",
                "https://www.typescriptlang.org/docs/handbook/2/functions.html",
                "https://www.typescriptlang.org/docs/handbook/basic-types.html",
            ],
        ),
        q(
            "t16",
            "typescript",
            "How do you configure tsconfig.json strict mode?",
            &["strict", "strictNullChecks", "tsconfig"],
            &[
                "https://www.typescriptlang.org/tsconfig",
                "https://www.typescriptlang.org/docs/handbook/tsconfig-json.html",
                "https://www.typescriptlang.org/docs/handbook/compiler-options.html",
            ],
        ),
        q(
            "t17",
            "typescript",
            "How do you use discriminated unions in TypeScript?",
            &["discriminated", "tag", "kind", "switch"],
            &[
                "https://www.typescriptlang.org/docs/handbook/2/narrowing.html",
                "https://www.typescriptlang.org/docs/handbook/unions-and-intersections.html",
                "https://www.typescriptlang.org/docs/handbook/typescript-in-5-minutes-func.html",
            ],
        ),
        q(
            "t18",
            "typescript",
            "How do you type React component props in TypeScript?",
            &["React.FC", "Props", "interface", "children"],
            &[
                "https://www.typescriptlang.org/docs/handbook/jsx.html",
                "https://react.dev/learn/typescript",
                "https://react.dev/reference/react/Component",
                "https://react.dev/learn/passing-props-to-a-component",
            ],
        ),
        q(
            "t19",
            "typescript",
            "What is the keyof operator in TypeScript?",
            &["keyof", "index type", "property name"],
            &[
                "https://www.typescriptlang.org/docs/handbook/2/keyof-types.html",
                "https://www.typescriptlang.org/docs/handbook/2/indexed-access-types.html",
                "https://www.typescriptlang.org/docs/handbook/2/types-from-types.html",
                "https://www.typescriptlang.org/docs/handbook/2/mapped-types.html",
                "https://www.typescriptlang.org/docs/handbook/advanced-types.html",
            ],
        ),
        q(
            "t20",
            "typescript",
            "How do you use type narrowing with the in operator?",
            &["in", "narrowing", "property check"],
            &[
                "https://www.typescriptlang.org/docs/handbook/2/narrowing.html",
                "https://www.typescriptlang.org/docs/handbook/advanced-types.html",
                "https://www.typescriptlang.org/docs/handbook/2/everyday-types.html",
                "https://www.typescriptlang.org/docs/handbook/2/objects.html",
            ],
        ),
        q(
            "t21",
            "typescript",
            "How do you create a namespace in TypeScript?",
            &["namespace", "module", "export"],
            &[
                "https://www.typescriptlang.org/docs/handbook/namespaces.html",
                "https://www.typescriptlang.org/docs/handbook/namespaces-and-modules.html",
                "https://www.typescriptlang.org/docs/handbook/modules.html",
            ],
        ),
        q(
            "t22",
            "typescript",
            "How do you use the Omit utility type in TypeScript?",
            &["Omit", "Exclude", "key"],
            &[
                "https://www.typescriptlang.org/docs/handbook/utility-types.html",
                "https://www.typescriptlang.org/docs/handbook/release-notes/typescript-3-5.html",
                "https://www.typescriptlang.org/docs/handbook/2/mapped-types.html",
            ],
        ),
        q(
            "t23",
            "typescript",
            "How do you write a type-safe event emitter in TypeScript?",
            &["EventEmitter", "generic", "on", "emit"],
            &[
                "https://www.typescriptlang.org/docs/handbook/2/generics.html",
                "https://www.typescriptlang.org/docs/handbook/declaration-files/by-example.html",
                "https://www.typescriptlang.org/docs/handbook/2/functions.html",
                "https://nodejs.org/api/events.html",
            ],
        ),
        q(
            "t24",
            "typescript",
            "What are index signatures in TypeScript?",
            &["index signature", "[key: string]", "dynamic"],
            &[
                "https://www.typescriptlang.org/docs/handbook/2/objects.html",
                "https://www.typescriptlang.org/docs/handbook/interfaces.html",
                "https://www.typescriptlang.org/docs/handbook/2/types-from-types.html",
            ],
        ),
        q(
            "t25",
            "typescript",
            "How do you use satisfies operator in TypeScript?",
            &["satisfies", "type check", "inference"],
            &[
                "https://www.typescriptlang.org/docs/handbook/release-notes/typescript-4-9.html",
                "https://www.typescriptlang.org/docs/handbook/2/types-from-types.html",
                "https://www.typescriptlang.org/docs/handbook/2/everyday-types.html",
            ],
        ),
    ]
}

fn q(id: &str, lang: &str, question: &str, keywords: &[&str], urls: &[&str]) -> AgentQuestion {
    AgentQuestion {
        id: id.to_string(),
        language: lang.to_string(),
        question: question.to_string(),
        expected_keywords: keywords.iter().map(|s| s.to_string()).collect(),
        source_urls: urls.iter().map(|s| s.to_string()).collect(),
    }
}

// ── OpenIndex ingestion ────────────────────────────────────────────────────

async fn ingest_for_openindex(
    questions: &[AgentQuestion],
    embedder: Arc<dyn EmbeddingPipeline>,
) -> HybridIndex {
    let dim = embedder.dimension();
    let config = HybridConfig {
        max_chunks_per_url: 8,
        ..Default::default()
    };
    #[cfg(feature = "reranker")]
    let index = {
        let reranker = OnnxReranker::new().expect("Failed to load cross-encoder reranker model");
        tracing::info!("Cross-encoder reranker loaded: {}", reranker.name());
        HybridIndex::with_config(dim, config).with_reranker(Arc::new(reranker))
    };
    #[cfg(not(feature = "reranker"))]
    let index = HybridIndex::with_config(dim, config);
    let distiller = Distiller::new().with_embedder(embedder);

    // Collect unique URLs (strip fragment to avoid duplicate content)
    let mut unique_urls: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for q in questions {
        for url in &q.source_urls {
            // Strip #fragment — e.g. Vec.html#method.iter → Vec.html
            let base_url = url.split('#').next().unwrap_or(url).to_string();
            if seen.insert(base_url.clone()) {
                unique_urls.push(base_url);
            }
        }
    }

    tracing::info!(
        "Ingesting {} unique documentation URLs for OpenIndex...",
        unique_urls.len()
    );

    let mut total_chunks = 0usize;
    for url in &unique_urls {
        match distiller.crawl(url).await {
            Ok(output) => {
                let n = output.chunks.len();
                if let Err(e) = index.ingest(&output.chunks) {
                    tracing::warn!("Ingest failed for {url}: {e}");
                } else {
                    total_chunks += n;
                }
            }
            Err(e) => {
                tracing::warn!("Crawl failed for {url}: {e}");
            }
        }
    }

    tracing::info!(
        "OpenIndex ready: {} chunks from {} URLs",
        total_chunks,
        unique_urls.len()
    );
    index
}

// ── Aggregate results ──────────────────────────────────────────────────────

fn aggregate(backend: &str, results: &[QuestionResult]) -> BackendAggregate {
    let n = results.len();
    if n == 0 {
        return BackendAggregate {
            backend: backend.to_string(),
            ..Default::default()
        };
    }

    let correct = results.iter().filter(|r| r.correct).count();
    let nf = n as f64;

    let avg_search_ms: f64 = results.iter().map(|r| r.latency.search_ms).sum::<f64>() / nf;
    let avg_fetch_ms: f64 = results.iter().map(|r| r.latency.fetch_ms).sum::<f64>() / nf;
    let avg_llm_ms: f64 = results.iter().map(|r| r.latency.llm_ms).sum::<f64>() / nf;
    let avg_total_ms: f64 = results.iter().map(|r| r.latency.total_ms).sum::<f64>() / nf;

    let total_input: usize = results.iter().map(|r| r.tokens.input).sum();
    let total_output: usize = results.iter().map(|r| r.tokens.output).sum();

    let total_fetches: usize = results.iter().map(|r| r.fetches).sum();
    let avg_fetches = total_fetches as f64 / nf;

    let total_llm_cost: f64 = results
        .iter()
        .map(|r| r.cost.llm_input + r.cost.llm_output)
        .sum();
    let total_search_cost: f64 = results.iter().map(|r| r.cost.search_api).sum();
    let total_judge_cost: f64 = results.iter().map(|r| r.cost.judge).sum();
    let total_overall_cost = total_llm_cost + total_search_cost;

    let mut latencies: Vec<f64> = results.iter().map(|r| r.latency.total_ms).collect();
    latencies.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p50 = percentile(&latencies, 50.0);
    let p95 = percentile(&latencies, 95.0);

    BackendAggregate {
        backend: backend.to_string(),
        questions_total: n,
        questions_correct: correct,
        accuracy: correct as f64 / nf,
        avg_latency: LatencyBreakdown {
            search_ms: avg_search_ms,
            fetch_ms: avg_fetch_ms,
            llm_ms: avg_llm_ms,
            total_ms: avg_total_ms,
        },
        p50_latency_ms: p50,
        p95_latency_ms: p95,
        avg_tokens: TokenBreakdown {
            input: total_input / n,
            output: total_output / n,
            total: (total_input + total_output) / n,
        },
        total_tokens: TokenBreakdown {
            input: total_input,
            output: total_output,
            total: total_input + total_output,
        },
        total_fetches,
        avg_fetches,
        total_llm_cost,
        total_search_cost,
        total_judge_cost,
        total_overall_cost,
        cost_per_correct_answer: if correct > 0 {
            total_overall_cost / correct as f64
        } else {
            0.0
        },
    }
}

fn percentile(sorted: &[f64], pct: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((pct / 100.0) * (sorted.len() - 1) as f64) as usize;
    sorted[idx.min(sorted.len() - 1)]
}

// ── Output formatters ──────────────────────────────────────────────────────

fn print_table(aggregates: &[BackendAggregate], questions: &[AgentQuestion]) {
    let rust_count = questions.iter().filter(|q| q.language == "rust").count();
    let ts_count = questions
        .iter()
        .filter(|q| q.language == "typescript")
        .count();

    println!("\n=== Agent Cost Benchmark ===");
    println!(
        "Questions: {} ({} Rust, {} TypeScript) | Search results per query: top-k\n",
        questions.len(),
        rust_count,
        ts_count,
    );

    println!(
        "{:<12} {:>8}  {:>18}  {:>10}  {:>40}  {:>12}  {:>10}",
        "Backend",
        "Accuracy",
        "Avg Tokens(in/out)",
        "Pg Fetches",
        "Avg Latency(ms)",
        "Total Cost",
        "$/Correct",
    );
    println!(
        "{:<12} {:>8}  {:>18}  {:>10}  {:>10} {:>10} {:>10} {:>10}  {:>12}  {:>10}",
        "", "", "", "", "search", "fetch", "llm", "total", "", "",
    );
    println!("{}", "\u{2500}".repeat(132));

    for agg in aggregates {
        let fetches_str = if agg.total_fetches == 0 {
            "0".to_string()
        } else {
            format!("{} ({:.0}/q)", agg.total_fetches, agg.avg_fetches)
        };
        println!(
            "{:<12} {:>7.0}%  {:>8} / {:<8}  {:>10}  {:>10.0} {:>10.0} {:>10.0} {:>10.0}  {:>11}  {:>10}",
            agg.backend,
            agg.accuracy * 100.0,
            agg.avg_tokens.input,
            agg.avg_tokens.output,
            fetches_str,
            agg.avg_latency.search_ms,
            agg.avg_latency.fetch_ms,
            agg.avg_latency.llm_ms,
            agg.avg_latency.total_ms,
            format!("${:.4}", agg.total_overall_cost),
            format!("${:.4}", agg.cost_per_correct_answer),
        );
    }

    // Token efficiency comparison
    let oi = aggregates.iter().find(|a| a.backend == "openindex");
    if let Some(oi_agg) = oi {
        let oi_tokens = oi_agg.avg_tokens.input as f64;
        if oi_tokens > 0.0 {
            println!("\nToken Efficiency:");
            for agg in aggregates {
                if agg.backend != "openindex" {
                    let ratio = agg.avg_tokens.input as f64 / oi_tokens;
                    println!(
                        "  {} uses ~{:.1}x more input tokens than OpenIndex",
                        agg.backend, ratio,
                    );
                }
            }
        }
    }

    // Judge cost (benchmark overhead, not included in backend costs)
    let total_judge: f64 = aggregates.iter().map(|a| a.total_judge_cost).sum();
    if total_judge > 0.0 {
        println!(
            "\nLLM Judge Cost (benchmark overhead, not included in backend costs): ${:.4}",
            total_judge
        );
    }
    println!();
}

#[derive(Serialize)]
struct JsonOutput {
    questions_total: usize,
    backends: Vec<JsonBackend>,
}

#[derive(Serialize)]
struct JsonBackend {
    backend: String,
    accuracy: f64,
    questions_total: usize,
    questions_correct: usize,
    avg_input_tokens: usize,
    avg_output_tokens: usize,
    total_input_tokens: usize,
    total_output_tokens: usize,
    total_page_fetches: usize,
    avg_page_fetches: f64,
    avg_latency_ms: f64,
    p50_latency_ms: f64,
    p95_latency_ms: f64,
    total_llm_cost_usd: f64,
    total_search_cost_usd: f64,
    total_judge_cost_usd: f64,
    total_cost_usd: f64,
    cost_per_correct_answer_usd: f64,
}

fn print_json(aggregates: &[BackendAggregate], questions: &[AgentQuestion]) {
    let output = JsonOutput {
        questions_total: questions.len(),
        backends: aggregates
            .iter()
            .map(|a| JsonBackend {
                backend: a.backend.clone(),
                accuracy: a.accuracy,
                questions_total: a.questions_total,
                questions_correct: a.questions_correct,
                avg_input_tokens: a.avg_tokens.input,
                avg_output_tokens: a.avg_tokens.output,
                total_input_tokens: a.total_tokens.input,
                total_output_tokens: a.total_tokens.output,
                total_page_fetches: a.total_fetches,
                avg_page_fetches: a.avg_fetches,
                avg_latency_ms: a.avg_latency.total_ms,
                p50_latency_ms: a.p50_latency_ms,
                p95_latency_ms: a.p95_latency_ms,
                total_llm_cost_usd: a.total_llm_cost,
                total_search_cost_usd: a.total_search_cost,
                total_judge_cost_usd: a.total_judge_cost,
                total_cost_usd: a.total_overall_cost,
                cost_per_correct_answer_usd: a.cost_per_correct_answer,
            })
            .collect(),
    };
    println!("{}", serde_json::to_string_pretty(&output).unwrap());
}

fn print_question_details(results: &[QuestionResult]) {
    // Group by backend
    let mut backends: Vec<String> = Vec::new();
    for r in results {
        if !backends.contains(&r.backend) {
            backends.push(r.backend.clone());
        }
    }

    for backend in &backends {
        let backend_results: Vec<&QuestionResult> =
            results.iter().filter(|r| r.backend == *backend).collect();
        let incorrect: Vec<&&QuestionResult> =
            backend_results.iter().filter(|r| !r.correct).collect();

        println!("\n=== Per-Question Details: {} ===", backend);
        println!(
            "Total: {} | Correct: {} | Incorrect: {}\n",
            backend_results.len(),
            backend_results.len() - incorrect.len(),
            incorrect.len(),
        );

        if incorrect.is_empty() {
            println!("  All questions answered correctly!");
            continue;
        }

        println!("INCORRECT answers:");
        println!("{}", "\u{2500}".repeat(100));
        for r in &incorrect {
            let answer_preview: String = r.answer.chars().take(200).collect();
            let answer_display = if r.answer.len() > 200 {
                format!("{answer_preview}...")
            } else {
                answer_preview
            };
            println!("  [{}] {}", r.question_id, r.question_text);
            println!("  Expected concepts: {:?}", r.expected_keywords);
            println!("  Answer: {}", answer_display.replace('\n', "\n          "));
            if let Some(ref err) = r.error {
                println!("  Error: {err}");
            }
            println!("{}", "\u{2500}".repeat(100));
        }
    }
    println!();
}

// ── Main entry point ───────────────────────────────────────────────────────

pub async fn run_agent_cost(
    pipeline: &str,
    output: &str,
    backends_csv: &str,
    top_k: usize,
    model: &str,
) {
    let gemini_key = std::env::var("GEMINI_API_KEY").ok();
    let serper_key = std::env::var("SERPER_API_KEY").ok();
    let exa_key = std::env::var("EXA_API_KEY").ok();
    let tavily_key = std::env::var("TAVILY_API_KEY").ok();

    let backends: Vec<&str> = backends_csv.split(',').map(|s| s.trim()).collect();
    let questions = build_questions();

    // If no Gemini key, run retrieval-only evaluation
    if gemini_key.is_none() {
        if backends.contains(&"openindex") {
            let embedder = super::get_embedder(pipeline);
            let index = ingest_for_openindex(&questions, embedder.clone()).await;
            run_retrieval_eval(&questions, &index, embedder.as_ref(), top_k);
        } else {
            eprintln!(
                "Warning: GEMINI_API_KEY not set — cannot call LLM. Set it to run the full benchmark."
            );
            eprintln!(
                "Tip: Run with -b openindex to use retrieval-only evaluation (no API key needed)."
            );
        }
        return;
    }
    let gemini_key = gemini_key.unwrap();

    // Build OpenIndex if needed
    let embedder = super::get_embedder(pipeline);
    let oi_index = if backends.contains(&"openindex") {
        Some(ingest_for_openindex(&questions, embedder.clone()).await)
    } else {
        None
    };

    let mut all_aggregates = Vec::new();
    let mut all_results: Vec<QuestionResult> = Vec::new();

    for &backend in &backends {
        match backend {
            "serper" => {
                if serper_key.is_none() {
                    tracing::warn!("SERPER_API_KEY not set — skipping serper backend");
                    continue;
                }
                let key = serper_key.as_ref().unwrap();
                let results = run_backend_questions(
                    &questions,
                    backend,
                    &gemini_key,
                    model,
                    top_k,
                    |q, k| {
                        let search_start = Instant::now();
                        let hits = search_serper_raw(key, &q.question, k);
                        let search_ms = search_start.elapsed().as_secs_f64() * 1000.0;

                        let fetch_start = Instant::now();
                        let (contexts, num_fetches) = fetch_serper_contents(hits);
                        let fetch_ms = fetch_start.elapsed().as_secs_f64() * 1000.0;

                        (
                            contexts,
                            search_ms,
                            fetch_ms,
                            SERPER_COST_PER_SEARCH,
                            num_fetches,
                        )
                    },
                );
                all_aggregates.push(aggregate(backend, &results));
                all_results.extend(results);
            }
            "exa" => {
                if exa_key.is_none() {
                    tracing::warn!("EXA_API_KEY not set — skipping exa backend");
                    continue;
                }
                let key = exa_key.as_ref().unwrap();
                let results = run_backend_questions(
                    &questions,
                    backend,
                    &gemini_key,
                    model,
                    top_k,
                    |q, k| {
                        let start = Instant::now();
                        let contexts = search_exa(key, &q.question, k);
                        let search_ms = start.elapsed().as_secs_f64() * 1000.0;
                        let cost = EXA_COST_PER_SEARCH;
                        (contexts, search_ms, 0.0, cost, 0)
                    },
                );
                all_aggregates.push(aggregate(backend, &results));
                all_results.extend(results);
            }
            "tavily" => {
                if tavily_key.is_none() {
                    tracing::warn!("TAVILY_API_KEY not set — skipping tavily backend");
                    continue;
                }
                let key = tavily_key.as_ref().unwrap();
                let results = run_backend_questions(
                    &questions,
                    backend,
                    &gemini_key,
                    model,
                    top_k,
                    |q, k| {
                        let start = Instant::now();
                        let contexts = search_tavily(key, &q.question, k);
                        let search_ms = start.elapsed().as_secs_f64() * 1000.0;
                        let cost = TAVILY_COST_PER_SEARCH;
                        (contexts, search_ms, 0.0, cost, 0)
                    },
                );
                all_aggregates.push(aggregate(backend, &results));
                all_results.extend(results);
            }
            "openindex" => {
                let index = oi_index.as_ref().expect("OpenIndex should be initialized");
                let emb = embedder.clone();
                let results = run_backend_questions(
                    &questions,
                    backend,
                    &gemini_key,
                    model,
                    top_k,
                    |q, k| {
                        let start = Instant::now();
                        let contexts = search_openindex(index, emb.as_ref(), &q.question, k);
                        let search_ms = start.elapsed().as_secs_f64() * 1000.0;
                        (contexts, search_ms, 0.0, 0.0, 0) // OpenIndex is free (local)
                    },
                );
                all_aggregates.push(aggregate(backend, &results));
                all_results.extend(results);
            }
            other => {
                tracing::warn!("Unknown backend: {other} — skipping");
            }
        }
    }

    match output {
        "json" => print_json(&all_aggregates, &questions),
        _ => {
            print_table(&all_aggregates, &questions);
            print_question_details(&all_results);
        }
    }
}

fn run_backend_questions<F>(
    questions: &[AgentQuestion],
    backend: &str,
    gemini_key: &str,
    model: &str,
    top_k: usize,
    search_fn: F,
) -> Vec<QuestionResult>
where
    // Returns: (contexts, search_ms, fetch_ms, search_api_cost, num_fetches)
    F: Fn(&AgentQuestion, usize) -> (Vec<SearchContext>, f64, f64, f64, usize),
{
    let mut results = Vec::new();

    for (i, question) in questions.iter().enumerate() {
        // Rate-limit Gemini API calls (free tier: ~15 RPM, 2 calls/question)
        if i > 0 {
            std::thread::sleep(std::time::Duration::from_secs(10));
        }
        tracing::info!(
            "[{backend}] ({}/{}) {}",
            i + 1,
            questions.len(),
            &question.question[..question.question.len().min(60)]
        );

        let total_start = Instant::now();

        // Search phase
        let (contexts, search_ms, fetch_ms, search_api_cost, fetches) = search_fn(question, top_k);
        let context_results = contexts.len();
        let context_chars: usize = contexts.iter().map(|c| c.content.len()).sum();

        // Build prompt and call Gemini
        let prompt = build_agent_prompt(&question.question, &contexts);
        let llm_start = Instant::now();
        let (answer, tokens, error) = match call_gemini(gemini_key, &prompt, model) {
            Ok((ans, tok)) => (ans, tok, None),
            Err(e) => {
                tracing::warn!("[{backend}] Gemini error on {}: {e}", question.id);
                (String::new(), TokenBreakdown::default(), Some(e))
            }
        };
        let llm_ms = llm_start.elapsed().as_secs_f64() * 1000.0;
        let total_ms = total_start.elapsed().as_secs_f64() * 1000.0;

        // LLM judge evaluation
        let judge = llm_judge(
            gemini_key,
            model,
            &question.question,
            &answer,
            &question.expected_keywords,
        );

        let llm_input_cost = tokens.input as f64 * GEMINI_INPUT_COST_PER_TOKEN;
        let llm_output_cost = tokens.output as f64 * GEMINI_OUTPUT_COST_PER_TOKEN;
        let judge_cost = judge.judge_tokens.input as f64 * GEMINI_INPUT_COST_PER_TOKEN
            + judge.judge_tokens.output as f64 * GEMINI_OUTPUT_COST_PER_TOKEN;

        // Per-question detail log
        let verdict = if judge.correct {
            "CORRECT"
        } else {
            "INCORRECT"
        };
        let answer_preview: String = answer.chars().take(120).collect();
        println!("  [{verdict}] {}: {}", question.id, question.question,);
        if !judge.correct {
            println!("     Answer: {answer_preview}...");
            println!("     Expected: {:?}", question.expected_keywords);
        }

        results.push(QuestionResult {
            question_id: question.id.clone(),
            backend: backend.to_string(),
            question_text: question.question.clone(),
            expected_keywords: question.expected_keywords.clone(),
            answer,
            correct: judge.correct,
            latency: LatencyBreakdown {
                search_ms,
                fetch_ms,
                llm_ms,
                total_ms,
            },
            tokens,
            cost: CostBreakdown {
                llm_input: llm_input_cost,
                llm_output: llm_output_cost,
                search_api: search_api_cost,
                judge: judge_cost,
                total: llm_input_cost + llm_output_cost + search_api_cost,
            },
            context_results,
            context_chars,
            fetches,
            error,
        });
    }

    results
}

// ── Retrieval-only evaluation (no LLM needed) ────────────────────────────

fn run_retrieval_eval(
    questions: &[AgentQuestion],
    index: &HybridIndex,
    embedder: &dyn EmbeddingPipeline,
    top_k: usize,
) {
    println!("\n=== Retrieval-Only Evaluation (keyword matching in search results) ===\n");

    let mut rust_correct = 0usize;
    let mut rust_total = 0usize;
    let mut ts_correct = 0usize;
    let mut ts_total = 0usize;

    for question in questions {
        let contexts = search_openindex(index, embedder, &question.question, top_k);

        // Concatenate all retrieved content
        let all_content: String = contexts
            .iter()
            .map(|c| format!("{} {}", c.title, c.content))
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();

        // Check if at least half of expected keywords appear in retrieved content
        let keyword_hits: usize = question
            .expected_keywords
            .iter()
            .filter(|kw| all_content.contains(&kw.to_lowercase()))
            .count();
        let required = (question.expected_keywords.len() + 1) / 2; // ceil(len/2)
        let correct = keyword_hits >= required;

        let verdict = if correct { "HIT " } else { "MISS" };
        let found_kws: Vec<&str> = question
            .expected_keywords
            .iter()
            .filter(|kw| all_content.contains(&kw.to_lowercase()))
            .map(|s| s.as_str())
            .collect();
        let missing_kws: Vec<&str> = question
            .expected_keywords
            .iter()
            .filter(|kw| !all_content.contains(&kw.to_lowercase()))
            .map(|s| s.as_str())
            .collect();

        println!("  [{verdict}] {}: {}", question.id, question.question);
        if !correct {
            println!("        Found: {:?}", found_kws);
            println!("        Missing: {:?}", missing_kws);
            for (i, ctx) in contexts.iter().enumerate() {
                let preview: String = ctx.content.chars().take(120).collect();
                println!(
                    "        [{}] {} | {}",
                    i + 1,
                    ctx.url,
                    preview.replace('\n', " ")
                );
            }
        }

        if question.language == "rust" {
            rust_total += 1;
            if correct {
                rust_correct += 1;
            }
        } else {
            ts_total += 1;
            if correct {
                ts_correct += 1;
            }
        }
    }

    let total_correct = rust_correct + ts_correct;
    let total = rust_total + ts_total;
    println!("\n────────────────────────────────────────────────────");
    println!("Retrieval accuracy (keyword match in top-{top_k} results):");
    println!(
        "  Rust: {}/{} ({:.0}%)",
        rust_correct,
        rust_total,
        rust_correct as f64 / rust_total as f64 * 100.0
    );
    println!(
        "  TS:   {}/{} ({:.0}%)",
        ts_correct,
        ts_total,
        ts_correct as f64 / ts_total as f64 * 100.0
    );
    println!(
        "  Total: {}/{} ({:.0}%)",
        total_correct,
        total,
        total_correct as f64 / total as f64 * 100.0
    );
    println!("────────────────────────────────────────────────────\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test that all source URLs from failing questions are actually crawlable.
    /// This verifies the extractor produces content from these pages.
    #[tokio::test]
    async fn test_crawl_coverage_for_failing_questions() {
        // URLs from questions that previously returned "no information" answers.
        // Grouped by question ID.
        let test_urls = vec![
            // r08 - borrow checker
            (
                "r08",
                "https://doc.rust-lang.org/book/ch04-02-references-and-borrowing.html",
            ),
            (
                "r08",
                "https://doc.rust-lang.org/rust-by-example/scope/borrow.html",
            ),
            // r16 - parse string to int
            (
                "r16",
                "https://doc.rust-lang.org/rust-by-example/conversion/from_into.html",
            ),
            (
                "r16",
                "https://doc.rust-lang.org/std/str/trait.FromStr.html",
            ),
            // r21 - serde JSON
            ("r21", "https://serde.rs/json.html"),
            ("r21", "https://serde.rs/derive.html"),
            // t04 - utility types
            (
                "t04",
                "https://www.typescriptlang.org/docs/handbook/utility-types.html",
            ),
            // t05 - enums
            (
                "t05",
                "https://www.typescriptlang.org/docs/handbook/enums.html",
            ),
            // t06 - async
            ("t06", "https://javascript.info/async-await"),
            // t07 - unknown type
            (
                "t07",
                "https://www.typescriptlang.org/docs/handbook/release-notes/typescript-3-0.html",
            ),
            (
                "t07",
                "https://www.typescriptlang.org/docs/handbook/basic-types.html",
            ),
            // t09 - mapped types
            (
                "t09",
                "https://www.typescriptlang.org/docs/handbook/2/mapped-types.html",
            ),
            // t10 - conditional types
            (
                "t10",
                "https://www.typescriptlang.org/docs/handbook/2/conditional-types.html",
            ),
            // t12 - type assertion
            (
                "t12",
                "https://www.typescriptlang.org/docs/handbook/basic-types.html",
            ),
            // t13 - tuples
            (
                "t13",
                "https://www.typescriptlang.org/docs/handbook/basic-types.html",
            ),
            // t19 - keyof
            (
                "t19",
                "https://www.typescriptlang.org/docs/handbook/2/keyof-types.html",
            ),
            // t20 - in operator narrowing
            (
                "t20",
                "https://www.typescriptlang.org/docs/handbook/2/narrowing.html",
            ),
            (
                "t20",
                "https://www.typescriptlang.org/docs/handbook/2/objects.html",
            ),
            // t22 - Omit
            (
                "t22",
                "https://www.typescriptlang.org/docs/handbook/utility-types.html",
            ),
            // t25 - satisfies
            (
                "t25",
                "https://www.typescriptlang.org/docs/handbook/release-notes/typescript-4-9.html",
            ),
        ];

        let distiller = Distiller::new();
        let mut failures = Vec::new();

        for (qid, url) in &test_urls {
            match distiller.crawl(url).await {
                Ok(output) => {
                    let total_chars: usize = output.chunks.iter().map(|c| c.content.len()).sum();
                    println!(
                        "[OK]  {qid} {url} -> {} chunks, {total_chars} chars",
                        output.chunks.len(),
                    );
                    if output.chunks.is_empty() {
                        failures.push(format!("{qid}: {url} -> 0 chunks"));
                    }
                }
                Err(e) => {
                    println!("[FAIL] {qid} {url} -> {e}");
                    failures.push(format!("{qid}: {url} -> {e}"));
                }
            }
        }

        if !failures.is_empty() {
            println!("\n=== CRAWL FAILURES ===");
            for f in &failures {
                println!("  {f}");
            }
        }
        // Don't assert — we want to see all results even if some fail
        println!(
            "\nTotal: {} URLs tested, {} failures",
            test_urls.len(),
            failures.len()
        );
    }
}
