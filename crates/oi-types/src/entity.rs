use serde::Serialize;

/// Known UGC (User-Generated Content) platforms where domain-level staking
/// provides zero passthrough. Only entity-level stakes count.
const UGC_PLATFORMS: &[&str] = &[
    "youtube.com",
    "reddit.com",
    "x.com",
    "linkedin.com",
    "github.com",
    "medium.com",
    "tiktok.com",
    "instagram.com",
    "facebook.com",
];

/// A reference to a stakeable entity — either a domain or a sub-path within a domain.
///
/// Examples:
/// - `"example.com"` → domain-only (backward compat)
/// - `"youtube.com/c/3blue1brown"` → YouTube channel entity
/// - `"reddit.com/r/rust"` → subreddit entity
/// - `"x.com/elonmusk"` → X/Twitter profile
///
/// Fields are private to enforce invariants. Use accessor methods or [`EntityRef::parse`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct EntityRef {
    domain: String,
    entity_path: Option<String>,
}

impl EntityRef {
    /// Parse a raw entity string into an `EntityRef`.
    ///
    /// Strips protocol prefix if present, lowercases, validates characters,
    /// and normalizes platform aliases (e.g., twitter.com → x.com, www. stripped).
    pub fn parse(raw: &str) -> Result<Self, EntityParseError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(EntityParseError::Empty);
        }

        // Strip protocol prefix
        let without_proto = trimmed
            .strip_prefix("https://")
            .or_else(|| trimmed.strip_prefix("http://"))
            .unwrap_or(trimmed);

        // Strip trailing slash
        let without_trailing = without_proto.trim_end_matches('/');

        // Split into domain and path
        let (domain_raw, path_raw) = match without_trailing.find('/') {
            Some(pos) => (&without_trailing[..pos], Some(&without_trailing[pos + 1..])),
            None => (without_trailing, None),
        };

        let domain = domain_raw.to_lowercase();

        // Validate domain characters (a-z, 0-9, '.', '-')
        if !domain.chars().all(is_domain_char) {
            return Err(EntityParseError::InvalidChars);
        }

        // Validate domain structure
        if !domain.contains('.') {
            return Err(EntityParseError::NoDot);
        }
        if domain.len() > 253 {
            return Err(EntityParseError::TooLong);
        }
        // Reject leading/trailing dot or dash
        if domain.starts_with('.') || domain.ends_with('.')
            || domain.starts_with('-') || domain.ends_with('-')
        {
            return Err(EntityParseError::InvalidChars);
        }

        // Validate path characters
        if let Some(path) = path_raw
            && !path.is_empty() && !path.chars().all(is_path_char)
        {
            return Err(EntityParseError::InvalidChars);
        }

        // Normalize platform aliases (www. stripping, twitter→x)
        let domain = normalize_domain(&domain);

        // Build entity path
        let entity_path = path_raw
            .filter(|p| !p.is_empty())
            .map(|p| normalize_path(&domain, p));

        Ok(EntityRef { domain, entity_path })
    }

    /// Canonical string representation (lowercased, normalized).
    pub fn canonical(&self) -> String {
        match &self.entity_path {
            Some(path) => format!("{}/{}", self.domain, path),
            None => self.domain.clone(),
        }
    }

    /// Whether this is a domain-only reference (no sub-path).
    pub fn is_domain_only(&self) -> bool {
        self.entity_path.is_none()
    }

    /// The domain portion.
    pub fn domain(&self) -> &str {
        &self.domain
    }

    /// The entity sub-path, if any.
    pub fn entity_path(&self) -> Option<&str> {
        self.entity_path.as_deref()
    }

    /// Check if this entity's domain is a known UGC platform.
    pub fn is_ugc_platform(&self) -> bool {
        UGC_PLATFORMS.contains(&self.domain.as_str())
    }
}

impl std::fmt::Display for EntityRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.canonical())
    }
}

/// Custom deserialize that validates through `parse()` to enforce invariants.
impl<'de> serde::Deserialize<'de> for EntityRef {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        struct Raw {
            domain: String,
            entity_path: Option<String>,
        }
        let raw = Raw::deserialize(deserializer)?;
        let input = match &raw.entity_path {
            Some(path) if !path.is_empty() => format!("{}/{}", raw.domain, path),
            _ => raw.domain,
        };
        EntityRef::parse(&input).map_err(serde::de::Error::custom)
    }
}

/// Check if a character is allowed in domain names (post-lowercase).
fn is_domain_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '-')
}

/// Check if a character is allowed in entity paths.
fn is_path_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/' | '@' | '-')
}

/// Normalize domain: strip `www.` universally, then apply aliases.
fn normalize_domain(domain: &str) -> String {
    let domain = domain.strip_prefix("www.").unwrap_or(domain);
    match domain {
        "twitter.com" => "x.com".to_string(),
        other => other.to_string(),
    }
}

