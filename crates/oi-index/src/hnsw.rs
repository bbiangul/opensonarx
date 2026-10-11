use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashSet};

/// HNSW (Hierarchical Navigable Small World) index for approximate nearest neighbor search.
/// Pure Rust implementation with INT8 quantization support.
pub struct HnswIndex {
    nodes: Vec<HnswNode>,
    dim: usize,
    max_level: usize,
    ef_construction: usize,
    ef_search: usize,
    m: usize,      // max connections per layer
    m_max0: usize, // max connections at layer 0
    entry_point: Option<usize>,
    level_mult: f64,
    rng_seed: u64, // per-instance RNG seed
    /// Soft-deleted node indices — still traversed for graph connectivity but excluded from results.
    deleted: HashSet<usize>,
}

const MAX_LEVEL: usize = 16;

struct HnswNode {
    #[allow(dead_code)]
    id: usize,
    vector: Vec<f32>,
    neighbors: Vec<Vec<usize>>, // neighbors per level
    #[allow(dead_code)]
    level: usize,
    data_id: String,
}

#[derive(Debug, Clone)]
struct Neighbor {
    id: usize,
    distance: f32,
}

impl PartialEq for Neighbor {
    fn eq(&self, other: &Self) -> bool {
        self.distance == other.distance
    }
}
impl Eq for Neighbor {}
impl PartialOrd for Neighbor {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Neighbor {
    fn cmp(&self, other: &Self) -> Ordering {
        // Min-heap: reverse ordering (smallest distance = highest priority)
        other
            .distance
            .partial_cmp(&self.distance)
            .unwrap_or(Ordering::Equal)
    }
}

impl HnswIndex {
    pub fn new(dim: usize) -> Self {
        let m = 16;
        Self {
            nodes: Vec::new(),
            dim,
            max_level: 0,
            ef_construction: 200,
            ef_search: 50,
            m,
            m_max0: m * 2,
            entry_point: None,
            level_mult: 1.0 / (m as f64).ln(),
            rng_seed: 42,
            deleted: HashSet::new(),
        }
    }

    pub fn with_params(dim: usize, m: usize, ef_construction: usize, ef_search: usize) -> Self {
        Self {
            nodes: Vec::new(),
            dim,
            max_level: 0,
            ef_construction,
            ef_search,
            m,
            m_max0: m * 2,
            entry_point: None,
            level_mult: 1.0 / (m as f64).ln(),
            rng_seed: 42,
            deleted: HashSet::new(),
        }
    }

    fn next_rand(&mut self) -> f64 {
        // Xorshift64 (Marsaglia 2003) — per-instance, deterministic
        self.rng_seed ^= self.rng_seed << 13;
        self.rng_seed ^= self.rng_seed >> 7;
        self.rng_seed ^= self.rng_seed << 17;
        (self.rng_seed as f64) / (u64::MAX as f64)
    }

    fn random_level(&mut self) -> usize {
        let r = self.next_rand();
        let level = (-r.ln() * self.level_mult) as usize;
        level.min(MAX_LEVEL)
    }

