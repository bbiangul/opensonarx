use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use oi_types::{SearchEngine, SearchParams, SearchResultItem};

use super::{extract_attr, strip_html_tags, url_decode, urlencoded};

/// Bing HTML scraper — no API key required. Fragile: depends on HTML structure.
pub struct BingScraper;

impl SearchEngine for BingScraper {
    fn name(&self) -> &str {
        "bing"
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
        let url = bing_search_url(query, limit, params);

        let agent = super::http_agent();
        let body = match agent
            .get(&url)
            .header("User-Agent", "Mozilla/5.0 (compatible; OpenSonarX/0.1)")
            .call()
        {
            Ok(mut resp) => match resp.body_mut().read_to_vec() {
                Ok(buf) => String::from_utf8_lossy(&buf).into_owned(),
                Err(_) => return Err(()),
            },
            Err(e) => {
                tracing::warn!("Bing search failed: {e}");
                return Err(());
            }
        };

        Ok(if params.freshness.is_some() {
            parse_bing_news_html(&body, limit)
        } else {
            parse_bing_html(&body, limit)
        })
    }
}

fn bing_search_url(query: &str, limit: usize, params: &SearchParams) -> String {
    let q = if let Some(ref site) = params.site {
        format!("site:{site} {query}")
    } else {
        query.to_string()
    };
    let path = if params.freshness.is_some() {
        "news/search"
    } else {
        "search"
    };
    let mut url = format!("https://www.bing.com/{path}?q={}", urlencoded(&q));
    if params.freshness.is_none() {
        url.push_str(&format!("&count={limit}"));
    }
    if let Some(ref locale) = params.locale {
        // Bing otherwise commonly serves an en-US result mix from datacenter
        // IPs even when the query itself is Portuguese.
        url.push_str(&format!("&setlang={}", urlencoded(locale)));
    }
    if let Some(ref country) = params.gl {
        url.push_str(&format!(
            "&cc={}",
            urlencoded(&country.to_ascii_uppercase())
        ));
    }
    url
}

/// Parse Bing News cards. News uses `data-url`/`data-title` metadata instead
/// of the web result redirect anchors, so treating it as normal web HTML
/// silently returns zero candidates.
fn parse_bing_news_html(html: &str, limit: usize) -> Vec<SearchResultItem> {
    let marker = "class=\"news-card newsitem";
    let mut results = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut pos = 0;
    while results.len() < limit {
        let Some(relative) = html[pos..].find(marker) else {
            break;
        };
        let marker_abs = pos + relative;
        let block_start = html[..marker_abs].rfind('<').unwrap_or(marker_abs);
        let next_start = marker_abs + marker.len();
        let block_end = html[next_start..]
            .find(marker)
            .map(|next| next_start + next)
            .unwrap_or(html.len());
        let block = &html[block_start..block_end];
        let url = extract_attr(block, "data-url").unwrap_or_default();
        if !url.starts_with("http") || !seen.insert(url.clone()) {
            pos = next_start;
            continue;
        }
        let title = extract_attr(block, "data-title").unwrap_or_default();
        let snippet = block
            .find("class=\"snippet\"")
            .map(|snippet_pos| strip_html_tags(&super::extract_tag_text(&block[snippet_pos..])))
            .unwrap_or_default();
        let rank = (results.len() + 1) as u32;
        let mut result = external_result(rank, url, title, snippet);
        result.published_at = parse_bing_news_published_at(block);
        results.push(result);
        pos = next_start;
    }
    results
}

fn parse_bing_news_published_at(block: &str) -> Option<i64> {
    let mut remaining = block;
    while let Some(index) = remaining.find("aria-label=\"") {
        let value_start = index + "aria-label=\"".len();
        let tail = &remaining[value_start..];
        let value_end = tail.find('"')?;
        let label = tail[..value_end].to_ascii_lowercase();
        let seconds = if label == "just now" {
            Some(0)
        } else if label == "yesterday" {
            Some(86_400)
        } else if label.ends_with(" ago") {
            let amount = label
                .split_whitespace()
                .next()
                .and_then(|value| value.parse::<i64>().ok());
            amount.and_then(|amount| {
                if label.contains("minute") {
                    Some(amount * 60)
                } else if label.contains("hour") {
                    Some(amount * 3_600)
                } else if label.contains("day") {
                    Some(amount * 86_400)
                } else {
                    None
                }
            })
        } else {
            None
        };
        if let Some(seconds) = seconds {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?
                .as_secs() as i64;
            return Some(now.saturating_sub(seconds));
        }
        remaining = &tail[value_end + 1..];
    }
    None
}

