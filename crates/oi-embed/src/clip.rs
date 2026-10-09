use oi_types::{EmbeddingPipeline, Result};
use ort::session::Session;
use ort::value::Value;
use std::sync::Mutex;
use tokenizers::Tokenizer;

use crate::model_cache::{CLIP_TEXT_MODEL, CLIP_TOKENIZER, CLIP_VISION_MODEL, ModelCache};
use crate::projection::ProjectionMatrix;

const CLIP_DIM: usize = 768;
const OUTPUT_DIM: usize = 384;
const MAX_TOKENS: usize = 512; // Jina CLIP v1 supports up to 8192, we cap at 512
const IMAGE_SIZE: usize = 224;
const PROJECTION_FILENAME: &str = "jina-clip-v1/projection_pca_384.bin";

/// Jina CLIP v1 image normalization (ImageNet stats)
const CLIP_MEAN: [f32; 3] = [0.48145466, 0.4578275, 0.40821073];
const CLIP_STD: [f32; 3] = [0.26862954, 0.261_302_6, 0.275_777_1];

/// Jina CLIP v1 multimodal embedding pipeline via ONNX.
/// 768-dim text+image embeddings projected to 384-dim for index compatibility.
/// INT8 quantized: ~138MB text + ~88MB vision.
///
/// On first load, computes a PCA projection matrix from a diverse calibration
/// corpus and caches it. Subsequent loads are instant.
pub struct ClipEmbedder {
    text_session: Mutex<Session>,
    vision_session: Mutex<Session>,
    tokenizer: Tokenizer,
    projection: ProjectionMatrix,
}

/// Round up to the next power of 2.
/// Jina CLIP v1 uses ALiBi positional encoding, and its ONNX graph requires
/// sequence lengths to be powers of 2 for the internal bias computation.
fn next_power_of_2(n: usize) -> usize {
    if n <= 1 {
        return 1;
    }
    let mut v = n - 1;
    v |= v >> 1;
    v |= v >> 2;
    v |= v >> 4;
    v |= v >> 8;
    v |= v >> 16;
    v + 1
}

/// Run a single text through a CLIP text session, returning a raw 768-dim embedding.
/// Used during PCA calibration before the ClipEmbedder is fully constructed.
///
/// Jina CLIP v1's ONNX text model only accepts `input_ids` — the attention mask
/// is computed internally from pad tokens (pad_token_id=0). Sequences are padded
/// to the next power of 2 for ALiBi compatibility.
fn raw_text_embed(session: &mut Session, tokenizer: &Tokenizer, text: &str) -> Result<Vec<f32>> {
    let batch = raw_text_embed_batch(session, tokenizer, &[text])?;
    Ok(batch.into_iter().next().unwrap())
}

/// Batched text embedding through CLIP text session.
/// All texts are tokenized, padded to the same power-of-2 length, and run in a single
/// ONNX inference pass. Returns raw 768-dim embeddings (before PCA projection).
fn raw_text_embed_batch(
    session: &mut Session,
    tokenizer: &Tokenizer,
    texts: &[&str],
) -> Result<Vec<Vec<f32>>> {
    if texts.is_empty() {
        return Ok(vec![]);
    }

    let encodings: Vec<_> = texts
        .iter()
        .map(|t| {
            tokenizer
                .encode(*t, true)
                .map_err(|e| oi_types::Error::Internal(format!("CLIP tokenize: {e}")))
        })
        .collect::<Result<Vec<_>>>()?;

    let batch_size = encodings.len();
    let max_len = encodings
        .iter()
        .map(|e| e.get_ids().len().min(MAX_TOKENS))
        .max()
        .unwrap_or(1);
    // Pad to next power of 2 for ALiBi attention bias alignment
    let padded_len = next_power_of_2(max_len);

    let mut input_ids_flat = vec![0i64; batch_size * padded_len]; // 0 = pad_token_id
    for (i, enc) in encodings.iter().enumerate() {
        let ids = enc.get_ids();
        let len = ids.len().min(MAX_TOKENS);
        for (j, &id) in ids[..len].iter().enumerate() {
            input_ids_flat[i * padded_len + j] = id as i64;
        }
    }

    let shape: [usize; 2] = [batch_size, padded_len];
    let input_ids_val = Value::from_array((&shape as &[usize], input_ids_flat))
        .map_err(|e| oi_types::Error::Internal(format!("Value: {e}")))?;

    let inputs = vec![(
        std::borrow::Cow::from("input_ids"),
        ort::session::SessionInputValue::from(input_ids_val),
    )];

    let outputs = session
        .run(inputs)
        .map_err(|e| oi_types::Error::Internal(format!("CLIP text inference: {e}")))?;

    // FP16 model outputs f16, FP32 outputs f32 — handle both
    // Output shape: [batch_size, CLIP_DIM]
    let all_embeddings: Vec<f32> = match outputs[0].try_extract_tensor::<f32>() {
        Ok((_shape, data)) => data.to_vec(),
        Err(_) => {
            let (_shape, data) = outputs[0]
                .try_extract_tensor::<half::f16>()
                .map_err(|e| oi_types::Error::Internal(format!("CLIP output: {e}")))?;
            data.iter().map(|x| x.to_f32()).collect()
        }
    };

    let mut results = Vec::with_capacity(batch_size);
    for b in 0..batch_size {
        let offset = b * CLIP_DIM;
        results.push(all_embeddings[offset..offset + CLIP_DIM].to_vec());
    }
    Ok(results)
}

