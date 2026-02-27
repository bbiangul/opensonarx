use sha2::{Digest, Sha256};

/// Deterministic text → 384-dim embedding using SHA-256 hash as PRNG seed.
/// Same text always produces the same vector.
pub fn text_to_embedding(text: &str, dim: usize) -> Vec<f32> {
    let hash = Sha256::digest(text.as_bytes());
    let mut seed = u64::from_le_bytes(hash[0..8].try_into().unwrap());

    let mut v = vec![0.0f32; dim];
    for x in &mut v {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        *x = (seed as f64 / u64::MAX as f64) as f32 * 2.0 - 1.0;
    }

    // Normalize
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 1e-10 {
        for x in &mut v {
            *x /= norm;
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic() {
        let a = text_to_embedding("rust programming", 384);
        let b = text_to_embedding("rust programming", 384);
        assert_eq!(a, b);
    }

    #[test]
    fn different_texts_differ() {
        let a = text_to_embedding("rust", 384);
        let b = text_to_embedding("python", 384);
        assert_ne!(a, b);
    }

    #[test]
    fn normalized() {
        let v = text_to_embedding("test query", 384);
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 0.01);
    }
}
