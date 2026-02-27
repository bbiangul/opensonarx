use oi_types::facts::{ExtractedEntity, Fact, FactExtractionResult};
use oi_types::traits::FactExtractionPipeline;
use oi_types::Result;
use ort::session::Session;
use ort::value::Value;
use std::borrow::Cow;
use std::sync::Mutex;

use crate::model_cache::{ModelCache, GLINER_MULTI_MODEL, GLINER_MULTI_TOKENIZER};
use crate::normalizer;

/// Default entity labels for zero-shot NER.
const DEFAULT_ENTITY_LABELS: &[&str] = &[
    "person",
    "organization",
    "location",
    "date",
    "event",
    "product",
    "quantity",
    "money",
];

/// Default relation labels for zero-shot RE.
const DEFAULT_RELATION_LABELS: &[&str] = &[
    "founded_by",
    "born_in",
    "located_in",
    "capital_of",
    "ceo_of",
    "plays_for",
    "member_of",
    "part_of",
];

/// GLiNER special token IDs (from the tokenizer's added_tokens).
#[allow(dead_code)]
const ENT_TOKEN_ID: i64 = 250103; // <<ENT>>
#[allow(dead_code)]
const SEP_SPECIAL_TOKEN_ID: i64 = 250104; // <<SEP>>

/// Maximum span width the model evaluates (words).
const MAX_SPAN_WIDTH: usize = 12;

/// GLiNER-based fact extractor using ONNX runtime.
///
/// Supports configurable entity/relation labels at runtime (zero-shot).
/// Thread-safe via `Mutex<Session>` (same pattern as MiniLmEmbedder).
pub struct GlinerFactExtractor {
    session: Mutex<Session>,
    tokenizer: tokenizers::Tokenizer,
    entity_labels: Vec<String>,
    #[allow(dead_code)]
    relation_labels: Vec<String>,
    entity_threshold: f64,
    #[allow(dead_code)]
    relation_threshold: f64,
}

impl GlinerFactExtractor {
    /// Load the default GLiNER multi model.
    pub fn new() -> Result<Self> {
        let cache = ModelCache::new();
        let model_path = cache.ensure_model(&GLINER_MULTI_MODEL)?;
        let tokenizer_path = cache.ensure_model(&GLINER_MULTI_TOKENIZER)?;

        Self::from_paths(&model_path, &tokenizer_path)
    }

    /// Load from explicit model and tokenizer paths.
    pub fn from_paths(
        model_path: &std::path::Path,
        tokenizer_path: &std::path::Path,
    ) -> Result<Self> {
        let threads = crate::intra_threads();

        let session = Session::builder()
            .map_err(|e| oi_types::Error::Internal(format!("ORT session builder: {e}")))?
            .with_intra_threads(threads)
            .map_err(|e| oi_types::Error::Internal(format!("ORT threads: {e}")))?
            .commit_from_file(model_path)
            .map_err(|e| oi_types::Error::Internal(format!("ORT load model: {e}")))?;

        let tokenizer = tokenizers::Tokenizer::from_file(tokenizer_path)
            .map_err(|e| oi_types::Error::Internal(format!("Tokenizer load: {e}")))?;

        Ok(Self {
            session: Mutex::new(session),
            tokenizer,
            entity_labels: DEFAULT_ENTITY_LABELS.iter().map(|s| s.to_string()).collect(),
            relation_labels: DEFAULT_RELATION_LABELS.iter().map(|s| s.to_string()).collect(),
            entity_threshold: 0.5,
            relation_threshold: 0.5,
        })
    }

    /// Set custom confidence thresholds for entity and relation extraction.
    pub fn with_thresholds(mut self, entity_threshold: f64, relation_threshold: f64) -> Self {
        self.entity_threshold = entity_threshold;
        self.relation_threshold = relation_threshold;
        self
    }

    /// Set custom entity labels for zero-shot NER.
    pub fn with_entity_labels(mut self, labels: Vec<String>) -> Self {
        self.entity_labels = labels;
        self
    }