/// Platform-specific path normalization.
fn normalize_path(domain: &str, path: &str) -> String {
    match domain {
        "youtube.com" => normalize_youtube_path(path),
        "reddit.com" => normalize_reddit_path(path),
        "x.com" => normalize_x_path(path),
        _ => path.to_string(),
    }
}

fn normalize_youtube_path(path: &str) -> String {
    // youtube.com/c/Name → youtube.com/c/name
    // youtube.com/@Handle → youtube.com/@handle (preserve @)
    // youtube.com/channel/UC... → preserve as-is (channel IDs are case-sensitive)
    if path.starts_with("c/") || path.starts_with("@") {
        path.to_lowercase()
    } else {
        path.to_string()
    }
}

fn normalize_reddit_path(path: &str) -> String {
    // reddit.com/r/Rust → reddit.com/r/rust
    // reddit.com/user/Name → reddit.com/user/name
    if path.starts_with("r/") || path.starts_with("user/") {
        path.to_lowercase()
    } else {
        path.to_string()
    }
}

fn normalize_x_path(path: &str) -> String {
    // X/Twitter handles are case-insensitive: x.com/ElonMusk → x.com/elonmusk
    path.to_lowercase()
}

/// Extract an entity string from a URL, if the URL matches a known platform pattern.
///
/// Returns `None` for domain-only URLs (e.g., `https://example.com`).
/// Returns `Some(canonical)` for URLs with entity sub-paths.
pub fn entity_from_url(url: &str) -> Option<String> {
    let entity = EntityRef::parse(url).ok()?;
    if entity.entity_path.is_some() {
        Some(entity.canonical())
    } else {
        None
    }
}

/// Errors from parsing an entity reference string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntityParseError {
    Empty,
    NoDot,
    TooLong,
    InvalidChars,
}

impl std::fmt::Display for EntityParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "entity ref cannot be empty"),
            Self::NoDot => write!(f, "domain must contain '.'"),
            Self::TooLong => write!(f, "entity ref exceeds 253 bytes"),
            Self::InvalidChars => write!(f, "contains invalid characters"),
        }
    }
}

