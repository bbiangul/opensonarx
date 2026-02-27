# `oi seed` — Technical Docs Corpus Builder

Batch-seed the local OpenSonarX search index from known documentation sources. Designed to build a day-0 corpus of high-quality technical content for AI agents.

## Quick Start

```bash
# Seed from a sitemap
oi seed sitemap https://docs.rs/sitemap/tokio/sitemap.xml

# Seed from a URL list
oi seed urls ./my-urls.txt

# Seed top 500 Rust crates from docs.rs
oi seed registry docs-rs --top 500

# Seed all English MDN Web Docs
oi seed registry mdn
```

## Built-in Sources

### docs.rs (Rust ecosystem)

```bash
oi seed registry docs-rs --top <N> [-c <concurrency>]
```

Fetches the top N crates by download count from the [crates.io API](https://crates.io/api/v1/crates?sort=downloads), then builds `https://docs.rs/crate/{name}/latest` entry URLs for each.

- **Content**: Rust API documentation (auto-generated from doc comments)
- **Scale**: ~160k crates available; `--top 500` covers the most-used ecosystem
- **Rate limiting**: 500ms delay between crates.io API pages
- **Example crates at top-500**: serde, tokio, rand, reqwest, clap, hyper, axum, tracing, regex, chrono, anyhow, thiserror, futures, bytes, log, syn, quote, proc-macro2, libc, etc.

### MDN Web Docs

```bash
oi seed registry mdn [-c <concurrency>]
```

Fetches the official English sitemap from `https://developer.mozilla.org/sitemaps/en-us/sitemap.xml.gz`, decompresses it, and extracts all documentation URLs.

- **Content**: Web platform reference — HTML, CSS, JavaScript, Web APIs, HTTP, accessibility, SVG, MathML
- **Scale**: ~12,000+ pages covering the full web platform
- **Source**: Mozilla's canonical sitemap (gzipped XML)
- **Covers**: MDN reference docs, guides, tutorials, and glossary entries

### Custom Sitemap

```bash
oi seed sitemap <url> [-c <concurrency>]
```

Fetches a sitemap.xml (or sitemap.xml.gz) and crawls all listed URLs. Supports:

- Standard `<urlset>` sitemaps
- Sitemap index files (`<sitemapindex>`) with one level of nested sitemap resolution
- Gzipped sitemaps (`.xml.gz`) via binary fetch + gzip decompression

A curated list of 43 verified sitemap URLs is available at [`examples/seed-sitemaps.txt`](examples/seed-sitemaps.txt). To seed all of them:

```bash
while IFS= read -r url; do
  [[ "$url" =~ ^#.*$ || -z "$url" ]] && continue
  oi seed sitemap "$url" -c 8
done < examples/seed-sitemaps.txt
```

The list covers:

| Category | Sources | Pages |
|----------|---------|-------|
| **Languages** | Python, Node.js, Go, TypeScript, C#/.NET, Kotlin, PHP, Ruby | ~10k+ |
| **Frameworks** | Next.js, Vue, Angular, Django, FastAPI, Flask, Laravel, Rails, Spring | ~3k+ |
| **Databases** | PostgreSQL, Redis, MongoDB, Elasticsearch | ~2k+ |
| **Cloud/Infra** | AWS, GCP, Docker, Kubernetes, Cloudflare, Vercel, GitLab, Supabase, Firebase, Nginx | ~20k+ |
| **AI/ML** | PyTorch, TensorFlow, Hugging Face, LangChain, OpenAI, Anthropic | ~5k+ |
| **DevTools** | Git, Webpack, Vite, ESLint, gRPC, Grafana, Ansible | ~2k+ |

**Not available** (no sitemap found): Apple Developer, cppreference, React, Svelte, Tailwind CSS, MySQL, SQLite, HashiCorp, GitHub Docs, GraphQL, Prometheus.

### Custom URL List

```bash
oi seed urls <file> [-c <concurrency>]
```

Reads URLs from a text file, one per line. Lines starting with `#` are treated as comments and blank lines are skipped.

```text
# Core Rust docs
https://doc.rust-lang.org/std/index.html
https://doc.rust-lang.org/book/title-page.html
https://doc.rust-lang.org/cargo/index.html

# Tokio ecosystem
https://docs.rs/tokio/latest/tokio/
https://docs.rs/hyper/latest/hyper/
https://docs.rs/axum/latest/axum/
```

## Options

| Flag | Default | Description |
|------|---------|-------------|
| `-c, --concurrency` | 4 | Number of concurrent crawl workers |
| `-v, --verbose` | off | Print per-URL progress (`[OK]`, `[SKIP]`, `[ERR]`) |
| `--top` | 100 | (docs-rs only) Number of top crates to seed |

## How It Works

1. **Resolve URLs** from the chosen source (sitemap, file, or registry)
2. **Check robots.txt** for each domain (fetched once, cached per-domain)
3. **Crawl each URL** through the `Distiller` pipeline: fetch HTML, extract to markdown, quality-gate, chunk, embed
4. **Ingest chunks** into the local HybridIndex (HNSW + BM25) at `~/.opensonarx/index/`
5. **Save periodically** (every 500 indexed URLs) and on completion
6. **Print summary**: total, indexed, failed, robots-skipped, final doc count

## Index Location

Seeded content is stored at `~/.opensonarx/index/` — the same path used by `oi network node` and `oi sentinel start`. This means:

- Running `oi network node` after seeding will serve the seeded content to P2P peers
- Re-running `oi seed` adds to the existing index (duplicates are skipped via `chunk_id` dedup)
- Use `oi index info` to inspect the index
- Use `oi index export` to create a portable archive for sharing with other nodes

## Recommended Seed Strategy

For a comprehensive technical docs corpus:

```bash
# 1. Built-in sources
oi seed registry docs-rs --top 500 -c 8
oi seed registry mdn -c 8

# 2. All 43 verified sitemaps
while IFS= read -r url; do
  [[ "$url" =~ ^#.*$ || -z "$url" ]] && continue
  oi seed sitemap "$url" -c 8
done < examples/seed-sitemaps.txt

# 3. Verify
oi index info
```
