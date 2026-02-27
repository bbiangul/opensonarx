use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Known models with expected SHA-256 hashes for integrity verification.
pub struct ModelManifest {
    pub filename: &'static str,
    pub sha256: Option<&'static str>,
    pub url: &'static str,
}

/// GLiNER Multi v2.1 — full precision (~1.1GB, best accuracy).
pub static GLINER_MULTI_MODEL: ModelManifest = ModelManifest {
    filename: "gliner-multi-v2.1/model.onnx",
    sha256: None,
    url: "https://huggingface.co/onnx-community/gliner_multi-v2.1/resolve/main/onnx/model.onnx",
};

/// GLiNER Multi v2.1 — int8 quantized (~349MB, faster but lower accuracy).
pub static GLINER_MULTI_MODEL_QUANTIZED: ModelManifest = ModelManifest {
    filename: "gliner-multi-v2.1/model_quantized.onnx",
    sha256: None,
    url: "https://huggingface.co/onnx-community/gliner_multi-v2.1/resolve/main/onnx/model_quantized.onnx",
};

pub static GLINER_MULTI_TOKENIZER: ModelManifest = ModelManifest {
    filename: "gliner-multi-v2.1/tokenizer.json",
    sha256: None,
    url: "https://huggingface.co/onnx-community/gliner_multi-v2.1/resolve/main/tokenizer.json",
};

/// GLiNER Multitask Large v0.5 — NER + relation extraction (~300M params).
pub static GLINER_MULTITASK_MODEL: ModelManifest = ModelManifest {
    filename: "gliner-multitask-large-v0.5/model.onnx",
    sha256: None,
    url: "https://huggingface.co/knowledgator/gliner-multitask-large-v0.5/resolve/main/onnx/model.onnx",
};

pub static GLINER_MULTITASK_TOKENIZER: ModelManifest = ModelManifest {
    filename: "gliner-multitask-large-v0.5/tokenizer.json",
    sha256: None,
    url: "https://huggingface.co/knowledgator/gliner-multitask-large-v0.5/resolve/main/tokenizer.json",
};

/// Manages model file caching for ONNX models.
/// Checks `$OI_MODEL_DIR` or falls back to `~/.cache/opensonarx/models/`.
pub struct ModelCache {
    cache_dir: PathBuf,
}

impl ModelCache {
    pub fn new() -> Self {
        let cache_dir = Self::resolve_cache_dir();
        Self { cache_dir }
    }

    pub fn with_dir(dir: PathBuf) -> Self {
        Self { cache_dir: dir }
    }

    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    pub fn ensure_model(&self, manifest: &ModelManifest) -> Result<PathBuf, ModelCacheError> {
        let path = self.cache_dir.join(manifest.filename);

        if path.exists() {
            if let Some(expected) = manifest.sha256 {
                let actual = sha256_file(&path)?;
                if actual != expected {
                    tracing::warn!(
                        "SHA-256 mismatch for {}, re-downloading",
                        manifest.filename
                    );
                    fs::remove_file(&path).map_err(ModelCacheError::Io)?;
                    return self.download(manifest);
                }
            }
            tracing::debug!("Model cached: {}", manifest.filename);
            return Ok(path);
        }

        self.download(manifest)
    }

    pub fn model_path(&self, manifest: &ModelManifest) -> PathBuf {
        self.cache_dir.join(manifest.filename)
    }

    pub fn is_cached(&self, manifest: &ModelManifest) -> bool {
        self.cache_dir.join(manifest.filename).exists()
    }

    /// Streaming download — writes chunks directly to disk instead of buffering
    /// the entire file in memory. Supports files of any size (tested with 1.1GB+).
    fn download(&self, manifest: &ModelManifest) -> Result<PathBuf, ModelCacheError> {
        let path = self.cache_dir.join(manifest.filename);
        let tmp_path = path.with_extension("part");

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(ModelCacheError::Io)?;
        }

        tracing::info!("Downloading {} from {}", manifest.filename, manifest.url);

        let resp = ureq::get(manifest.url)
            .call()
            .map_err(|e| ModelCacheError::Download(format!("{e}")))?;

        // Stream body to a temp file in 64KB chunks (no size limit)
        let mut reader = resp.into_body().into_reader();
        let mut file =
            fs::File::create(&tmp_path).map_err(ModelCacheError::Io)?;
        let mut hasher = manifest.sha256.map(|_| Sha256::new());
        let mut total_bytes: u64 = 0;
        let mut last_report: u64 = 0;
        let mut buf = [0u8; 65536];

        loop {
            let n = reader
                .read(&mut buf)
                .map_err(|e| ModelCacheError::Download(format!("stream read: {e}")))?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n]).map_err(ModelCacheError::Io)?;
            if let Some(ref mut h) = hasher {
                h.update(&buf[..n]);
            }
            total_bytes += n as u64;

            // Progress every 50MB
            let mb = total_bytes / (1024 * 1024);
            if mb >= last_report + 50 {
                tracing::info!("  {mb} MB downloaded...");
                last_report = mb;
            }
        }

        file.flush().map_err(ModelCacheError::Io)?;
        drop(file);

        // Verify hash if specified
        if let Some(expected) = manifest.sha256 {
            let actual = format!("{:x}", hasher.unwrap().finalize());
            if actual != expected {
                let _ = fs::remove_file(&tmp_path);
                return Err(ModelCacheError::HashMismatch {
                    expected: expected.to_string(),
                    actual,
                });
            }
        }

        // Atomic rename: tmp -> final
        fs::rename(&tmp_path, &path).map_err(ModelCacheError::Io)?;
        tracing::info!(
            "Downloaded {} ({} bytes)",
            manifest.filename,
            total_bytes
        );

        Ok(path)
    }

    fn resolve_cache_dir() -> PathBuf {
        if let Ok(dir) = std::env::var("OI_MODEL_DIR") {
            return PathBuf::from(dir);
        }
        if let Some(home) = dirs_home() {
            return home.join(".cache").join("opensonarx").join("models");
        }
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

fn dirs_home() -> Option<PathBuf> {
    std::env::var("HOME").ok().map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_dir_creation() {
        let tmp = std::env::temp_dir().join("oi_facts_cache_test");
        let _ = fs::remove_dir_all(&tmp);
        let cache = ModelCache::with_dir(tmp.clone());
        assert_eq!(cache.cache_dir(), tmp.as_path());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_model_path_resolution() {
        let tmp = std::env::temp_dir().join("oi_facts_path_test");
        let cache = ModelCache::with_dir(tmp.clone());
        let path = cache.model_path(&GLINER_MULTI_MODEL);
        assert!(path.to_string_lossy().contains("gliner-multi-v2.1"));
        assert!(path.to_string_lossy().contains("model.onnx"));
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_not_cached_initially() {
        let tmp = std::env::temp_dir().join("oi_facts_not_cached_test");
        let _ = fs::remove_dir_all(&tmp);
        let cache = ModelCache::with_dir(tmp.clone());
        assert!(!cache.is_cached(&GLINER_MULTI_MODEL));
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_sha256_file_roundtrip() {
        let tmp = std::env::temp_dir().join("oi_facts_sha_test");
        fs::create_dir_all(&tmp).unwrap();
        let file_path = tmp.join("test.bin");
        fs::write(&file_path, b"hello world").unwrap();
        let hash = sha256_file(&file_path).unwrap();
        assert_eq!(
            hash,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_default_cache_uses_home() {
        let cache = ModelCache::new();
        let dir = cache.cache_dir();
        assert!(!dir.to_string_lossy().is_empty());
    }
}
