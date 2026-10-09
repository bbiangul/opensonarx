use oi_types::{SearchEngine, SearchResultItem};

/// Key-free MediaWiki full-text search used only as a bounded factual
/// reference fallback. It is deliberately not a general web-search provider:
/// one Wikimedia corpus cannot satisfy source-independence or provider-
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
        let host = wikipedia_host_for_query(query);
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

fn wikipedia_host_for_query(query: &str) -> &'static str {
    let lower = query.to_lowercase();
    let words = lower
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    let portuguese_cue = query
        .chars()
        .any(|character| "áàâãçéêíóôõúüÁÀÂÃÇÉÊÍÓÔÕÚÜ".contains(character))
        || words.iter().any(|word| {
            matches!(
                *word,
                "como"
                    | "criou"
                    | "explicada"
                    | "funciona"
                    | "fundou"
                    | "presidente"
                    | "quantica"
                    | "quem"
            )
        });
    if portuguese_cue {
        "pt.wikipedia.org"
    } else {
        "en.wikipedia.org"
    }
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
    fn mediawiki_query_routes_portuguese_and_english_to_matching_editions() {
        assert_eq!(
            wikipedia_host_for_query("como funciona computação quântica"),
            "pt.wikipedia.org"
        );
        assert_eq!(
            wikipedia_host_for_query("quantum computing explained"),
            "en.wikipedia.org"
        );
        assert!(
            wikipedia_search_url(
                wikipedia_host_for_query("computação quântica explicada"),
                "computação quântica explicada",
                5,
            )
            .starts_with("https://pt.wikipedia.org/")
        );
    }
}