impl ClipEmbedder {
    /// Load the CLIP model, downloading if necessary.
    pub fn new() -> Result<Self> {
        let cache = ModelCache::new();
        Self::with_cache(&cache)
    }

    /// Load from a specific model cache directory.
    ///
    /// On first load, embeds a 500-text calibration corpus through the CLIP text
    /// encoder to compute a PCA projection matrix (768 -> 384 dims). This takes
    /// ~5-10 seconds and is cached for all future loads.
    pub fn with_cache(cache: &ModelCache) -> Result<Self> {
        let text_path = cache.ensure_model(&CLIP_TEXT_MODEL)?;
        let vision_path = cache.ensure_model(&CLIP_VISION_MODEL)?;
        let tokenizer_path = cache.ensure_model(&CLIP_TOKENIZER)?;

        // FP16 model needs optimization level capped at Level1 to avoid
        // SimplifiedLayerNormFusion crash in ORT's graph optimizer.
        let threads = crate::intra_threads();
        let mut text_session = Session::builder()
            .and_then(|b| {
                b.with_optimization_level(ort::session::builder::GraphOptimizationLevel::Level1)
            })
            .and_then(|b| b.with_intra_threads(threads))
            .and_then(|b| b.commit_from_file(&text_path))
            .map_err(|e| oi_types::Error::Internal(format!("CLIP text session: {e}")))?;

        let vision_session = Session::builder()
            .and_then(|b| b.with_intra_threads(threads))
            .and_then(|b| b.commit_from_file(&vision_path))
            .map_err(|e| oi_types::Error::Internal(format!("CLIP vision session: {e}")))?;

        let tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| oi_types::Error::Internal(format!("CLIP tokenizer: {e}")))?;

        // Load cached PCA projection or compute from calibration corpus
        let projection_path = cache.cache_dir().join(PROJECTION_FILENAME);
        let projection = if projection_path.exists() {
            tracing::debug!(
                "Loading cached PCA projection from {}",
                projection_path.display()
            );
            ProjectionMatrix::from_file(&projection_path)?
        } else {
            tracing::info!(
                "Computing PCA projection matrix from calibration corpus (one-time, ~5-10s)..."
            );
            let corpus = crate::corpus::calibration_corpus();
            let mut embeddings = Vec::with_capacity(corpus.len());

            for (i, text) in corpus.iter().enumerate() {
                let emb = raw_text_embed(&mut text_session, &tokenizer, text)?;
                embeddings.push(emb);
                if (i + 1) % 100 == 0 {
                    tracing::info!("PCA calibration: embedded {}/{} texts", i + 1, corpus.len());
                }
            }

            let proj = ProjectionMatrix::from_pca(&embeddings, CLIP_DIM, OUTPUT_DIM);
            proj.save(&projection_path)?;
            tracing::info!(
                "PCA projection matrix cached at {}",
                projection_path.display()
            );
            proj
        };

        Ok(Self {
            text_session: Mutex::new(text_session),
            vision_session: Mutex::new(vision_session),
            tokenizer,
            projection,
        })
    }

    /// Preprocess an image for Jina CLIP: resize to 224x224, normalize.
    fn preprocess_image(image_bytes: &[u8]) -> Result<(Vec<usize>, Vec<f32>)> {
        use image::GenericImageView;
        use image::imageops::FilterType;

        let img = image::load_from_memory(image_bytes)
            .map_err(|e| oi_types::Error::Internal(format!("Image load: {e}")))?;

        let resized = img.resize_exact(IMAGE_SIZE as u32, IMAGE_SIZE as u32, FilterType::Lanczos3);

        // NCHW layout: [1, 3, 224, 224]
        let mut tensor = vec![0.0f32; 3 * IMAGE_SIZE * IMAGE_SIZE];

        let plane = IMAGE_SIZE * IMAGE_SIZE;
        for (x, y, pixel) in resized.pixels() {
            let [r, g, b, _] = pixel.0;
            let (xu, yu) = (x as usize, y as usize);
            let idx = yu * IMAGE_SIZE + xu;
            tensor[idx] = (r as f32 / 255.0 - CLIP_MEAN[0]) / CLIP_STD[0];
            tensor[plane + idx] = (g as f32 / 255.0 - CLIP_MEAN[1]) / CLIP_STD[1];
            tensor[2 * plane + idx] = (b as f32 / 255.0 - CLIP_MEAN[2]) / CLIP_STD[2];
        }

        Ok((vec![1, 3, IMAGE_SIZE, IMAGE_SIZE], tensor))
    }
}

