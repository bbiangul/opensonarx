use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Known models with expected SHA-256 hashes for integrity verification.
pub struct ModelManifest {
    pub filename: &'static str,
    pub sha256: Option<&'static str>,
    pub url: &'static str,
}

pub static MINILM_MODEL: ModelManifest = ModelManifest {
    filename: "arctic-embed-s/model_quantized.onnx",
    sha256: None,
    url: "https://huggingface.co/Snowflake/snowflake-arctic-embed-s/resolve/main/onnx/model_quantized.onnx",
};

pub static MINILM_TOKENIZER: ModelManifest = ModelManifest {
    filename: "arctic-embed-s/tokenizer.json",
    sha256: None,
    url: "https://huggingface.co/Snowflake/snowflake-arctic-embed-s/resolve/main/tokenizer.json",
};

// ── Cross-encoder reranker ──
pub static RERANKER_MODEL: ModelManifest = ModelManifest {
    filename: "cross-encoder-ms-marco-MiniLM-L-6-v2/model.onnx",
    sha256: None,
    url: "https://huggingface.co/cross-encoder/ms-marco-MiniLM-L-6-v2/resolve/main/onnx/model.onnx",
};

pub static RERANKER_TOKENIZER: ModelManifest = ModelManifest {
    filename: "cross-encoder-ms-marco-MiniLM-L-6-v2/tokenizer.json",
    sha256: None,
    url: "https://huggingface.co/cross-encoder/ms-marco-MiniLM-L-6-v2/resolve/main/tokenizer.json",
};

// ── Arctic Embed M (768-dim) ──
pub static ARCTIC_M_MODEL: ModelManifest = ModelManifest {
    filename: "arctic-embed-m-v2.0/model.onnx",
    sha256: None,
    url: "https://huggingface.co/Snowflake/snowflake-arctic-embed-m-v2.0/resolve/main/onnx/model.onnx",
};

pub static ARCTIC_M_TOKENIZER: ModelManifest = ModelManifest {
    filename: "arctic-embed-m-v2.0/tokenizer.json",
    sha256: None,
    url: "https://huggingface.co/Snowflake/snowflake-arctic-embed-m-v2.0/resolve/main/tokenizer.json",
};

// ── Arctic Embed L (1024-dim) ──
pub static ARCTIC_L_MODEL: ModelManifest = ModelManifest {
    filename: "arctic-embed-l-v2.0/model.onnx",
    sha256: None,
    url: "https://huggingface.co/Snowflake/snowflake-arctic-embed-l-v2.0/resolve/main/onnx/model.onnx",
};

pub static ARCTIC_L_TOKENIZER: ModelManifest = ModelManifest {
    filename: "arctic-embed-l-v2.0/tokenizer.json",
    sha256: None,
    url: "https://huggingface.co/Snowflake/snowflake-arctic-embed-l-v2.0/resolve/main/tokenizer.json",
};

// NOTE: text_model_quantized.onnx has a known bug where ALiBi bias is hardcoded
// to 16 tokens (https://huggingface.co/jinaai/jina-clip-v1/discussions/31).
// Using FP16 model which has the fix for dynamic sequence lengths.
pub static CLIP_TEXT_MODEL: ModelManifest = ModelManifest {
    filename: "jina-clip-v1/text_model_fp16.onnx",
    sha256: None,
    url: "https://huggingface.co/jinaai/jina-clip-v1/resolve/main/onnx/text_model_fp16.onnx",
};

pub static CLIP_VISION_MODEL: ModelManifest = ModelManifest {
    filename: "jina-clip-v1/vision_model_quantized.onnx",
    sha256: None,
    url: "https://huggingface.co/jinaai/jina-clip-v1/resolve/main/onnx/vision_model_quantized.onnx",
};

pub static CLIP_TOKENIZER: ModelManifest = ModelManifest {
    filename: "jina-clip-v1/tokenizer.json",
    sha256: None,
    url: "https://huggingface.co/jinaai/jina-clip-v1/resolve/main/tokenizer.json",
};

/// Manages model file caching for ONNX models.
/// Checks `$OI_MODEL_DIR` or falls back to `~/.cache/opensonarx/models/`.
pub struct ModelCache {
    cache_dir: PathBuf,
}

impl ModelCache {
    /// Create a new ModelCache, resolving the cache directory.
    pub fn new() -> Self {
        let cache_dir = Self::resolve_cache_dir();
        Self { cache_dir }
    }

    /// Create a ModelCache with an explicit directory.
    pub fn with_dir(dir: PathBuf) -> Self {
        Self { cache_dir: dir }
    }