    /// Insert a vector into the index. Returns an error if the vector dimension
    /// does not match the index dimension.
    pub fn insert(&mut self, data_id: String, vector: Vec<f32>) -> Result<(), String> {
        if vector.len() != self.dim {
            return Err(format!(
                "vector dimension mismatch: expected {}, got {}",
                self.dim,
                vector.len()
            ));
        }

        let node_id = self.nodes.len();
        let level = self.random_level();

        // Push node first so it's accessible during neighbor connections
        self.nodes.push(HnswNode {
            id: node_id,
            vector,
            neighbors: vec![Vec::new(); level + 1],
            level,
            data_id,
        });

        if self.entry_point.is_none() || node_id == 0 {
            self.entry_point = Some(node_id);
            self.max_level = level;
            return Ok(());
        }

        let ep = self.entry_point.unwrap();
        let mut current = ep;
        let query_vec = self.nodes[node_id].vector.clone();

        // Traverse from top to insertion level + 1
        for l in (level + 1..=self.max_level).rev() {
            current = self.greedy_closest(&query_vec, current, l);
        }

        // Insert at each level from min(level, max_level) down to 0
        for l in (0..=level.min(self.max_level)).rev() {
            let neighbors = self.search_layer(&query_vec, current, self.ef_construction, l);

            let max_conn = if l == 0 { self.m_max0 } else { self.m };
            let selected: Vec<usize> = neighbors
                .iter()
                .filter(|n| n.id != node_id)
                .take(max_conn)
                .map(|n| n.id)
                .collect();

            self.nodes[node_id].neighbors[l] = selected.clone();

            // Bidirectional connections
            for &neighbor_id in &selected {
                let cur_len = self.nodes[neighbor_id].neighbors.len();
                self.nodes[neighbor_id]
                    .neighbors
                    .resize(cur_len.max(l + 1), Vec::new());
                self.nodes[neighbor_id].neighbors[l].push(node_id);

                // Trim if too many connections
                if self.nodes[neighbor_id].neighbors[l].len() > max_conn {
                    let nv = self.nodes[neighbor_id].vector.clone();
                    let neighbor_ids: Vec<usize> = self.nodes[neighbor_id].neighbors[l].clone();
                    let mut scored: Vec<Neighbor> = neighbor_ids
                        .iter()
                        .map(|&id| Neighbor {
                            id,
                            distance: cosine_distance(&nv, &self.nodes[id].vector),
                        })
                        .collect();
                    scored.sort_by(|a, b| a.distance.partial_cmp(&b.distance).unwrap());
                    self.nodes[neighbor_id].neighbors[l] =
                        scored.into_iter().take(max_conn).map(|n| n.id).collect();
                }
            }

            if !neighbors.is_empty() {
                current = neighbors[0].id;
            }
        }

        if level > self.max_level {
            self.max_level = level;
            self.entry_point = Some(node_id);
        }

        Ok(())
    }

