use oi_types::{SearchEngine, SearchParams, SearchResultItem};

/// Key-free MediaWiki full-text search. It is one corpus, not a general
/// web-search provider: it cannot satisfy source-independence or provider-
/// redundancy claims, and its snippets never become answer evidence.
pub struct WikipediaSearch;

impl SearchEngine for WikipediaSearch {
    fn name(&self) -> &str {
        "wikipedia"
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
        let host = &wikipedia_host(params.locale.as_deref());
        let url = wikipedia_search_url(host, query, limit);
        let agent = super::http_agent();
        let body = match agent
            .get(&url)
            .header(
                "User-Agent",
                "OpenSonarX/0.1 (factual reference discovery; contact via project repository)",
            )
            .call()
        {
            Ok(mut response) => response.body_mut().read_to_vec().map_err(|_| ())?,
            Err(error) => {
                tracing::warn!("Wikipedia search failed: {error}");
                return Err(());
            }
        };
        parse_wikipedia_search(&body, host, limit)
    }
}

/// The Wikipedia edition for the caller's locale (`pt-BR` → `pt`), English
/// when none is given. The edition is the caller's choice, never guessed from
/// the words of the query.
fn wikipedia_host(locale: Option<&str>) -> String {
    let language = locale
        .and_then(|locale| locale.split(['-', '_']).next())
        .map(str::to_ascii_lowercase)
        .filter(|language| {
            (2..=3).contains(&language.len()) && language.bytes().all(|b| b.is_ascii_lowercase())
        })
        .unwrap_or_else(|| "en".to_string());
    format!("{language}.wikipedia.org")
}

fn wikipedia_search_url(host: &str, query: &str, limit: usize) -> String {
    format!(
        "https://{host}/w/api.php?action=query&list=search&srsearch={}&srlimit={}&format=json&utf8=1",
        super::urlencoded(query),
        limit.clamp(1, 10),
    )
}

fn parse_wikipedia_search(
    body: &[u8],
    host: &str,
    limit: usize,
) -> std::result::Result<Vec<SearchResultItem>, ()> {
    let payload: serde_json::Value = serde_json::from_slice(body).map_err(|_| ())?;
    let rows = payload
        .get("query")
        .and_then(|query| query.get("search"))
        .and_then(serde_json::Value::as_array)
        .ok_or(())?;
    let mut results = Vec::new();
    for row in rows.iter().take(limit) {
        let Some(title) = row.get("title").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let page_id = row
            .get("pageid")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default();
        let snippet = row
            .get("snippet")
            .and_then(serde_json::Value::as_str)
            .map(super::strip_html_tags)
            .unwrap_or_default();
        let mut article = url::Url::parse(&format!("https://{host}")).map_err(|_| ())?;
        article
            .path_segments_mut()
            .map_err(|_| ())?
            .clear()
            .push("wiki")
            .push(&title.replace(' ', "_"));
        let rank = (results.len() + 1) as u32;
        results.push(SearchResultItem {
            result_id: format!("wikipedia_{page_id}"),
            rank,
            url: article.into(),
            title: title.to_string(),
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
            source: "wikipedia".to_string(),
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
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mediawiki_search_without_promoting_snippets_to_content() {
        let body = br#"{"query":{"search":[{"title":"Dario Amodei","pageid":74329563,"snippet":"He co-<span class=\"searchmatch\">founded</span> Anthropic."}]}}"#;
        let results = parse_wikipedia_search(body, "en.wikipedia.org", 5).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].source, "wikipedia");
        assert_eq!(results[0].title, "Dario Amodei");
        assert_eq!(results[0].snippet, "He co-founded Anthropic.");
        assert!(results[0].content_md.is_empty());
        assert_eq!(results[0].url, "https://en.wikipedia.org/wiki/Dario_Amodei");
    }

    #[test]
    fn mediawiki_query_is_bounded_and_encoded() {
        let url = wikipedia_search_url("en.wikipedia.org", "Anthropic founders history", 50);
        assert!(url.contains("srsearch=Anthropic+founders+history"));
        assert!(url.contains("srlimit=10"));
    }

    #[test]
    fn edition_follows_the_caller_locale_and_defaults_to_english() {
        assert_eq!(wikipedia_host(Some("pt-BR")), "pt.wikipedia.org");
        assert_eq!(wikipedia_host(Some("es_AR")), "es.wikipedia.org");
        assert_eq!(wikipedia_host(Some("DE")), "de.wikipedia.org");
        assert_eq!(wikipedia_host(None), "en.wikipedia.org");
        assert_eq!(wikipedia_host(Some("")), "en.wikipedia.org");
        assert_eq!(wikipedia_host(Some("x/../evil")), "en.wikipedia.org");
    }
}
