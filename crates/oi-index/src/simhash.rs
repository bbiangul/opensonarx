//! SimHash: locality-sensitive hashing for near-duplicate text detection.
//!
//! Maps similar text to similar 64-bit hashes. Two documents with
//! Hamming distance <= 8 (out of 64 bits) are near-duplicates.
//!
//! `SimHashIndex` uses band-based LSH (Locality-Sensitive Hashing) for
//! O(1) average-case duplicate lookups instead of O(n) linear scan.

use std::collections::HashMap;

/// Compute a 64-bit SimHash of the given text using character trigrams.
///
/// Character trigrams are language-independent and catch partial overlaps
/// in rephrased text (e.g. "the quick brown fox" vs "a fast brown fox").
pub fn simhash(text: &str) -> u64 {
    let text = text.to_lowercase();
    let chars: Vec<char> = text.chars().collect();

    if chars.len() < 3 {
        // For very short text, hash the whole thing
        return fnv1a(text.as_bytes());
    }

    let mut v = [0i32; 64];

    for window in chars.windows(3) {
        let trigram: String = window.iter().collect();
        let hash = fnv1a(trigram.as_bytes());

        for (bit, count) in v.iter_mut().enumerate() {
            if (hash >> bit) & 1 == 1 {
                *count += 1;
            } else {
                *count -= 1;
            }
        }
    }

    let mut fingerprint: u64 = 0;
    for (bit, count) in v.iter().enumerate() {
        if *count > 0 {
            fingerprint |= 1u64 << bit;
        }
    }
    fingerprint
}

