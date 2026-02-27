/// Known content-area selectors, checked in priority order.
const CONTENT_SELECTORS: &[&str] = &[
    "#handbook-content",   // Gatsby (TypeScript handbook)
    ".theme-doc-markdown", // Docusaurus
    ".markdown-body",      // GitHub
    "article",             // Semantic HTML
    "[role=\"article\"]",
    "[role=\"main\"]",
    "main",
    ".main-content-block", // TypeScript handbook layout
    ".doc-content",
    ".post-content",
    ".article-content",
    ".content",
    "#content",
];

/// Try to find a content container with enough text to be the main content area.
fn find_content_root(document: &scraper::Html) -> Option<ego_tree::NodeId> {
    use scraper::Selector;
    for sel_str in CONTENT_SELECTORS {
        if let Ok(sel) = Selector::parse(sel_str) {
            for el in document.select(&sel) {
                let text_len: usize = el.text().map(|t| t.trim().len()).sum();
                if text_len >= 200 {
                    return Some(el.id());
                }
            }
        }
    }
    None
}

/// Extract readable content from HTML and convert to Markdown.
/// Uses `scraper` (html5ever) for DOM-based parsing with boilerplate removal.
pub fn html_to_markdown(html: &str) -> ExtractedContent {
    use scraper::{Html, Selector};

    let document = Html::parse_document(html);

    // Extract title from <title> tag
    let title_sel = Selector::parse("title").unwrap();
    let title = document
        .select(&title_sel)
        .next()
        .map(|el| el.text().collect::<String>().trim().to_string())
        .filter(|t| !t.is_empty());

    // Collect IDs of elements to remove (boilerplate)
    let boilerplate_tags = [
        "nav", "aside", "footer", "header", "form", "script", "style", "noscript",
    ];

    // Wikipedia-specific and generic boilerplate class/id patterns
    let boilerplate_patterns = [
        "mw-navigation", "mw-panel", "mw-footer", "mw-header", "mw-jump-link",
        "navbox", "infobox", "noprint", "sidebar", "toc", "catlinks",
        "mw-editsection", "reference", "reflist", "mw-cite", "language-list",
        "cookie", "banner", "advertisement", "popup", "modal", "social-share",
        "breadcrumb", "skip-link", "sr-only",
        // Docusaurus
        "pagination-nav", "theme-doc-footer", "theme-doc-version-badge",
        "theme-doc-breadcrumbs", "table-of-contents", "docSidebarContainer",
        // Gatsby / TypeScript handbook
        "handbook-toc", "like-dislike-subnav", "edit-this-page",
        // Generic doc-site patterns
        "on-this-page", "doc-footer", "navigation-link",
    ];

    // Build a set of node IDs to skip
    let mut skip_ids = std::collections::HashSet::new();

    // Skip boilerplate tags
    for tag in &boilerplate_tags {
        if let Ok(sel) = Selector::parse(tag) {
            for el in document.select(&sel) {
                skip_ids.insert(el.id());
                // Also skip all descendants
                for desc in el.descendants() {
                    skip_ids.insert(desc.id());
                }
            }
        }
    }

    // Skip elements with boilerplate class/id patterns
    // We check all elements with class or id attributes
    // Use word-boundary matching: pattern must match a whole CSS class name or
    // appear as a prefix/substring delimited by hyphens in the id.
    // Never strip root structural elements (html, body, main, article) — only
    // containers like div, section, span, aside, etc.
    let root_tags: std::collections::HashSet<&str> =
        ["html", "body", "main", "article"].into_iter().collect();
    let all_sel = Selector::parse("[class], [id]").unwrap();
    for el in document.select(&all_sel) {
        let tag_name = el.value().name.local.as_ref();
        if root_tags.contains(tag_name) {
            continue;
        }

        let class_attr = el.value().attr("class").unwrap_or("");
        let id_attr = el.value().attr("id").unwrap_or("");

        let is_boilerplate = boilerplate_patterns.iter().any(|pattern| {
            // Check each CSS class individually (space-separated)
            class_attr.split_whitespace().any(|cls| {
                cls == *pattern
                    || cls.starts_with(&format!("{pattern}-"))
                    || cls.starts_with(&format!("{pattern}_")) // Docusaurus hash suffix
            }) || id_attr == *pattern
                || id_attr.starts_with(&format!("{pattern}-"))
                || id_attr.ends_with(&format!("-{pattern}"))
        });

        if is_boilerplate {
            skip_ids.insert(el.id());
            for desc in el.descendants() {
                skip_ids.insert(desc.id());
            }
        }
    }

    // Walk the DOM and convert to markdown, scoped to content root if found
    let mut markdown = String::new();
    let content_root = find_content_root(&document);
    if let Some(root_id) = content_root {
        if let Some(root_node) = document.tree.get(root_id) {
            for child in root_node.children() {
                walk_dom(child, &skip_ids, &mut markdown);
            }
        }
    } else {
        for child in document.tree.root().children() {
            walk_dom(child, &skip_ids, &mut markdown);
        }
    }

    let cleaned = collapse_newlines(&markdown);

    ExtractedContent {
        markdown: cleaned,
        title,
    }
}