    /// Tokenize a single word (no special tokens added).
    fn tokenize_word(&self, word: &str) -> Result<Vec<i64>> {
        let enc = self
            .tokenizer
            .encode(word, false)
            .map_err(|e| oi_types::Error::Internal(format!("Tokenize word '{word}': {e}")))?;
        Ok(enc.get_ids().iter().map(|&id| id as i64).collect())
    }

    /// Run inference and extract entities from logits.
    ///
    /// Follows the gline-rs approach: tokenize each word individually and
    /// concatenate, rather than tokenizing the whole prompt at once.
    fn extract_entities(&self, text: &str) -> Result<Vec<ExtractedEntity>> {
        // 1. Split text into words with byte offsets
        let words = split_words(text);
        let num_words = words.len();
        if num_words == 0 {
            return Ok(Vec::new());
        }

        // 2. Build prompt as word list and tokenize word-by-word:
        //    [CLS] <<ENT>> label1 <<ENT>> label2 ... <<SEP>> word1 word2 ... [SEP]
        //
        // Entity labels are each preceded by <<ENT>>, and <<SEP>> separates
        // labels from text. Each word is tokenized individually for exact
        // subword-to-word alignment.

        let mut input_ids: Vec<i64> = vec![1]; // [CLS]
        let mut words_mask: Vec<i64> = vec![0]; // CLS gets 0

        // Tokenize entity labels: <<ENT>> label <<ENT>> label ... <<SEP>>
        for label in &self.entity_labels {
            // <<ENT>> token
            input_ids.push(ENT_TOKEN_ID);
            words_mask.push(0);
            // Label subword tokens
            let label_tokens = self.tokenize_word(label)?;
            for &tok in &label_tokens {
                input_ids.push(tok);
                words_mask.push(0);
            }
        }

        // <<SEP>> token
        input_ids.push(SEP_SPECIAL_TOKEN_ID);
        words_mask.push(0);
        // Note: gline-rs increments word_id after <<SEP>>, making text words 1-indexed.
        // We start word_id at 1 explicitly for clarity.

        // 3. Tokenize text words: each gets 1-indexed word_id on its first subword
        for (word_idx, (word_text, _, _)) in words.iter().enumerate() {
            let word_tokens = self.tokenize_word(word_text)?;
            for (sub_idx, &tok) in word_tokens.iter().enumerate() {
                input_ids.push(tok);
                // Only the first subword of each text word gets the word index
                words_mask.push(if sub_idx == 0 {
                    (word_idx + 1) as i64
                } else {
                    0
                });
            }
        }

        // [SEP] token
        input_ids.push(2); // [SEP]
        words_mask.push(0);

        let seq_len = input_ids.len();
        let attention_mask: Vec<i64> = vec![1; seq_len];

        // 4. text_lengths: [batch, 1]
        let text_lengths: Vec<i64> = vec![num_words as i64];

        // 5. Generate span indices: all (start, end) pairs within max_width
        let mut span_idx_data: Vec<i64> = Vec::new();
        let mut span_mask_data: Vec<bool> = Vec::new();

        for start in 0..num_words {
            for width in 0..MAX_SPAN_WIDTH {
                let end = start + width;
                if end < num_words {
                    span_idx_data.push(start as i64);
                    span_idx_data.push(end as i64);
                    span_mask_data.push(true);
                } else {
                    // Padding span — masked out, placeholder indices
                    span_idx_data.push(0);
                    span_idx_data.push(0);
                    span_mask_data.push(false);
                }
            }
        }
        let num_spans = span_mask_data.len();

        // Ensure we have at least one span
        if num_spans == 0 {
            return Ok(Vec::new());
        }

        // 7. Create input tensors
        let seq_shape = [1usize, seq_len];
        let tl_shape = [1usize, 1usize];
        let span_idx_shape = [1usize, num_spans, 2usize];
        let span_mask_shape = [1usize, num_spans];

        let input_ids_val = Value::from_array((&seq_shape as &[usize], input_ids))
            .map_err(|e| oi_types::Error::Internal(format!("Value input_ids: {e}")))?;
        let attention_val = Value::from_array((&seq_shape as &[usize], attention_mask))
            .map_err(|e| oi_types::Error::Internal(format!("Value attention_mask: {e}")))?;
        let words_mask_val = Value::from_array((&seq_shape as &[usize], words_mask))
            .map_err(|e| oi_types::Error::Internal(format!("Value words_mask: {e}")))?;
        let text_lengths_val = Value::from_array((&tl_shape as &[usize], text_lengths))
            .map_err(|e| oi_types::Error::Internal(format!("Value text_lengths: {e}")))?;
        let span_idx_val = Value::from_array((&span_idx_shape as &[usize], span_idx_data))
            .map_err(|e| oi_types::Error::Internal(format!("Value span_idx: {e}")))?;
        let span_mask_val = Value::from_array((&span_mask_shape as &[usize], span_mask_data))
            .map_err(|e| oi_types::Error::Internal(format!("Value span_mask: {e}")))?;

        let inputs = vec![
            (
                Cow::from("input_ids"),
                ort::session::SessionInputValue::from(input_ids_val),
            ),
            (
                Cow::from("attention_mask"),
                ort::session::SessionInputValue::from(attention_val),
            ),
            (
                Cow::from("words_mask"),
                ort::session::SessionInputValue::from(words_mask_val),
            ),
            (
                Cow::from("text_lengths"),
                ort::session::SessionInputValue::from(text_lengths_val),
            ),
            (
                Cow::from("span_idx"),
                ort::session::SessionInputValue::from(span_idx_val),
            ),
            (
                Cow::from("span_mask"),
                ort::session::SessionInputValue::from(span_mask_val),
            ),
        ];

        // 8. Run inference
        let mut session = self
            .session
            .lock()
            .map_err(|e| oi_types::Error::Internal(format!("Lock: {e}")))?;

        let outputs = session
            .run(inputs)
            .map_err(|e| oi_types::Error::Internal(format!("ORT run: {e}")))?;

        // 9. Parse output logits
        let (tensor_shape, logits) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| oi_types::Error::Internal(format!("Extract tensor: {e}")))?;