/// Hamming distance between two 64-bit hashes (number of differing bits).
pub fn hamming_distance(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// Check if a new document is a near-duplicate of any existing document.
/// Returns true if a near-duplicate is found (should reject).
///
/// O(n) linear scan — kept for backward compatibility and tests.
/// For production use, prefer `SimHashIndex::is_near_duplicate()`.
pub fn is_near_duplicate(new_hash: u64, existing: &[u64], threshold: u32) -> bool {
    existing.iter().any(|&h| hamming_distance(new_hash, h) <= threshold)
}

/// Default near-duplicate threshold: 8 bits out of 64.
///
/// Character trigram SimHash produces Hamming distances of 4-7 for minor
/// word swaps and rephrasing, ~10 for same-topic-but-different articles,
/// and 16+ for genuinely different content. A threshold of 8 catches
/// paraphrased near-duplicates while allowing distinct articles on the
/// same topic through.
pub const DEFAULT_THRESHOLD: u32 = 8;

/// Number of bands for LSH lookup.
const NUM_BANDS: usize = 8;
/// Bits per band (64 / 8 = 8).
const BITS_PER_BAND: usize = 8;

/// Band-based LSH index for O(1) average-case near-duplicate detection.
///
/// Splits each 64-bit SimHash into 8 bands of 8 bits each. Two hashes with
/// Hamming distance <= 8 share at least one identical 8-bit band with high
/// probability. Lookup checks only candidates from matching bands, yielding
/// O(b * avg_bucket_size) instead of O(n).
pub struct SimHashIndex {
    /// 8 hash tables, one per band. Key = 8-bit band value, Value = list of full hashes.
    bands: [HashMap<u8, Vec<u64>>; NUM_BANDS],
    /// All stored hashes (for serialization)
    hashes: Vec<u64>,
}

impl SimHashIndex {
    pub fn new() -> Self {
        Self {
            bands: std::array::from_fn(|_| HashMap::new()),
            hashes: Vec::new(),
        }
    }

    /// Build a SimHashIndex from a flat list of hashes (for backward-compatible deserialization).
    pub fn from_hashes(hashes: Vec<u64>) -> Self {
        let mut index = Self::new();
        for &h in &hashes {
            index.insert_into_bands(h);
        }
        index.hashes = hashes;
        index
    }

    /// Insert a hash into the index.
    pub fn insert(&mut self, hash: u64) {
        self.insert_into_bands(hash);
        self.hashes.push(hash);
    }

    fn insert_into_bands(&mut self, hash: u64) {
        for (band_idx, table) in self.bands.iter_mut().enumerate() {
            let band_val = extract_band(hash, band_idx);
            table.entry(band_val).or_default().push(hash);
        }
    }

    /// Check if a hash is a near-duplicate of any stored hash.
    ///
    /// Collects candidate hashes from all bands where the band bits match,
    /// then checks only those candidates for Hamming distance <= threshold.
    /// Average case: O(b * avg_bucket) instead of O(n).
    pub fn is_near_duplicate(&self, hash: u64, threshold: u32) -> bool {
        // Collect candidate hashes from matching bands (deduplicated via early return)
        for (band_idx, table) in self.bands.iter().enumerate() {
            let band_val = extract_band(hash, band_idx);
            if let Some(candidates) = table.get(&band_val) {
                for &candidate in candidates {
                    if hamming_distance(hash, candidate) <= threshold {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Number of stored hashes.
    pub fn len(&self) -> usize {
        self.hashes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.hashes.is_empty()
    }

    /// Export as flat Vec<u64> for serialization (backward-compatible).
    pub fn to_hashes(&self) -> &[u64] {
        &self.hashes
    }
}

impl Default for SimHashIndex {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract the 8-bit band value at the given band index from a 64-bit hash.
#[inline]
fn extract_band(hash: u64, band_idx: usize) -> u8 {
    let shift = band_idx * BITS_PER_BAND;
    ((hash >> shift) & 0xFF) as u8
}

/// FNV-1a 64-bit hash.
fn fnv1a(data: &[u8]) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x00000100000001B3;

    let mut hash = FNV_OFFSET;
    for &byte in data {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identical_text_same_hash() {
        let text = "The quick brown fox jumps over the lazy dog";
        assert_eq!(simhash(text), simhash(text));
    }

    #[test]
    fn test_similar_text_low_hamming() {
        let a = "Rust is a systems programming language focused on safety and performance";
        let b = "Rust is a systems programming language focused on safety and speed";
        let ha = simhash(a);
        let hb = simhash(b);
        let dist = hamming_distance(ha, hb);
        // Single word swap yields ~7 bits difference, well within threshold of 10
        assert!(
            dist <= DEFAULT_THRESHOLD,
            "similar text should have Hamming distance <= {DEFAULT_THRESHOLD}, got {dist}"
        );
    }

    #[test]
    fn test_different_text_high_hamming() {
        let a = "Rust is a systems programming language focused on safety and performance";
        let b = "The global economy shows signs of recovery as inflation rates decline across major markets";
        let ha = simhash(a);
        let hb = simhash(b);
        let dist = hamming_distance(ha, hb);
        assert!(
            dist > DEFAULT_THRESHOLD,
            "completely different text should have Hamming distance > {DEFAULT_THRESHOLD}, got {dist}"
        );
    }

    #[test]
    fn test_empty_text() {
        let h = simhash("");
        assert_eq!(h, simhash(""));
    }

    #[test]
    fn test_short_text() {
        let h1 = simhash("ab");
        let h2 = simhash("ab");
        assert_eq!(h1, h2);

        let h3 = simhash("xy");
        assert_ne!(h1, h3);
    }

    #[test]
    fn test_hamming_distance_identical() {
        assert_eq!(hamming_distance(0, 0), 0);
        assert_eq!(hamming_distance(u64::MAX, u64::MAX), 0);
    }

    #[test]
    fn test_hamming_distance_one_bit() {
        assert_eq!(hamming_distance(0, 1), 1);
        assert_eq!(hamming_distance(0b1010, 0b1011), 1);
    }

    #[test]
    fn test_hamming_distance_all_bits() {
        assert_eq!(hamming_distance(0, u64::MAX), 64);
    }

    #[test]
    fn test_is_near_duplicate_found() {
        let text1 = "The quick brown fox jumps over the lazy dog";
        let text2 = "The quick brown fox leaps over the lazy dog";
        let h1 = simhash(text1);
        let h2 = simhash(text2);
        assert!(is_near_duplicate(h2, &[h1], DEFAULT_THRESHOLD));
    }

    #[test]
    fn test_is_near_duplicate_not_found() {
        let text1 = "Rust programming language systems safety concurrency";
        let text2 = "Global climate change impacts ocean temperatures significantly";
        let h1 = simhash(text1);
        let h2 = simhash(text2);
        assert!(!is_near_duplicate(h2, &[h1], DEFAULT_THRESHOLD));
    }

    #[test]
    fn test_is_near_duplicate_empty_existing() {
        let h = simhash("any text");
        assert!(!is_near_duplicate(h, &[], DEFAULT_THRESHOLD));
    }

    #[test]
    fn test_case_insensitive() {
        let a = "Rust Programming Language";
        let b = "rust programming language";
        assert_eq!(simhash(a), simhash(b));
    }

    #[test]
    fn test_rephrased_content_detected() {
        let original = "Bitcoin has surged to new highs as institutional investors pour money into cryptocurrency markets. The price of Bitcoin topped fifty thousand dollars for the first time since the bull run of twenty twenty one.";
        let rephrased = "Bitcoin surged to new highs as institutional investors poured capital into crypto markets. Bitcoin's price topped fifty thousand dollars for the first time since the twenty twenty one bull run.";
        let ha = simhash(original);
        let hb = simhash(rephrased);
        let dist = hamming_distance(ha, hb);
        assert!(
            dist <= DEFAULT_THRESHOLD,
            "rephrased content should be detected as near-duplicate, got Hamming distance {dist}"
        );
    }

    // ── SimHashIndex tests ──

    #[test]
    fn test_simhash_index_basic() {
        let mut idx = SimHashIndex::new();
        let h1 = simhash("Rust is a systems programming language focused on safety and performance");
        idx.insert(h1);

        // Near-duplicate should be found
        let h2 = simhash("Rust is a systems programming language focused on safety and speed");
        assert!(idx.is_near_duplicate(h2, DEFAULT_THRESHOLD));

        // Different text should not be found
        let h3 = simhash("The global economy shows signs of recovery as inflation rates decline across major markets");
        assert!(!idx.is_near_duplicate(h3, DEFAULT_THRESHOLD));
    }

    #[test]
    fn test_simhash_index_empty() {
        let idx = SimHashIndex::new();
        let h = simhash("any text here");
        assert!(!idx.is_near_duplicate(h, DEFAULT_THRESHOLD));
        assert!(idx.is_empty());
    }

    #[test]
    fn test_simhash_index_from_hashes() {
        let h1 = simhash("The quick brown fox jumps over the lazy dog");
        let h2 = simhash("Python is great for data science");
        let idx = SimHashIndex::from_hashes(vec![h1, h2]);

        assert_eq!(idx.len(), 2);

        // Near-duplicate of h1
        let h3 = simhash("The quick brown fox leaps over the lazy dog");
        assert!(idx.is_near_duplicate(h3, DEFAULT_THRESHOLD));
    }

    #[test]
    fn test_simhash_index_to_hashes_roundtrip() {
        let mut idx = SimHashIndex::new();
        let hashes: Vec<u64> = (0..100)
            .map(|i| simhash(&format!("unique document number {i} about topic {}", i * 7 + 3)))
            .collect();
        for &h in &hashes {
            idx.insert(h);
        }

        // Rebuild from exported hashes
        let exported = idx.to_hashes().to_vec();
        let idx2 = SimHashIndex::from_hashes(exported);
        assert_eq!(idx2.len(), hashes.len());

        // Same duplicate detection behavior
        let test_hash = hashes[0];
        assert!(idx2.is_near_duplicate(test_hash, DEFAULT_THRESHOLD));
    }

    #[test]
    fn test_extract_band() {
        let hash: u64 = 0xAABBCCDDEEFF0011;
        // Band 0 = lowest 8 bits = 0x11
        assert_eq!(extract_band(hash, 0), 0x11);
        // Band 1 = bits 8-15 = 0x00
        assert_eq!(extract_band(hash, 1), 0x00);
        // Band 7 = highest 8 bits = 0xAA
        assert_eq!(extract_band(hash, 7), 0xAA);
    }

    #[test]
    fn test_simhash_index_many_inserts() {
        let mut idx = SimHashIndex::new();
        // Insert 10K distinct hashes
        for i in 0..10_000 {
            let h = simhash(&format!("completely unique document about subject number {i} with extra words {}", i * 13 + 7));
            idx.insert(h);
        }
        assert_eq!(idx.len(), 10_000);

        // Query should still be fast and correct
        let test = simhash("completely unique document about subject number 42 with extra words 553");
        assert!(idx.is_near_duplicate(test, DEFAULT_THRESHOLD));
    }
}
