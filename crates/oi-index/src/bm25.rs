use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// BM25 index for full-text keyword search
pub struct Bm25Index {
    /// doc_id -> original text
    docs: Vec<(String, String)>,
    /// doc_id -> index in `docs` for O(1) content retrieval (rebuilt on deserialize)
    doc_id_index: HashMap<String, usize>,
    /// Stored document lengths (in tokens) to avoid re-tokenization during search
    doc_lengths: Vec<f64>,
    /// term -> list of (doc_index, term_frequency)
    inverted_index: HashMap<String, Vec<(usize, f64)>>,
    /// Average document length
    avg_dl: f64,
    /// BM25 parameters
    k1: f64,
    b: f64,
    /// Soft-deleted doc indices — excluded from search results but still in storage.
    deleted: HashSet<usize>,
}

impl Bm25Index {
    pub fn new() -> Self {
        Self {
            docs: Vec::new(),
            doc_id_index: HashMap::new(),
            doc_lengths: Vec::new(),
            inverted_index: HashMap::new(),
            avg_dl: 0.0,
            k1: 1.2,
            b: 0.75,
            deleted: HashSet::new(),
        }
    }

    pub fn insert(&mut self, doc_id: String, text: String) {
        let doc_idx = self.docs.len();
        let tokens = tokenize(&text);
        let doc_len = tokens.len() as f64;

        // Count term frequencies
        let mut tf_map: HashMap<&str, usize> = HashMap::new();
        for token in &tokens {
            *tf_map.entry(token.as_str()).or_default() += 1;
        }

        for (term, count) in tf_map {
            let tf = count as f64;
            self.inverted_index
                .entry(term.to_string())
                .or_default()
                .push((doc_idx, tf));
        }

        self.doc_id_index.insert(doc_id.clone(), doc_idx);
        self.docs.push((doc_id, text));
        self.doc_lengths.push(doc_len);

        // Update average doc length
        let n = self.docs.len() as f64;
        self.avg_dl = ((self.avg_dl * (n - 1.0)) + doc_len) / n;
    }

    pub fn search(&self, query: &str, top_k: usize) -> Vec<(String, f64)> {
        let query_tokens = tokenize(query);
        let n = self.docs.len() as f64;
        let mut scores: HashMap<usize, f64> = HashMap::new();

        for token in &query_tokens {
            if let Some(postings) = self.inverted_index.get(token.as_str()) {
                let df = postings.len() as f64;
                let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();

                for &(doc_idx, tf) in postings {
                    let doc_len = self.doc_lengths[doc_idx];
                    let tf_norm =
                        (tf * (self.k1 + 1.0)) / (tf + self.k1 * (1.0 - self.b + self.b * doc_len / self.avg_dl));
                    *scores.entry(doc_idx).or_default() += idf * tf_norm;
                }
            }
        }

        let mut results: Vec<(usize, f64)> = scores
            .into_iter()
            .filter(|(idx, _)| !self.deleted.contains(idx))
            .collect();
        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

        results
            .into_iter()
            .take(top_k)
            .map(|(idx, score)| (self.docs[idx].0.clone(), score))
            .collect()
    }

    /// Retrieve the stored content for a document by its ID. O(1) via HashMap.
    pub fn get_doc_content(&self, doc_id: &str) -> Option<&str> {
        self.doc_id_index
            .get(doc_id)
            .filter(|idx| !self.deleted.contains(idx))
            .map(|&idx| self.docs[idx].1.as_str())
    }

    /// Soft-delete a document by doc_id. Returns true if found and marked.
    pub fn mark_deleted(&mut self, doc_id: &str) -> bool {
        if let Some(&idx) = self.doc_id_index.get(doc_id) {
            self.deleted.insert(idx);
            true
        } else {
            false
        }
    }

