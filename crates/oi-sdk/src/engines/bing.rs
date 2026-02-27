use oi_types::{SearchEngine, SearchResultItem};

use super::{extract_attr, strip_html_tags, urlencoded};

/// Bing HTML scraper — no API key required. Fragile: depends on HTML structure.
pub struct BingScraper;

impl SearchEngine for BingScraper {
    fn name(&self) -> &str {
        "bing"
    }

    fn search(&self, query: &str, limit: usize) -> Vec<SearchResultItem> {
        let url = format!(
            "https://www.bing.com/search?q={}&count={limit}",
            urlencoded(query)
        );

        let agent = super::http_agent();
        let body = match agent.get(&url)
            .header("User-Agent", "Mozilla/5.0 (compatible; OpenSonarX/0.1)")
            .call()
        {
            Ok(mut resp) => match resp.body_mut().read_to_vec() {
                Ok(buf) => String::from_utf8_lossy(&buf).into_owned(),
                Err(_) => return Vec::new(),
            },
            Err(e) => {
                tracing::warn!("Bing search failed: {e}");
                return Vec::new();
            }
        };

        parse_bing_html(&body, limit)
    }
}

/// Parse Bing search results HTML. Looks for `<li class="b_algo">` blocks.
fn parse_bing_html(html: &str, limit: usize) -> Vec<SearchResultItem> {
    let mut results = Vec::new();
    let mut pos = 0;

    while results.len() < limit {
        // Bing wraps each result in <li class="b_algo">
        let marker = "class=\"b_algo\"";
        let Some(idx) = html[pos..].find(marker) else {
            break;
        };
        let abs = pos + idx + marker.len();

        // Find the <a href="..."> within this result block
        let search_end = (abs + 2000).min(html.len());
        let Some(a_pos) = html[abs..search_end].find("<a href=\"") else {
            pos = abs;
            continue;
        };
        let href_start = abs + a_pos;
        let url = extract_attr(&html[href_start..], "href").unwrap_or_default();

        if !url.starts_with("http") {
            pos = href_start + 10;
            continue;
        }

        // Title is the text inside the <a> tag
        let title = strip_html_tags(&super::extract_tag_text(&html[href_start..]));

        // Snippet: look for <p> or <div class="b_caption"> after the link
        let snippet_end = (href_start + 3000).min(html.len());
        let snippet = if let Some(p_pos) = html[href_start..snippet_end].find("<p") {
            let p_abs = href_start + p_pos;
            strip_html_tags(&super::extract_tag_text(&html[p_abs..]))
        } else {
            String::new()
        };

        let rank = (results.len() + 1) as u32;
        results.push(SearchResultItem {
            result_id: format!("bing_{rank}"),
            rank,
            url,
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
            source: "bing".to_string(),
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
        });

        pos = href_start + 10;
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_bing_html() {
        let html = r#"
        <li class="b_algo">
            <a href="https://example.com">Example Site</a>
            <p>An example description.</p>
        </li>
        <li class="b_algo">
            <a href="https://test.com">Test Site</a>
            <p>A test description.</p>
        </li>
        "#;
        let results = parse_bing_html(html, 10);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].url, "https://example.com");
        assert_eq!(results[0].source, "bing");
        assert_eq!(results[0].title, "Example Site");
        assert_eq!(results[1].url, "https://test.com");
    }

    #[test]
    fn test_engine_name() {
        assert_eq!(BingScraper.name(), "bing");
        assert!(!BingScraper.requires_api_key());
    }
}
