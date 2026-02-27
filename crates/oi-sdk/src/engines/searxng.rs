use oi_types::{SearchEngine, SearchResultItem};

use super::urlencoded;

/// SearXNG adapter — queries a self-hosted SearXNG instance via JSON API.
pub struct SearXNG {
    instance_url: String,
}

impl SearXNG {
    pub fn new(instance_url: String) -> Self {
        Self {
            instance_url: instance_url.trim_end_matches('/').to_string(),
        }
    }
}

impl SearchEngine for SearXNG {
    fn name(&self) -> &str {
        "searxng"
    }

    fn search(&self, query: &str, limit: usize) -> Vec<SearchResultItem> {
        let url = format!(
            "{}/search?format=json&q={}&pageno=1",
            self.instance_url,
            urlencoded(query)
        );

        let agent = super::http_agent();
        let body = match agent.get(&url)
            .header("User-Agent", "Mozilla/5.0 (compatible; OpenSonarX/0.1)")
            .call()
        {
            Ok(mut resp) => match resp.body_mut().read_to_vec() {
                Ok(buf) => buf,
                Err(_) => return Vec::new(),
            },
            Err(e) => {
                tracing::warn!("SearXNG search failed: {e}");
                return Vec::new();
            }
        };

        parse_searxng_json(&body, limit)
    }
}

fn parse_searxng_json(data: &[u8], limit: usize) -> Vec<SearchResultItem> {
    let json: serde_json::Value = match serde_json::from_slice(data) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let Some(results) = json.get("results").and_then(|r| r.as_array()) else {
        return Vec::new();
    };

    results
        .iter()
        .take(limit)
        .enumerate()
        .map(|(i, r)| {
            let rank = (i + 1) as u32;
            SearchResultItem {
                result_id: format!("searxng_{rank}"),
                rank,
                url: r.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                title: r.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                snippet: r.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string(),
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
                source: "searxng".to_string(),
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
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_searxng_json() {
        let json = serde_json::json!({
            "results": [
                {"url": "https://example.com", "title": "Example", "content": "A site"},
                {"url": "https://test.com", "title": "Test", "content": "Another"}
            ]
        });
        let data = serde_json::to_vec(&json).unwrap();
        let results = parse_searxng_json(&data, 10);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].source, "searxng");
    }
}
