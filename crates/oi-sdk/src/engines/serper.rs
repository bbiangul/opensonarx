use oi_types::{SearchEngine, SearchParams, SearchResultItem};
use std::sync::atomic::{AtomicBool, Ordering};

/// Serper.dev Google Search API adapter — requires an API key.
pub struct Serper {
    api_key: String,
    quota_exhausted: AtomicBool,
}

impl Serper {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            quota_exhausted: AtomicBool::new(false),
        }
    }
}

impl SearchEngine for Serper {
    fn name(&self) -> &str {
        "serper"
    }

    fn requires_api_key(&self) -> bool {
        true
    }

    fn search(&self, query: &str, limit: usize) -> Vec<SearchResultItem> {
        self.search_with_params(query, limit, &SearchParams::default())
    }

    fn is_available(&self) -> bool {
        !self.quota_exhausted.load(Ordering::Relaxed)
    }

    fn search_checked(&self, query: &str, limit: usize) -> Result<Vec<SearchResultItem>, ()> {
        self.search_with_params_checked(query, limit, &SearchParams::default())
    }

    fn search_with_params(
        &self,
        query: &str,
        limit: usize,
        params: &SearchParams,
    ) -> Vec<SearchResultItem> {
        self.search_with_params_checked(query, limit, params)
            .unwrap_or_default()
    }

    fn search_with_params_checked(
        &self,
        query: &str,
        limit: usize,
        params: &SearchParams,
    ) -> Result<Vec<SearchResultItem>, ()> {
        if !self.is_available() {
            return Err(());
        }
        let q = if let Some(ref site) = params.site {
            format!("site:{site} {query}")
        } else {
            query.to_string()
        };

        let mut payload = serde_json::json!({
            "q": q,
            "num": limit.min(100)
        });
        if let Some(ref gl) = params.gl {
            payload["gl"] = serde_json::Value::String(gl.clone());
            payload["hl"] = serde_json::Value::String(gl.clone());
        }
        if let Some(ref locale) = params.locale {
            payload["hl"] = serde_json::Value::String(locale.clone());
        }
        let json_bytes = payload.to_string();

        let agent = super::http_agent_status_policy(false);
        let body = match agent
            .post("https://google.serper.dev/search")
            .header("X-API-KEY", &self.api_key)
            .header("Content-Type", "application/json")
            .send(json_bytes.as_bytes())
        {
            Ok(mut resp) => {
                let status = resp.status().as_u16();
                match resp.body_mut().read_to_vec() {
                    Ok(buf) => {
                        if status >= 400 {
                            let reason = provider_failure_reason(&buf);
                            if reason == "quota_exhausted" {
                                self.quota_exhausted.store(true, Ordering::Relaxed);
                            }
                            // Never log arbitrary provider bodies or credentials.
                            tracing::warn!(status, reason, "Serper API request rejected");
                            return Err(());
                        }
                        buf
                    }
                    Err(e) => {
                        tracing::warn!("Serper response read failed: {e}");
                        return Err(());
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Serper search failed: {e}");
                return Err(());
            }
        };

        if serde_json::from_slice::<serde_json::Value>(&body).is_err() {
            return Err(());
        }
        Ok(parse_serper_json(&body, limit))
    }
}

fn provider_failure_reason(body: &[u8]) -> &'static str {
    let value = serde_json::from_slice::<serde_json::Value>(body).unwrap_or_default();
    match value.get("message").and_then(|v| v.as_str()) {
        Some("Not enough credits") => "quota_exhausted",
        _ => "provider_rejected_request",
    }
}

/// Parse Serper API JSON response.
fn parse_serper_json(data: &[u8], limit: usize) -> Vec<SearchResultItem> {
    let json: serde_json::Value = match serde_json::from_slice(data) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let Some(organic) = json.get("organic").and_then(|r| r.as_array()) else {
        return Vec::new();
    };

    organic
        .iter()
        .take(limit)
        .enumerate()
        .map(|(i, r)| {
            let rank = (i + 1) as u32;
            SearchResultItem {
                result_id: format!("serper_{rank}"),
                rank,
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
                source: "serper".to_string(),
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
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quota_errors_are_not_successful_empty_searches() {
        assert_eq!(
            provider_failure_reason(br#"{"message":"Not enough credits","statusCode":400}"#),
            "quota_exhausted"
        );
        assert_eq!(
            provider_failure_reason(b"sensitive unknown body"),
            "provider_rejected_request"
        );
        let provider = Serper::new("unit-test-placeholder".into());
        provider.quota_exhausted.store(true, Ordering::Relaxed);
        assert!(!provider.is_available());
        assert!(
            provider
                .search_with_params_checked("example", 5, &SearchParams::default())
                .is_err()
        );
    }

    #[test]
    fn test_parse_serper_json() {
        let json = serde_json::json!({
            "organic": [
                {
                    "title": "Rust Programming Language",
                    "link": "https://rust-lang.org",
                    "snippet": "A language empowering everyone to build reliable software."
                },
                {
                    "title": "Rust Wikipedia",
                    "link": "https://en.wikipedia.org/wiki/Rust_(programming_language)",
                    "snippet": "Rust is a general-purpose programming language."
                }
            ]
        });
        let data = serde_json::to_vec(&json).unwrap();
        let results = parse_serper_json(&data, 10);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].url, "https://rust-lang.org");
        assert_eq!(results[0].source, "serper");
        assert_eq!(results[0].title, "Rust Programming Language");
        assert_eq!(results[1].rank, 2);
    }

    #[test]
    fn test_parse_serper_empty() {
        let json = serde_json::json!({ "searchParameters": { "q": "test" } });
        let data = serde_json::to_vec(&json).unwrap();
        let results = parse_serper_json(&data, 10);
        assert!(results.is_empty());
    }
}
