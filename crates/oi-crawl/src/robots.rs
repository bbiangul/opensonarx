use std::collections::HashMap;

/// A single path rule: Allow or Disallow.
#[derive(Debug, Clone)]
struct PathRule {
    path: String,
    allowed: bool,
}

/// Parsed rules from a single user-agent group in robots.txt.
#[derive(Debug, Clone, Default)]
struct AgentGroup {
    rules: Vec<PathRule>,
    crawl_delay: Option<u64>,
}

/// Parsed rules from a robots.txt file (RFC 9309 compliant).
///
/// Supports:
/// - Agent-specific groups: if an `OpenSonarX` group exists, only those rules
///   apply (the `*` wildcard group is ignored per RFC 9309 Section 2.2.1).
/// - `Allow` and `Disallow` directives.
/// - Longest-match: the rule with the longest matching path prefix wins.
///   Ties between Allow and Disallow of the same length resolve to Allow.
#[derive(Debug, Clone, Default)]
pub struct RobotsRules {
    /// Backward-compatible list of disallowed paths (for callers that read this directly).
    pub disallowed: Vec<String>,
    pub crawl_delay: Option<u64>,
    /// Internal rules for longest-match evaluation.
    rules: Vec<PathRule>,
}

impl RobotsRules {
    /// Check if a path is allowed by these rules using longest-match.
    ///
    /// If no rule matches the path, it is allowed by default.
    /// If multiple rules match, the one with the longest path prefix wins.
    /// If Allow and Disallow have the same length, Allow wins (RFC 9309).
    pub fn is_path_allowed(&self, path: &str) -> bool {
        let mut best_len: usize = 0;
        let mut best_allowed = true; // default: allowed

        for rule in &self.rules {
            if path.starts_with(&rule.path) && rule.path.len() >= best_len {
                if rule.path.len() > best_len {
                    best_len = rule.path.len();
                    best_allowed = rule.allowed;
                } else if rule.allowed {
                    // Same length: Allow wins
                    best_allowed = true;
                }
            }
        }

        best_allowed
    }
}

/// Caches and checks robots.txt rules per domain.
#[derive(Debug, Default)]
pub struct RobotsChecker {
    cache: HashMap<String, RobotsRules>,
}

/// Parse a robots.txt file and extract rules applicable to our crawler.
///
/// Per RFC 9309:
/// - If a group for `OpenSonarX` exists, use ONLY that group (ignore `*`).
/// - Otherwise, use the `*` wildcard group.
/// - Supports both `Allow` and `Disallow` directives.
/// - Uses longest-match for path evaluation.
pub fn parse_robots_txt(text: &str) -> RobotsRules {
    // Parse all groups keyed by lowercase user-agent name.
    // Grouping: A User-agent: line following a rule starts a new group.
    // Multiple consecutive User-agent: lines form one group.
    // Blank lines and comments are ignored for grouping purposes.
    let mut groups: HashMap<String, AgentGroup> = HashMap::new();
    let mut current_agents: Vec<String> = Vec::new();
    let mut saw_rules = false; // whether we've seen rules in the current group

    for line in text.lines() {
        let line = line.trim();

        // Skip comments and blank lines (they don't affect grouping)
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let lower = line.to_lowercase();

        if lower.starts_with("user-agent:") {
            let agent = line["user-agent:".len()..].trim();
            let agent = agent.split('#').next().unwrap_or("").trim().to_lowercase();
            if !agent.is_empty() {
                // If we've seen rules, this User-agent: starts a new group
                if saw_rules {
                    current_agents.clear();
                    saw_rules = false;
                }
                current_agents.push(agent);
            }
        } else if !current_agents.is_empty() {
            if lower.starts_with("disallow:") {
                let path = line["disallow:".len()..].trim();
                let path = path.split('#').next().unwrap_or("").trim();
                if !path.is_empty() {
                    for agent in &current_agents {
                        let group = groups.entry(agent.clone()).or_default();
                        group.rules.push(PathRule {
                            path: path.to_string(),
                            allowed: false,
                        });
                    }
                }
                saw_rules = true;
            } else if lower.starts_with("allow:") {
                let path = line["allow:".len()..].trim();
                let path = path.split('#').next().unwrap_or("").trim();
                if !path.is_empty() {
                    for agent in &current_agents {
                        let group = groups.entry(agent.clone()).or_default();
                        group.rules.push(PathRule {
                            path: path.to_string(),
                            allowed: true,
                        });
                    }
                }
                saw_rules = true;
            } else if lower.starts_with("crawl-delay:") {
                let val = line["crawl-delay:".len()..].trim();
                let val = val.split('#').next().unwrap_or("").trim();
                if let Ok(delay) = val.parse::<u64>() {
                    for agent in &current_agents {
                        groups.entry(agent.clone()).or_default().crawl_delay = Some(delay);
                    }
                }
                saw_rules = true;
            }
        }
    }

    // RFC 9309: if agent-specific group exists, use it; otherwise use wildcard
    let chosen = groups.get("opensonarx").or_else(|| groups.get("*"));

    match chosen {
        Some(group) => {
            let disallowed = group
                .rules
                .iter()
                .filter(|r| !r.allowed)
                .map(|r| r.path.clone())
                .collect();
            RobotsRules {
                disallowed,
                crawl_delay: group.crawl_delay,
                rules: group.rules.clone(),
            }
        }
        None => RobotsRules::default(),
    }
}

