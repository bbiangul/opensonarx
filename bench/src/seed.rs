use oi_index::HybridIndex;
use oi_types::traits::SearchIndex;
use oi_types::{ChunkMeta, ContentChunk};
use rand::Rng;

use crate::embed::text_to_embedding;

const DIM: usize = 384;

pub static DOMAINS: &[&str] = &[
    "docs.rust-lang.org",
    "stackoverflow.com",
    "arxiv.org",
    "github.com",
    "en.wikipedia.org",
    "developer.mozilla.org",
    "news.ycombinator.com",
    "medium.com",
    "dev.to",
    "crates.io",
    "reddit.com",
    "python.org",
    "golang.org",
    "nodejs.org",
    "reactjs.org",
    "kubernetes.io",
    "docker.com",
    "aws.amazon.com",
    "cloud.google.com",
    "azure.microsoft.com",
    "docs.python.org",
    "doc.rust-lang.org",
    "pkg.go.dev",
    "learn.microsoft.com",
    "developer.apple.com",
    "pytorch.org",
    "tensorflow.org",
    "huggingface.co",
    "openai.com",
    "anthropic.com",
    "security.stackexchange.com",
    "crypto.stackexchange.com",
    "math.stackexchange.com",
    "cs.stackexchange.com",
    "electronics.stackexchange.com",
    "dba.stackexchange.com",
    "serverfault.com",
    "superuser.com",
    "askubuntu.com",
    "unix.stackexchange.com",
    "codereview.stackexchange.com",
    "softwareengineering.stackexchange.com",
    "gamedev.stackexchange.com",
    "datascience.stackexchange.com",
    "stats.stackexchange.com",
    "ai.stackexchange.com",
    "devops.stackexchange.com",
    "webapps.stackexchange.com",
    "ux.stackexchange.com",
    "pm.stackexchange.com",
];

pub static TOPICS: &[&str] = &[
    "rust programming language systems safety concurrency",
    "python machine learning data science numpy pandas",
    "javascript web development react frontend typescript",
    "kubernetes container orchestration deployment pods",
    "database sql postgres optimization indexing queries",
    "cryptography blockchain decentralized protocol staking",
    "neural network deep learning transformer attention model",
    "distributed systems consensus replication fault tolerance",
    "linux kernel operating system memory management process",
    "compiler optimization abstract syntax tree parsing llvm",
    "networking tcp http protocol websocket grpc",
    "security vulnerability exploit mitigation defense audit",
    "cloud computing serverless microservices api gateway",
    "testing continuous integration deployment pipeline ci cd",
    "functional programming monads category theory haskell",
    "embedded systems iot sensor real time firmware",
    "graphics rendering shader gpu compute vulkan",
    "natural language processing tokenization embeddings bert",
    "game engine physics simulation collision detection unity",
    "robotics control systems path planning autonomous navigation",
];

/// Seed a HybridIndex with `n` documents using clustered embeddings.
pub fn seed_index(n: usize) -> HybridIndex {
    let index = HybridIndex::new(DIM);
    let mut rng = rand::thread_rng();
    let num_clusters = TOPICS.len();
    let batch_size = 1000;

    for batch_start in (0..n).step_by(batch_size) {
        let batch_end = (batch_start + batch_size).min(n);
        let chunks: Vec<ContentChunk> = (batch_start..batch_end)
            .map(|i| {
                let cluster = i % num_clusters;
                let domain = DOMAINS[i % DOMAINS.len()];
                let topic = TOPICS[cluster];
                let text = format!("{topic} document {i} with extra content for realistic sizing");

                // Clustered embedding: base from topic + small perturbation
                let mut emb = text_to_embedding(&format!("{topic}_cluster_{cluster}"), DIM);
                for x in &mut emb {
                    *x += rng.gen_range(-0.05..0.05);
                }
                // Re-normalize
                let norm: f32 = emb.iter().map(|x| x * x).sum::<f32>().sqrt();
                if norm > 1e-10 {
                    for x in &mut emb {
                        *x /= norm;
                    }
                }

                let stake_usd: f64 = rng.gen_range(10.0..5000.0);
                let reputation: f64 = rng.gen_range(0.5..1.0);
                let _ = (stake_usd, reputation); // Used by bench server for context

                let now = 1700000000u64 + rng.gen_range(0..30 * 86400);

                ContentChunk {
                    chunk_id: format!("doc_{i}"),
                    content: text,
                    embedding: Some(emb),
                    seq: 0,
                    meta: Some(ChunkMeta {
                        url: format!("https://{domain}/doc/{i}"),
                        title: format!("{} — Document {i}", TOPICS[cluster].split_whitespace().next().unwrap_or("Doc")),
                        domain: domain.to_string(),
                        content_hash: format!("sha256_{i:08x}"),
                        crawled_at: Some(now),
                        content_type: Some("text/html".to_string()),
                        entity: None,
                        content_kind: None,
                        ..Default::default()
                    }),
                }
            })
            .collect();

        index.ingest(&chunks).unwrap();
    }

    tracing::info!("Seeded index with {} documents", n);
    index
}
