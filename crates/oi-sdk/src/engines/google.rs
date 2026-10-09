use oi_types::{SearchEngine, SearchResultItem};

use super::{strip_html_tags, urlencoded};

/// Google HTML scraper — no API key required. Fragile: depends on HTML structure.
pub struct GoogleScraper;

impl SearchEngine for GoogleScraper {
    fn name(&self) -> &str {
        "google"
    }

    fn search(&self, query: &str, limit: usize) -> Vec<SearchResultItem> {
        let url = format!(
            "https://www.google.com/search?q={}&num={limit}",
            urlencoded(query)
        );

        let agent = super::http_agent();
        let body = match agent
            .get(&url)
            .header("User-Agent", "Mozilla/5.0 (compatible; OpenSonarX/0.1)")
            .call()
        {
            Ok(mut resp) => match resp.body_mut().read_to_vec() {
                Ok(buf) => String::from_utf8_lossy(&buf).into_owned(),
                Err(_) => return Vec::new(),
            },
            Err(e) => {
                tracing::warn!("Google search failed: {e}");
                return Vec::new();
            }
        };

        parse_google_html(&body, limit)
    }
}

/// Parse Google search results HTML. Looks for `<a href="/url?q=..."` patterns.
fn parse_google_html(html: &str, limit: usize) -> Vec<SearchResultItem> {
    let mut results = Vec::new();
    let mut pos = 0;

    while results.len() < limit {
        // Google wraps result links in <a href="/url?q=ENCODED_URL&..."
        let marker = "/url?q=";
        let Some(idx) = html[pos..].find(marker) else {
            break;
        };
        let abs = pos + idx;
        let val_start = abs + marker.len();
        let val_end = html[val_start..]
            .find('&')
            .map(|i| i + val_start)
            .unwrap_or_else(|| super::bounded_byte_end(html, val_start, 500));
        let raw_url = super::url_decode(&html[val_start..val_end]);

        if !raw_url.starts_with("http") {
            pos = val_end;
            continue;
        }

        // Try to find the title in a nearby <h3> tag
        let search_end = super::bounded_byte_end(html, val_end, 2000);
        let title = if let Some(h3_pos) = html[val_end..search_end].find("<h3") {
            let h3_abs = val_end + h3_pos;
            strip_html_tags(&super::extract_tag_text(&html[h3_abs..]))
        } else {
            String::new()
        };

        // Try to find snippet in a nearby <span> after the title
        let snippet_search = super::bounded_byte_end(html, val_end, 3000);
        let snippet = if let Some(span_pos) = html[val_end..snippet_search].find("data-sncf=") {
            let span_abs = val_end + span_pos;
            strip_html_tags(&super::extract_tag_text(&html[span_abs..]))
        } else {
            String::new()
        };

        let rank = (results.len() + 1) as u32;
        results.push(SearchResultItem {
            result_id: format!("google_{rank}"),
            rank,
            url: raw_url,
            title,
            snippet,
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
            source: "google".to_string(),
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
        });

        pos = val_end;
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_google_html() {
        let html = r#"
        <a href="/url?q=https%3A%2F%2Fexample.com&sa=U">
            <h3>Example Site</h3>
        </a>
        <a href="/url?q=https%3A%2F%2Ftest.com&sa=U">
            <h3>Test Site</h3>
        </a>
        "#;
        let results = parse_google_html(html, 10);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].url, "https://example.com");
        assert_eq!(results[0].source, "google");
        assert_eq!(results[1].url, "https://test.com");
    }

    #[test]
    fn test_engine_name() {
        assert_eq!(GoogleScraper.name(), "google");
        assert!(!GoogleScraper.requires_api_key());
    }
}
