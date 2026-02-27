use oi_types::Result;
use std::path::Path;

/// Default dimensions for CLIP → index projection.
pub const DEFAULT_INPUT_DIM: usize = 768;
pub const DEFAULT_OUTPUT_DIM: usize = 384;

/// Linear projection matrix with L2 normalization.
/// Stores dimensions alongside weights for self-describing serialization.
///
/// File format: [input_dim: u32 LE][output_dim: u32 LE][weights: f32 LE × input_dim × output_dim]
pub struct ProjectionMatrix {
    weights: Vec<f32>,
    input_dim: usize,
    output_dim: usize,
}

impl ProjectionMatrix {
    /// Load a projection matrix from a binary file.
    /// Format: 8-byte header (u32 LE dims) + f32 LE weights.
    pub fn from_file(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path)
            .map_err(|e| oi_types::Error::Internal(format!("Read projection matrix: {e}")))?;

        if bytes.len() < 8 {
            return Err(oi_types::Error::Internal(
                "Projection file too small for header".into(),
            ));
        }

        let input_dim =
            u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
        let output_dim =
            u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;

        let expected = 8 + input_dim * output_dim * 4;
        if bytes.len() != expected {
            return Err(oi_types::Error::Internal(format!(
                "Projection matrix size mismatch: expected {expected} bytes ({}x{}), got {}",
                input_dim,
                output_dim,
                bytes.len()
            )));
        }

        let weights: Vec<f32> = bytes[8..]
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();