impl RobotsChecker {
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
        }
    }

    /// Check whether rules for a domain are already cached.
    pub fn has_rules(&self, domain: &str) -> bool {
        self.cache.contains_key(domain)
    }

    /// Add parsed rules for a domain.
    pub fn add_rules(&mut self, domain: &str, rules: RobotsRules) {
        self.cache.insert(domain.to_string(), rules);
    }

    /// Check if a URL is allowed by robots.txt rules.
    /// Returns `true` if the URL is allowed (or no rules are cached for the domain).
    pub fn is_allowed(&self, url: &str) -> bool {
        let (domain, path) = match extract_domain_and_path(url) {
            Some(dp) => dp,
            None => return true, // Can't parse -> allow
        };

        let rules = match self.cache.get(&domain) {
            Some(r) => r,
            None => return true, // No rules cached -> allow
        };

        rules.is_path_allowed(&path)
    }

    /// Get the crawl delay for a domain (if any).
    pub fn crawl_delay(&self, domain: &str) -> Option<u64> {
        self.cache.get(domain).and_then(|r| r.crawl_delay)
    }
}

/// Extract (domain, path) from a URL string.
fn extract_domain_and_path(url: &str) -> Option<(String, String)> {
    // Strip scheme
    let after_scheme = url.split("://").nth(1)?;
    let slash_pos = after_scheme.find('/');
    match slash_pos {
        Some(pos) => {
            let domain = after_scheme[..pos].to_string();
            let path = after_scheme[pos..].to_string();
            Some((domain, path))
        }
        None => Some((after_scheme.to_string(), "/".to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_basic_robots_txt() {
        let txt = "\
User-agent: *
Disallow: /private/
Disallow: /admin/
Crawl-delay: 5
";
        let rules = parse_robots_txt(txt);
        assert_eq!(rules.disallowed, vec!["/private/", "/admin/"]);
        assert_eq!(rules.crawl_delay, Some(5));
    }

    #[test]
    fn test_is_allowed_disallowed_path() {
        let rules = parse_robots_txt("User-agent: *\nDisallow: /private/\nDisallow: /admin/\n");
        let mut checker = RobotsChecker::new();
        checker.add_rules("example.com", rules);

        assert!(!checker.is_allowed("https://example.com/private/secret"));
        assert!(!checker.is_allowed("https://example.com/admin/dashboard"));
    }

    #[test]
    fn test_is_allowed_allowed_path() {
        let rules = parse_robots_txt("User-agent: *\nDisallow: /private/\n");
        let mut checker = RobotsChecker::new();
        checker.add_rules("example.com", rules);

        assert!(checker.is_allowed("https://example.com/public/page"));
        assert!(checker.is_allowed("https://example.com/about"));
    }

    #[test]
    fn test_crawl_delay_parsing() {
        let rules = parse_robots_txt("User-agent: *\nCrawl-delay: 10\n");
        let mut checker = RobotsChecker::new();
        checker.add_rules("example.com", rules);

        assert_eq!(checker.crawl_delay("example.com"), Some(10));
        assert_eq!(checker.crawl_delay("other.com"), None);
    }

    #[test]
    fn test_opensonarx_user_agent() {
        let txt = "\
User-agent: Googlebot
Disallow: /google-only/

User-agent: OpenSonarX
Disallow: /no-index/
Crawl-delay: 2
";
        let rules = parse_robots_txt(txt);
        assert!(rules.disallowed.contains(&"/no-index/".to_string()));
        assert_eq!(rules.crawl_delay, Some(2));
        // Should NOT include Googlebot rules
        assert!(!rules.disallowed.contains(&"/google-only/".to_string()));
    }

    #[test]
    fn test_agent_specific_overrides_wildcard() {
        // RFC 9309: when agent-specific group exists, ignore wildcard
        let txt = "\
User-agent: *
Disallow: /everything/
Crawl-delay: 10

User-agent: OpenSonarX
Disallow: /specific-only/
Crawl-delay: 2
";
        let rules = parse_robots_txt(txt);
        // Should use OpenSonarX group, not wildcard
        assert_eq!(rules.disallowed, vec!["/specific-only/"]);
        assert!(!rules.disallowed.contains(&"/everything/".to_string()));
        assert_eq!(rules.crawl_delay, Some(2));
    }

    #[test]
    fn test_allow_directive() {
        let txt = "\
User-agent: *
Disallow: /private/
Allow: /private/public-page
";
        let rules = parse_robots_txt(txt);
        // /private/public-page should be allowed (longer match)
        assert!(rules.is_path_allowed("/private/public-page"));
        // /private/secret should still be disallowed
        assert!(!rules.is_path_allowed("/private/secret"));
    }

    #[test]
    fn test_longest_match_wins() {
        let txt = "\
User-agent: *
Disallow: /a
Allow: /a/b
Disallow: /a/b/c
";
        let rules = parse_robots_txt(txt);
        assert!(!rules.is_path_allowed("/a/x")); // matches /a (disallow)
        assert!(rules.is_path_allowed("/a/b/x")); // matches /a/b (allow)
        assert!(!rules.is_path_allowed("/a/b/c/d")); // matches /a/b/c (disallow)
    }

    #[test]
    fn test_same_length_allow_wins() {
        // RFC 9309: same-length rules — Allow wins
        let txt = "\
User-agent: *
Disallow: /test
Allow: /test
";
        let rules = parse_robots_txt(txt);
        assert!(rules.is_path_allowed("/test"));
    }

    #[test]
    fn test_no_rules_for_domain_allows() {
        let checker = RobotsChecker::new();
        assert!(checker.is_allowed("https://unknown.com/anything"));
    }

    #[test]
    fn test_comments_and_blank_lines() {
        let txt = "\
# This is a comment
User-agent: *

# Another comment
Disallow: /secret/ # inline comment
Crawl-delay: 3 # seconds
";
        let rules = parse_robots_txt(txt);
        assert_eq!(rules.disallowed, vec!["/secret/"]);
        assert_eq!(rules.crawl_delay, Some(3));
    }

    #[test]
    fn test_empty_disallow_is_skipped() {
        let txt = "User-agent: *\nDisallow:\n";
        let rules = parse_robots_txt(txt);
        assert!(rules.disallowed.is_empty());
    }

    #[test]
    fn test_url_without_path() {
        let rules = parse_robots_txt("User-agent: *\nDisallow: /blocked\n");
        let mut checker = RobotsChecker::new();
        checker.add_rules("example.com", rules);

        // URL with no trailing slash gets path "/"
        assert!(checker.is_allowed("https://example.com"));
    }

    #[test]
    fn test_has_rules() {
        let mut checker = RobotsChecker::new();
        assert!(!checker.has_rules("example.com"));
        checker.add_rules("example.com", RobotsRules::default());
        assert!(checker.has_rules("example.com"));
    }

    #[test]
    fn test_multiple_agents_same_group() {
        // Multiple User-agent lines before rules apply to all listed agents
        let txt = "\
User-agent: OpenSonarX
User-agent: Googlebot
Disallow: /shared-block/
";
        let rules = parse_robots_txt(txt);
        assert!(rules.disallowed.contains(&"/shared-block/".to_string()));
    }
}
