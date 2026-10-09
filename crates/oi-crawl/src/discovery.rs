//! Content discovery engines: RSS/Atom, Sitemap, and link extraction.

use std::collections::HashSet;

use quick_xml::Reader;
use quick_xml::events::Event;
use url::Url;

/// A discovered URL with metadata about how it was found.
#[derive(Debug, Clone)]
pub struct DiscoveredUrl {
    pub url: String,
    pub source: DiscoverySource,
    /// Last modification time (from sitemap `<lastmod>` or feed `<pubDate>/<updated>`).
    pub last_modified: Option<String>,
}

/// How the URL was discovered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoverySource {
    /// From an RSS or Atom feed.
    Feed,
    /// From a sitemap.xml.
    Sitemap,
    /// Extracted from page HTML links.
    Link,
}

// ── RSS / Atom Feed Parsing ──

/// Parse an RSS or Atom feed XML and extract entry URLs.
pub fn parse_feed(xml: &str) -> Vec<DiscoveredUrl> {
    // Try RSS first, then Atom
    let results = parse_rss(xml);
    if results.is_empty() {
        parse_atom(xml)
    } else {
        results
    }
}

fn parse_rss(xml: &str) -> Vec<DiscoveredUrl> {
    let mut reader = Reader::from_str(xml);
    let mut results = Vec::new();
    let mut buf = Vec::new();
    let mut in_item = false;
    let mut current_tag = String::new();
    let mut current_link = String::new();
    let mut current_pub_date = String::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let raw_name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                let name = strip_ns(&raw_name);
                if name == "item" {
                    in_item = true;
                    current_link.clear();
                    current_pub_date.clear();
                }
                if in_item {
                    current_tag = name.to_string();
                }
            }
            Ok(Event::Text(ref e)) if in_item => {
                if let Ok(text) = e.unescape() {
                    let text = text.trim().to_string();
                    if !text.is_empty() {
                        match current_tag.as_str() {
                            "link" => current_link = text,
                            "pubDate" => current_pub_date = text,
                            _ => {}
                        }
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                let raw_name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                let name = strip_ns(&raw_name);
                if name == "item" && !current_link.is_empty() {
                    results.push(DiscoveredUrl {
                        url: current_link.clone(),
                        source: DiscoverySource::Feed,
                        last_modified: if current_pub_date.is_empty() {
                            None
                        } else {
                            Some(current_pub_date.clone())
                        },
                    });
                    in_item = false;
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    results
}

fn parse_atom(xml: &str) -> Vec<DiscoveredUrl> {
    let mut reader = Reader::from_str(xml);
    let mut results = Vec::new();
    let mut buf = Vec::new();
    let mut in_entry = false;
    let mut current_tag = String::new();
    let mut current_link = String::new();
    let mut current_updated = String::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                let raw_name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                let name = strip_ns(&raw_name);
                if name == "entry" {
                    in_entry = true;
                    current_link.clear();
                    current_updated.clear();
                }
                if in_entry && name == "link" {
                    // Atom links are in attributes: <link href="..." rel="alternate"/>
                    let mut href = String::new();
                    let mut rel = String::new();
                    for attr in e.attributes().flatten() {
                        let key = String::from_utf8_lossy(attr.key.as_ref()).to_string();
                        let val = String::from_utf8_lossy(&attr.value).to_string();
                        if key == "href" {
                            href = val;
                        } else if key == "rel" {
                            rel = val;
                        }
                    }
                    if !href.is_empty() && (rel.is_empty() || rel == "alternate") {
                        current_link = href;
                    }
                }
                if in_entry {
                    current_tag = name.to_string();
                }
            }
            Ok(Event::Text(ref e)) if in_entry => {
                if let Ok(text) = e.unescape() {
                    let text = text.trim().to_string();
                    if !text.is_empty() && (current_tag == "updated" || current_tag == "published")
                    {
                        current_updated = text;
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                let raw_name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                let name = strip_ns(&raw_name);
                if name == "entry" && !current_link.is_empty() {
                    results.push(DiscoveredUrl {
                        url: current_link.clone(),
                        source: DiscoverySource::Feed,
                        last_modified: if current_updated.is_empty() {
                            None
                        } else {
                            Some(current_updated.clone())
                        },
                    });
                    in_entry = false;
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    results
}

// ── Sitemap Parsing ──

/// Strip XML namespace prefix from a tag name (e.g., "ns:url" → "url").
fn strip_ns(name: &str) -> &str {
    name.rsplit_once(':').map_or(name, |(_, local)| local)
}

/// Parse a sitemap.xml and extract URLs with optional `<lastmod>`.
/// Handles both `<urlset>` sitemaps and `<sitemapindex>` (returns sub-sitemap URLs).
/// Supports namespaced tags (e.g., `<ns:url>`, `<ns:loc>`).
pub fn parse_sitemap(xml: &str) -> Vec<DiscoveredUrl> {
    let mut reader = Reader::from_str(xml);
    let mut results = Vec::new();
    let mut buf = Vec::new();
    let mut in_url = false;
    let mut in_sitemap = false;
    let mut current_tag = String::new();
    let mut current_loc = String::new();
    let mut current_lastmod = String::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let raw_name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                let name = strip_ns(&raw_name);
                match name {
                    "url" => {
                        in_url = true;
                        current_loc.clear();
                        current_lastmod.clear();
                    }
                    "sitemap" => {
                        in_sitemap = true;
                        current_loc.clear();
                        current_lastmod.clear();
                    }
                    _ => {}
                }
                if in_url || in_sitemap {
                    current_tag = name.to_string();
                }
            }
            Ok(Event::Text(ref e)) if in_url || in_sitemap => {
                if let Ok(text) = e.unescape() {
                    let text = text.trim().to_string();
                    if !text.is_empty() {
                        match current_tag.as_str() {
                            "loc" => current_loc = text,
                            "lastmod" => current_lastmod = text,
                            _ => {}
                        }
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                let raw_name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                let name = strip_ns(&raw_name);
                if (name == "url" || name == "sitemap") && !current_loc.is_empty() {
                    results.push(DiscoveredUrl {
                        url: current_loc.clone(),
                        source: DiscoverySource::Sitemap,
                        last_modified: if current_lastmod.is_empty() {
                            None
                        } else {
                            Some(current_lastmod.clone())
                        },
                    });
                    in_url = false;
                    in_sitemap = false;
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    results
}

// ── Link Extraction ──

/// Extract internal links from HTML content for a given base URL.
/// Returns deduplicated absolute URLs on the same domain.
pub fn extract_links(html: &str, base_url: &str) -> Vec<DiscoveredUrl> {
    let base = match Url::parse(base_url) {
        Ok(u) => u,
        Err(_) => return Vec::new(),
    };
    let base_domain = match base.domain() {
        Some(d) => d.to_lowercase(),
        None => return Vec::new(),
    };

    let mut seen = HashSet::new();
    let mut results = Vec::new();

    // Simple href extraction — handles <a href="...">
    for href in extract_href_values(html) {
        let resolved = match base.join(&href) {
            Ok(u) => u,
            Err(_) => continue,
        };

        // Only same-domain links
        let domain = match resolved.domain() {
            Some(d) => d.to_lowercase(),
            None => continue,
        };
        if domain != base_domain {
            continue;
        }

        // Normalize: strip fragment, ensure https
        let mut normalized = resolved.clone();
        normalized.set_fragment(None);
        let url_str = normalized.to_string();

        // Skip non-content URLs
        if should_skip_url(&url_str) {
            continue;
        }

        if seen.insert(url_str.clone()) {
            results.push(DiscoveredUrl {
                url: url_str,
                source: DiscoverySource::Link,
                last_modified: None,
            });
        }
    }

    results
}

/// Extract cross-domain links from HTML content.
/// Returns links pointing to domains other than the base URL's domain.
pub fn extract_cross_domain_links(html: &str, base_url: &str) -> Vec<DiscoveredUrl> {
    let base = match Url::parse(base_url) {
        Ok(u) => u,
        Err(_) => return Vec::new(),
    };
    let base_domain = match base.domain() {
        Some(d) => d.to_lowercase(),
        None => return Vec::new(),
    };

    let mut seen = HashSet::new();
    let mut results = Vec::new();

    for href in extract_href_values(html) {
        let resolved = match Url::parse(&href).or_else(|_| base.join(&href)) {
            Ok(u) => u,
            Err(_) => continue,
        };

        let domain = match resolved.domain() {
            Some(d) => d.to_lowercase(),
            None => continue,
        };

        // Only cross-domain links
        if domain == base_domain {
            continue;
        }

        // Must be http/https
        if resolved.scheme() != "http" && resolved.scheme() != "https" {
            continue;
        }

        let mut normalized = resolved;
        normalized.set_fragment(None);
        let url_str = normalized.to_string();

        if seen.insert(url_str.clone()) {
            results.push(DiscoveredUrl {
                url: url_str,
                source: DiscoverySource::Link,
                last_modified: None,
            });
        }
    }

    results
}

fn extract_href_values(html: &str) -> Vec<String> {
    let mut hrefs = Vec::new();
    let lower = html.to_lowercase();
    let mut search_from = 0;

    while let Some(pos) = lower[search_from..].find("href=") {
        let abs_pos = search_from + pos + 5;
        if abs_pos >= html.len() {
            break;
        }
        let rest = &html[abs_pos..];
        let value = if let Some(stripped) = rest.strip_prefix('"') {
            stripped.split('"').next()
        } else if let Some(stripped) = rest.strip_prefix('\'') {
            stripped.split('\'').next()
        } else {
            rest.split_whitespace()
                .next()
                .map(|s| s.trim_end_matches('>'))
        };
        if let Some(v) = value {
            let v = v.trim();
            if !v.is_empty()
                && !v.starts_with('#')
                && !v.starts_with("javascript:")
                && !v.starts_with("mailto:")
            {
                hrefs.push(v.to_string());
            }
        }
        search_from = abs_pos + 1;
    }
    hrefs
}

fn should_skip_url(url: &str) -> bool {
    let lower = url.to_lowercase();
    // Skip common non-content paths
    let skip_extensions = [
        ".jpg", ".jpeg", ".png", ".gif", ".svg", ".webp", ".ico", ".css", ".js", ".woff", ".woff2",
        ".ttf", ".eot", ".pdf", ".zip", ".tar", ".gz", ".mp3", ".mp4", ".avi", ".xml", ".json",
        ".rss",
    ];
    for ext in &skip_extensions {
        if lower.ends_with(ext) {
            return true;
        }
    }
    // Use path-segment matching: check that the skip pattern appears as a
    // complete path segment (preceded by '/' and followed by '/' or end).
    let skip_segments = [
        "login", "signup", "register", "logout", "admin", "wp-admin", "cart", "checkout",
    ];
    if let Some(path_start) = lower.find("://").map(|i| i + 3) {
        let path = lower[path_start..]
            .find('/')
            .map_or("", |i| &lower[path_start + i..]);
        for seg in &skip_segments {
            let pattern = format!("/{seg}");
            if let Some(pos) = path.find(&pattern) {
                let after = pos + pattern.len();
                if after >= path.len()
                    || path.as_bytes()[after] == b'/'
                    || path.as_bytes()[after] == b'?'
                {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_rss_feed() {
        let rss = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
  <channel>
    <title>Test Feed</title>
    <item>
      <title>Article 1</title>
      <link>https://example.com/article-1</link>
      <pubDate>Mon, 01 Jan 2024 12:00:00 GMT</pubDate>
    </item>
    <item>
      <title>Article 2</title>
      <link>https://example.com/article-2</link>
    </item>
  </channel>
</rss>"#;

        let urls = parse_feed(rss);
        assert_eq!(urls.len(), 2);
        assert_eq!(urls[0].url, "https://example.com/article-1");
        assert_eq!(urls[0].source, DiscoverySource::Feed);
        assert!(urls[0].last_modified.is_some());
        assert_eq!(urls[1].url, "https://example.com/article-2");
        assert!(urls[1].last_modified.is_none());
    }

    #[test]
    fn test_parse_atom_feed() {
        let atom = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>Test Atom Feed</title>
  <entry>
    <title>Entry 1</title>
    <link href="https://example.com/entry-1" rel="alternate"/>
    <updated>2024-01-01T12:00:00Z</updated>
  </entry>
  <entry>
    <title>Entry 2</title>
    <link href="https://example.com/entry-2"/>
    <published>2024-01-02T12:00:00Z</published>
  </entry>
</feed>"#;

        let urls = parse_feed(atom);
        assert_eq!(urls.len(), 2);
        assert_eq!(urls[0].url, "https://example.com/entry-1");
        assert_eq!(urls[0].source, DiscoverySource::Feed);
        assert_eq!(urls[1].url, "https://example.com/entry-2");
    }

    #[test]
    fn test_parse_sitemap() {
        let sitemap = r#"<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
  <url>
    <loc>https://example.com/page-1</loc>
    <lastmod>2024-01-01</lastmod>
  </url>
  <url>
    <loc>https://example.com/page-2</loc>
  </url>
</urlset>"#;

        let urls = parse_sitemap(sitemap);
        assert_eq!(urls.len(), 2);
        assert_eq!(urls[0].url, "https://example.com/page-1");
        assert_eq!(urls[0].source, DiscoverySource::Sitemap);
        assert!(urls[0].last_modified.is_some());
        assert_eq!(urls[1].url, "https://example.com/page-2");
    }

    #[test]
    fn test_parse_sitemap_index() {
        let index = r#"<?xml version="1.0" encoding="UTF-8"?>
<sitemapindex xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
  <sitemap>
    <loc>https://example.com/sitemap-news.xml</loc>
    <lastmod>2024-01-01</lastmod>
  </sitemap>
</sitemapindex>"#;

        let urls = parse_sitemap(index);
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].url, "https://example.com/sitemap-news.xml");
    }

    #[test]
    fn test_parse_sitemap_with_namespace() {
        // Some sitemaps use namespace prefixes on tags
        let sitemap = r#"<?xml version="1.0" encoding="UTF-8"?>
<ns:urlset xmlns:ns="http://www.sitemaps.org/schemas/sitemap/0.9">
  <ns:url>
    <ns:loc>https://example.com/ns-page-1</ns:loc>
    <ns:lastmod>2024-06-01</ns:lastmod>
  </ns:url>
  <ns:url>
    <ns:loc>https://example.com/ns-page-2</ns:loc>
  </ns:url>
</ns:urlset>"#;

        let urls = parse_sitemap(sitemap);
        assert_eq!(urls.len(), 2);
        assert_eq!(urls[0].url, "https://example.com/ns-page-1");
        assert!(urls[0].last_modified.is_some());
        assert_eq!(urls[1].url, "https://example.com/ns-page-2");
    }

    #[test]
    fn test_extract_links() {
        let html = r##"
<html>
<body>
  <a href="/article/1">Article 1</a>
  <a href="https://example.com/article/2">Article 2</a>
  <a href="https://other.com/page">External</a>
  <a href="#section">Anchor</a>
  <a href="javascript:void(0)">JS</a>
  <a href="/image.jpg">Image</a>
</body>
</html>"##;

        let links = extract_links(html, "https://example.com/");
        // Should include /article/1 and /article/2, skip external, anchor, js, image
        assert_eq!(links.len(), 2);
        assert!(links.iter().any(|l| l.url.contains("/article/1")));
        assert!(links.iter().any(|l| l.url.contains("/article/2")));
    }

    #[test]
    fn test_extract_cross_domain_links() {
        let html = r#"
<a href="https://other.com/page">External</a>
<a href="/internal">Internal</a>
<a href="https://another.org/article">Another</a>
"#;

        let links = extract_cross_domain_links(html, "https://example.com/");
        assert_eq!(links.len(), 2);
        assert!(links.iter().any(|l| l.url.contains("other.com")));
        assert!(links.iter().any(|l| l.url.contains("another.org")));
    }

    #[test]
    fn test_skip_non_content_urls() {
        assert!(should_skip_url("https://example.com/style.css"));
        assert!(should_skip_url("https://example.com/script.js"));
        assert!(should_skip_url("https://example.com/photo.jpg"));
        assert!(should_skip_url("https://example.com/wp-admin/edit"));
        assert!(should_skip_url("https://example.com/login"));
        assert!(should_skip_url("https://example.com/login/"));
        assert!(!should_skip_url("https://example.com/article/great-news"));
        // Should NOT skip URLs that merely contain the word as a substring
        assert!(!should_skip_url("https://example.com/blog/loginoverview"));
        assert!(!should_skip_url("https://example.com/article/admin-tips"));
    }
}
