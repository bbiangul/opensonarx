//! Real crawl + bounded local-corpus ablation. Never a market superiority test.
//! Run with --features onnx,facts-onnx; requires actual cached models and Ollama.
use oi_sdk::{AnswerModelConfig, AnswerPolicy, ChatModel, OpenSonarXClient, SdkConfig};
use oi_types::{ClaimExtractor, EmbeddingPipeline, EvidencePassage, FactExtractionPipeline};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{fs, path::Path, sync::Arc, time::Instant};

#[derive(Deserialize)]
struct Manifest {
    pages: Vec<String>,
    queries: Vec<Case>,
}
#[derive(Deserialize)]
struct Case {
    query: String,
    answerable: bool,
    #[serde(default)]
    gold_all: Vec<String>,
    #[serde(default)]
    gold_any: Vec<String>,
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.
}
fn write_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?
        .write_all(bytes)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if !(4..=7).contains(&args.len()) {
        return Err("usage: manifest.json gliner-model-directory NEW-output-directory [retained-html-directory] [retained-claims-directory] [only-variant]".into());
    }
    let manifest_bytes = fs::read(&args[1])?;
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes)?;
    let out = Path::new(&args[3]);
    fs::create_dir(out)?; // Refuse to overwrite an earlier run.
    write_new(&out.join("manifest.json"), &manifest_bytes)?;
    let git = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()?;
    let model_started = Instant::now();
    let embedder = Arc::new(oi_embed::MiniLmEmbedder::new()?); // No mock fallback.
    let embedding_load_ms = ms(model_started);
    let ner_started = Instant::now();
    let ner = oi_facts::GlinerFactExtractor::from_paths(
        &Path::new(&args[2]).join("model_quantized.onnx"),
        &Path::new(&args[2]).join("tokenizer.json"),
    )?;
    let ner_load_ms = ms(ner_started);
    let mut config = AnswerModelConfig::new("gpt-oss:20b");
    config.think = Some("low".into());
    config.timeout_secs = 90;
    let model = ChatModel::new(config.clone());
    let mut corpora: Vec<Vec<oi_types::ContentChunk>> = vec![vec![], vec![], vec![]];
    let mut legacy_duplicate_units = Vec::new();
    let mut pages: Vec<Value> = Vec::new();
    let mut robots = std::collections::HashMap::new();
    for (page_number, url) in manifest.pages.iter().enumerate() {
        eprintln!(
            "collecting page {} of {}",
            page_number + 1,
            manifest.pages.len()
        );
        let origin = format!("https://{}", oi_sdk::source::host(url));
        if !robots.contains_key(&origin) {
            let fetched = oi_crawl::fetcher::fetch_url(&format!("{origin}/robots.txt")).await;
            let rule = match fetched {
                Ok(f) if f.status == 404 => Some(oi_crawl::robots::parse_robots_txt("")),
                Ok(f) if f.status < 300 => Some(oi_crawl::robots::parse_robots_txt(&f.html)),
                _ => None, // Unavailable robots is not permission to crawl.
            };
            robots.insert(origin.clone(), rule);
        }
        let path = url.strip_prefix(&origin).unwrap_or("/");
        if !robots[&origin]
            .as_ref()
            .is_some_and(|r| r.is_path_allowed(path))
        {
            pages.push(json!({"url":url,"error":"robots unavailable or disallowed"}));
            continue;
        }
        let fetched_at = Instant::now();
        let reused = args
            .get(4)
            .map(|root| Path::new(root).join(format!("page-{page_number}.html")));
        let fetch = if let Some(snapshot) = reused {
            oi_crawl::fetcher::FetchResult {
                url: url.clone(),
                html: fs::read_to_string(snapshot)?,
                status: 200,
                content_type: "text/html".into(),
                fetch_time_ms: 0,
            }
        } else {
            match oi_crawl::fetcher::fetch_url(url).await {
                Ok(f) if f.status < 300 => f,
                Ok(f) => {
                    pages.push(json!({"url":url,"http_status":f.status,"error":"fetch status"}));
                    continue;
                }
                Err(e) => {
                    pages.push(json!({"url":url,"error":e.to_string()}));
                    continue;
                }
            }
        };
        let fetch_ms = ms(fetched_at);
        write_new(
            &out.join(format!("page-{page_number}.html")),
            fetch.html.as_bytes(),
        )?;
        let extracted_at = Instant::now();
        let crawled = match oi_crawl::pipeline::process_html(url, &fetch.html, 256, 64) {
            Ok(c) => c,
            Err(e) => {
                pages.push(json!({"url":url,"error":e.to_string(),"fetch_ms":fetch_ms}));
                continue;
            }
        };
        let html_chunk_ms = ms(extracted_at);
        write_new(
            &out.join(format!("page-{page_number}.md")),
            crawled.markdown.as_bytes(),
        )?;
        let total_chunks = crawled.chunks.len();
        // Freeze one shared budget: first eight chunks, no selection after seeing answers.
        let mut plain: Vec<_> = crawled.chunks.into_iter().take(8).collect();
        let embedding_at = Instant::now();
        oi_crawl::pipeline::embed_chunks(&mut plain, embedder.as_ref())?;
        let embedding_ms = ms(embedding_at);
        let mut entities = plain.clone();
        let ner_at = Instant::now();
        let mut entity_count = 0;
        let mut ner_errors = Vec::new();
        for chunk in &mut entities {
            match ner.extract(&chunk.content) {
                Ok(f) => {
                    entity_count += f.entities.len();
                    if let Some(meta) = &mut chunk.meta {
                        meta.facts = Some(f);
                    }
                }
                Err(e) => ner_errors.push(e.to_string()),
            }
        }
        let ner_ms = ms(ner_at);
        let passages: Vec<_> = plain
            .iter()
            .enumerate()
            .map(|(n, c)| EvidencePassage {
                id: format!("p{n}"),
                url: url.clone(),
                title: c.meta.as_ref().map(|m| m.title.clone()).unwrap_or_default(),
                text: oi_sdk::answer::plain_text(&c.content),
                retained: true,
                published_at: c.meta.as_ref().and_then(|m| m.published_at),
                observed_at: None,
            })
            .collect();
        let claim_at = Instant::now();
        let proposed = if let Some(root) = args.get(5) {
            let recorded: Value = serde_json::from_slice(&fs::read(
                Path::new(root).join(format!("page-{page_number}-metrics.json")),
            )?)?;
            Ok(oi_types::ClaimExtraction {
                request_kind: oi_types::RequestKind::Question,
                destination_passage_id: None,
                missing: vec![],
                claims: recorded["claims"]
                    .as_array()
                    .ok_or("missing recorded claims")?
                    .iter()
                    .map(|c| oi_types::ProposedClaim {
                        text: c["claim"].as_str().unwrap_or_default().into(),
                        substitution: None,
                        quotes: vec![oi_types::ProposedQuote {
                            passage_id: c["passage_id"].as_str().unwrap_or_default().into(),
                            quote: c["quote"].as_str().unwrap_or_default().into(),
                        }],
                    })
                    .collect(),
            })
        } else {
            model.extract("What factual statements about the document's subject are explicitly made in these passages? Give at most eight self-contained statements, retaining dates, negation and attribution when present.", &passages)
        };
        let claims_ms = ms(claim_at);
        let mut augmented = plain.clone();
        legacy_duplicate_units.extend(plain.iter().cloned());
        let mut rejected = 0;
        let mut accepted = Vec::new();
        let mut claim_error = None;
        let extra_embedding_at = Instant::now();
        match proposed {
            Ok(extraction) => {
                for (number, claim) in extraction.claims.into_iter().take(8).enumerate() {
                    let located = claim.quotes.iter().find_map(|quote| {
                        passages
                            .iter()
                            .position(|p| p.id == quote.passage_id)
                            .and_then(|index| {
                                oi_sdk::answer::locate_quote(
                                    &passages[index].text,
                                    &quote.quote,
                                    20,
                                )
                                .map(|span| (index, span))
                            })
                    });
                    if let Some((index, span)) = located {
                        // Distinct, source-derived context; do not forge a new hash
                        // for identical content to evade the production deduplicator.
                        let mut unit = plain[index].clone();
                        unit.chunk_id = format!("{}-claim-{number}", unit.chunk_id);
                        let mut old_projection = unit.clone();
                        let body = &passages[index].text;
                        let at = body.find(&span).ok_or("located span disappeared")?;
                        let mut from = at.saturating_sub(300);
                        while !body.is_char_boundary(from) {
                            from += 1;
                        }
                        let mut to = (at + span.len() + 300).min(body.len());
                        while !body.is_char_boundary(to) {
                            to -= 1;
                        }
                        unit.content = body[from..to].into();
                        if let Some(meta) = &mut unit.meta {
                            meta.content_hash = hash(unit.content.as_bytes());
                        }
                        unit.embedding = Some(embedder.embed_text(&claim.text)?);
                        old_projection.embedding = unit.embedding.clone();
                        legacy_duplicate_units.push(old_projection);
                        augmented.push(unit);
                        accepted.push(json!({"claim":claim.text,"quote":span,"passage_id":passages[index].id}));
                    } else {
                        rejected += 1;
                    }
                }
            }
            Err(e) => claim_error = Some(e.to_string()),
        }
        let extra_embedding_ms = ms(extra_embedding_at);
        // Production admits at most eight chunks per URL. Replace, do not
        // append beyond that cap: at most four source-derived claim contexts,
        // with the remaining slots occupied by ordinary source chunks.
        let derived = augmented.split_off(plain.len());
        let keep = derived.len().min(4);
        augmented = plain[..plain.len().saturating_sub(keep)].to_vec();
        augmented.extend(derived.into_iter().take(keep));
        let row = json!({"url":url,"html_sha256":hash(fetch.html.as_bytes()),
            "markdown_sha256":hash(crawled.markdown.as_bytes()),"html_bytes":fetch.html.len(),
            "markdown_bytes":crawled.markdown.len(),"available_chunks":total_chunks,"sampled_chunks":plain.len(),
            "fetch_ms":fetch_ms,"fetch_is_snapshot_read":args.len()>=5,
            "claims_replayed_from_real_capture":args.get(5),
            "html_quality_chunk_ms":html_chunk_ms,"embedding_ms":embedding_ms,
            "ner_ms":ner_ms,"entities":entity_count,"ner_errors":ner_errors,"relation_facts":0,
            "claims_ms":claims_ms,"claim_embedding_ms":extra_embedding_ms,
            "claims":accepted,"rejected_claims":rejected,"claim_error":claim_error,
            "serialized_chunk_bytes":[serde_json::to_vec(&plain)?.len(),serde_json::to_vec(&entities)?.len(),serde_json::to_vec(&augmented)?.len()]});
        write_new(
            &out.join(format!("page-{page_number}-metrics.json")),
            &serde_json::to_vec_pretty(&row)?,
        )?;
        pages.push(row);
        corpora[0].extend(plain);
        corpora[1].extend(entities);
        corpora[2].extend(augmented);
    }
    let names = ["plain", "entities", "claim_augmented"];
    let audit_client = OpenSonarXClient::with_embedder(SdkConfig::default(), embedder.clone())
        .with_search_engines(vec![]);
    audit_client.ingest(&legacy_duplicate_units)?;
    let duplicate_projection_audit = json!({"attempted_units":legacy_duplicate_units.len(),
        "active_units":audit_client.indexed_document_count()});
    let mut clients: Vec<OpenSonarXClient> = Vec::new();
    let mut ingest = Vec::new();
    for (i, chunks) in corpora.iter().enumerate() {
        let client = OpenSonarXClient::with_embedder(
            SdkConfig {
                answer_model: Some(config.clone()),
                search_engines: Some(vec![]),
                ..Default::default()
            },
            embedder.clone(),
        )
        .with_search_engines(vec![])
        .with_answer_policy(AnswerPolicy {
            min_source_families: 1,
            max_hydrations: 0,
            ..Default::default()
        });
        let t = Instant::now();
        client.ingest(chunks)?;
        let active = client.indexed_document_count();
        let active_projections = client
            .index()
            .get_chunks_for_replication(None, 1000)?
            .iter()
            .filter(|c| c.chunk_id.contains("-claim-"))
            .count();
        if i == 2 && active_projections == 0 {
            return Err("claim variant added no active index units; invalid ablation".into());
        }
        ingest.push(
            json!({"variant":names[i],"chunks":chunks.len(),"active_index_chunks":active,
            "active_claim_projections":active_projections,"index_ingest_ms":ms(t)}),
        );
        clients.push(client);
    }
    let mut answers = Vec::new();
    for (case_number, case) in manifest.queries.iter().enumerate() {
        // Rotate execution order; every query/variant has a new answer-cache key.
        for offset in 0..3 {
            let i = (case_number + offset) % 3;
            if args.get(6).is_some_and(|selected| selected != names[i]) {
                continue;
            }
            eprintln!(
                "answer case {} / {}, variant {}",
                case_number + 1,
                manifest.queries.len(),
                names[i]
            );
            let t = Instant::now();
            let outcome = clients[i].search_answered(&case.query, 5, &Default::default());
            let row = match outcome {
                Ok(answer) => {
                    let text = answer.answer.summary.to_lowercase();
                    let emitted = answer.answer.status == "extractive";
                    let keyword_match = case
                        .gold_all
                        .iter()
                        .all(|g| text.contains(&g.to_lowercase()))
                        && (case.gold_any.is_empty()
                            || case
                                .gold_any
                                .iter()
                                .any(|g| text.contains(&g.to_lowercase())));
                    json!({"case":case_number,"query":case.query,"variant":names[i],"answerable":case.answerable,
                        "latency_ms":ms(t),"emitted":emitted,"gold_keyword_match":keyword_match,
                        "unsupported_emission":!case.answerable && emitted,"answer":answer})
                }
                Err(e) => {
                    json!({"case":case_number,"query":case.query,"variant":names[i],"latency_ms":ms(t),"error":e.to_string()})
                }
            };
            write_new(
                &out.join(format!("case-{case_number}-{i}.json")),
                &serde_json::to_vec_pretty(&row)?,
            )?;
            answers.push(row);
        }
    }
    let report = json!({"status":"exploratory_not_confirmatory","git_head":String::from_utf8_lossy(&git.stdout).trim(),
        "manifest_sha256":hash(&manifest_bytes),"embedding":"real arctic-embed-s INT8 ONNX",
        "ner":"real gliner_multi-v2.1 quantized ONNX","chat":"real Ollama gpt-oss:20b think=low",
        "embedding_load_ms":embedding_load_ms,"ner_load_ms":ner_load_ms,
        "pages":pages,"ingest":ingest,"answers":answers,"duplicate_projection_audit":duplicate_projection_audit,
        "claims_replayed_from_real_capture":args.get(5),"only_variant":args.get(6),
        "limitations":["Four selected documentation URLs; not a representative crawl or sealed holdout.",
        "First eight 256-word chunks per page; HTML retained once and reused in all variants.",
        "One run per answer; rotated variant order; model nondeterminism remains.",
        "Single-source policy override for this local-corpus test, not production defaults.",
        "Claims variant is a benchmark-only source-preserving augmentation, not deployed architecture.",
        "Gold keyword matching is diagnostic, not a semantic correctness judgment.",
        "No paid API calls; local compute, energy and hosting costs are NOT zero or measured.",
        "P2P/discovery/market baseline and real GLiNER relation extraction are not exercised."]});
    write_new(
        &out.join("report.json"),
        &serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{}", out.join("report.json").display());
    Ok(())
}
