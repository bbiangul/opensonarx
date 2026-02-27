/// Minimum chunk size as a fraction of max_tokens. Trailing chunks smaller
/// than this are merged into the previous chunk to avoid tiny fragments.
const MIN_CHUNK_RATIO: f64 = 0.25;

/// Split text into chunks of approximately `max_tokens` tokens with `overlap` token overlap.
/// Uses whitespace tokenization (word-level).
/// Tiny trailing chunks (< 25% of max_tokens) are merged into the previous chunk.
pub fn chunk_text(text: &str, max_tokens: usize, overlap: usize) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return Vec::new();
    }

    let mut chunks = Vec::new();
    let mut start = 0;

    while start < words.len() {
        let end = (start + max_tokens).min(words.len());
        let chunk: String = words[start..end].join(" ");
        chunks.push(chunk);

        if end >= words.len() {
            break;
        }

        // Advance by (max_tokens - overlap)
        let step = if max_tokens > overlap {
            max_tokens - overlap
        } else {
            1
        };
        start += step;
    }

    // Merge tiny trailing chunk into previous
    let min_words = ((max_tokens as f64) * MIN_CHUNK_RATIO) as usize;
    if chunks.len() >= 2 {
        let last_word_count = chunks.last().unwrap().split_whitespace().count();
        if last_word_count < min_words {
            let last = chunks.pop().unwrap();
            if let Some(prev) = chunks.last_mut() {
                prev.push(' ');
                prev.push_str(&last);
            }
        }
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_chunking() {
        let text = (0..1024).map(|i| format!("word{i}")).collect::<Vec<_>>().join(" ");
        let chunks = chunk_text(&text, 512, 128);
        assert!(chunks.len() >= 2);
        assert!(chunks[0].split_whitespace().count() <= 512);
    }

    #[test]
    fn test_overlap() {
        let text = (0..100).map(|i| format!("w{i}")).collect::<Vec<_>>().join(" ");
        let chunks = chunk_text(&text, 50, 10);
        // Second chunk should start 40 words in (50 - 10 overlap)
        let first_words: Vec<&str> = chunks[0].split_whitespace().collect();
        let second_words: Vec<&str> = chunks[1].split_whitespace().collect();
        // Last 10 words of chunk 0 should match first 10 of chunk 1
        assert_eq!(first_words[40..50], second_words[0..10]);
    }

    #[test]
    fn test_small_text() {
        let text = "hello world";
        let chunks = chunk_text(text, 512, 128);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "hello world");
    }

    #[test]
    fn test_empty_text() {
        let chunks = chunk_text("", 512, 128);
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_deterministic() {
        let text = "the quick brown fox jumps over the lazy dog and more text here";
        let c1 = chunk_text(text, 5, 2);
        let c2 = chunk_text(text, 5, 2);
        assert_eq!(c1, c2);
    }
}
