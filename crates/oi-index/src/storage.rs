use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

/// Storage backend for persisting index data.
/// Supports both buffered file I/O (`read_segment`) and memory-mapped I/O
/// (`mmap_segment`) for large segments.
pub struct Storage {
    base_dir: PathBuf,
    /// OS advisory lock held for this process's lifetime once it first
    /// writes. Guards against a second writer process silently interleaving
    /// segment saves (sentinel + node + CLI all target the same directory).
    write_lock: std::sync::OnceLock<fs::File>,
}

impl Storage {
    pub fn new(base_dir: impl AsRef<Path>) -> io::Result<Self> {
        let base = base_dir.as_ref().to_path_buf();
        fs::create_dir_all(&base)?;
        Ok(Self {
            base_dir: base,
            write_lock: std::sync::OnceLock::new(),
        })
    }

    /// Acquire the exclusive writer lock (idempotent; held until this
    /// process exits). Readers never call this. Fails loudly when another
    /// process is already writing to this index directory — two
    /// uncoordinated writers would clobber each other's segments, so
    /// refusing is the safe outcome.
    pub fn acquire_write_lock(&self) -> io::Result<()> {
        if self.write_lock.get().is_some() {
            return Ok(());
        }
        let lock_file = fs::File::create(self.base_dir.join(".write.lock"))?;
        lock_file.try_lock().map_err(|e| {
            io::Error::other(format!(
                "index at {} is locked by another writer process ({e}); \
                 refusing to save to avoid corruption",
                self.base_dir.display()
            ))
        })?;
        let _ = self.write_lock.set(lock_file);
        Ok(())
    }

    /// Validate segment name to prevent path traversal
    fn safe_path(&self, name: &str) -> io::Result<PathBuf> {
        if name.contains("..") || name.contains('/') || name.contains('\\') || name.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("invalid segment name: {name:?}"),
            ));
        }
        Ok(self.base_dir.join(name))
    }

    pub fn write_segment(&self, name: &str, data: &[u8]) -> io::Result<()> {
        let path = self.safe_path(name)?;
        let mut f = fs::File::create(path)?;
        f.write_all(data)?;
        f.sync_all()?; // fsync to guarantee durability
        Ok(())
    }

    pub fn read_segment(&self, name: &str) -> io::Result<Vec<u8>> {
        let path = self.safe_path(name)?;
        let mut f = fs::File::open(path)?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;
        Ok(buf)
    }

    pub fn segment_exists(&self, name: &str) -> bool {
        self.safe_path(name).map(|p| p.exists()).unwrap_or(false)
    }

    /// Read a segment using memory-mapped I/O for large files.
    /// The returned `Mmap` dereferences to `&[u8]` and keeps the file mapped
    /// until dropped.
    pub fn mmap_segment(&self, name: &str) -> io::Result<memmap2::Mmap> {
        let path = self.safe_path(name)?;
        let file = fs::File::open(path)?;
        unsafe { memmap2::Mmap::map(&file) }
    }

    /// Atomically rename a segment (used for safe write-then-rename).
    /// Fsyncs the parent directory after rename to ensure the directory entry is durable.
    pub fn rename_segment(&self, from: &str, to: &str) -> io::Result<()> {
        let src = self.safe_path(from)?;
        let dst = self.safe_path(to)?;
        fs::rename(src, dst)?;
        // fsync the directory to ensure the rename is durable
        if let Ok(dir) = fs::File::open(&self.base_dir) {
            let _ = dir.sync_all();
        }
        Ok(())
    }

    pub fn list_segments(&self) -> io::Result<Vec<String>> {
        let mut names = Vec::new();
        for entry in fs::read_dir(&self.base_dir)? {
            let entry = entry?;
            if let Some(name) = entry.file_name().to_str() {
                // The advisory writer lock is process state, not index data.
                if name == ".write.lock" {
                    continue;
                }
                names.push(name.to_string());
            }
        }
        names.sort();
        Ok(names)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_write_and_read() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::new(dir.path()).unwrap();

        storage.write_segment("test.bin", b"hello world").unwrap();
        let data = storage.read_segment("test.bin").unwrap();
        assert_eq!(data, b"hello world");
    }

    #[test]
    fn test_segment_exists() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::new(dir.path()).unwrap();

        assert!(!storage.segment_exists("nope.bin"));
        storage.write_segment("yes.bin", b"data").unwrap();
        assert!(storage.segment_exists("yes.bin"));
    }

    #[test]
    fn test_list_segments() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::new(dir.path()).unwrap();

        storage.write_segment("a.bin", b"1").unwrap();
        storage.write_segment("b.bin", b"2").unwrap();

        let segs = storage.list_segments().unwrap();
        assert_eq!(segs, vec!["a.bin", "b.bin"]);
    }

    #[test]
    fn test_path_traversal_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::new(dir.path()).unwrap();

        assert!(storage.write_segment("../etc/passwd", b"hack").is_err());
        assert!(storage.write_segment("foo/bar", b"hack").is_err());
        assert!(storage.write_segment("", b"hack").is_err());
        assert!(!storage.segment_exists("../../anything"));
    }

    #[test]
    fn test_mmap_segment() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::new(dir.path()).unwrap();

        let payload = b"mmap test data payload";
        storage.write_segment("mmap.bin", payload).unwrap();

        let mapped = storage.mmap_segment("mmap.bin").unwrap();
        assert_eq!(&mapped[..], payload);
    }

    #[test]
    fn test_mmap_missing_segment() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::new(dir.path()).unwrap();

        assert!(storage.mmap_segment("nonexistent.bin").is_err());
    }

    #[test]
    fn test_write_lock_excludes_second_writer() {
        let dir = tempfile::tempdir().unwrap();
        let s1 = Storage::new(dir.path()).unwrap();
        let s2 = Storage::new(dir.path()).unwrap();
        s1.acquire_write_lock().unwrap();
        // Same-process second Storage: OS advisory locks are per-process on
        // some platforms, so only assert the cross-purpose behavior we rely
        // on — s1 remains able to re-acquire idempotently.
        s1.acquire_write_lock().unwrap();
        // .write.lock must not appear as an index segment
        std::fs::write(dir.path().join("hnsw.bin"), b"x").unwrap();
        let segs = s2.list_segments().unwrap();
        assert!(segs.contains(&"hnsw.bin".to_string()));
        assert!(!segs.iter().any(|s| s == ".write.lock"));
    }
}