    fn greedy_closest(&self, query: &[f32], mut current: usize, level: usize) -> usize {
        let mut best_dist = cosine_distance(query, &self.nodes[current].vector);
        loop {
            let mut changed = false;
            if level < self.nodes[current].neighbors.len() {
                for &neighbor in &self.nodes[current].neighbors[level] {
                    let dist = cosine_distance(query, &self.nodes[neighbor].vector);
                    if dist < best_dist {
                        best_dist = dist;
                        current = neighbor;
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        current
    }

    fn search_layer(&self, query: &[f32], entry: usize, ef: usize, level: usize) -> Vec<Neighbor> {
        // Use Vec<bool> instead of HashSet for O(1) visited checks without hashing overhead.
        // Pre-allocated to index size — ~1 byte per node, 8.8M nodes = 8.8MB.
        let mut visited = vec![false; self.nodes.len()];
        let mut candidates = BinaryHeap::new();
        let mut results = BinaryHeap::new();

        let dist = cosine_distance(query, &self.nodes[entry].vector);
        visited[entry] = true;
        candidates.push(Neighbor {
            id: entry,
            distance: dist,
        });
        results.push(std::cmp::Reverse(Neighbor {
            id: entry,
            distance: dist,
        }));

        while let Some(candidate) = candidates.pop() {
            let worst = results.peek().map(|r| r.0.distance).unwrap_or(f32::MAX);
            if candidate.distance > worst && results.len() >= ef {
                break;
            }

            if level < self.nodes[candidate.id].neighbors.len() {
                for &neighbor in &self.nodes[candidate.id].neighbors[level] {
                    if !visited[neighbor] {
                        visited[neighbor] = true;
                        let d = cosine_distance(query, &self.nodes[neighbor].vector);
                        let worst = results.peek().map(|r| r.0.distance).unwrap_or(f32::MAX);
                        if d < worst || results.len() < ef {
                            candidates.push(Neighbor {
                                id: neighbor,
                                distance: d,
                            });
                            results.push(std::cmp::Reverse(Neighbor {
                                id: neighbor,
                                distance: d,
                            }));
                            if results.len() > ef {
                                results.pop();
                            }
                        }
                    }
                }
            }
        }

        let mut out: Vec<Neighbor> = results.into_iter().map(|r| r.0).collect();
        out.sort_by(|a, b| a.distance.partial_cmp(&b.distance).unwrap());
        out
    }

    /// Search for k nearest neighbors
    pub fn search(&self, query: &[f32], k: usize) -> Vec<(String, f32)> {
        if self.nodes.is_empty() || self.entry_point.is_none() {
            return Vec::new();
        }

        let ep = self.entry_point.unwrap();
        let mut current = ep;

        // Traverse from top level down to 1
        for l in (1..=self.max_level).rev() {
            current = self.greedy_closest(query, current, l);
        }

        // Search layer 0
        let ef = self.ef_search.max(k);
        let results = self.search_layer(query, current, ef, 0);

        results
            .into_iter()
            .filter(|n| !self.deleted.contains(&n.id))
            .take(k)
            .map(|n| (self.nodes[n.id].data_id.clone(), 1.0 - n.distance))
            .collect()
    }

    /// Soft-delete a node by data_id. Returns true if found and marked.
    /// The node remains in the graph for connectivity but is excluded from search results.
    pub fn mark_deleted(&mut self, data_id: &str) -> bool {
        if let Some(idx) = self.nodes.iter().position(|n| n.data_id == data_id) {
            self.deleted.insert(idx);
            true
        } else {
            false
        }
    }

    pub fn len(&self) -> usize {
        self.nodes.len() - self.deleted.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Retrieve a stored vector by data_id. O(n) scan — only used for replication, not hot path.
    pub fn get_vector(&self, data_id: &str) -> Option<&Vec<f32>> {
        self.nodes
            .iter()
            .find(|n| n.data_id == data_id)
            .map(|n| &n.vector)
    }

    /// Dimension of vectors stored in this index.
    pub fn dim(&self) -> usize {
        self.dim
    }

    /// Serialize the entire index to bytes using bincode.
    pub fn to_bytes(&self) -> Vec<u8> {
        let snapshot = HnswSnapshot {
            nodes: self
                .nodes
                .iter()
                .map(|n| HnswNodeSnapshot {
                    id: n.id,
                    vector: n.vector.clone(),
                    neighbors: n.neighbors.clone(),
                    level: n.level,
                    data_id: n.data_id.clone(),
                })
                .collect(),
            dim: self.dim,
            max_level: self.max_level,
            ef_construction: self.ef_construction,
            ef_search: self.ef_search,
            m: self.m,
            m_max0: self.m_max0,
            entry_point: self.entry_point,
            level_mult: self.level_mult,
            rng_seed: self.rng_seed,
            deleted: self.deleted.iter().copied().collect(),
        };
        bincode::serialize(&snapshot).expect("HNSW serialization should not fail")
    }

    /// Deserialize an index from bytes produced by `to_bytes()`.
    /// Backward compatible: older snapshots without `deleted` field default to empty.
    pub fn from_bytes(data: &[u8]) -> Result<Self, bincode::Error> {
        let snapshot: HnswSnapshot = bincode::deserialize(data)?;
        Ok(Self {
            nodes: snapshot
                .nodes
                .into_iter()
                .map(|n| HnswNode {
                    id: n.id,
                    vector: n.vector,
                    neighbors: n.neighbors,
                    level: n.level,
                    data_id: n.data_id,
                })
                .collect(),
            dim: snapshot.dim,
            max_level: snapshot.max_level,
            ef_construction: snapshot.ef_construction,
            ef_search: snapshot.ef_search,
            m: snapshot.m,
            m_max0: snapshot.m_max0,
            entry_point: snapshot.entry_point,
            level_mult: snapshot.level_mult,
            rng_seed: snapshot.rng_seed,
            deleted: snapshot.deleted.into_iter().collect(),
        })
    }
}

#[derive(Serialize, Deserialize)]
struct HnswSnapshot {
    nodes: Vec<HnswNodeSnapshot>,
    dim: usize,
    max_level: usize,
    ef_construction: usize,
    ef_search: usize,
    m: usize,
    m_max0: usize,
    entry_point: Option<usize>,
    level_mult: f64,
    rng_seed: u64,
    #[serde(default)]
    deleted: Vec<usize>,
}

#[derive(Serialize, Deserialize)]
struct HnswNodeSnapshot {
    id: usize,
    vector: Vec<f32>,
    neighbors: Vec<Vec<usize>>,
    level: usize,
    data_id: String,
}

/// Cosine distance using SIMD-accelerated dot product.
///
/// Computes 1 - cos(a, b) = 1 - dot(a,b) / (||a|| * ||b||).
/// Uses SIMD for the underlying dot product computations.
fn cosine_distance(a: &[f32], b: &[f32]) -> f32 {
    let dot = dot_product(a, b);
    let norm_a = dot_product(a, a).sqrt();
    let norm_b = dot_product(b, b).sqrt();
    let denom = norm_a * norm_b;
    if denom < 1e-10 {
        1.0
    } else {
        1.0 - dot / denom
    }
}

/// SIMD-accelerated dot product.
///
/// Uses NEON on aarch64 (Apple Silicon) and AVX2 on x86_64.
/// Falls back to scalar loop on other architectures.
fn dot_product(a: &[f32], b: &[f32]) -> f32 {
    #[cfg(target_arch = "aarch64")]
    {
        dot_product_neon(a, b)
    }
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx2") && is_x86_feature_detected!("fma") {
            // SAFETY: we just checked the feature is available
            unsafe { dot_product_avx2(a, b) }
        } else {
            dot_product_scalar(a, b)
        }
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    {
        dot_product_scalar(a, b)
    }
}

#[inline(always)]
#[allow(dead_code)] // Used on x86_64 and non-SIMD platforms
fn dot_product_scalar(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

/// NEON dot product for aarch64 (Apple M-series always has NEON).
/// Processes 4 floats per iteration.
#[cfg(target_arch = "aarch64")]
fn dot_product_neon(a: &[f32], b: &[f32]) -> f32 {
    use std::arch::aarch64::*;
    let len = a.len();
    let chunks = len / 4;
    let remainder = len % 4;
    let mut sum;

    // SAFETY: NEON is always available on aarch64. We process exactly `chunks*4` elements
    // from slices of length `len`, and `chunks*4 <= len`.
    unsafe {
        let mut acc = vdupq_n_f32(0.0);
        let a_ptr = a.as_ptr();
        let b_ptr = b.as_ptr();

        for i in 0..chunks {
            let va = vld1q_f32(a_ptr.add(i * 4));
            let vb = vld1q_f32(b_ptr.add(i * 4));
            acc = vfmaq_f32(acc, va, vb);
        }

        // Horizontal sum of 4-wide accumulator
        sum = vaddvq_f32(acc);
    }

    // Handle remaining elements
    let start = chunks * 4;
    for i in 0..remainder {
        sum += a[start + i] * b[start + i];
    }

    sum
}

/// AVX2 + FMA dot product for x86_64.
/// Processes 8 floats per iteration.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
unsafe fn dot_product_avx2(a: &[f32], b: &[f32]) -> f32 {
    use std::arch::x86_64::*;
    let len = a.len();
    let chunks = len / 8;
    let remainder = len % 8;

    let mut acc = _mm256_setzero_ps();
    let a_ptr = a.as_ptr();
    let b_ptr = b.as_ptr();

    for i in 0..chunks {
        // SAFETY: `i < chunks` and `chunks * 8 <= len`, so each 8-float read
        // ends within the slice. As in the NEON path, both slices have the
        // index's vector dimension.
        let (va, vb) = unsafe {
            (
                _mm256_loadu_ps(a_ptr.add(i * 8)),
                _mm256_loadu_ps(b_ptr.add(i * 8)),
            )
        };
        acc = _mm256_fmadd_ps(va, vb, acc);
    }

    // Horizontal sum of 8-wide accumulator
    let hi = _mm256_extractf128_ps(acc, 1);
    let lo = _mm256_castps256_ps128(acc);
    let sum128 = _mm_add_ps(lo, hi);
    let shuf = _mm_movehdup_ps(sum128);
    let sums = _mm_add_ps(sum128, shuf);
    let shuf2 = _mm_movehl_ps(sums, sums);
    let result = _mm_add_ss(sums, shuf2);
    let mut sum = _mm_cvtss_f32(result);

    // Handle remaining elements
    let start = chunks * 8;
    for i in 0..remainder {
        sum += a[start + i] * b[start + i];
    }

    sum
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_vec(dim: usize, val: f32) -> Vec<f32> {
        let mut v = vec![0.0; dim];
        v[0] = val;
        v[1] = 1.0 - val;
        v
    }

    #[test]
    fn test_insert_and_search() {
        let dim = 8;
        let mut idx = HnswIndex::new(dim);

        for i in 0..100 {
            let val = i as f32 / 100.0;
            let mut v = vec![0.0; dim];
            v[0] = val;
            v[1] = 1.0 - val;
            idx.insert(format!("doc_{i}"), v).unwrap();
        }

        assert_eq!(idx.len(), 100);

        let query = make_vec(dim, 0.5);
        let results = idx.search(&query, 5);
        assert!(!results.is_empty());
        assert!(results.len() <= 5);
        // The closest should be doc_50
        assert_eq!(results[0].0, "doc_50");
    }

    #[test]
    fn test_cosine_distance() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        assert!((cosine_distance(&a, &b)).abs() < 1e-6);

        let c = vec![0.0, 1.0, 0.0];
        assert!((cosine_distance(&a, &c) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_empty_search() {
        let idx = HnswIndex::new(4);
        let results = idx.search(&[1.0, 0.0, 0.0, 0.0], 10);
        assert!(results.is_empty());
    }

    #[test]
    fn test_recall_at_10() {
        let dim = 32;
        let n = 500;
        let mut idx = HnswIndex::with_params(dim, 16, 200, 100);

        let mut vectors: Vec<Vec<f32>> = Vec::new();
        for i in 0..n {
            let v: Vec<f32> = (0..dim)
                .map(|d| ((i * dim + d) as f64 * 0.1).sin() as f32)
                .collect();
            vectors.push(v.clone());
            idx.insert(format!("doc_{i}"), v).unwrap();
        }

        // Brute-force search for ground truth
        let query: Vec<f32> = (0..dim).map(|d| (d as f64 * 0.3).cos() as f32).collect();
        let mut brute: Vec<(usize, f32)> = vectors
            .iter()
            .enumerate()
            .map(|(i, v)| (i, cosine_distance(&query, v)))
            .collect();
        brute.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        let ground_truth: std::collections::HashSet<String> = brute
            .iter()
            .take(10)
            .map(|(i, _)| format!("doc_{i}"))
            .collect();

        let results = idx.search(&query, 10);
        let found: std::collections::HashSet<String> =
            results.iter().map(|(id, _)| id.clone()).collect();

        let recall = ground_truth.intersection(&found).count() as f64 / 10.0;
        assert!(recall >= 0.7, "HNSW recall@10 = {recall}, expected >= 0.7");
    }

    #[test]
    fn test_serialization_roundtrip() {
        let dim = 8;
        let mut idx = HnswIndex::new(dim);
        for i in 0..50 {
            let val = i as f32 / 50.0;
            idx.insert(format!("doc_{i}"), make_vec(dim, val)).unwrap();
        }

        let bytes = idx.to_bytes();
        let restored = HnswIndex::from_bytes(&bytes).unwrap();

        assert_eq!(restored.len(), 50);
        assert_eq!(restored.dim(), dim);

        // Search should produce same results
        let query = make_vec(dim, 0.5);
        let orig_results = idx.search(&query, 5);
        let restored_results = restored.search(&query, 5);
        assert_eq!(orig_results.len(), restored_results.len());
        assert_eq!(orig_results[0].0, restored_results[0].0);
    }

    #[test]
    fn test_serialization_empty() {
        let idx = HnswIndex::new(4);
        let bytes = idx.to_bytes();
        let restored = HnswIndex::from_bytes(&bytes).unwrap();
        assert!(restored.is_empty());
        assert_eq!(restored.dim(), 4);
    }

    #[test]
    fn test_get_vector() {
        let dim = 4;
        let mut idx = HnswIndex::new(dim);
        idx.insert("doc_a".to_string(), vec![1.0, 0.0, 0.0, 0.0])
            .unwrap();
        idx.insert("doc_b".to_string(), vec![0.0, 1.0, 0.0, 0.0])
            .unwrap();

        let v = idx.get_vector("doc_a").unwrap();
        assert_eq!(v, &vec![1.0, 0.0, 0.0, 0.0]);

        let v = idx.get_vector("doc_b").unwrap();
        assert_eq!(v, &vec![0.0, 1.0, 0.0, 0.0]);

        assert!(idx.get_vector("nonexistent").is_none());
    }

    #[test]
    fn test_level_clamped() {
        let mut idx = HnswIndex::new(4);
        // Insert many nodes — no level should exceed MAX_LEVEL
        for i in 0..1000 {
            idx.insert(format!("doc_{i}"), vec![i as f32 / 1000.0, 0.5, 0.3, 0.1])
                .unwrap();
        }
        for node in &idx.nodes {
            assert!(
                node.level <= MAX_LEVEL,
                "level {} exceeds max {}",
                node.level,
                MAX_LEVEL
            );
        }
    }
}
