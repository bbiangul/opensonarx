use sha2::{Digest, Sha256};

/// Compute a SHA-256 hash of the content string.
pub fn compute_content_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Compute cosine similarity between two embedding vectors.
/// Returns 0.0 if either vector is zero-length or has zero magnitude.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let mut dot = 0.0f64;
    let mut mag_a = 0.0f64;
    let mut mag_b = 0.0f64;

    for (x, y) in a.iter().zip(b.iter()) {
        let x = *x as f64;
        let y = *y as f64;
        dot += x * y;
        mag_a += x * x;
        mag_b += y * y;
    }

    let denom = mag_a.sqrt() * mag_b.sqrt();
    if denom == 0.0 {
        return 0.0;
    }

    (dot / denom) as f32
}

/// Determine if content has drifted between two versions.
///
/// Logic:
/// - If content hashes are identical, no drift.
/// - If embeddings are available, compute cosine similarity. If similarity < threshold, drifted.
/// - If no embeddings, any hash difference means drifted.
pub fn has_drifted(
    old_hash: &str,
    new_hash: &str,
    old_embedding: Option<&[f32]>,
    new_embedding: Option<&[f32]>,
    threshold: f32,
) -> bool {
    // Same content hash → no drift
    if old_hash == new_hash {
        return false;
    }

    // If both embeddings available, use cosine similarity
    if let (Some(old_emb), Some(new_emb)) = (old_embedding, new_embedding) {
        let sim = cosine_similarity(old_emb, new_emb);
        return sim < threshold;
    }

    // Hash differs, no embeddings to compare → drifted
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_same_content_no_drift() {
        let content = "Hello, world!";
        let hash1 = compute_content_hash(content);
        let hash2 = compute_content_hash(content);
        assert!(!has_drifted(&hash1, &hash2, None, None, 0.85));
    }

    #[test]
    fn test_different_content_drift() {
        let hash1 = compute_content_hash("Hello, world!");
        let hash2 = compute_content_hash("Completely different content");
        assert!(has_drifted(&hash1, &hash2, None, None, 0.85));
    }

    #[test]
    fn test_similar_embeddings_no_drift() {
        let hash1 = compute_content_hash("version 1");
        let hash2 = compute_content_hash("version 2");

        // Nearly identical embeddings (high similarity)
        let emb1 = vec![1.0, 0.0, 0.0, 0.0];
        let emb2 = vec![0.99, 0.01, 0.0, 0.0];

        let sim = cosine_similarity(&emb1, &emb2);
        assert!(sim > 0.85, "similarity should be > 0.85, got {sim}");
        assert!(!has_drifted(&hash1, &hash2, Some(&emb1), Some(&emb2), 0.85));
    }

    #[test]
    fn test_dissimilar_embeddings_drift() {
        let hash1 = compute_content_hash("version 1");
        let hash2 = compute_content_hash("version 2");

        // Orthogonal embeddings (zero similarity)
        let emb1 = vec![1.0, 0.0, 0.0, 0.0];
        let emb2 = vec![0.0, 1.0, 0.0, 0.0];

        let sim = cosine_similarity(&emb1, &emb2);
        assert!(sim < 0.85, "similarity should be < 0.85, got {sim}");
        assert!(has_drifted(&hash1, &hash2, Some(&emb1), Some(&emb2), 0.85));
    }

    #[test]
    fn test_cosine_similarity_identical() {
        let emb = vec![1.0, 2.0, 3.0];
        let sim = cosine_similarity(&emb, &emb);
        assert!((sim - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_mismatched_lengths() {
        let a = vec![1.0, 2.0];
        let b = vec![1.0, 2.0, 3.0];
        let sim = cosine_similarity(&a, &b);
        assert_eq!(sim, 0.0);
    }

    #[test]
    fn test_cosine_similarity_empty() {
        let sim = cosine_similarity(&[], &[]);
        assert_eq!(sim, 0.0);
    }

    #[test]
    fn test_compute_content_hash_deterministic() {
        let h1 = compute_content_hash("test content");
        let h2 = compute_content_hash("test content");
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_compute_content_hash_different() {
        let h1 = compute_content_hash("content a");
        let h2 = compute_content_hash("content b");
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_same_hash_with_embeddings_still_no_drift() {
        let hash = compute_content_hash("same");
        // Even with very different embeddings, same hash means no drift
        let emb1 = vec![1.0, 0.0];
        let emb2 = vec![0.0, 1.0];
        assert!(!has_drifted(&hash, &hash, Some(&emb1), Some(&emb2), 0.85));
    }

    #[test]
    fn test_partial_embeddings_falls_back_to_hash() {
        let hash1 = compute_content_hash("v1");
        let hash2 = compute_content_hash("v2");
        let emb = vec![1.0, 0.0];
        // Only one embedding available → falls back to hash comparison → drifted
        assert!(has_drifted(&hash1, &hash2, Some(&emb), None, 0.85));
        assert!(has_drifted(&hash1, &hash2, None, Some(&emb), 0.85));
    }
}