    pub fn len(&self) -> usize {
        self.docs.len() - self.deleted.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Suggest a spelling correction for the query using the index vocabulary.
    ///
    /// Each out-of-vocabulary token of 5+ chars is corrected to the most
    /// document-frequent in-vocab term within edit distance 1 (or 2 for 8+
    /// char tokens) — e.g. "javascrpt" → "javascript", "promisses" →
    /// "promises". Candidates must appear in >= 3 documents so rare/junk
    /// terms never attract corrections. Returns `Some(corrected_query)` only
    /// if at least one token changed; in-vocab tokens are never touched, so
    /// novel-but-real terms survive as long as they've been indexed anywhere.
    pub fn correct_query(&self, query: &str) -> Option<String> {
        const MIN_TOKEN_LEN: usize = 5;

        // Tokens of the query that ARE in the vocabulary — used as context
        // for the co-occurrence guard below.
        let context_tokens: Vec<String> = tokenize(query)
            .into_iter()
            .filter(|t| self.inverted_index.contains_key(t))
            .collect();

        let mut changed = false;
        let corrected: Vec<String> = query
            .split_whitespace()
            .map(|word| {
                let token: String = word
                    .to_lowercase()
                    .chars()
                    .filter(|c| c.is_alphanumeric())
                    .collect();
                if token.len() < MIN_TOKEN_LEN
                    || token.chars().any(|c| c.is_numeric())
                    || self.inverted_index.contains_key(&token)
                {
                    return word.to_string();
                }
                match self.nearest_term(&token, &context_tokens) {
                    Some(term) => {
                        changed = true;
                        term
                    }
                    None => word.to_string(),
                }
            })
            .collect();

        changed.then(|| corrected.join(" "))
    }

    /// Nearest vocabulary term to `token` under strict guards tuned against
    /// false positives (a small corpus makes those cheap and costly:
    /// "tokio" → "tokyo", "mechanical" → "technical" both shipped before
    /// these guards existed):
    /// - edit distance exactly 1 (distance-2 corrections are net-negative);
    /// - same first letter (real typos rarely miss the first letter);
    /// - document frequency >= 2;
    /// - CO-OCCURRENCE: when the query has other in-vocab tokens, the
    ///   candidate must share at least one document with one of them —
    ///   "tokyo" never appears near "rust"/"async", "javascript" constantly
    ///   appears near "async", so context kills the impostor.
    ///
    /// Ties break by higher document frequency. Linear scan with length
    /// pruning — fine for corpora up to a few hundred thousand terms.
    fn nearest_term(&self, token: &str, context_tokens: &[String]) -> Option<String> {
        const MIN_DF: usize = 2;
        let tok_len = token.len();
        let first_char = token.chars().next()?;
        let mut best: Option<(&str, usize)> = None; // (term, df)
        for (term, postings) in &self.inverted_index {
            let df = postings.len();
            if df < MIN_DF
                || term.len().abs_diff(tok_len) > 1
                || !term.starts_with(first_char)
            {
                continue;
            }
            if bounded_levenshtein(token, term, 1) != Some(1) {
                continue;
            }
            if !context_tokens.is_empty() && !self.co_occurs(term, context_tokens) {
                continue;
            }
            if best.is_none_or(|(_, bdf)| df > bdf) {
                best = Some((term, df));
            }
        }
        best.map(|(term, _)| term.to_string())
    }

    /// Does `term` share at least one document with any of `context_tokens`?
    fn co_occurs(&self, term: &str, context_tokens: &[String]) -> bool {
        let Some(term_postings) = self.inverted_index.get(term) else {
            return false;
        };
        let term_docs: HashSet<usize> = term_postings.iter().map(|&(d, _)| d).collect();
        context_tokens.iter().any(|ctx| {
            self.inverted_index
                .get(ctx)
                .is_some_and(|p| p.iter().any(|&(d, _)| term_docs.contains(&d)))
        })
    }
}

/// Levenshtein distance if it is <= `max`, else None (early-exit banded DP).
fn bounded_levenshtein(a: &str, b: &str, max: usize) -> Option<usize> {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.len().abs_diff(b.len()) > max {
        return None;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0usize; b.len() + 1];
    for i in 1..=a.len() {
        curr[0] = i;
        let mut row_min = curr[0];
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
            row_min = row_min.min(curr[j]);
        }
        if row_min > max {
            return None;
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    (prev[b.len()] <= max).then_some(prev[b.len()])
}

impl Default for Bm25Index {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Serialize, Deserialize)]
struct Bm25Snapshot {
    docs: Vec<(String, String)>,
    doc_lengths: Vec<f64>,
    inverted_index: HashMap<String, Vec<(usize, f64)>>,
    avg_dl: f64,
    k1: f64,
    b: f64,
    #[serde(default)]
    deleted: Vec<usize>,
}

impl Bm25Index {
    /// Serialize the entire index to bytes using bincode.
    pub fn to_bytes(&self) -> Vec<u8> {
        let snapshot = Bm25Snapshot {
            docs: self.docs.clone(),
            doc_lengths: self.doc_lengths.clone(),
            inverted_index: self.inverted_index.clone(),
            avg_dl: self.avg_dl,
            k1: self.k1,
            b: self.b,
            deleted: self.deleted.iter().copied().collect(),
        };
        bincode::serialize(&snapshot).expect("BM25 serialization should not fail")
    }

