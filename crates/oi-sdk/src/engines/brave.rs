use oi_types::{SearchEngine, SearchParams, SearchResultItem};

use super::urlencoded;

/// Brave Search JSON API adapter — requires an API key.
pub struct BraveSearch {
    api_key: String,
}

impl BraveSearch {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }
}

impl SearchEngine for BraveSearch {
    fn name(&self) -> &str {
        "brave"
    }

    fn requires_api_key(&self) -> bool {
        true
    }

    fn search(&self, query: &str, limit: usize) -> Vec<SearchResultItem> {
        self.search_with_params(query, limit, &SearchParams::default())
    }

    fn search_with_params(
        &self,
        query: &str,
        limit: usize,
        params: &SearchParams,
    ) -> Vec<SearchResultItem> {
        let q = if let Some(ref site) = params.site {
            format!("site:{site} {query}")
        } else {
            query.to_string()
        };

        let mut url = format!(
            "https://api.search.brave.com/res/v1/web/search?q={}&count={limit}",
            urlencoded(&q)
        );
        if let Some(ref gl) = params.gl {
            url.push_str(&format!("&country={}", urlencoded(gl)));
        }

        let agent = super::http_agent();
        let body = match agent
            .get(&url)
            .header("Accept", "application/json")
            .header("X-Subscription-Token", &self.api_key)
            .call()
        {
            Ok(mut resp) => match resp.body_mut().read_to_vec() {
                Ok(buf) => buf,
                Err(_) => return Vec::new(),
            },
            Err(e) => {
                tracing::warn!("Brave search failed: {e}");
                return Vec::new();
            }
        };

        parse_brave_json(&body, limit)
    }
}

/// Parse Brave Search API JSON response.
fn parse_brave_json(data: &[u8], limit: usize) -> Vec<SearchResultItem> {
    let json: serde_json::Value = match serde_json::from_slice(data) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let Some(web_results) = json
        .get("web")
        .and_then(|w| w.get("results"))
        .and_then(|r| r.as_array())
    else {
        return Vec::new();
    };

    web_results
        .iter()
        .take(limit)
        .enumerate()
        .map(|(i, r)| {
            let rank = (i + 1) as u32;
            SearchResultItem {
                result_id: format!("brave_{rank}"),
                rank,
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
                snippet: r
                    .get("description")
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
                source: "brave".to_string(),
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
    fn test_parse_brave_json() {
        let json = serde_json::json!({
            "web": {
                "results": [
                    {
                        "url": "https://example.com",
                        "title": "Example",
                        "description": "An example site"
                    },
                    {
                        "url": "https://test.com",
                        "title": "Test",
                        "description": "A test site"
                    }
                ]
            }
        });
        let data = serde_json::to_vec(&json).unwrap();
        let results = parse_brave_json(&data, 10);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].url, "https://example.com");
        assert_eq!(results[0].source, "brave");
        assert_eq!(results[1].title, "Test");
    }
}
