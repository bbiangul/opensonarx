use oi_types::{SearchEngine, SearchResultItem};

use super::urlencoded;

/// Qwant JSON API adapter — EU-based search, no API key needed.
pub struct Qwant;

impl SearchEngine for Qwant {
    fn name(&self) -> &str {
        "qwant"
    }

    fn search(&self, query: &str, limit: usize) -> Vec<SearchResultItem> {
        let url = format!(
            "https://api.qwant.com/v3/search/web?q={}&count={limit}&locale=en_US",
            urlencoded(query)
        );

        let agent = super::http_agent();
        let body = match agent
            .get(&url)
            .header("User-Agent", "Mozilla/5.0 (compatible; OpenSonarX/0.1)")
            .call()
        {
            Ok(mut resp) => match resp.body_mut().read_to_vec() {
                Ok(buf) => buf,
                Err(_) => return Vec::new(),
            },
            Err(e) => {
                tracing::warn!("Qwant search failed: {e}");
                return Vec::new();
            }
        };

        parse_qwant_json(&body, limit)
    }
}

fn parse_qwant_json(data: &[u8], limit: usize) -> Vec<SearchResultItem> {
    let json: serde_json::Value = match serde_json::from_slice(data) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let Some(items) = json
        .get("data")
        .and_then(|d| d.get("result"))
        .and_then(|r| r.get("items"))
        .and_then(|i| i.as_array())
    else {
        return Vec::new();
    };

    items
        .iter()
        .take(limit)
        .enumerate()
        .map(|(i, r)| {
            let rank = (i + 1) as u32;
            SearchResultItem {
                result_id: format!("qwant_{rank}"),
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
                    .get("desc")
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
                source: "qwant".to_string(),
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
    fn test_parse_qwant_json() {
        let json = serde_json::json!({
            "data": {
                "result": {
                    "items": [
                        {"url": "https://example.com", "title": "Example", "desc": "A site"},
                        {"url": "https://test.com", "title": "Test", "desc": "Another"}
                    ]
                }
            }
        });
        let data = serde_json::to_vec(&json).unwrap();
        let results = parse_qwant_json(&data, 10);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].source, "qwant");
    }

    #[test]
    fn test_engine_name() {
        assert_eq!(Qwant.name(), "qwant");
        assert!(!Qwant.requires_api_key());
    }
}
