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
