//! Character-level text normalization shared by lexical scoring and quote
//! verification. Nothing here knows what a word means.

/// Lowercase, strip Latin diacritics, and collapse every run of
/// non-alphanumeric characters into one space.
pub(crate) fn fold_text(text: &str) -> String {
    let mut folded = String::with_capacity(text.len());
    let mut previous_space = true;
    for character in text.chars().flat_map(char::to_lowercase) {
        let character = match character {
            'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            'ñ' => 'n',
            value => value,
        };
        if character.is_alphanumeric() {
            folded.push(character);
            previous_space = false;
        } else if !previous_space {
            folded.push(' ');
            previous_space = true;
        }
    }
    folded.trim_end().to_string()
}

/// Folded tokens of at least two characters, in order, with repeats.
pub(crate) fn terms(text: &str) -> Vec<String> {
    fold_text(text)
        .split(' ')
        .filter(|term| term.chars().count() > 1)
        .map(str::to_string)
        .collect()
}

/// Share of `query_terms` that occur in `candidate_terms`. Two terms of
/// five or more characters also match when one is a prefix of the other
/// ("promise" / "promises"): inflection without a vocabulary.
pub(crate) fn term_coverage(query_terms: &[String], candidate_terms: &[String]) -> f64 {
    if query_terms.is_empty() {
        return 0.0;
    }
    let matched = query_terms
        .iter()
        .filter(|query_term| {
            candidate_terms.iter().any(|candidate| {
                *query_term == candidate
                    || (query_term.len() >= 5
                        && candidate.len() >= 5
                        && (query_term.starts_with(candidate.as_str())
                            || candidate.starts_with(query_term.as_str())))
            })
        })
        .count();
    matched as f64 / query_terms.len() as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folding_strips_diacritics_case_and_punctuation() {
        assert_eq!(
            fold_text("  Computação Quântica — O QUÊ?  "),
            "computacao quantica o que"
        );
        assert_eq!(fold_text(""), "");
    }

    #[test]
    fn terms_drop_single_characters_and_keep_order() {
        assert_eq!(terms("O que é a Pix?"), vec!["que", "pix"]);
    }

    #[test]
    fn coverage_is_the_matched_share_of_query_terms() {
        let query = terms("rust borrow checker");
        assert_eq!(term_coverage(&query, &terms("The Rust checker")), 2.0 / 3.0);
        assert_eq!(
            term_coverage(&terms("promise"), &terms("JavaScript promises")),
            1.0
        );
        assert_eq!(term_coverage(&terms("rust"), &terms("trust")), 0.0);
        assert_eq!(term_coverage(&[], &query), 0.0);
    }
}
