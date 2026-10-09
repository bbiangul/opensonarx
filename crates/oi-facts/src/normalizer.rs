/// Normalize an entity mention based on its type.
pub fn normalize_entity(mention: &str, entity_type: &str) -> String {
    let collapsed = collapse_whitespace(mention);
    match entity_type {
        "date" => normalize_date(&collapsed).unwrap_or_else(|| collapsed.to_lowercase()),
        "quantity" | "money" => {
            normalize_number(&collapsed).unwrap_or_else(|| collapsed.to_lowercase())
        }
        "person" => normalize_person(&collapsed),
        _ => collapsed.to_lowercase(),
    }
}

/// Collapse all whitespace runs to a single space and trim.
pub fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Normalize dates to ISO 8601 (YYYY-MM-DD) where possible.
pub fn normalize_date(s: &str) -> Option<String> {
    let trimmed = s.trim();

    // Already ISO: 2026-02-20
    if let Some((y, rest)) = trimmed.split_once('-')
        && y.len() == 4
        && y.chars().all(|c| c.is_ascii_digit())
        && rest.len() == 5
        && rest.chars().filter(|c| *c == '-').count() == 1
    {
        return Some(trimmed.to_string());
    }

    // European: DD/MM/YYYY or DD.MM.YYYY
    for sep in &['/', '.'] {
        let parts: Vec<&str> = trimmed.split(*sep).collect();
        if parts.len() == 3
            && let (Ok(d), Ok(m), Ok(y)) = (
                parts[0].parse::<u32>(),
                parts[1].parse::<u32>(),
                parts[2].parse::<u32>(),
            )
            && (1000..=9999).contains(&y)
            && (1..=12).contains(&m)
            && (1..=31).contains(&d)
        {
            return Some(format!("{y:04}-{m:02}-{d:02}"));
        }
    }

    // US-style: MM/DD/YYYY (if first number > 12, treat as DD/MM)
    // Handled above already — ambiguous cases default to DD/MM

    None
}

/// Normalize number expressions to plain digits.
/// "1.5 million" -> "1500000", "3.2 billion" -> "3200000000", "45%" -> "45%"
pub fn normalize_number(s: &str) -> Option<String> {
    let trimmed = s.trim().to_lowercase();

    // Percentage: keep as-is
    if trimmed.ends_with('%') {
        return Some(trimmed.to_string());
    }

    // Try to parse "X million/billion/trillion" patterns
    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    if parts.len() == 2
        && let Ok(num) = parts[0].parse::<f64>()
    {
        let multiplier = match parts[1] {
            "thousand" | "k" => Some(1_000.0),
            "million" | "m" => Some(1_000_000.0),
            "billion" | "b" => Some(1_000_000_000.0),
            "trillion" | "t" => Some(1_000_000_000_000.0),
            _ => None,
        };
        if let Some(mult) = multiplier {
            let result = (num * mult) as i64;
            return Some(result.to_string());
        }
    }

    // Try to parse as plain number
    if let Ok(num) = trimmed.parse::<f64>() {
        if num == num.floor() {
            return Some(format!("{}", num as i64));
        }
        return Some(format!("{num}"));
    }

    None
}

/// Normalize person names: strip common titles, lowercase.
pub fn normalize_person(s: &str) -> String {
    let titles = [
        "Dr.", "Dr", "Mr.", "Mr", "Mrs.", "Mrs", "Ms.", "Ms", "Prof.", "Prof", "Sir", "Dame",
        "Lord", "Lady", "Rev.", "Rev", "Fr.", "Fr", "Sr.", "Sr", "Jr.", "Jr",
    ];
    let mut result = s.to_string();
    for title in &titles {
        if result.starts_with(title) && result.chars().nth(title.len()).is_some_and(|c| c == ' ') {
            result = result[title.len()..].trim_start().to_string();
            break; // Only strip one title
        }
    }
    result.to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collapse_whitespace() {
        assert_eq!(collapse_whitespace("  hello   world  "), "hello world");
        assert_eq!(collapse_whitespace("a\t\nb"), "a b");
    }

    #[test]
    fn test_normalize_date_iso() {
        assert_eq!(normalize_date("2026-02-20"), Some("2026-02-20".to_string()));
    }

    #[test]
    fn test_normalize_date_european() {
        assert_eq!(normalize_date("20/02/2026"), Some("2026-02-20".to_string()));
        assert_eq!(normalize_date("20.02.2026"), Some("2026-02-20".to_string()));
    }

    #[test]
    fn test_normalize_number_million() {
        assert_eq!(normalize_number("1.5 million"), Some("1500000".to_string()));
        assert_eq!(
            normalize_number("3.2 billion"),
            Some("3200000000".to_string())
        );
    }

    #[test]
    fn test_normalize_number_percentage() {
        assert_eq!(normalize_number("45%"), Some("45%".to_string()));
    }

    #[test]
    fn test_normalize_person_titles() {
        assert_eq!(normalize_person("Dr. Jane Smith"), "jane smith");
        assert_eq!(normalize_person("Mr. John Doe"), "john doe");
        assert_eq!(normalize_person("Prof. Albert Einstein"), "albert einstein");
    }

    #[test]
    fn test_normalize_entity_generic() {
        assert_eq!(normalize_entity("Google LLC", "organization"), "google llc");
        assert_eq!(normalize_entity("NEW YORK", "location"), "new york");
    }

    #[test]
    fn test_normalize_empty() {
        assert_eq!(normalize_entity("", "person"), "");
        assert_eq!(collapse_whitespace(""), "");
    }

    #[test]
    fn test_normalize_unicode() {
        assert_eq!(normalize_entity("São Paulo", "location"), "são paulo");
        assert_eq!(normalize_entity("Zürich", "location"), "zürich");
    }
}