    /// Get the cache directory path.
    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    /// Resolve the model path, downloading if necessary.
    pub fn ensure_model(&self, manifest: &ModelManifest) -> Result<PathBuf, ModelCacheError> {
        let path = self.cache_dir.join(manifest.filename);

        if path.exists() {
            // Verify SHA-256 if hash is known
            if let Some(expected) = manifest.sha256 {
                let actual = sha256_file(&path)?;
                if actual != expected {
                    tracing::warn!("SHA-256 mismatch for {}, re-downloading", manifest.filename);
                    fs::remove_file(&path).map_err(ModelCacheError::Io)?;
                    return self.download(manifest);
                }
            }
            tracing::debug!("Model cached: {}", manifest.filename);
            return Ok(path);
        }

        self.download(manifest)
    }

    /// Get the expected path for a model file (without downloading).
    pub fn model_path(&self, manifest: &ModelManifest) -> PathBuf {
        self.cache_dir.join(manifest.filename)
    }

    /// Check if a model file exists in the cache.
    pub fn is_cached(&self, manifest: &ModelManifest) -> bool {
        self.cache_dir.join(manifest.filename).exists()
    }

    fn download(&self, manifest: &ModelManifest) -> Result<PathBuf, ModelCacheError> {
        let path = self.cache_dir.join(manifest.filename);

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(ModelCacheError::Io)?;
        }

        tracing::info!("Downloading {} from {}", manifest.filename, manifest.url);

        // Use ureq for synchronous download (model download happens at init)
        // Models can be up to ~100MB, raise the body limit
        let resp = ureq::get(manifest.url)
            .call()
            .map_err(|e| ModelCacheError::Download(format!("{e}")))?;

        let bytes = resp
            .into_body()
            .with_config()
            .limit(1200 * 1024 * 1024) // 1.2GB limit for large models
            .read_to_vec()
            .map_err(|e| ModelCacheError::Download(format!("read body: {e}")))?;

        // Verify SHA-256 if hash is known
        if let Some(expected) = manifest.sha256 {
            let actual = sha256_bytes(&bytes);
            if actual != expected {
                return Err(ModelCacheError::HashMismatch {
                    expected: expected.to_string(),
                    actual,
                });
            }
        }

        fs::write(&path, &bytes).map_err(ModelCacheError::Io)?;
        tracing::info!("Downloaded {} ({} bytes)", manifest.filename, bytes.len());

        Ok(path)
    }

    fn resolve_cache_dir() -> PathBuf {
        // Check $OI_MODEL_DIR first
        if let Ok(dir) = std::env::var("OI_MODEL_DIR") {
            return PathBuf::from(dir);
        }

        // Fall back to ~/.cache/opensonarx/models/
        if let Some(home) = dirs_home() {
            return home.join(".cache").join("opensonarx").join("models");
        }

        // Last resort: current directory
        PathBuf::from("models")
    }
}

impl Default for ModelCache {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub enum ModelCacheError {
    Io(std::io::Error),
    Download(String),
    HashMismatch { expected: String, actual: String },
}

impl std::fmt::Display for ModelCacheError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::Download(e) => write!(f, "Download failed: {e}"),
            Self::HashMismatch { expected, actual } => {
                write!(f, "SHA-256 mismatch: expected {expected}, got {actual}")
            }
        }
    }
}

impl std::error::Error for ModelCacheError {}

impl From<ModelCacheError> for oi_types::Error {
    fn from(e: ModelCacheError) -> Self {
        oi_types::Error::Internal(e.to_string())
    }
}

fn sha256_file(path: &Path) -> Result<String, ModelCacheError> {
    let mut file = fs::File::open(path).map_err(ModelCacheError::Io)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file.read(&mut buf).map_err(ModelCacheError::Io)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn sha256_bytes(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var("HOME").ok().map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_cache_dir_creation() {
        let tmp = std::env::temp_dir().join("oi_model_cache_test");
        let _ = fs::remove_dir_all(&tmp);
        let cache = ModelCache::with_dir(tmp.clone());
        assert_eq!(cache.cache_dir(), tmp.as_path());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_model_path_resolution() {
        let tmp = std::env::temp_dir().join("oi_model_path_test");
        let cache = ModelCache::with_dir(tmp.clone());
        let path = cache.model_path(&MINILM_MODEL);
        assert!(path.to_string_lossy().contains("arctic-embed-s"));
        assert!(path.to_string_lossy().contains("model_quantized.onnx"));
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_not_cached_initially() {
        let tmp = std::env::temp_dir().join("oi_model_not_cached_test");
        let _ = fs::remove_dir_all(&tmp);
        let cache = ModelCache::with_dir(tmp.clone());
        assert!(!cache.is_cached(&MINILM_MODEL));
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_sha256_bytes() {
        let hash = sha256_bytes(b"hello world");
        assert_eq!(
            hash,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn test_sha256_file() {
        let tmp = std::env::temp_dir().join("oi_sha256_test.bin");
        fs::write(&tmp, b"hello world").unwrap();
        let hash = sha256_file(&tmp).unwrap();
        assert_eq!(
            hash,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn test_default_cache_uses_home() {
        // This test just verifies default resolution doesn't panic
        let cache = ModelCache::new();
        let dir = cache.cache_dir();
        assert!(!dir.to_string_lossy().is_empty());
    }
}