/// Recursively walk the DOM tree and convert to markdown.
fn walk_dom(
    node_ref: ego_tree::NodeRef<'_, scraper::Node>,
    skip_ids: &std::collections::HashSet<ego_tree::NodeId>,
    out: &mut String,
) {
    use scraper::Node;

    if skip_ids.contains(&node_ref.id()) {
        return;
    }

    match node_ref.value() {
        Node::Text(text) => {
            let t = text.text.trim();
            if !t.is_empty() {
                // Normalize internal whitespace
                let normalized: String = t.split_whitespace().collect::<Vec<_>>().join(" ");
                if !out.is_empty() && !out.ends_with('\n') && !out.ends_with(' ') {
                    out.push(' ');
                }
                out.push_str(&normalized);
            }
        }
        Node::Element(el) => {
            let tag = el.name.local.as_ref();
            match tag {
                "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                    let level = tag[1..].parse::<usize>().unwrap_or(1);
                    out.push('\n');
                    for _ in 0..level {
                        out.push('#');
                    }
                    out.push(' ');
                    for child in node_ref.children() {
                        walk_dom(child, skip_ids, out);
                    }
                    out.push('\n');
                    return; // children already processed
                }
                "p" => {
                    out.push_str("\n\n");
                    for child in node_ref.children() {
                        walk_dom(child, skip_ids, out);
                    }
                    out.push_str("\n\n");
                    return;
                }
                "br" => {
                    out.push('\n');
                }
                "li" => {
                    out.push_str("\n- ");
                    for child in node_ref.children() {
                        walk_dom(child, skip_ids, out);
                    }
                    return;
                }
                "blockquote" => {
                    out.push_str("\n> ");
                    for child in node_ref.children() {
                        walk_dom(child, skip_ids, out);
                    }
                    out.push('\n');
                    return;
                }
                "pre" => {
                    out.push_str("\n```\n");
                    for child in node_ref.children() {
                        walk_dom(child, skip_ids, out);
                    }
                    out.push_str("\n```\n");
                    return;
                }
                "code" => {
                    // Only inline code (not inside <pre>)
                    if let Some(parent) = node_ref.parent()
                        && let scraper::Node::Element(parent_el) = parent.value()
                        && parent_el.name.local.as_ref() == "pre"
                    {
                        // Inside pre, just emit text directly
                        for child in node_ref.children() {
                            walk_dom(child, skip_ids, out);
                        }
                        return;
                    }
                    out.push('`');
                    for child in node_ref.children() {
                        walk_dom(child, skip_ids, out);
                    }
                    out.push('`');
                    return;
                }
                _ => {}
            }
        }
        _ => {}
    }

    // Recurse into children for all other nodes
    for child in node_ref.children() {
        walk_dom(child, skip_ids, out);
    }
}

/// Decode common HTML entities.
/// Kept for potential use by date extraction functions operating on raw HTML.
#[allow(dead_code)]
fn decode_entities(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '&' {
            let mut entity = String::new();
            for ec in chars.by_ref() {
                if ec == ';' {
                    break;
                }
                entity.push(ec);
                if entity.len() > 10 {
                    // Not a real entity, emit as-is
                    result.push('&');
                    result.push_str(&entity);
                    entity.clear();
                    break;
                }
            }
            if entity.is_empty() {
                continue;
            }
            match entity.as_str() {
                "amp" => result.push('&'),
                "lt" => result.push('<'),
                "gt" => result.push('>'),
                "quot" => result.push('"'),
                "apos" => result.push('\''),
                "nbsp" => result.push(' '),
                "mdash" => result.push('\u{2014}'),
                "ndash" => result.push('\u{2013}'),
                "hellip" => result.push_str("..."),
                "laquo" => result.push('\u{00AB}'),
                "raquo" => result.push('\u{00BB}'),
                "copy" => result.push('\u{00A9}'),
                other => {
                    // Try numeric entity: &#123; or &#x1F;
                    if let Some(stripped) = other.strip_prefix('#') {
                        let code = if let Some(hex) = stripped.strip_prefix('x') {
                            u32::from_str_radix(hex, 16).ok()
                        } else {
                            stripped.parse::<u32>().ok()
                        };
                        if let Some(c) = code.and_then(char::from_u32) {
                            result.push(c);
                        } else {
                            result.push('&');
                            result.push_str(other);
                            result.push(';');
                        }
                    } else {
                        // Unknown entity, emit as-is
                        result.push('&');
                        result.push_str(other);
                        result.push(';');
                    }
                }
            }
        } else {
            result.push(ch);
        }
    }

    result
}