    /// Deserialize an index from bytes produced by `to_bytes()`.
    /// Backward compatible: older snapshots without `deleted` field default to empty.
    pub fn from_bytes(data: &[u8]) -> Result<Self, bincode::Error> {
        let snapshot: Bm25Snapshot = bincode::deserialize(data)?;
        // Rebuild the doc_id_index from the deserialized docs
        let doc_id_index: HashMap<String, usize> = snapshot
            .docs
            .iter()
            .enumerate()
            .map(|(idx, (id, _))| (id.clone(), idx))
            .collect();
        Ok(Self {
            docs: snapshot.docs,
            doc_id_index,
            doc_lengths: snapshot.doc_lengths,
            inverted_index: snapshot.inverted_index,
            avg_dl: snapshot.avg_dl,
            k1: snapshot.k1,
            b: snapshot.b,
            deleted: snapshot.deleted.into_iter().collect(),
        })
    }
}

fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty() && s.len() > 1)
        .map(String::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_and_search() {
        let mut idx = Bm25Index::new();
        idx.insert("doc1".into(), "the quick brown fox jumps over the lazy dog".into());
        idx.insert("doc2".into(), "a fast brown car races past the sleeping cat".into());
        idx.insert("doc3".into(), "quantum computing research paper on algorithms".into());

        let results = idx.search("brown fox", 10);
        assert!(!results.is_empty());
        assert_eq!(results[0].0, "doc1");
    }

    #[test]
    fn test_exact_match() {
        let mut idx = Bm25Index::new();
        idx.insert("doc1".into(), "opensonarx decentralized search protocol".into());
        idx.insert("doc2".into(), "recipe for chocolate cake baking tips".into());

        let results = idx.search("opensonarx", 5);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "doc1");
    }

    #[test]
    fn test_no_results() {
        let mut idx = Bm25Index::new();
        idx.insert("doc1".into(), "hello world".into());

        let results = idx.search("nonexistent", 5);
        assert!(results.is_empty());
    }

    #[test]
    fn test_empty_index() {
        let idx = Bm25Index::new();
        let results = idx.search("anything", 5);
        assert!(results.is_empty());
    }

    #[test]
    fn test_tokenize() {
        let tokens = tokenize("Hello, World! This is a test.");
        assert!(tokens.contains(&"hello".to_string()));
        assert!(tokens.contains(&"world".to_string()));
        assert!(tokens.contains(&"test".to_string()));
        // Single-char tokens filtered out
        assert!(!tokens.contains(&"a".to_string()));
    }

    #[test]
    fn test_serialization_roundtrip() {
        let mut idx = Bm25Index::new();
        idx.insert("doc1".into(), "the quick brown fox jumps over the lazy dog".into());
        idx.insert("doc2".into(), "a fast brown car races past the sleeping cat".into());
        idx.insert("doc3".into(), "quantum computing research paper on algorithms".into());

        let bytes = idx.to_bytes();
        let restored = Bm25Index::from_bytes(&bytes).unwrap();

        assert_eq!(restored.len(), 3);

        let orig_results = idx.search("brown fox", 10);
        let restored_results = restored.search("brown fox", 10);
        assert_eq!(orig_results.len(), restored_results.len());
        assert_eq!(orig_results[0].0, restored_results[0].0);
    }

    #[test]
    fn test_serialization_empty() {
        let idx = Bm25Index::new();
        let bytes = idx.to_bytes();
        let restored = Bm25Index::from_bytes(&bytes).unwrap();
        assert!(restored.is_empty());
    }

    #[test]
    fn test_get_doc_content() {
        let mut idx = Bm25Index::new();
        idx.insert("doc1".into(), "the quick brown fox".into());
        idx.insert("doc2".into(), "quantum computing research".into());

        assert_eq!(idx.get_doc_content("doc1"), Some("the quick brown fox"));
        assert_eq!(idx.get_doc_content("doc2"), Some("quantum computing research"));
        assert_eq!(idx.get_doc_content("nonexistent"), None);
    }

    #[test]
    fn test_get_doc_content_after_deserialize() {
        let mut idx = Bm25Index::new();
        idx.insert("doc1".into(), "hello world content".into());

        let bytes = idx.to_bytes();
        let restored = Bm25Index::from_bytes(&bytes).unwrap();

        assert_eq!(restored.get_doc_content("doc1"), Some("hello world content"));
        assert_eq!(restored.get_doc_content("nonexistent"), None);
    }

    /// Index where "javascript" and "promises" appear in 3+ docs (correction
    /// candidates require df >= 3).
    fn spell_test_index() -> Bm25Index {
        let mut idx = Bm25Index::new();
        for i in 0..4 {
            idx.insert(
                format!("js{i}"),
                "javascript promises make async programming easier".into(),
            );
            idx.insert(format!("py{i}"), "python asyncio event loop".into());
        }
        idx
    }

    #[test]
    fn test_correct_query_typos() {
        let idx = spell_test_index();
        let corrected = idx.correct_query("javascrpt promisses async").unwrap();
        assert_eq!(corrected, "javascript promises async");
    }

    #[test]
    fn test_correct_query_cooccurrence_guard() {
        // "tokio" must NOT correct to "tokyo": tokyo never co-occurs with the
        // query's other terms (rust/async), even though it's 1 edit away.
        let mut idx = Bm25Index::new();
        for i in 0..4 {
            idx.insert(format!("rust{i}"), "rust async runtime programming".into());
            idx.insert(format!("city{i}"), "tokyo is the capital of japan".into());
        }
        assert_eq!(idx.correct_query("rust async runtime tokio"), None);
        // But with no in-vocab context, a lone plausible typo still corrects.
        assert_eq!(idx.correct_query("tokyoo").as_deref(), Some("tokyo"));
    }

    #[test]
    fn test_correct_query_first_letter_and_dist1_guards() {
        let idx = spell_test_index();
        // distance 2 → never corrected (was "mechanical" → "technical")
        assert_eq!(idx.correct_query("javascrp"), None);
        // different first letter → never corrected
        assert_eq!(idx.correct_query("javascrpt xromises").as_deref(), Some("javascript xromises"));
    }

    #[test]
    fn test_correct_query_no_change_when_in_vocab() {
        let idx = spell_test_index();
        assert_eq!(idx.correct_query("javascript promises"), None);
    }

    #[test]
    fn test_correct_query_leaves_unknown_terms() {
        // "kubernetes" has no near neighbor in vocab — must NOT be "corrected".
        let idx = spell_test_index();
        assert_eq!(idx.correct_query("kubernetes"), None);
    }

    #[test]
    fn test_correct_query_ignores_short_and_rare() {
        let mut idx = spell_test_index();
        // "zebra" appears in only 1 doc — below df threshold, attracts nothing.
        idx.insert("z".into(), "zebra stripes".into());
        assert_eq!(idx.correct_query("zebro"), None);
        // short tokens untouched
        assert_eq!(idx.correct_query("pyth"), None);
    }

    #[test]
    fn test_bounded_levenshtein() {
        assert_eq!(bounded_levenshtein("javascrpt", "javascript", 2), Some(1));
        assert_eq!(bounded_levenshtein("promisses", "promises", 2), Some(1));
        assert_eq!(bounded_levenshtein("abc", "xyz", 2), None);
        assert_eq!(bounded_levenshtein("same", "same", 1), Some(0));
    }

    #[test]
    fn test_doc_lengths_stored() {
        let mut idx = Bm25Index::new();
        idx.insert("d1".into(), "one two three".into());
        idx.insert("d2".into(), "alpha beta gamma delta epsilon".into());
        assert_eq!(idx.doc_lengths.len(), 2);
        assert_eq!(idx.doc_lengths[0], 3.0); // "one", "two", "three"
        assert_eq!(idx.doc_lengths[1], 5.0); // "alpha", "beta", "gamma", "delta", "epsilon"
    }
}