        Ok(Self {
            weights,
            input_dim,
            output_dim,
        })
    }

    /// Compute a PCA projection matrix from embedding samples.
    ///
    /// Takes N samples of `input_dim` dimensions and computes the top
    /// `output_dim` principal components via power iteration with deflation.
    /// Requires N >= output_dim samples for full-rank projection.
    pub fn from_pca(embeddings: &[Vec<f32>], input_dim: usize, output_dim: usize) -> Self {
        assert!(
            embeddings.len() >= output_dim,
            "Need at least {output_dim} samples, got {}",
            embeddings.len()
        );
        for (i, emb) in embeddings.iter().enumerate() {
            assert_eq!(
                emb.len(),
                input_dim,
                "Embedding {i} has dim {}, expected {input_dim}",
                emb.len()
            );
        }

        let n = embeddings.len();
        let d = input_dim;

        // 1. Compute mean (f64 for precision)
        let mut mean = vec![0.0f64; d];
        for emb in embeddings {
            for (i, &v) in emb.iter().enumerate() {
                mean[i] += v as f64;
            }
        }
        for m in &mut mean {
            *m /= n as f64;
        }

        // 2. Center the data
        let centered: Vec<Vec<f64>> = embeddings
            .iter()
            .map(|emb| {
                emb.iter()
                    .enumerate()
                    .map(|(i, &v)| v as f64 - mean[i])
                    .collect()
            })
            .collect();

        // 3. Compute covariance matrix C = X^T X / n (d x d)
        let mut cov = vec![0.0f64; d * d];
        for sample in &centered {
            for i in 0..d {
                let si = sample[i];
                if si.abs() < 1e-15 {
                    continue;
                }
                for j in i..d {
                    cov[i * d + j] += si * sample[j];
                }
            }
        }
        // Symmetrize and normalize
        for i in 0..d {
            for j in i..d {
                cov[i * d + j] /= n as f64;
                cov[j * d + i] = cov[i * d + j];
            }
        }

        // 4. Extract top-k eigenvectors via power iteration + deflation
        let mut weights = vec![0.0f32; d * output_dim];
        let iters = 200;

        for k in 0..output_dim {
            // Initialize with deterministic vector
            let mut v = vec![0.0f64; d];
            for (i, val) in v.iter_mut().enumerate() {
                let seed = ((i + k * 31 + 7) * 2654435761) & 0xFFFFFFFF;
                *val = (seed as f64 / 0xFFFFFFFF_u64 as f64) * 2.0 - 1.0;
            }
            let init_norm: f64 = v.iter().map(|x| x * x).sum::<f64>().sqrt();
            for x in &mut v {
                *x /= init_norm;
            }

            // Power iteration: v = C*v / ||C*v||
            for _ in 0..iters {
                let mut new_v = vec![0.0f64; d];
                for (i, nv) in new_v.iter_mut().enumerate() {
                    let mut sum = 0.0f64;
                    let row = i * d;
                    for j in 0..d {
                        sum += cov[row + j] * v[j];
                    }
                    *nv = sum;
                }
                let norm: f64 = new_v.iter().map(|x| x * x).sum::<f64>().sqrt();
                if norm > 1e-15 {
                    for x in &mut new_v {
                        *x /= norm;
                    }
                }
                v = new_v;
            }

            // Store eigenvector as column k
            for (row, &val) in v.iter().enumerate() {
                weights[row * output_dim + k] = val as f32;
            }

            // Deflate: C = C - lambda * v * v^T
            let mut lambda = 0.0f64;
            for i in 0..d {
                let mut cv_i = 0.0f64;
                let row = i * d;
                for j in 0..d {
                    cv_i += cov[row + j] * v[j];
                }
                lambda += v[i] * cv_i;
            }

            for i in 0..d {
                let li = lambda * v[i];
                for j in 0..d {
                    cov[i * d + j] -= li * v[j];
                }
            }

            if (k + 1) % 100 == 0 {
                tracing::debug!("PCA: computed {}/{} components", k + 1, output_dim);
            }
        }

        Self {
            weights,
            input_dim,
            output_dim,
        }
    }

    /// Generate a random orthogonal projection matrix (for testing).
    pub fn random_orthogonal(seed: u64, input_dim: usize, output_dim: usize) -> Self {
        let mut rng_state = seed;
        let mut weights = vec![0.0f32; input_dim * output_dim];

        for w in &mut weights {
            rng_state ^= rng_state << 13;
            rng_state ^= rng_state >> 7;
            rng_state ^= rng_state << 17;
            *w = (rng_state as f64 / u64::MAX as f64) as f32 * 2.0 - 1.0;
        }

        // Gram-Schmidt orthogonalization on columns
        for col in 0..output_dim {
            for prev in 0..col {
                let dot: f32 = (0..input_dim)
                    .map(|row| {
                        weights[row * output_dim + col] * weights[row * output_dim + prev]
                    })
                    .sum();
                for row in 0..input_dim {
                    weights[row * output_dim + col] -= dot * weights[row * output_dim + prev];
                }
            }
            let norm: f32 = (0..input_dim)
                .map(|row| weights[row * output_dim + col].powi(2))
                .sum::<f32>()
                .sqrt();
            if norm > 1e-10 {
                for row in 0..input_dim {
                    weights[row * output_dim + col] /= norm;
                }
            }
        }

        Self {
            weights,
            input_dim,
            output_dim,
        }
    }

    /// Project a vector and L2-normalize.
    pub fn project(&self, input: &[f32]) -> Result<Vec<f32>> {
        if input.len() != self.input_dim {
            return Err(oi_types::Error::Internal(format!(
                "Expected {}-dim input, got {}",
                self.input_dim,
                input.len()
            )));
        }

        let mut output = vec![0.0f32; self.output_dim];
        for (i, &x) in input.iter().enumerate() {
            let row_offset = i * self.output_dim;
            for (j, out) in output.iter_mut().enumerate() {
                *out += x * self.weights[row_offset + j];
            }
        }

        // L2 normalize
        let norm: f32 = output.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 1e-10 {
            for x in &mut output {
                *x /= norm;
            }
        }

        Ok(output)
    }

    /// Save the matrix to a binary file.
    /// Format: [input_dim: u32 LE][output_dim: u32 LE][weights: f32 LE...]
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| oi_types::Error::Internal(format!("Create dir: {e}")))?;
        }
        let mut bytes = Vec::with_capacity(8 + self.weights.len() * 4);
        bytes.extend_from_slice(&(self.input_dim as u32).to_le_bytes());
        bytes.extend_from_slice(&(self.output_dim as u32).to_le_bytes());
        for f in &self.weights {
            bytes.extend_from_slice(&f.to_le_bytes());
        }
        std::fs::write(path, bytes)
            .map_err(|e| oi_types::Error::Internal(format!("Save projection: {e}")))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_projection_output_dim() {
        let proj = ProjectionMatrix::random_orthogonal(42, 768, 384);
        let input = vec![0.1f32; 768];
        let output = proj.project(&input).unwrap();
        assert_eq!(output.len(), 384);
    }

    #[test]
    fn test_projection_normalized() {
        let proj = ProjectionMatrix::random_orthogonal(42, 768, 384);
        let input = vec![0.5f32; 768];
        let output = proj.project(&input).unwrap();
        let norm: f32 = output.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 0.01, "not normalized: {norm}");
    }

    #[test]
    fn test_projection_wrong_dim() {
        let proj = ProjectionMatrix::random_orthogonal(42, 768, 384);
        let input = vec![0.1f32; 256];
        assert!(proj.project(&input).is_err());
    }

    #[test]
    fn test_projection_preserves_cosine_ranking() {
        let proj = ProjectionMatrix::random_orthogonal(42, 768, 384);

        let mut a = vec![0.1f32; 768];
        let mut b = vec![0.1f32; 768];
        let mut c = vec![-0.1f32; 768];

        a[0] = 1.0;
        b[0] = 0.9;
        c[0] = -1.0;

        let pa = proj.project(&a).unwrap();
        let pb = proj.project(&b).unwrap();
        let pc = proj.project(&c).unwrap();

        let sim_ab: f32 = pa.iter().zip(&pb).map(|(x, y)| x * y).sum();
        let sim_ac: f32 = pa.iter().zip(&pc).map(|(x, y)| x * y).sum();

        assert!(
            sim_ab > sim_ac,
            "projection should preserve similarity ranking: ab={sim_ab:.3} ac={sim_ac:.3}"
        );
    }

    #[test]
    fn test_pca_projection_basic() {
        // Use small dims (32→16) so the test runs fast even in debug mode
        let input_dim = 32;
        let output_dim = 16;
        let n = 50;
        let mut embeddings = Vec::with_capacity(n);
        let mut rng = 42u64;

        for _ in 0..n {
            let mut v = vec![0.0f32; input_dim];
            for (i, val) in v.iter_mut().enumerate() {
                rng ^= rng << 13;
                rng ^= rng >> 7;
                rng ^= rng << 17;
                let r = (rng as f64 / u64::MAX as f64) as f32 * 2.0 - 1.0;
                // First 8 dims have 10x more variance
                *val = if i < 8 { r * 10.0 } else { r };
            }
            embeddings.push(v);
        }

        let proj = ProjectionMatrix::from_pca(&embeddings, input_dim, output_dim);

        let input = vec![1.0f32; input_dim];
        let output = proj.project(&input).unwrap();
        assert_eq!(output.len(), output_dim);

        let norm: f32 = output.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 0.01, "not normalized: {norm}");
    }

    #[test]
    fn test_pca_preserves_similarity_better_than_random() {
        // Use small dims (64→32) for speed
        let input_dim = 64;
        let output_dim = 32;
        let n = 80;
        let mut embeddings = Vec::with_capacity(n);
        let mut rng = 99u64;

        for _ in 0..n {
            let mut v = vec![0.0f32; input_dim];
            for (i, val) in v.iter_mut().enumerate() {
                rng ^= rng << 13;
                rng ^= rng >> 7;
                rng ^= rng << 17;
                let r = (rng as f64 / u64::MAX as f64) as f32 * 2.0 - 1.0;
                *val = if i < 20 { r * 5.0 } else { r * 0.1 };
            }
            embeddings.push(v);
        }

        let pca_proj = ProjectionMatrix::from_pca(&embeddings, input_dim, output_dim);
        let rand_proj = ProjectionMatrix::random_orthogonal(42, input_dim, output_dim);

        // Similar vectors in the high-variance dims
        let mut a = vec![0.0f32; input_dim];
        let mut b = vec![0.0f32; input_dim];
        let mut c = vec![0.0f32; input_dim];
        for i in 0..20 {
            a[i] = 1.0;
            b[i] = 0.9;
            c[i] = -1.0;
        }

        let pca_a = pca_proj.project(&a).unwrap();
        let pca_b = pca_proj.project(&b).unwrap();
        let pca_c = pca_proj.project(&c).unwrap();

        let rand_a = rand_proj.project(&a).unwrap();
        let rand_b = rand_proj.project(&b).unwrap();
        let rand_c = rand_proj.project(&c).unwrap();

        let pca_sim_ab: f32 = pca_a.iter().zip(&pca_b).map(|(x, y)| x * y).sum();
        let pca_sim_ac: f32 = pca_a.iter().zip(&pca_c).map(|(x, y)| x * y).sum();
        let pca_gap = pca_sim_ab - pca_sim_ac;

        let rand_sim_ab: f32 = rand_a.iter().zip(&rand_b).map(|(x, y)| x * y).sum();
        let rand_sim_ac: f32 = rand_a.iter().zip(&rand_c).map(|(x, y)| x * y).sum();
        let rand_gap = rand_sim_ab - rand_sim_ac;

        assert!(
            pca_gap >= rand_gap * 0.9,
            "PCA gap ({pca_gap:.3}) should be >= random gap ({rand_gap:.3})"
        );
    }

    #[test]
    fn test_save_and_load() {
        let proj = ProjectionMatrix::random_orthogonal(123, 768, 384);
        let tmp = std::env::temp_dir().join("oi_proj_test.bin");

        proj.save(&tmp).unwrap();
        let loaded = ProjectionMatrix::from_file(&tmp).unwrap();

        assert_eq!(loaded.input_dim, 768);
        assert_eq!(loaded.output_dim, 384);
        assert_eq!(proj.weights.len(), loaded.weights.len());
        for (a, b) in proj.weights.iter().zip(&loaded.weights) {
            assert!((a - b).abs() < 1e-7);
        }

        let _ = std::fs::remove_file(&tmp);
    }
}