        let ndims = tensor_shape.len();
        let num_classes = self.entity_labels.len();

        let mut entities = Vec::new();

        if ndims == 4 {
            // Output: [batch, num_words, max_width, num_classes]
            let out_words = tensor_shape[1] as usize;
            let out_width = tensor_shape[2] as usize;
            let out_classes = tensor_shape[3] as usize;

            for w in 0..out_words.min(num_words) {
                for s in 0..out_width.min(MAX_SPAN_WIDTH) {
                    let end_word = w + s;
                    if end_word >= num_words {
                        break;
                    }

                    for c in 0..out_classes.min(num_classes) {
                        let idx = w * out_width * out_classes + s * out_classes + c;
                        let score = sigmoid(logits[idx] as f64);

                        if score >= self.entity_threshold {
                            if let Some(ent) =
                                self.make_entity_from_words(text, &words, w, end_word, c, score)
                            {
                                entities.push(ent);
                            }
                        }
                    }
                }
            }
        } else if ndims == 3 {
            // Fallback: [batch, num_spans, num_classes]
            let out_spans = tensor_shape[1] as usize;
            let out_classes = tensor_shape[2] as usize;

            let mut span_i = 0;
            for start in 0..num_words {
                for width in 0..MAX_SPAN_WIDTH {
                    let end = start + width;
                    if end >= num_words || span_i >= out_spans {
                        break;
                    }

                    for c in 0..out_classes.min(num_classes) {
                        let idx = span_i * out_classes + c;
                        let score = sigmoid(logits[idx] as f64);

                        if score >= self.entity_threshold {
                            if let Some(ent) =
                                self.make_entity_from_words(text, &words, start, end, c, score)
                            {
                                entities.push(ent);
                            }
                        }
                    }
                    span_i += 1;
                }
            }
        } else {
            tracing::warn!(
                "Unexpected GLiNER output shape: {tensor_shape:?} (ndims={ndims})"
            );
        }