fn collapse_newlines(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut newline_count = 0;
    for ch in s.chars() {
        if ch == '\n' {
            newline_count += 1;
            if newline_count <= 2 {
                result.push(ch);
            }
        } else {
            newline_count = 0;
            result.push(ch);
        }
    }
    result.trim().to_string()
}

pub struct ExtractedContent {
    pub markdown: String,
    pub title: Option<String>,
}

/// Extract a publish date from HTML using common meta tags and JSON-LD.
///
/// Checks (in order):
/// 1. `<meta property="article:published_time" content="...">`
/// 2. `<meta name="date" content="...">`
/// 3. `<meta name="DC.date" content="...">`
/// 4. `<time datetime="...">` (first occurrence)
/// 5. JSON-LD `"datePublished"` in `<script type="application/ld+json">` blocks
///
/// Returns the first found date string (ISO 8601 or whatever the source provides).
pub fn extract_publish_date(html: &str) -> Option<String> {
    // Strategy: scan for specific patterns without a full HTML parser.
    let html_lower = html.to_lowercase();

    // 1. <meta property="article:published_time" content="...">
    if let Some(date) = extract_meta_content(&html_lower, html, "property", "article:published_time") {
        return Some(date);
    }

    // 2. <meta name="date" content="...">
    if let Some(date) = extract_meta_content(&html_lower, html, "name", "date") {
        return Some(date);
    }

    // 3. <meta name="DC.date" content="...">
    if let Some(date) = extract_meta_content(&html_lower, html, "name", "dc.date") {
        return Some(date);
    }

    // 4. <time datetime="...">
    if let Some(date) = extract_time_datetime(&html_lower, html) {
        return Some(date);
    }

    // 5. JSON-LD datePublished
    if let Some(date) = extract_jsonld_date(html) {
        return Some(date);
    }

    None
}

/// Extract content attribute from a <meta> tag matching a given attr/value pair.
/// `html_lower` is the lowercased HTML, `html_orig` is the original (for extracting exact values).
fn extract_meta_content(html_lower: &str, html_orig: &str, attr: &str, value: &str) -> Option<String> {
    // Search for meta tags containing the attribute
    let search_attr = format!("{attr}=\"{value}\"");
    let mut search_pos = 0;

    while let Some(attr_pos) = html_lower[search_pos..].find(&search_attr) {
        let abs_pos = search_pos + attr_pos;

        // Find the enclosing <meta ...> tag
        let tag_start = html_lower[..abs_pos].rfind('<')?;
        let tag_end = html_lower[abs_pos..].find('>')? + abs_pos;
        let tag = &html_orig[tag_start..=tag_end];
        let tag_lower = &html_lower[tag_start..=tag_end];

        // Verify this is a <meta tag
        if tag_lower.trim_start_matches('<').trim_start().starts_with("meta") {
            // Extract content="..."
            if let Some(content) = extract_attr_value(tag, tag_lower, "content") {
                let trimmed = content.trim().to_string();
                if !trimmed.is_empty() {
                    return Some(trimmed);
                }
            }
        }

        search_pos = abs_pos + search_attr.len();
    }

    None
}

/// Extract the value of an attribute from a tag string.
fn extract_attr_value(tag: &str, tag_lower: &str, attr_name: &str) -> Option<String> {
    let search = format!("{attr_name}=\"");
    let pos = tag_lower.find(&search)?;
    let start = pos + search.len();
    let end = tag[start..].find('"')? + start;
    Some(tag[start..end].to_string())
}

/// Extract datetime from the first <time datetime="..."> tag.
fn extract_time_datetime(html_lower: &str, html_orig: &str) -> Option<String> {
    let marker = "datetime=\"";
    let pos = html_lower.find(marker)?;

    // Make sure it's inside a <time tag
    let tag_start = html_lower[..pos].rfind('<')?;
    let tag_name_region = &html_lower[tag_start..pos];
    if !tag_name_region.trim_start_matches('<').trim_start().starts_with("time") {
        return None;
    }

    let val_start = pos + marker.len();
    let val_end = html_orig[val_start..].find('"')? + val_start;
    let value = html_orig[val_start..val_end].trim().to_string();
    if value.is_empty() {
        return None;
    }
    Some(value)
}