impl std::error::Error for EntityParseError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_domain_only() {
        let e = EntityRef::parse("example.com").unwrap();
        assert_eq!(e.domain, "example.com");
        assert!(e.entity_path.is_none());
        assert!(e.is_domain_only());
        assert_eq!(e.canonical(), "example.com");
    }

    #[test]
    fn test_parse_with_protocol() {
        let e = EntityRef::parse("https://example.com").unwrap();
        assert_eq!(e.domain, "example.com");
        assert!(e.is_domain_only());
    }

    #[test]
    fn test_parse_youtube_channel() {
        let e = EntityRef::parse("youtube.com/c/3Blue1Brown").unwrap();
        assert_eq!(e.domain, "youtube.com");
        assert_eq!(e.entity_path(), Some("c/3blue1brown"));
        assert_eq!(e.canonical(), "youtube.com/c/3blue1brown");
        assert!(!e.is_domain_only());
    }

    #[test]
    fn test_parse_youtube_handle() {
        let e = EntityRef::parse("youtube.com/@veritasium").unwrap();
        assert_eq!(e.entity_path(), Some("@veritasium"));
    }

    #[test]
    fn test_parse_youtube_channel_id() {
        // Channel IDs should NOT be lowercased
        let e = EntityRef::parse("youtube.com/channel/UCxyz123ABC").unwrap();
        assert_eq!(e.entity_path(), Some("channel/UCxyz123ABC"));
    }

    #[test]
    fn test_parse_reddit_subreddit() {
        let e = EntityRef::parse("reddit.com/r/Rust").unwrap();
        assert_eq!(e.domain, "reddit.com");
        assert_eq!(e.entity_path(), Some("r/rust"));
        assert_eq!(e.canonical(), "reddit.com/r/rust");
    }

    #[test]
    fn test_parse_reddit_user() {
        let e = EntityRef::parse("reddit.com/user/SpezIsABanana").unwrap();
        assert_eq!(e.entity_path(), Some("user/spezisabanana"));
    }

    #[test]
    fn test_twitter_to_x_alias() {
        let e = EntityRef::parse("twitter.com/elonmusk").unwrap();
        assert_eq!(e.domain, "x.com");
        assert_eq!(e.entity_path(), Some("elonmusk"));
        assert_eq!(e.canonical(), "x.com/elonmusk");
    }

    #[test]
    fn test_x_normalization() {
        let e = EntityRef::parse("x.com/ElonMusk").unwrap();
        assert_eq!(e.entity_path(), Some("elonmusk"));
    }

    #[test]
    fn test_www_stripping_universal() {
        // www. is stripped for ALL domains, not just a hardcoded list
        let e = EntityRef::parse("www.youtube.com/c/test").unwrap();
        assert_eq!(e.domain, "youtube.com");

        let e = EntityRef::parse("www.example.com").unwrap();
        assert_eq!(e.domain, "example.com");

        let e = EntityRef::parse("www.nytimes.com").unwrap();
        assert_eq!(e.domain, "nytimes.com");

        // www.x.com also works
        let e = EntityRef::parse("www.x.com/elonmusk").unwrap();
        assert_eq!(e.domain, "x.com");

        // www.twitter.com → x.com
        let e = EntityRef::parse("www.twitter.com/elonmusk").unwrap();
        assert_eq!(e.domain, "x.com");
    }

    #[test]
    fn test_trailing_slash() {
        let e = EntityRef::parse("reddit.com/r/rust/").unwrap();
        assert_eq!(e.entity_path(), Some("r/rust"));
    }

    #[test]
    fn test_ugc_platform() {
        assert!(EntityRef::parse("youtube.com").unwrap().is_ugc_platform());
        assert!(EntityRef::parse("reddit.com/r/rust").unwrap().is_ugc_platform());
        assert!(EntityRef::parse("twitter.com/elonmusk").unwrap().is_ugc_platform());
        assert!(EntityRef::parse("www.tiktok.com").unwrap().is_ugc_platform());
        assert!(EntityRef::parse("www.instagram.com").unwrap().is_ugc_platform());
        assert!(!EntityRef::parse("nytimes.com").unwrap().is_ugc_platform());
        assert!(!EntityRef::parse("example.com").unwrap().is_ugc_platform());
    }

    #[test]
    fn test_reject_empty() {
        assert_eq!(EntityRef::parse(""), Err(EntityParseError::Empty));
    }

    #[test]
    fn test_reject_no_dot() {
        assert_eq!(EntityRef::parse("localhost"), Err(EntityParseError::NoDot));
    }

    #[test]
    fn test_reject_leading_trailing_dot() {
        assert_eq!(
            EntityRef::parse(".example.com"),
            Err(EntityParseError::InvalidChars)
        );
        assert_eq!(
            EntityRef::parse("example.com."),
            Err(EntityParseError::InvalidChars)
        );
    }

    #[test]
    fn test_reject_leading_trailing_dash() {
        assert_eq!(
            EntityRef::parse("-example.com"),
            Err(EntityParseError::InvalidChars)
        );
        assert_eq!(
            EntityRef::parse("example.com-"),
            Err(EntityParseError::InvalidChars)
        );
    }

    #[test]
    fn test_reject_invalid_path_chars() {
        assert_eq!(
            EntityRef::parse("example.com/path with spaces"),
            Err(EntityParseError::InvalidChars)
        );
    }

    #[test]
    fn test_reject_invalid_domain_chars() {
        // Spaces in domain
        assert_eq!(
            EntityRef::parse("ex ample.com"),
            Err(EntityParseError::InvalidChars)
        );
        // HTML in domain
        assert_eq!(
            EntityRef::parse("ex<script>.com"),
            Err(EntityParseError::InvalidChars)
        );
        // Underscore in domain (not valid DNS)
        assert_eq!(
            EntityRef::parse("ex_ample.com"),
            Err(EntityParseError::InvalidChars)
        );
    }

    #[test]
    fn test_serde_roundtrip() {
        let entity = EntityRef::parse("youtube.com/c/3blue1brown").unwrap();
        let json = serde_json::to_string(&entity).unwrap();
        let deserialized: EntityRef = serde_json::from_str(&json).unwrap();
        assert_eq!(entity, deserialized);
    }

    #[test]
    fn test_serde_domain_only() {
        let entity = EntityRef::parse("example.com").unwrap();
        let json = serde_json::to_string(&entity).unwrap();
        let deserialized: EntityRef = serde_json::from_str(&json).unwrap();
        assert_eq!(entity, deserialized);
        assert!(deserialized.entity_path.is_none());
    }

    #[test]
    fn test_serde_rejects_invalid() {
        // Invalid domain should fail deserialization
        let bad_json = r#"{"domain":"no-dot","entity_path":null}"#;
        let result: std::result::Result<EntityRef, _> = serde_json::from_str(bad_json);
        assert!(result.is_err());
    }

    #[test]
    fn test_entity_from_url() {
        assert_eq!(
            entity_from_url("https://youtube.com/c/3Blue1Brown"),
            Some("youtube.com/c/3blue1brown".to_string())
        );
        assert_eq!(entity_from_url("https://example.com"), None);
        assert_eq!(
            entity_from_url("https://reddit.com/r/rust"),
            Some("reddit.com/r/rust".to_string())
        );
    }

    #[test]
    fn test_display() {
        let e = EntityRef::parse("youtube.com/c/test").unwrap();
        assert_eq!(format!("{e}"), "youtube.com/c/test");
    }

    #[test]
    fn test_accessor_methods() {
        let e = EntityRef::parse("youtube.com/c/test").unwrap();
        assert_eq!(e.domain(), "youtube.com");
        assert_eq!(e.entity_path(), Some("c/test"));

        let e = EntityRef::parse("example.com").unwrap();
        assert_eq!(e.domain(), "example.com");
        assert_eq!(e.entity_path(), None);
    }
}