        // 10. Non-maximum suppression: keep highest confidence, remove overlaps
        entities.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap());
        let mut kept = Vec::new();
        for ent in &entities {
            let overlaps = kept
                .iter()
                .any(|k: &ExtractedEntity| ent.start < k.end && k.start < ent.end);
            if !overlaps {
                kept.push(ent.clone());
            }
        }
        kept.sort_by_key(|e| e.start);

        Ok(kept)
    }

    /// Create an entity from word-level span indices.
    fn make_entity_from_words(
        &self,
        text: &str,
        words: &[(String, usize, usize)],
        start_word: usize,
        end_word: usize,
        label_idx: usize,
        confidence: f64,
    ) -> Option<ExtractedEntity> {
        let (_, byte_start, _) = &words[start_word];
        let (_, _, byte_end) = &words[end_word];

        if *byte_start >= text.len() || *byte_end > text.len() || byte_start >= byte_end {
            return None;
        }

        let raw_mention = &text[*byte_start..*byte_end];
        // Strip trailing punctuation that the model often includes
        let mention = raw_mention.trim_end_matches(|c: char| c.is_ascii_punctuation());
        if mention.trim().is_empty() {
            return None;
        }
        let byte_end_trimmed = *byte_start + mention.len();

        let entity_type = self
            .entity_labels
            .get(label_idx)
            .cloned()
            .unwrap_or_else(|| "unknown".to_string());

        let normalized = normalizer::normalize_entity(mention, &entity_type);

        Some(ExtractedEntity {
            mention: mention.to_string(),
            normalized,
            entity_type,
            start: *byte_start,
            end: byte_end_trimmed,
            confidence,
        })
    }
}

fn sigmoid(x: f64) -> f64 {
    1.0 / (1.0 + (-x).exp())
}

/// Split text into words with byte offsets.
/// Handles CJK characters as individual words.
fn split_words(text: &str) -> Vec<(String, usize, usize)> {
    let mut words = Vec::new();
    let mut chars = text.char_indices().peekable();

    while let Some(&(start_idx, ch)) = chars.peek() {
        if ch.is_whitespace() {
            chars.next();
            continue;
        }

        // CJK character = individual word
        if is_cjk(ch) {
            let len = ch.len_utf8();
            words.push((ch.to_string(), start_idx, start_idx + len));
            chars.next();
            continue;
        }

        // Collect non-whitespace, non-CJK characters as a word
        let mut end_idx = start_idx;
        let mut word = String::new();
        while let Some(&(idx, c)) = chars.peek() {
            if c.is_whitespace() || is_cjk(c) {
                break;
            }
            word.push(c);
            end_idx = idx + c.len_utf8();
            chars.next();
        }
        if !word.is_empty() {
            words.push((word, start_idx, end_idx));
        }
    }
    words
}

/// Check if a character is CJK (Chinese, Japanese Kanji, Korean).
fn is_cjk(c: char) -> bool {
    matches!(c,
        '\u{4E00}'..='\u{9FFF}'   // CJK Unified Ideographs
        | '\u{3400}'..='\u{4DBF}' // CJK Extension A
        | '\u{3040}'..='\u{309F}' // Hiragana
        | '\u{30A0}'..='\u{30FF}' // Katakana
        | '\u{F900}'..='\u{FAFF}' // CJK Compatibility
        | '\u{AC00}'..='\u{D7AF}' // Hangul Syllables
    )
}

impl FactExtractionPipeline for GlinerFactExtractor {
    fn extract(&self, text: &str) -> Result<FactExtractionResult> {
        let start = std::time::Instant::now();

        if text.is_empty() {
            return Ok(FactExtractionResult::default());
        }

        let entities = self.extract_entities(text)?;

        // Create relation triplets between consecutive entity pairs
        // (real relation extraction would use the multitask model)
        let mut facts = Vec::new();
        for pair in entities.windows(2) {
            facts.push(Fact {
                subject: pair[0].clone(),
                predicate: "related_to".to_string(),
                object: pair[1].clone(),
                confidence: (pair[0].confidence + pair[1].confidence) / 2.0,
            });
        }

        let elapsed = start.elapsed().as_secs_f64() * 1000.0;

        Ok(FactExtractionResult {
            entities,
            facts,
            language: None,
            extraction_time_ms: elapsed,
        })
    }

    fn supported_entity_types(&self) -> &[&str] {
        DEFAULT_ENTITY_LABELS
    }

    fn supported_relation_types(&self) -> &[&str] {
        DEFAULT_RELATION_LABELS
    }

    fn name(&self) -> &str {
        "gliner"
    }
}

