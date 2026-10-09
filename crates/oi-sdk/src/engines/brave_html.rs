use std::collections::HashSet;

use oi_types::{SearchEngine, SearchParams, SearchResultItem};

use super::{extract_attr, strip_html_tags, urlencoded};

/// Keyless adapter for Brave Search's public HTML result surface.
///
/// This is intentionally a separate engine from [`super::brave::BraveSearch`]:
/// the latter is the contracted JSON API and requires a subscription token,
/// while this adapter is a best-effort fallback protected by the same cache,
/// concurrency limit, and circuit breaker as the other HTML providers.
pub struct BraveHtml;

impl SearchEngine for BraveHtml {
    fn name(&self) -> &str {
        "brave-html"
    }

    fn search(&self, query: &str, limit: usize) -> Vec<SearchResultItem> {
        self.search_checked(query, limit).unwrap_or_default()
    }

    fn search_checked(
        &self,
        query: &str,
        limit: usize,
    ) -> std::result::Result<Vec<SearchResultItem>, ()> {
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
    ) -> std::result::Result<Vec<SearchResultItem>, ()> {
        let q = if let Some(ref site) = params.site {
            format!("site:{site} {query}")
        } else {
            query.to_string()
        };
        let mut url = format!(
            "https://search.brave.com/search?q={}&source=web",
            urlencoded(&q)
        );
        if let Some(ref country) = params.gl {
            url.push_str(&format!("&country={}", urlencoded(country)));
        }
        if let Some(freshness) = params.freshness.as_deref().and_then(brave_freshness) {
            url.push_str("&tf=");
            url.push_str(freshness);
        }

        let accept_language = params
            .locale
            .as_deref()
            .map(|locale| format!("{locale},en;q=0.8"))
            .unwrap_or_else(|| "en-US,en;q=0.9".to_string());
        let agent = super::http_agent();
        let body = match agent
            .get(&url)
            // Identify the crawler truthfully. A provider that only works
            // when impersonating Chrome is not a production dependency.
            .header("User-Agent", "OpenSonarX/0.1 (+https://opensonarx.org)")
            .header("Accept", "text/html,application/xhtml+xml")
            .header("Accept-Language", &accept_language)
            .call()
        {
            Ok(mut response) => match response.body_mut().read_to_vec() {
                Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                Err(_) => return Err(()),
            },
            Err(error) => {
                tracing::warn!("Brave HTML search failed: {error}");
                return Err(());
            }
        };

        let results = parse_brave_html(&body, limit);
        if results.is_empty() && brave_html_is_challenge(&body) {
            tracing::warn!("Brave HTML search returned an anti-bot challenge");
            return Err(());
        }
        Ok(results)
    }
}

fn brave_freshness(value: &str) -> Option<&'static str> {
    match value.to_ascii_lowercase().as_str() {
        "day" | "24h" => Some("pd"),
        "week" => Some("pw"),
        "month" => Some("pm"),
        "year" => Some("py"),
        _ => None,
    }
}

/// Parse the organic `data-type="web"` cards from Brave's rendered HTML.
/// The result URL, title, and body are all scoped to one card so links in
/// knowledge panels, image clusters, and the footer cannot become crawl leads.
fn parse_brave_html(html: &str, limit: usize) -> Vec<SearchResultItem> {
    const WEB_MARKER: &str = "data-type=\"web\"";
    let mut results = Vec::new();
    let mut seen_urls = HashSet::new();
    let mut pos = 0;

    while results.len() < limit {
        let Some(relative_marker) = html[pos..].find(WEB_MARKER) else {
            break;
        };
        let marker = pos + relative_marker;
        let block_start = html[..marker]
            .rfind("<div class=\"snippet")
            .unwrap_or(marker);
        let after_marker = marker + WEB_MARKER.len();
        let block_end = html[after_marker..]
            .find(WEB_MARKER)
            .map(|next| after_marker + next)
            .unwrap_or(html.len());
        let block = &html[block_start..block_end];

        let result_content = block
            .find("class=\"result-content")
            .map(|index| &block[index..])
            .unwrap_or(block);
        let Some(anchor_offset) = result_content.find("<a ") else {
            pos = after_marker;
            continue;
        };
        let anchor = &result_content[anchor_offset..];
        let url = extract_attr(anchor, "href").unwrap_or_default();
        if !matches!(url::Url::parse(&url), Ok(parsed) if matches!(parsed.scheme(), "http" | "https"))
            || !seen_urls.insert(url.clone())
        {
            pos = after_marker;
            continue;
        }

        let title = block
            .find("search-snippet-title")
            .map(|title_marker| {
                let tag_start = block[..title_marker].rfind('<').unwrap_or(title_marker);
                extract_element_text(&block[tag_start..], "div")
            })
            .unwrap_or_default();
        let snippet = block
            .find("class=\"generic-snippet")
            .and_then(|generic| {
                block[generic..]
                    .find("class=\"content ")
                    .map(|i| generic + i)
            })
            .map(|content_marker| {
                let tag_start = block[..content_marker].rfind('<').unwrap_or(content_marker);
                extract_element_text(&block[tag_start..], "div")
            })
            .unwrap_or_default();

        let rank = (results.len() + 1) as u32;
        results.push(external_result(rank, url, title, snippet));
        pos = after_marker;
    }

    results
}

