//! Locating a quote in retained text.
//!
//! These are the deterministic checks behind every citation: the text a
//! model reads is the plain rendering of the retained Markdown, and a quote
//! counts only if it occurs in that text character for character after a
//! fixed normalization of whitespace and typographic punctuation. They live
//! here so that every layer applies the same check, including a node that
//! verifies claims it receives from another node.

/// Render retained Markdown as the plain text a model reads and quotes.
/// Verification runs against this same text, so a quote never has to
/// reproduce link or emphasis syntax.
pub fn plain_text(markdown: &str) -> String {
    let mut plain = String::with_capacity(markdown.len());
    let mut characters = markdown.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '*' | '`' => {}
            '!' if characters.peek() == Some(&'[') => {}
            '[' => {
                let mut label = String::new();
                let mut closed = false;
                for next in characters.by_ref() {
                    if next == ']' {
                        closed = true;
                        break;
                    }
                    label.push(next);
                }
                if closed && characters.peek() == Some(&'(') {
                    for next in characters.by_ref() {
                        if next == ')' {
                            break;
                        }
                    }
                    plain.push_str(&label);
                } else {
                    plain.push('[');
                    plain.push_str(&label);
                    if closed {
                        plain.push(']');
                    }
                }
            }
            '#' if plain.is_empty() || plain.ends_with('\n') => {
                while characters.peek() == Some(&'#') {
                    characters.next();
                }
                if characters.peek() == Some(&' ') {
                    characters.next();
                }
            }
            value => plain.push(value),
        }
    }
    plain
}

/// Canonical characters for exact matching: whitespace runs collapse to one
/// space and typographic quotes and dashes map to their ASCII forms. Each
/// canonical character keeps the byte range it came from.
fn canonical(text: &str) -> Vec<(char, usize, usize)> {
    let mut out: Vec<(char, usize, usize)> = Vec::with_capacity(text.len());
    for (offset, character) in text.char_indices() {
        let end = offset + character.len_utf8();
        let mapped = match character {
            '\u{2018}' | '\u{2019}' | '\u{201B}' | '\u{2032}' => '\'',
            '\u{201C}' | '\u{201D}' | '\u{201F}' | '\u{2033}' | '\u{00AB}' | '\u{00BB}' => '"',
            '\u{2010}'..='\u{2015}' | '\u{2212}' => '-',
            value if value.is_whitespace() => ' ',
            value => value,
        };
        if mapped == ' ' {
            match out.last_mut() {
                Some(last) if last.0 == ' ' => last.2 = end,
                Some(_) => out.push((' ', offset, end)),
                None => {}
            }
        } else {
            out.push((mapped, offset, end));
        }
    }
    while out.last().is_some_and(|last| last.0 == ' ') {
        out.pop();
    }
    out
}

/// Locate `quote` verbatim in `passage` after canonical normalization and
/// return the span as it appears in the passage. `None` when the quote is
/// shorter than `min_chars` or does not occur.
pub fn locate_quote(passage: &str, quote: &str, min_chars: usize) -> Option<String> {
    let needle = canonical(quote);
    if needle.len() < min_chars.max(1) {
        return None;
    }
    let haystack = canonical(passage);
    if needle.len() > haystack.len() {
        return None;
    }
    let start = (0..=haystack.len() - needle.len()).find(|&start| {
        haystack[start..start + needle.len()]
            .iter()
            .zip(&needle)
            .all(|(left, right)| left.0 == right.0)
    })?;
    let from = haystack[start].1;
    let to = haystack[start + needle.len() - 1].2;
    Some(passage[from..to].to_string())
}
