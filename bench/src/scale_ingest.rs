//! Scale-ingest: pour N MS MARCO passages (with cached embeddings) through
//! the real ingestion path into a target index. Used to validate index
//! behavior — latency, dedup, blend thresholds, corrector vocab scan — at
//! corpus sizes live crawling can't reach in reasonable time.

use oi_index::{HybridIndex, Storage};
use oi_types::traits::SearchIndex;
use oi_types::{ChunkMeta, ContentChunk};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

pub fn run_scale_ingest(limit: usize, index_path: &str) {
    let home = std::env::var("HOME").map(PathBuf::from).expect("HOME");
    let dir = home.join(".cache").join("openindex").join("msmarco");

    println!("Loading {limit} cached embeddings...");
    let t = Instant::now();
    let (ids, embeddings) = load_embeddings_prefix(
        &dir.join("embeddings_minilm.bin"),
        &dir.join("embeddings_minilm_ids.txt"),
        limit,
    )
    .expect("load embeddings");
    println!(
        "  {} embeddings in {:.1}s",
        ids.len(),
        t.elapsed().as_secs_f64()
    );

    println!("Loading passages...");
    let t = Instant::now();
    let wanted: std::collections::HashSet<&str> = ids.iter().map(|s| s.as_str()).collect();
    let mut passages: HashMap<String, String> = HashMap::with_capacity(ids.len());
    {
        use std::io::BufRead;
        let f = std::fs::File::open(dir.join("collection.tsv")).expect("collection.tsv");
        let reader = std::io::BufReader::with_capacity(4 << 20, f);
        for line in reader.lines() {
            let line = line.expect("read line");
            if let Some((pid, text)) = line.split_once('\t') {
                if wanted.contains(pid) {
                    passages.insert(pid.to_string(), text.to_string());
                    if passages.len() == ids.len() {
                        break;
                    }
                }
            }
        }
    }
    println!(
        "  {} passages in {:.1}s",
        passages.len(),
        t.elapsed().as_secs_f64()
    );

    println!("Opening index at {index_path}...");
    let storage = Storage::new(PathBuf::from(index_path)).expect("storage");
    let index = if storage.segment_exists("hnsw.bin") {
        HybridIndex::load(&storage).expect("load index")
    } else {
        HybridIndex::new(384)
    };
    let start_docs = index.doc_count();
    println!("  starting docs: {start_docs}");

    let t = Instant::now();
    let mut batch: Vec<ContentChunk> = Vec::with_capacity(1000);
    let mut ingested_batches = 0;
    for (pid, emb) in ids.iter().zip(embeddings) {
        let Some(text) = passages.get(pid) else {
            continue;
        };
        let title: String = text
            .split_whitespace()
            .take(8)
            .collect::<Vec<_>>()
            .join(" ");
        let hash = format!("msmarco-{pid}");
        batch.push(ContentChunk {
            chunk_id: format!("ms_{pid}"),
            content: text.clone(),
            embedding: Some(emb),
            seq: 0,
            meta: Some(ChunkMeta {
                url: format!("https://msmarco.example/passage/{pid}"),
                title,
                domain: "msmarco.example".to_string(),
                content_hash: hash,
                crawled_at: Some(1_750_000_000),
                content_type: Some("text/plain".to_string()),
                quality_score: Some(0.6),
                ..Default::default()
            }),
        });
        if batch.len() >= 1000 {
            index.ingest(&batch).expect("ingest");
            batch.clear();
            ingested_batches += 1;
            if ingested_batches % 10 == 0 {
                let done = ingested_batches * 1000;
                let rate = done as f64 / t.elapsed().as_secs_f64();
                println!("  {done} ingested ({rate:.0} docs/s)");
            }
        }
    }
    if !batch.is_empty() {
        index.ingest(&batch).expect("ingest");
    }
    let ingest_secs = t.elapsed().as_secs_f64();

    let t = Instant::now();
    index.save(&storage).expect("save");
    println!(
        "\nDone: {} -> {} docs | ingest {:.1}s ({:.0} docs/s) | save {:.1}s",
        start_docs,
        index.doc_count(),
        ingest_secs,
        (index.doc_count() - start_docs) as f64 / ingest_secs.max(0.001),
        t.elapsed().as_secs_f64()
    );
}

/// Read only the first `limit` rows of the embedding cache.
fn load_embeddings_prefix(
    bin_path: &std::path::Path,
    ids_path: &std::path::Path,
    limit: usize,
) -> Result<(Vec<String>, Vec<Vec<f32>>), String> {
    use std::io::{BufRead, Read};

    let f = std::fs::File::open(bin_path).map_err(|e| format!("open cache: {e}"))?;
    let mut r = std::io::BufReader::with_capacity(4 << 20, f);
    let mut buf8 = [0u8; 8];
    let mut buf4 = [0u8; 4];
    r.read_exact(&mut buf8).map_err(|e| format!("count: {e}"))?;
    let count = u64::from_le_bytes(buf8) as usize;
    r.read_exact(&mut buf4).map_err(|e| format!("dim: {e}"))?;
    let dim = u32::from_le_bytes(buf4) as usize;
    let n = count.min(limit);

    let mut embeddings = Vec::with_capacity(n);
    let mut row = vec![0u8; dim * 4];
    for _ in 0..n {
        r.read_exact(&mut row).map_err(|e| format!("row: {e}"))?;
        let emb: Vec<f32> = row
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect();
        embeddings.push(emb);
    }

    let ids_file = std::fs::File::open(ids_path).map_err(|e| format!("ids: {e}"))?;
    let ids: Vec<String> = std::io::BufReader::new(ids_file)
        .lines()
        .take(n)
        .collect::<Result<_, _>>()
        .map_err(|e| format!("ids read: {e}"))?;

    Ok((ids, embeddings))
}