fn external_result(rank: u32, url: String, title: String, snippet: String) -> SearchResultItem {
    SearchResultItem {
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
        decision: None,
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
        let marker_abs = pos + idx;
        let block_start = html[..marker_abs].rfind('<').unwrap_or(marker_abs);
        let after_marker = marker_abs + marker.len();
        let block_end = html[after_marker..]
            .find(marker)
            .map(|next| {
                let next_marker = after_marker + next;
                html[..next_marker].rfind('<').unwrap_or(next_marker)
            })
            .unwrap_or(html.len());
        let block = &html[block_start..block_end];

        // The current Bing page includes a favicon/attribution link before
        // the actual result. Select the anchor inside the result heading and
        // allow arbitrary attribute order (`<a target=... href=...>`).
        let anchor_search_start = block.find("<h2").unwrap_or(0);
        let Some(relative_a_pos) = block[anchor_search_start..].find("<a") else {
            pos = after_marker;
            continue;
        };
        let a_pos = anchor_search_start + relative_a_pos;
        let href = extract_attr(&block[a_pos..], "href").unwrap_or_default();
        let url = decode_bing_url(&href);

        if !url.starts_with("http") {
            pos = after_marker;
            continue;
        }

        // Title is the text inside the <a> tag
        let title = strip_html_tags(&super::extract_tag_text(&block[a_pos..]));

        // Snippet: look for <p> or <div class="b_caption"> after the link
        let snippet = if let Some(relative_p_pos) = block[a_pos..].find("<p") {
            let p_pos = a_pos + relative_p_pos;
            strip_html_tags(&super::extract_tag_text(&block[p_pos..]))
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
            decision: None,
        });

        pos = after_marker;
    }

    results
}

/// Resolve Bing's current `ck/a?...&u=a1BASE64URL...` result links to the
/// publisher URL. Direct publisher URLs are preserved for compatibility with
/// older/alternate result pages.
fn decode_bing_url(href: &str) -> String {
    if !href.contains("bing.com/ck/a") {
        return href.to_string();
    }
    let encoded = href
        .split('&')
        .find_map(|part| part.strip_prefix("u="))
        .map(url_decode)
        .unwrap_or_default();
    let payload = encoded.strip_prefix("a1").unwrap_or(&encoded);
    URL_SAFE_NO_PAD
        .decode(payload)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .filter(|url| url.starts_with("http://") || url.starts_with("https://"))
        .unwrap_or_default()
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
    fn test_parse_current_bing_attribute_order_and_redirect() {
        let encoded = URL_SAFE_NO_PAD.encode("https://pt.wikipedia.org/wiki/Pix");
        let html = format!(
            r#"<li class="b_algo"><div><a class="tilk" href="https://bing.com/attribution">Wikipedia</a></div><h2><a target="_blank" href="https://www.bing.com/ck/a?x=1&amp;u=a1{encoded}&amp;ntb=1">Pix – Wikipédia</a></h2><div class="b_caption"><p>Sistema de pagamentos instantâneos.</p></div></li>"#
        );
        let results = parse_bing_html(&html, 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url, "https://pt.wikipedia.org/wiki/Pix");
        assert_eq!(results[0].title, "Pix – Wikipédia");
        assert_eq!(results[0].snippet, "Sistema de pagamentos instantâneos.");
    }

    #[test]
    fn test_engine_name() {
        assert_eq!(BingScraper.name(), "bing");
        assert!(!BingScraper.requires_api_key());
    }

    #[test]
    fn test_bing_search_url_preserves_scope_and_brazilian_market() {
        let params = SearchParams {
            locale: Some("pt-BR".to_string()),
            gl: Some("br".to_string()),
            site: Some("g1.globo.com".to_string()),
            freshness: None,
        };
        let url = bing_search_url("pesquisa eleitoral hoje", 10, &params);
        assert!(url.contains("q=site%3Ag1.globo.com+pesquisa+eleitoral+hoje"));
        assert!(url.contains("setlang=pt-BR"));
        assert!(url.contains("cc=BR"));
    }

    #[test]
    fn test_bing_news_url_and_current_card_parser() {
        let params = SearchParams {
            locale: Some("pt-BR".to_string()),
            gl: Some("br".to_string()),
            freshness: Some("month".to_string()),
            ..Default::default()
        };
        let url = bing_search_url("pesquisa eleitoral hoje", 10, &params);
        assert!(url.starts_with("https://www.bing.com/news/search?"));
        assert!(!url.contains("count="));

        let html = r#"
          <div class="news-card newsitem cardcommon"
               data-url="https://example.com/pesquisa"
               data-title="Datafolha: Lula tem 39% &amp; Flávio 33%">
            <a class="title" href="https://example.com/pesquisa"><h2>Resultado</h2></a>
            <div class="snippet" title="Pesquisa publicada hoje">Pesquisa publicada hoje</div>
            <span tabindex="0" aria-label="2 hours ago">2h</span>
          </div>
          <div class="news-card newsitem cardcommon"
               data-url="https://example.org/segundo-turno"
               data-title="Segundo turno"><div class="snippet">47% a 43%</div></div>
        "#;
        let results = parse_bing_news_html(html, 10);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].url, "https://example.com/pesquisa");
        assert_eq!(results[0].title, "Datafolha: Lula tem 39% & Flávio 33%");
        assert_eq!(results[0].snippet, "Pesquisa publicada hoje");
        let age = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            - results[0].published_at.unwrap();
        assert!((7_190..=7_210).contains(&age));
        assert_eq!(results[1].url, "https://example.org/segundo-turno");
    }
}
