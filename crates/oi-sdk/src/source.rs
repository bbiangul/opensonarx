//! URL-structure helpers. A source family is the registrable domain of a
//! URL: a fact about its host, not a judgement about who owns or wrote it.

/// Host of a URL without scheme, port, `www.` or trailing dot.
pub fn host(url: &str) -> String {
    url.split("//")
        .nth(1)
        .unwrap_or(url)
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .rsplit('@')
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("")
        .trim_start_matches("www.")
        .trim_end_matches('.')
        .to_ascii_lowercase()
}

/// Registrable domain of a URL (`g1.globo.com` → `globo.com`,
/// `www.bcb.gov.br` → `bcb.gov.br`). Two pages in one family are never
/// counted as two reporting sources. Ownership across registrable domains is
/// not known here and stays unresolved in the claim lineage.
pub fn source_family(url: &str) -> String {
    let host = host(url);
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() <= 2 {
        return host;
    }
    // Country-code namespaces register names one level down (`com.br`,
    // `co.uk`, `gov.br`). This is DNS registration structure.
    const SECOND_LEVEL: [&str; 9] = ["ac", "co", "com", "edu", "gov", "mil", "net", "nom", "org"];
    let tld = labels[labels.len() - 1];
    let second = labels[labels.len() - 2];
    let keep = if tld.len() == 2 && SECOND_LEVEL.contains(&second) {
        3
    } else {
        2
    };
    labels[labels.len() - keep..].join(".")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_strips_scheme_port_userinfo_and_www() {
        assert_eq!(host("https://www.Example.com:8443/a?b#c"), "example.com");
        assert_eq!(host("http://user@example.org./x"), "example.org");
        assert_eq!(host("example.net/path"), "example.net");
        assert_eq!(host(""), "");
    }

    #[test]
    fn family_is_the_registrable_domain() {
        assert_eq!(
            source_family("https://g1.globo.com/politica/x"),
            "globo.com"
        );
        assert_eq!(source_family("https://www.bcb.gov.br/pix"), "bcb.gov.br");
        assert_eq!(source_family("https://news.bbc.co.uk/1"), "bbc.co.uk");
        assert_eq!(
            source_family("https://en.wikipedia.org/wiki/X"),
            "wikipedia.org"
        );
        assert_eq!(source_family("https://example.com"), "example.com");
        assert_eq!(source_family("localhost:8001/x"), "localhost");
    }
}