/// Extract datePublished from JSON-LD script blocks.
fn extract_jsonld_date(html: &str) -> Option<String> {
    let html_lower = html.to_lowercase();
    let marker = "application/ld+json";
    let mut search_pos = 0;

    while let Some(type_pos) = html_lower[search_pos..].find(marker) {
        let abs_pos = search_pos + type_pos;

        // Find </script> after this point
        let script_end_lower = "</script>";
        let end_pos = html_lower[abs_pos..].find(script_end_lower)?;
        let block_end = abs_pos + end_pos;

        // Find the start of actual JSON content (after the >)
        let content_start = html[abs_pos..].find('>')? + abs_pos + 1;
        if content_start < block_end {
            let json_str = html[content_start..block_end].trim();

            // Try to parse as JSON and look for datePublished
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(json_str)
                && let Some(date) = find_date_published(&value)
            {
                return Some(date);
            }
        }

        search_pos = block_end;
    }

    None
}

/// Recursively find "datePublished" in a JSON value.
fn find_date_published(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(serde_json::Value::String(date)) = map.get("datePublished") {
                return Some(date.clone());
            }
            // Recurse into nested objects/arrays
            for v in map.values() {
                if let Some(date) = find_date_published(v) {
                    return Some(date);
                }
            }
            None
        }
        serde_json::Value::Array(arr) => {
            for v in arr {
                if let Some(date) = find_date_published(v) {
                    return Some(date);
                }
            }
            None
        }
        _ => None,
    }
}

/// Try to parse a date string to a Unix timestamp (seconds since epoch).
/// Supports common ISO 8601 formats: YYYY-MM-DD and YYYY-MM-DDTHH:MM:SS variants.
pub fn parse_date_to_timestamp(date_str: &str) -> Option<i64> {
    // Try YYYY-MM-DDTHH:MM:SSZ or YYYY-MM-DDTHH:MM:SS+00:00 style
    // Simple parser for common cases without pulling in chrono
    let s = date_str.trim();

    // Extract date portion (YYYY-MM-DD)
    if s.len() < 10 {
        return None;
    }

    let year: i64 = s[0..4].parse().ok()?;
    if s.as_bytes().get(4)? != &b'-' {
        return None;
    }
    let month: i64 = s[5..7].parse().ok()?;
    if s.as_bytes().get(7)? != &b'-' {
        return None;
    }
    let day: i64 = s[8..10].parse().ok()?;

    // Validate ranges
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || year < 1970 {
        return None;
    }

    // Extract time if present
    let (hour, minute, second) = if s.len() > 10 && (s.as_bytes()[10] == b'T' || s.as_bytes()[10] == b' ') {
        let time_part = &s[11..];
        if time_part.len() >= 8 {
            let h: i64 = time_part[0..2].parse().ok()?;
            let m: i64 = time_part[3..5].parse().ok()?;
            let sec: i64 = time_part[6..8].parse().ok()?;
            (h, m, sec)
        } else {
            (0, 0, 0)
        }
    } else {
        (0, 0, 0)
    };

    // Convert to unix timestamp (approximate, ignoring leap seconds)
    // Days from epoch (1970-01-01) to the given date
    let days = days_from_epoch(year, month, day)?;
    let timestamp = days * 86400 + hour * 3600 + minute * 60 + second;

    Some(timestamp)
}