// Tests require model download — only run with `cargo test --features onnx`
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_words_latin() {
        let words = split_words("Hello world");
        assert_eq!(words.len(), 2);
        assert_eq!(words[0].0, "Hello");
        assert_eq!(words[1].0, "world");
    }

    #[test]
    fn test_split_words_cjk() {
        let words = split_words("习近平在北京");
        assert_eq!(words.len(), 6);
        assert_eq!(words[0].0, "习");
        assert_eq!(words[1].0, "近");
        assert_eq!(words[5].0, "京");
    }

    #[test]
    fn test_split_words_mixed() {
        let words = split_words("Apple在北京 opened");
        assert_eq!(words[0].0, "Apple");
        assert!(words.iter().any(|w| w.0 == "opened"));
    }

    // Helper: skip test if model not cached (avoid CI download)
    fn skip_if_no_model() -> bool {
        let cache = ModelCache::new();
        !cache.is_cached(&GLINER_MULTI_MODEL)
    }

    #[test]
    fn test_gliner_load() {
        if skip_if_no_model() {
            eprintln!("Skipping: GLiNER model not cached. Run `cargo test --features onnx` after downloading.");
            return;
        }
        let ext = GlinerFactExtractor::new();
        assert!(ext.is_ok(), "GLiNER should load: {:?}", ext.err());
    }

    #[test]
    fn test_gliner_basic_ner() {
        if skip_if_no_model() {
            return;
        }
        let ext = GlinerFactExtractor::new().unwrap();
        let result = ext
            .extract("Microsoft was founded by Bill Gates in Albuquerque.")
            .unwrap();
        assert!(!result.entities.is_empty(), "should find entities");
        let mentions: Vec<&str> = result.entities.iter().map(|e| e.mention.as_str()).collect();
        eprintln!("GLiNER found: {mentions:?}");
    }

    #[test]
    fn test_gliner_multilingual() {
        if skip_if_no_model() {
            return;
        }
        let ext = GlinerFactExtractor::new().unwrap();
        let texts = &[
            ("en", "Apple CEO Tim Cook announced new products."),
            ("pt", "O presidente Lula visitou São Paulo."),
            ("es", "Pedro Sánchez visitó la sede de la OTAN."),
            ("de", "Olaf Scholz traf sich mit Macron in Berlin."),
            ("fr", "Emmanuel Macron a reçu Merkel à Paris."),
            ("zh", "习近平主席在北京会见了马克龙。"),
            ("ja", "岸田首相は東京で会議を開催した。"),
        ];
        for (lang, text) in texts {
            let result = ext.extract(text).unwrap();
            eprintln!(
                "[{lang}] Found {} entities: {:?}",
                result.entities.len(),
                result
                    .entities
                    .iter()
                    .map(|e| e.mention.as_str())
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn test_gliner_empty_text() {
        if skip_if_no_model() {
            return;
        }
        let ext = GlinerFactExtractor::new().unwrap();
        let result = ext.extract("").unwrap();
        assert!(result.entities.is_empty());
    }

    #[test]
    fn test_gliner_deterministic() {
        if skip_if_no_model() {
            return;
        }
        let ext = GlinerFactExtractor::new().unwrap();
        let text = "Google was founded by Larry Page and Sergey Brin.";
        let r1 = ext.extract(text).unwrap();
        let r2 = ext.extract(text).unwrap();
        assert_eq!(r1.entities.len(), r2.entities.len());
        for (a, b) in r1.entities.iter().zip(r2.entities.iter()) {
            assert_eq!(a.mention, b.mention);
            assert_eq!(a.entity_type, b.entity_type);
        }
    }

    #[test]
    fn test_gliner_throughput() {
        if skip_if_no_model() {
            return;
        }
        let ext = GlinerFactExtractor::new().unwrap();
        let text = "Tesla CEO Elon Musk announced a new Gigafactory in Austin, Texas.";
        let n = 10;
        let start = std::time::Instant::now();
        for _ in 0..n {
            let _ = ext.extract(text).unwrap();
        }
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
        eprintln!(
            "GLiNER throughput: {:.1}ms/doc ({n} docs in {:.0}ms)",
            elapsed / n as f64,
            elapsed
        );
    }
}
