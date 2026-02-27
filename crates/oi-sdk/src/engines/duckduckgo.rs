use oi_types::{SearchEngine, SearchParams, SearchResultItem};

use super::{extract_attr, extract_tag_text, strip_html_tags, url_decode, urlencoded};

/// DuckDuckGo HTML scraper — always available, no API key needed.
pub struct DuckDuckGo;

impl SearchEngine for DuckDuckGo {
    fn name(&self) -> &str {
        "duckduckgo"
    }

    fn search(&self, query: &str, limit: usize) -> Vec<SearchResultItem> {
        self.search_with_params(query, limit, &SearchParams::default())
    }

    fn search_with_params(&self, query: &str, limit: usize, params: &SearchParams) -> Vec<SearchResultItem> {
        let q = if let Some(ref site) = params.site {
            format!("site:{site} {query}")
        } else {
            query.to_string()
        };

        let mut url = format!("https://html.duckduckgo.com/html/?q={}", urlencoded(&q));
        if let Some(ref locale) = params.locale {
            url.push_str(&format!("&kl={}", urlencoded(locale)));
        }

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
                tracing::warn!("DuckDuckGo search failed: {e}");
                return Vec::new();
            }
        };

        parse_duckduckgo_html(&body, limit)
    }
}

/// Parse DuckDuckGo HTML results page.
fn parse_duckduckgo_html(html: &str, limit: usize) -> Vec<SearchResultItem> {
    let mut results = Vec::new();
    let mut pos = 0;

    while results.len() < limit {
        let marker = "class=\"result__a\"";
        let Some(marker_pos) = html[pos..].find(marker) else {
            break;
        };
        let marker_pos = pos + marker_pos;

        let tag_start = html[..marker_pos].rfind('<').unwrap_or(marker_pos);
        let href = extract_attr(&html[tag_start..], "href").unwrap_or_default();

        let after_marker = marker_pos + marker.len();
        let title = extract_tag_text(&html[after_marker..]);

        let url = decode_ddg_url(&href);

        if url.is_empty() || !url.starts_with("http") {
            pos = after_marker;
            continue;
        }

        let snippet_search_end = (after_marker + 2000).min(html.len());
        let snippet =
            if let Some(spos) = html[after_marker..snippet_search_end].find("class=\"result__snippet\"") {
                let s = after_marker + spos;
                let text_start = s + "class=\"result__snippet\"".len();
                strip_html_tags(&extract_tag_text(&html[text_start..]))
            } else {
                String::new()
            };

        let rank = (results.len() + 1) as u32;
        results.push(SearchResultItem {
            result_id: format!("ddg_{rank}"),
            rank,
            url,
            title: strip_html_tags(&title),
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
            source: "duckduckgo".to_string(),
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

        pos = after_marker;
    }

    results
}

/// Decode DuckDuckGo redirect URLs.
fn decode_ddg_url(href: &str) -> String {
    if let Some(uddg_start) = href.find("uddg=") {
        let val_start = uddg_start + 5;
        let val_end = href[val_start..]
            .find('&')
            .map(|i| i + val_start)
            .unwrap_or(href.len());
        url_decode(&href[val_start..val_end])
    } else if href.starts_with("http") {
        href.to_string()
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_ddg_url() {
        let href = "//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fpage&rut=abc";
        assert_eq!(decode_ddg_url(href), "https://example.com/page");
    }

    #[test]
    fn test_decode_ddg_url_direct() {
        assert_eq!(decode_ddg_url("https://example.com"), "https://example.com");
    }

    #[test]
    fn test_parse_duckduckgo_html() {
        let html = r##"
        <div class="result">
            <a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Frust-lang.org&rut=x">Rust Language</a>
            <a class="result__snippet" href="#">A systems programming language</a>
        </div>
        <div class="result">
            <a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fdoc.rust-lang.org&rut=y">Rust Docs</a>
            <a class="result__snippet" href="#">Official documentation</a>
        </div>
        "##;
        let results = parse_duckduckgo_html(html, 10);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].url, "https://rust-lang.org");
        assert_eq!(results[0].title, "Rust Language");
        assert_eq!(results[0].source, "duckduckgo");
        assert_eq!(results[1].url, "https://doc.rust-lang.org");
        assert_eq!(results[1].rank, 2);
    }
}