/// Calculate days from 1970-01-01 to the given date.
fn days_from_epoch(year: i64, month: i64, day: i64) -> Option<i64> {
    // Adjust for months (using the algorithm from Howard Hinnant)
    let (y, m) = if month <= 2 {
        (year - 1, month + 9)
    } else {
        (year, month - 3)
    };

    let era = y / 400;
    let yoe = y - era * 400;
    let doy = (153 * m + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;

    Some(days)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_html() {
        let html = "<html><body><h1>Hello</h1><p>World</p></body></html>";
        let result = html_to_markdown(html);
        assert!(result.markdown.contains("# Hello"));
        assert!(result.markdown.contains("World"));
    }

    #[test]
    fn test_strips_script() {
        let html = "<p>visible</p><script>alert('xss')</script><p>also visible</p>";
        let result = html_to_markdown(html);
        assert!(result.markdown.contains("visible"));
        assert!(!result.markdown.contains("alert"));
    }

    #[test]
    fn test_extracts_title() {
        let html = "<html><head><title>My Page</title></head><body>Content</body></html>";
        let result = html_to_markdown(html);
        assert_eq!(result.title, Some("My Page".to_string()));
    }

    #[test]
    fn test_list_items() {
        let html = "<ul><li>One</li><li>Two</li><li>Three</li></ul>";
        let result = html_to_markdown(html);
        assert!(result.markdown.contains("- One"));
        assert!(result.markdown.contains("- Two"));
    }

    #[test]
    fn test_entity_decoding() {
        let html = "<p>Tom &amp; Jerry &lt;3 &gt; all</p>";
        let result = html_to_markdown(html);
        assert!(result.markdown.contains("Tom & Jerry <3 > all"));
    }

    #[test]
    fn test_numeric_entity_decoding() {
        let html = "<p>&#169; 2024 &#x2014; dash</p>";
        let result = html_to_markdown(html);
        assert!(result.markdown.contains("\u{00A9} 2024 \u{2014} dash"));
    }

    #[test]
    fn test_nbsp_decoding() {
        let html = "<p>hello&nbsp;world</p>";
        let result = html_to_markdown(html);
        assert!(result.markdown.contains("hello world"));
    }

    #[test]
    fn test_title_with_entities() {
        let html = "<html><head><title>Tom &amp; Jerry</title></head><body>x</body></html>";
        let result = html_to_markdown(html);
        assert_eq!(result.title, Some("Tom & Jerry".to_string()));
    }

    // ── Publish date extraction tests ──

    #[test]
    fn test_extract_article_published_time() {
        let html = r#"<html><head>
            <meta property="article:published_time" content="2024-01-15T10:30:00Z">
        </head><body>Content</body></html>"#;
        let date = extract_publish_date(html);
        assert_eq!(date, Some("2024-01-15T10:30:00Z".to_string()));
    }

    #[test]
    fn test_extract_meta_name_date() {
        let html = r#"<html><head>
            <meta name="date" content="2023-12-25">
        </head><body>Content</body></html>"#;
        let date = extract_publish_date(html);
        assert_eq!(date, Some("2023-12-25".to_string()));
    }

    #[test]
    fn test_extract_dc_date() {
        let html = r#"<html><head>
            <meta name="DC.date" content="2023-06-01">
        </head><body>Content</body></html>"#;
        let date = extract_publish_date(html);
        assert_eq!(date, Some("2023-06-01".to_string()));
    }

    #[test]
    fn test_extract_time_datetime() {
        let html = r#"<html><body>
            <article>
                <time datetime="2024-03-20T14:00:00Z">March 20, 2024</time>
                <p>Article content</p>
            </article>
        </body></html>"#;
        let date = extract_publish_date(html);
        assert_eq!(date, Some("2024-03-20T14:00:00Z".to_string()));
    }

    #[test]
    fn test_extract_jsonld_date_published() {
        let html = r#"<html><head>
            <script type="application/ld+json">
            {
                "@type": "Article",
                "headline": "Test Article",
                "datePublished": "2024-02-14T09:00:00Z",
                "author": "Test Author"
            }
            </script>
        </head><body>Content</body></html>"#;
        let date = extract_publish_date(html);
        assert_eq!(date, Some("2024-02-14T09:00:00Z".to_string()));
    }

    #[test]
    fn test_extract_jsonld_nested() {
        let html = r#"<html><head>
            <script type="application/ld+json">
            {
                "@context": "https://schema.org",
                "@graph": [
                    {
                        "@type": "WebPage",
                        "datePublished": "2024-05-01"
                    }
                ]
            }
            </script>
        </head><body>Content</body></html>"#;
        let date = extract_publish_date(html);
        assert_eq!(date, Some("2024-05-01".to_string()));
    }

    #[test]
    fn test_extract_publish_date_priority() {
        // article:published_time should take priority over others
        let html = r#"<html><head>
            <meta property="article:published_time" content="2024-01-01">
            <meta name="date" content="2024-02-02">
        </head><body>
            <time datetime="2024-03-03">March 3</time>
        </body></html>"#;
        let date = extract_publish_date(html);
        assert_eq!(date, Some("2024-01-01".to_string()));
    }

    #[test]
    fn test_extract_publish_date_none() {
        let html = "<html><head></head><body><p>No date here</p></body></html>";
        let date = extract_publish_date(html);
        assert_eq!(date, None);
    }

    #[test]
    fn test_parse_date_to_timestamp_date_only() {
        // 2024-01-01 00:00:00 UTC
        let ts = parse_date_to_timestamp("2024-01-01").unwrap();
        assert_eq!(ts, 1704067200);
    }

    #[test]
    fn test_parse_date_to_timestamp_with_time() {
        // 2024-01-01 12:00:00 UTC
        let ts = parse_date_to_timestamp("2024-01-01T12:00:00Z").unwrap();
        assert_eq!(ts, 1704110400);
    }

    #[test]
    fn test_parse_date_to_timestamp_invalid() {
        assert_eq!(parse_date_to_timestamp("not a date"), None);
        assert_eq!(parse_date_to_timestamp(""), None);
    }

    // ── Boilerplate removal tests ──

    #[test]
    fn test_strips_nav_element() {
        let html = r#"<html><body>
            <nav><ul><li>Home</li><li>About</li></ul></nav>
            <p>Article content here</p>
        </body></html>"#;
        let result = html_to_markdown(html);
        assert!(result.markdown.contains("Article content here"));
        assert!(!result.markdown.contains("Home"));
        assert!(!result.markdown.contains("About"));
    }

    #[test]
    fn test_strips_aside_sidebar() {
        let html = r#"<html><body>
            <aside><h3>Related Articles</h3><p>Some sidebar content</p></aside>
            <p>Main article text</p>
        </body></html>"#;
        let result = html_to_markdown(html);
        assert!(result.markdown.contains("Main article text"));
        assert!(!result.markdown.contains("Related Articles"));
        assert!(!result.markdown.contains("sidebar content"));
    }

    #[test]
    fn test_strips_footer() {
        let html = r#"<html><body>
            <p>Page content</p>
            <footer><p>Copyright 2024</p><a>Privacy Policy</a></footer>
        </body></html>"#;
        let result = html_to_markdown(html);
        assert!(result.markdown.contains("Page content"));
        assert!(!result.markdown.contains("Copyright"));
        assert!(!result.markdown.contains("Privacy Policy"));
    }

    #[test]
    fn test_strips_elements_by_class() {
        let html = r#"<html><body>
            <div class="mw-navigation"><ul><li>Main page</li><li>Contents</li></ul></div>
            <div class="content"><p>Encyclopedia article text</p></div>
        </body></html>"#;
        let result = html_to_markdown(html);
        assert!(result.markdown.contains("Encyclopedia article text"));
        assert!(!result.markdown.contains("Main page"));
        assert!(!result.markdown.contains("Contents"));
    }

    #[test]
    fn test_strips_elements_by_id() {
        let html = r#"<html><body>
            <div id="toc"><h2>Table of Contents</h2><ul><li>Section 1</li></ul></div>
            <p>Actual article body</p>
        </body></html>"#;
        let result = html_to_markdown(html);
        assert!(result.markdown.contains("Actual article body"));
        assert!(!result.markdown.contains("Table of Contents"));
    }

    #[test]
    fn test_wikipedia_article_structure() {
        let html = r#"<html><body>
            <div id="mw-navigation"><nav><ul><li>Main page</li><li>Random</li></ul></nav></div>
            <div class="mw-panel"><div class="portal"><h3>Navigation</h3></div></div>
            <div id="content">
                <h1>Test Article</h1>
                <p>This is the main article content about an important topic.</p>
                <div class="navbox"><table><tr><td>Nav links</td></tr></table></div>
                <div class="catlinks"><ul><li>Category:Test</li></ul></div>
                <span class="mw-editsection">[edit]</span>
            </div>
            <div id="mw-footer"><ul><li>Privacy</li><li>Terms</li></ul></div>
        </body></html>"#;
        let result = html_to_markdown(html);
        assert!(result.markdown.contains("Test Article"));
        assert!(result.markdown.contains("important topic"));
        assert!(!result.markdown.contains("Main page"));
        assert!(!result.markdown.contains("Random"));
        assert!(!result.markdown.contains("Navigation"));
        assert!(!result.markdown.contains("Nav links"));
        assert!(!result.markdown.contains("Category:Test"));
        assert!(!result.markdown.contains("[edit]"));
        assert!(!result.markdown.contains("Privacy"));
    }

    #[test]
    fn test_preserves_article_content() {
        let html = r#"<html><body>
            <article>
                <h1>Main Heading</h1>
                <p>First paragraph of the article.</p>
                <h2>Subheading</h2>
                <p>Second paragraph with more detail.</p>
            </article>
            <main>
                <p>Content in main element.</p>
            </main>
        </body></html>"#;
        let result = html_to_markdown(html);
        assert!(result.markdown.contains("# Main Heading"));
        assert!(result.markdown.contains("First paragraph"));
        assert!(result.markdown.contains("## Subheading"));
        assert!(result.markdown.contains("Second paragraph"));
        assert!(result.markdown.contains("Content in main element"));
    }

    #[test]
    fn test_real_wikipedia_structure() {
        // Matches actual Wikipedia 2024 HTML structure (vector skin)
        let html = r##"<html>
        <head><title>Microsoft - Wikipedia</title></head>
        <body class="skin-vector-2022">
            <div class="vector-header-container">
                <header class="vector-header mw-header">
                    <div class="vector-header-start">
                        <nav>Main menu content</nav>
                    </div>
                    <div id="p-search" class="vector-search-box-vue">Search box</div>
                </header>
            </div>
            <main id="content" class="mw-body">
                <header class="mw-body-header">
                    <h1 id="firstHeading">Microsoft</h1>
                </header>
                <div id="bodyContent" class="vector-body">
                    <div id="mw-content-text" class="mw-body-content">
                        <div class="mw-parser-output">
                            <table class="infobox"><tr><td>Infobox data</td></tr></table>
                            <p><b>Microsoft Corporation</b> is an American multinational technology conglomerate.</p>
                            <p>Founded in 1975 by Bill Gates and Paul Allen.</p>
                            <div class="toc" id="toc"><h2>Contents</h2></div>
                            <h2>History</h2>
                            <p>Microsoft was founded on April 4, 1975.</p>
                            <sup class="reference"><a href="#cite">[1]</a></sup>
                            <div class="reflist">References list here</div>
                            <div class="navbox">Navigation box content</div>
                        </div>
                    </div>
                </div>
            </main>
            <div id="mw-navigation"><nav>Site navigation</nav></div>
            <footer id="footer" class="mw-footer">Footer content</footer>
        </body>
        </html>"##;
        let result = html_to_markdown(html);
        eprintln!("Markdown ({} chars): {:?}", result.markdown.len(), &result.markdown);
        assert!(result.markdown.contains("Microsoft Corporation"), "Should have article text");
        assert!(result.markdown.contains("Founded in 1975"), "Should have second paragraph");
        assert!(result.markdown.contains("## History"), "Should have section headings");
        assert!(!result.markdown.contains("Site navigation"), "Should strip nav");
        assert!(!result.markdown.contains("Footer content"), "Should strip footer");
        assert!(!result.markdown.contains("Navigation box"), "Should strip navbox");
        assert!(!result.markdown.contains("Contents"), "Should strip toc");
    }

    #[test]
    fn test_mdbook_structure() {
        // Matches actual mdBook (Rust Book) HTML structure
        let html = r##"<html class="light sidebar-visible">
        <body>
        <div id="body-container">
            <nav id="sidebar" class="sidebar" aria-label="Table of contents">
                <div class="sidebar-scrollbox">Chapter list here</div>
            </nav>
            <div id="page-wrapper" class="page-wrapper">
                <div class="page">
                    <div id="menu-bar" class="menu-bar sticky">
                        <div class="left-buttons">
                            <label id="sidebar-toggle" for="sidebar-toggle-anchor">Toggle</label>
                        </div>
                    </div>
                    <div id="content" class="content">
                        <main>
                            <h1>What is Ownership?</h1>
                            <p>Ownership is a set of rules that govern how a Rust program manages memory.</p>
                            <h2>The Stack and the Heap</h2>
                            <p>Both the stack and the heap are parts of memory available to your code.</p>
                        </main>
                    </div>
                    <nav class="nav-wrapper" aria-label="Page navigation">
                        <a class="nav-chapters previous" href="ch03.html">Previous</a>
                        <a class="nav-chapters next" href="ch04-02.html">Next</a>
                    </nav>
                </div>
            </div>
        </div>
        </body></html>"##;
        let result = html_to_markdown(html);
        eprintln!("mdBook markdown ({} chars): {:?}", result.markdown.len(), &result.markdown);
        assert!(result.markdown.contains("Ownership"), "Should preserve main content");
        assert!(result.markdown.contains("Stack and the Heap"), "Should preserve headings");
        assert!(!result.markdown.contains("Chapter list"), "Should strip sidebar nav");
        assert!(!result.markdown.contains("Previous"), "Should strip page nav");
    }

    #[test]
    fn test_blockquote_conversion() {
        let html = r#"<html><body>
            <blockquote>This is a quoted passage from a notable source.</blockquote>
            <p>Regular text follows.</p>
        </body></html>"#;
        let result = html_to_markdown(html);
        assert!(result.markdown.contains("> This is a quoted passage"));
        assert!(result.markdown.contains("Regular text follows"));
    }

    // ── Content-area prioritization and Docusaurus/Gatsby tests ──

    #[test]
    fn test_docusaurus_structure() {
        let html = r##"<html>
        <head><title>Getting Started | My Docs</title></head>
        <body>
            <nav class="navbar">
                <a href="/">Home</a>
                <a href="/docs">Docs</a>
            </nav>
            <div class="docSidebarContainer_EfwR">
                <aside class="theme-doc-sidebar-container">
                    <ul><li>Introduction</li><li>Installation</li></ul>
                </aside>
            </div>
            <main>
                <div class="theme-doc-version-badge badge">Version: 2.0</div>
                <div class="theme-doc-breadcrumbs"><a>Docs</a> &gt; Getting Started</div>
                <article>
                    <div class="theme-doc-markdown markdown">
                        <h1>Getting Started</h1>
                        <p>Welcome to the documentation. This guide will help you get up and running with our framework quickly and efficiently. We cover installation, configuration, and basic usage patterns that every developer should know about.</p>
                        <h2>Installation</h2>
                        <p>Run the following command to install the package. Make sure you have Node.js version 16 or higher installed on your system before proceeding with the installation step.</p>
                        <pre><code>npm install my-framework</code></pre>
                    </div>
                </article>
                <div class="theme-doc-footer">
                    <div class="theme-doc-footer-edit-this-page">
                        <a href="https://github.com/edit">Edit this page</a>
                    </div>
                </div>
                <nav class="pagination-nav">
                    <a class="pagination-nav__link--prev" href="/intro">Previous: Intro</a>
                    <a class="pagination-nav__link--next" href="/config">Next: Configuration</a>
                </nav>
            </main>
        </body></html>"##;
        let result = html_to_markdown(html);
        eprintln!("Docusaurus markdown ({} chars): {:?}", result.markdown.len(), &result.markdown);
        // Content preserved
        assert!(result.markdown.contains("Getting Started"), "Should preserve heading");
        assert!(result.markdown.contains("npm install my-framework"), "Should preserve code");
        assert!(result.markdown.contains("Installation"), "Should preserve subheading");
        // Boilerplate stripped
        assert!(!result.markdown.contains("Introduction"), "Should strip sidebar items");
        assert!(!result.markdown.contains("Version: 2.0"), "Should strip version badge");
        assert!(!result.markdown.contains("Edit this page"), "Should strip edit link");
        assert!(!result.markdown.contains("Previous: Intro"), "Should strip pagination");
        assert!(!result.markdown.contains("Next: Configuration"), "Should strip pagination");
    }

    #[test]
    fn test_gatsby_typescript_handbook() {
        let html = r##"<html>
        <head><title>TypeScript: Narrowing</title></head>
        <body>
            <header>
                <nav><a href="/">TypeScript</a><a href="/docs">Docs</a></nav>
            </header>
            <div id="sidebar">
                <ul>
                    <li>The Basics</li>
                    <li>Everyday Types</li>
                    <li>Narrowing</li>
                </ul>
            </div>
            <div id="handbook-content">
                <h1>Narrowing</h1>
                <p>If we look at the padLeft function from earlier, we can see that TypeScript is able to narrow types by analyzing the control flow of the program. This is called narrowing, and it allows TypeScript to provide more precise type information at different points in your code.</p>
                <h2>typeof type guards</h2>
                <p>JavaScript supports a typeof operator which can give very basic information about the type of values we have at runtime. TypeScript expects this to return a certain set of strings and uses them to narrow types.</p>
                <pre><code>function padLeft(padding: number | string, input: string) {
  if (typeof padding === "number") {
    return " ".repeat(padding) + input;
  }
  return padding + input;
}</code></pre>
            </div>
            <div id="like-dislike-subnav">
                <button>Like</button>
                <button>Dislike</button>
            </div>
            <div class="navigation-link">
                <a href="/handbook/everyday-types">Previous: Everyday Types</a>
                <a href="/handbook/functions">Next: More on Functions</a>
            </div>
            <footer>
                <p>Made with love by the TypeScript team</p>
            </footer>
        </body></html>"##;
        let result = html_to_markdown(html);
        eprintln!("Gatsby handbook ({} chars): {:?}", result.markdown.len(), &result.markdown);
        // Content preserved
        assert!(result.markdown.contains("Narrowing"), "Should preserve heading");
        assert!(result.markdown.contains("typeof type guards"), "Should preserve subheading");
        assert!(result.markdown.contains("padLeft"), "Should preserve code block");
        assert!(result.markdown.contains("control flow"), "Should preserve body text");
        // Boilerplate stripped
        assert!(!result.markdown.contains("The Basics"), "Should strip sidebar");
        assert!(!result.markdown.contains("Everyday Types"), "Should strip sidebar/nav links");
        assert!(!result.markdown.contains("Like"), "Should strip like/dislike");
        assert!(!result.markdown.contains("Dislike"), "Should strip like/dislike");
        assert!(!result.markdown.contains("Made with love"), "Should strip footer");
    }

    #[test]
    fn test_content_area_fallback() {
        // Page with no known content selectors — should fall back to full-document extraction
        let html = r#"<html><body>
            <div class="wrapper">
                <h1>Simple Page</h1>
                <p>This is a page without any standard content containers.</p>
                <p>All content should still be extracted via the fallback path.</p>
            </div>
        </body></html>"#;
        let result = html_to_markdown(html);
        assert!(result.markdown.contains("Simple Page"), "Should preserve heading");
        assert!(result.markdown.contains("without any standard content containers"), "Should preserve body");
        assert!(result.markdown.contains("fallback path"), "Should preserve all text");
    }
}
