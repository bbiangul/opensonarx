//! INT8 scalar quantization for ~4x memory reduction.
//!
//! Maps f32 vectors to i8 with stored scale/offset for approximate reconstruction.
//! Useful for large HNSW indices where memory is the bottleneck.

/// A quantized representation of a f32 vector using INT8 (i8) values.
pub struct QuantizedVector {
    pub data: Vec<i8>,
    pub scale: f32,
    pub offset: f32,
}

impl QuantizedVector {
    /// Quantize a f32 vector to INT8.
    /// Maps [min, max] -> [-128, 127]
    pub fn from_f32(v: &[f32]) -> Self {
        let min = v.iter().cloned().fold(f32::INFINITY, f32::min);
        let max = v.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        let range = max - min;
        let scale = if range < 1e-10 { 1.0 } else { 255.0 / range };
        let offset = min;

        let data: Vec<i8> = v
            .iter()
            .map(|&x| {
                ((x - offset) * scale - 128.0)
                    .round()
                    .clamp(-128.0, 127.0) as i8
            })
            .collect();

        Self {
            data,
            scale,
            offset,
        }
    }

    /// Dequantize back to f32 (approximate reconstruction).
    pub fn to_f32(&self) -> Vec<f32> {
        self.data
            .iter()
            .map(|&x| (x as f32 + 128.0) / self.scale + self.offset)
            .collect()
    }

    /// Compute approximate cosine distance using quantized vectors.
    /// Returns a value in [0, 2] where 0 = identical, 1 = orthogonal, 2 = opposite.
    pub fn cosine_distance(a: &QuantizedVector, b: &QuantizedVector) -> f32 {
        let a_f32 = a.to_f32();
        let b_f32 = b.to_f32();

        let mut dot = 0.0f32;
        let mut norm_a = 0.0f32;
        let mut norm_b = 0.0f32;
        for i in 0..a_f32.len() {
            dot += a_f32[i] * b_f32[i];
            norm_a += a_f32[i] * a_f32[i];
            norm_b += b_f32[i] * b_f32[i];
        }
        let denom = norm_a.sqrt() * norm_b.sqrt();
        if denom < 1e-10 {
            1.0
        } else {
            1.0 - dot / denom
        }
    }

    /// Memory ratio: quantized size / original f32 size.
    /// For a 384-dim vector this is roughly 0.27 (~4x reduction).
    pub fn memory_ratio(dim: usize) -> f64 {
        // f32 = 4 bytes per element, i8 = 1 byte per element
        // plus 8 bytes overhead for scale (4 bytes) + offset (4 bytes)
        let original = dim * 4;
        let quantized = dim + 8;
        quantized as f64 / original as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quantize_roundtrip() {
        let original: Vec<f32> = (0..128).map(|i| (i as f32) / 128.0 - 0.5).collect();
        let quantized = QuantizedVector::from_f32(&original);
        let reconstructed = quantized.to_f32();

        assert_eq!(original.len(), reconstructed.len());
        let max_err = original
            .iter()
            .zip(reconstructed.iter())
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(
            max_err < 0.05,
            "max roundtrip error {max_err} exceeds 0.05"
        );
    }

    #[test]
    fn test_quantize_cosine_accuracy() {
        let a: Vec<f32> = (0..384).map(|i| ((i as f32) * 0.01).sin()).collect();
        let b: Vec<f32> = (0..384).map(|i| ((i as f32) * 0.02).cos()).collect();

        // f32 cosine distance
        let mut dot = 0.0f32;
        let mut na = 0.0f32;
        let mut nb = 0.0f32;
        for i in 0..a.len() {
            dot += a[i] * b[i];
            na += a[i] * a[i];
            nb += b[i] * b[i];
        }
        let f32_dist = 1.0 - dot / (na.sqrt() * nb.sqrt());

        // quantized cosine distance
        let qa = QuantizedVector::from_f32(&a);
        let qb = QuantizedVector::from_f32(&b);
        let q_dist = QuantizedVector::cosine_distance(&qa, &qb);

        let err = (f32_dist - q_dist).abs();
        assert!(
            err < 0.02,
            "cosine distance error {err} exceeds 0.02 (f32={f32_dist}, quantized={q_dist})"
        );
    }

    #[test]
    fn test_memory_ratio() {
        let ratio = QuantizedVector::memory_ratio(384);
        // (384 + 8) / (384 * 4) = 392 / 1536 ≈ 0.255
        assert!(
            ratio < 0.30,
            "memory ratio {ratio} should be roughly 0.27 for 384-dim"
        );
        assert!(
            ratio > 0.20,
            "memory ratio {ratio} is unexpectedly low"
        );
    }

    #[test]
    fn test_quantize_uniform() {
        // Uniform distribution across [-1, 1]
        let original: Vec<f32> = (0..256)
            .map(|i| (i as f32 / 128.0) - 1.0)
            .collect();
        let quantized = QuantizedVector::from_f32(&original);
        let reconstructed = quantized.to_f32();

        let max_err = original
            .iter()
            .zip(reconstructed.iter())
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(
            max_err < 0.05,
            "uniform distribution max error {max_err} exceeds 0.05"
        );
    }
}