fn extract_element_text(fragment: &str, tag: &str) -> String {
    let Some(content_start) = fragment.find('>').map(|index| index + 1) else {
        return String::new();
    };
    let closing = format!("</{tag}>");
    let content_end = fragment[content_start..]
        .find(&closing)
        .map(|index| content_start + index)
        .unwrap_or(fragment.len());
    strip_html_tags(&fragment[content_start..content_end])
}

fn brave_html_is_challenge(html: &str) -> bool {
    let lower = html.to_ascii_lowercase();
    (lower.contains("captcha") || lower.contains("challenge"))
        && !lower.contains("data-type=\"web\"")
}

fn external_result(rank: u32, url: String, title: String, snippet: String) -> SearchResultItem {
    SearchResultItem {
        result_id: format!("brave_html_{rank}"),
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
        source: "brave-html".to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_scoped_web_cards() {
        let html = r#"
        <a href="https://search.brave.com/help">Help</a>
        <div class="snippet hash" data-pos="1" data-type="web">
          <div class="result-content hash">
            <a href="https://example.com/tool?x=1&amp;y=2" class="hash">
              <div class="site-name-wrapper">Example</div>
              <div class="title search-snippet-title hash">Online &amp; Useful</div>
            </a>
            <div class="generic-snippet hash">
              <div class="content desktop hash">Run code &#x27;now&#x27;.</div>
            </div>
          </div>
        </div>
        <div class="snippet hash" data-pos="2" data-type="web">
          <div class="result-content hash">
            <a href="https://second.example/">
              <div class="title search-snippet-title hash"><b>Second</b> result</div>
            </a>
          </div>
        </div>
        "#;
        let results = parse_brave_html(html, 10);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].url, "https://example.com/tool?x=1&y=2");
        assert_eq!(results[0].title, "Online & Useful");
        assert_eq!(results[0].snippet, "Run code 'now'.");
        assert_eq!(results[0].source, "brave-html");
        assert_eq!(results[1].title, "Second result");
    }

    #[test]
    fn rejects_non_http_and_deduplicates_urls() {
        let html = r#"
        <div class="snippet" data-type="web"><div class="result-content">
          <a href="javascript:alert(1)"><div class="search-snippet-title">Bad</div></a>
        </div></div>
        <div class="snippet" data-type="web"><div class="result-content">
          <a href="https://example.com"><div class="search-snippet-title">Good</div></a>
        </div></div>
        <div class="snippet" data-type="web"><div class="result-content">
          <a href="https://example.com"><div class="search-snippet-title">Duplicate</div></a>
        </div></div>
        "#;
        let results = parse_brave_html(html, 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Good");
    }

    #[test]
    fn identifies_challenge_without_misclassifying_real_results() {
        assert!(brave_html_is_challenge("<title>Captcha challenge</title>"));
        assert!(!brave_html_is_challenge(
            "captcha support <div data-type=\"web\">result</div>"
        ));
    }

    #[test]
    fn maps_supported_freshness_windows() {
        assert_eq!(brave_freshness("day"), Some("pd"));
        assert_eq!(brave_freshness("WEEK"), Some("pw"));
        assert_eq!(brave_freshness("unknown"), None);
    }
}