impl EmbeddingPipeline for ClipEmbedder {
    fn embed_text(&self, text: &str) -> Result<Vec<f32>> {
        let batch = self.embed_text_batch(&[text])?;
        Ok(batch.into_iter().next().unwrap())
    }

    fn embed_text_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(vec![]);
        }
        let mut session = self
            .text_session
            .lock()
            .map_err(|e| oi_types::Error::Internal(format!("Session lock: {e}")))?;
        let raw_batch = raw_text_embed_batch(&mut session, &self.tokenizer, texts)?;
        raw_batch
            .into_iter()
            .map(|raw| self.projection.project(&raw))
            .collect()
    }

    fn dimension(&self) -> usize {
        OUTPUT_DIM
    }

    fn name(&self) -> &str {
        "jina-clip-v1"
    }

    fn supports_images(&self) -> bool {
        true
    }

    fn embed_image(&self, image_bytes: &[u8]) -> Result<Vec<f32>> {
        let (shape, data) = Self::preprocess_image(image_bytes)?;

        let pixel_val = Value::from_array((shape.as_slice(), data))
            .map_err(|e| oi_types::Error::Internal(format!("Value: {e}")))?;

        let inputs = vec![(
            std::borrow::Cow::from("pixel_values"),
            ort::session::SessionInputValue::from(pixel_val),
        )];

        let mut session = self
            .vision_session
            .lock()
            .map_err(|e| oi_types::Error::Internal(format!("Session lock: {e}")))?;
        let outputs = session
            .run(inputs)
            .map_err(|e| oi_types::Error::Internal(format!("CLIP vision inference: {e}")))?;

        let embedding: Vec<f32> = match outputs[0].try_extract_tensor::<f32>() {
            Ok((_shape, data)) => data[..CLIP_DIM].to_vec(),
            Err(_) => {
                let (_shape, data) = outputs[0]
                    .try_extract_tensor::<half::f16>()
                    .map_err(|e| oi_types::Error::Internal(format!("CLIP vision output: {e}")))?;
                data[..CLIP_DIM].iter().map(|x| x.to_f32()).collect()
            }
        };

        self.projection.project(&embedding)
    }
}

#[cfg(test)]
mod tests {
    // CLIP tests require model downloads (~362MB total).
    // Run with: cargo test -p oi-embed --features clip -- clip::tests
    use super::*;

    #[test]
    fn test_clip_text_embed() {
        let embedder = ClipEmbedder::new().expect("Failed to load CLIP model");
        let v = embedder.embed_text("a photo of a cat").unwrap();
        assert_eq!(v.len(), 384);
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_clip_supports_images() {
        let embedder = ClipEmbedder::new().expect("Failed to load CLIP model");
        assert!(embedder.supports_images());
    }

    #[test]
    fn test_clip_semantic_similarity() {
        let embedder = ClipEmbedder::new().expect("Failed to load CLIP model");

        let cat = embedder.embed_text("a photo of a cute cat").unwrap();
        let kitten = embedder.embed_text("small furry kitten sleeping").unwrap();
        let car = embedder.embed_text("red sports car on a highway").unwrap();

        let sim_cat_kitten: f32 = cat.iter().zip(&kitten).map(|(a, b)| a * b).sum();
        let sim_cat_car: f32 = cat.iter().zip(&car).map(|(a, b)| a * b).sum();

        assert!(
            sim_cat_kitten > sim_cat_car,
            "cat-kitten similarity ({sim_cat_kitten:.3}) should be > cat-car ({sim_cat_car:.3})"
        );
    }

    #[test]
    fn test_clip_deterministic() {
        let embedder = ClipEmbedder::new().expect("Failed to load CLIP model");
        let a = embedder.embed_text("test determinism").unwrap();
        let b = embedder.embed_text("test determinism").unwrap();
        assert_eq!(a, b);
    }
}
