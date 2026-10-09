use oi_sdk::{classify_query, OpenSonarXClient, SdkConfig};
use oi_types::QueryFilters;
use serde::Serialize;
use std::time::Instant;

#[derive(Clone, Copy)]
struct LiveCase {
    id: &'static str,
    query: &'static str,
    top1_any: &'static [&'static str],
    top1_none: &'static [&'static str],
    forbidden_top5: &'static [&'static str],
    expected_uncertainty: &'static str,
    expected_answer_status: &'static str,
}

const CASES: &[LiveCase] = &[
    LiveCase {
        id: "nav_claude",
        query: "claude",
        top1_any: &["claude.com/", "claude.ai/"],
        top1_none: &[],
        forbidden_top5: &["msmarco.example"],
        expected_uncertainty: "not_applicable",
        expected_answer_status: "destination",
    },
    LiveCase {
        id: "nav_chatgpt",
        query: "chatgpt",
        top1_any: &["chatgpt.com/", "openai.com/chatgpt"],
        top1_none: &[],
        forbidden_top5: &["msmarco.example"],
        expected_uncertainty: "not_applicable",
        expected_answer_status: "destination",
    },
    LiveCase {
        id: "nav_perplexity",
        query: "perplexity",
        top1_any: &["perplexity.ai/"],
        top1_none: &[],
        forbidden_top5: &["msmarco.example"],
        expected_uncertainty: "not_applicable",
        expected_answer_status: "destination",
    },
    LiveCase {
        id: "nav_gama_fund",
        query: "gama fund",
        top1_any: &["gamafund.com/"],
        top1_none: &[],
        forbidden_top5: &["msmarco.example"],
        expected_uncertainty: "not_applicable",
        expected_answer_status: "destination",
    },
    LiveCase {
        id: "nav_gama_fund_typo",
        query: "gama found",
        top1_any: &["gamafund.com/"],
        top1_none: &["luma.com/", "gama-am.ch", "gamainvestimentos.com.br"],
        forbidden_top5: &["msmarco.example"],
        expected_uncertainty: "not_applicable",
        expected_answer_status: "destination",
    },
    LiveCase {
        id: "transaction_gama_application",
        query: "gama fund inscrição",
        top1_any: &["gamafund.com/", "startups.com.br/", "luma.com/"],
        top1_none: &["gama-am.ch", "gamainvestimentos.com.br"],
        forbidden_top5: &["msmarco.example"],
        expected_uncertainty: "not_applicable",
        expected_answer_status: "destination",
    },
    LiveCase {
        id: "transaction_claude_login",
        query: "claude login",
        top1_any: &[
            "claude.ai/login",
            "claude.com/login",
            "claude.ai/magic-link",
        ],
        top1_none: &["login-Claude"],
        forbidden_top5: &["msmarco.example"],
        expected_uncertainty: "not_applicable",
        expected_answer_status: "destination",
    },
    LiveCase {
        id: "diligence_gama",
        query: "gama fund é legítimo",
        top1_any: &[],
        top1_none: &[
            "gamafund.com/",
            "gama-am.ch",
            "gamainvestimentos.com.br",
            "luma.com/",
        ],
        forbidden_top5: &["msmarco.example"],
        expected_uncertainty: "evidential_only",
        expected_answer_status: "insufficient_evidence",
    },
    LiveCase {
        id: "nav_nubank",
        query: "nubank",
        top1_any: &["nubank.com.br/"],
        top1_none: &[],
        forbidden_top5: &["msmarco.example"],
        expected_uncertainty: "not_applicable",
        expected_answer_status: "destination",
    },
    LiveCase {
        id: "nav_receita_federal",
        query: "receita federal",
        top1_any: &["gov.br/receitafederal"],
        top1_none: &[],
        forbidden_top5: &["msmarco.example"],
        expected_uncertainty: "not_applicable",
        expected_answer_status: "destination",
    },
    LiveCase {
        id: "ambiguous_java",
        query: "java",
        top1_any: &["java.com/", "oracle.com/java"],
        top1_none: &[],
        forbidden_top5: &["msmarco.example"],
        expected_uncertainty: "not_applicable",
        expected_answer_status: "destination",
    },
    LiveCase {
        id: "nav_python",
        query: "python",
        top1_any: &["python.org/"],
        top1_none: &["learnpython.org"],
        forbidden_top5: &["msmarco.example"],
        expected_uncertainty: "not_applicable",
        expected_answer_status: "destination",
    },
    LiveCase {
        id: "tool_python_compiler",
        query: "python online compiler",
        top1_any: &[],
        top1_none: &["python.org/"],
        forbidden_top5: &["msmarco.example"],
        expected_uncertainty: "not_applicable",
        expected_answer_status: "tool_results",
    },
    LiveCase {
        id: "current_brazil_election_poll",
        query: "pesquisa eleitoral presidente brasil hoje",
        top1_any: &[],
        top1_none: &[
            "/tudo-sobre/",
            "que-horas-sai",
            "quando-sai",
            "previsao",
            "ultimas-pesquisas",
            "receitafederal",
        ],
        forbidden_top5: &["msmarco.example", "receitafederal"],
        expected_uncertainty: "evidential_only",
        expected_answer_status: "extractive",
    },
    LiveCase {
        id: "factual_anthropic_founders",
        query: "quem fundou a Anthropic?",
        top1_any: &[],
        top1_none: &["claude.com/"],
        forbidden_top5: &["msmarco.example"],
        expected_uncertainty: "evidential_only",
        expected_answer_status: "insufficient_evidence",
    },
    LiveCase {
        id: "factual_pix_creator",
        query: "quem criou o Pix?",
        top1_any: &[],
        top1_none: &["pixbet", "instagram.com", "facebook.com"],
        forbidden_top5: &["msmarco.example", "pixbet"],
        expected_uncertainty: "evidential_only",
        expected_answer_status: "extractive",
    },
    LiveCase {
        id: "current_generic_ai_news",
        query: "últimas notícias de inteligência artificial hoje",
        top1_any: &[],
        top1_none: &["datafolha", "pesquisa-eleitoral", "receitafederal"],
        forbidden_top5: &[
            "datafolha",
            "msmarco.example",
            "pesquisa-eleitoral",
            "receitafederal",
        ],
        expected_uncertainty: "evidential_only",
        expected_answer_status: "insufficient_evidence",
    },
    LiveCase {
        id: "exploratory_quantum_definition",
        query: "o que é computação quântica",
        top1_any: &[],
        top1_none: &["receitafederal", "restaurante"],
        forbidden_top5: &["msmarco.example", "receitafederal"],
        expected_uncertainty: "not_applicable",
        expected_answer_status: "results_only",
    },
    LiveCase {
        id: "exploratory_tax_guidance",
        query: "como declarar imposto de renda 2026",
        top1_any: &[
            "gov.br/receitafederal",
            "gov.br/pt-br/servicos/declarar-meu-imposto-de-renda",
        ],
        top1_none: &[],
        forbidden_top5: &["msmarco.example", "restaurante"],
        expected_uncertainty: "not_applicable",
        expected_answer_status: "results_only",
    },
    LiveCase {
        id: "exploratory_dengue_guidance",
        query: "sintomas de dengue quando ir ao médico",
        top1_any: &[
            "gov.br/saude",
            "saude.sp.gov.br",
            "fiocruz.br",
            "paho.org",
            "who.int",
        ],
        top1_none: &[],
        forbidden_top5: &["msmarco.example", "receitafederal"],
        expected_uncertainty: "not_applicable",
        expected_answer_status: "results_only",
    },
    LiveCase {
        id: "current_bitcoin_price",
        query: "preço do bitcoin hoje",
        top1_any: &[],
        top1_none: &["receitafederal", "restaurante"],
        forbidden_top5: &["msmarco.example", "receitafederal"],
        expected_uncertainty: "evidential_only",
        expected_answer_status: "extractive",
    },
    LiveCase {
        id: "exploratory_local_restaurants",
        query: "restaurantes japoneses perto de mim",
        top1_any: &[],
        top1_none: &["receitafederal", "imposto-de-renda"],
        forbidden_top5: &["msmarco.example", "receitafederal", "imposto-de-renda"],
        expected_uncertainty: "not_applicable",
        expected_answer_status: "results_only",
    },
    LiveCase {
        id: "exploratory_notebook_recommendation",
        query: "melhor notebook para programar em 2026",
        top1_any: &[],
        top1_none: &["receitafederal", "restaurante"],
        forbidden_top5: &["msmarco.example", "receitafederal"],
        expected_uncertainty: "not_applicable",
        expected_answer_status: "results_only",
    },
    LiveCase {
        id: "current_brazil_president",
        query: "quem é o presidente do Brasil?",
        top1_any: &[
            "gov.br/planalto/pt-br/acesso-a-informacao/institucional/principais-cargos-e-respectivos-ocupantes",
            "gov.br/planalto/pt-br/conheca-a-presidencia/biografia-do-presidente",
        ],
        top1_none: &[],
        forbidden_top5: &[
            "msmarco.example",
            "receitafederal",
            "quem-criou-o-pix",
        ],
        expected_uncertainty: "evidential_only",
        expected_answer_status: "extractive",
    },
];

/// Every query that has already influenced the development regression suite.
/// Generalization captures reject exact normalized overlap with this set so a
/// passing regression case cannot be relabeled as held-out evidence.
pub(crate) fn development_query_texts() -> Vec<&'static str> {
    let mut queries = std::collections::HashSet::new();
    for case in CASES {
        queries.insert(case.query);
        queries.extend(query_variants(*case).iter().copied());
    }
    queries.into_iter().collect()
}

fn query_variants(case: LiveCase) -> &'static [&'static str] {
    match case.id {
        "nav_claude" => &[
            "claude",
            "site oficial claude",
            "claude site oficial",
            "página oficial claude",
        ],
        "nav_chatgpt" => &[
            "chatgpt",
            "site oficial chatgpt",
            "chatgpt site oficial",
            "página oficial chatgpt",
        ],
        "nav_perplexity" => &[
            "perplexity",
            "site oficial perplexity",
            "perplexity site oficial",
            "página oficial perplexity",
        ],
        "nav_gama_fund" => &[
            "gama fund",
            "site oficial gama fund",
            "gama fund site oficial",
            "página oficial gama fund",
        ],
        "nav_gama_fund_typo" => &[
            "gama found",
            "site oficial gama found",
            "gama found página oficial",
            "gama fnd",
        ],
        "transaction_gama_application" => &[
            "gama fund inscrição",
            "inscrição gama fund",
            "gama fund aplicar",
            "gama fund inscrever",
        ],
        "transaction_claude_login" => &[
            "claude login",
            "login claude",
            "acessar claude",
            "entrar claude",
        ],
        "diligence_gama" => &[
            "gama fund é legítimo",
            "gama fund é confiável",
            "gama fund é fraude",
            "gama fund é seguro",
        ],
        "nav_nubank" => &[
            "nubank",
            "site oficial nubank",
            "nubank site oficial",
            "página oficial nubank",
        ],
        "nav_receita_federal" => &[
            "receita federal",
            "site oficial receita federal",
            "receita federal site oficial",
            "página oficial receita federal",
        ],
        "ambiguous_java" => &[
            "java",
            "site oficial java",
            "java site oficial",
            "página oficial java",
        ],
        "nav_python" => &[
            "python",
            "site oficial python",
            "python site oficial",
            "página oficial python",
        ],
        "tool_python_compiler" => &[
            "python online compiler",
            "online python compiler",
            "compilador python online",
            "python browser compiler",
        ],
        "current_brazil_election_poll" => &[
            "pesquisa eleitoral presidente brasil hoje",
            "resultado pesquisa eleitoral presidente brasil hoje",
            "pesquisa eleitoral atual para presidente do brasil",
            "última pesquisa para presidente do brasil",
        ],
        "factual_anthropic_founders" => &[
            "quem fundou a Anthropic?",
            "quem são os fundadores da Anthropic?",
            "quem criou a Anthropic?",
            "who founded Anthropic?",
        ],
        "factual_pix_creator" => &[
            "quem criou o Pix?",
            "quem inventou o Pix?",
            "who created Pix?",
            "qual instituição criou o Pix?",
        ],
        "current_generic_ai_news" => &[
            "últimas notícias de inteligência artificial hoje",
            "notícias de IA hoje",
            "latest artificial intelligence news",
            "últimas notícias sobre IA",
        ],
        "exploratory_quantum_definition" => &[
            "o que é computação quântica",
            "como funciona computação quântica",
            "computação quântica explicada",
            "what is quantum computing",
        ],
        "exploratory_tax_guidance" => &[
            "como declarar imposto de renda 2026",
            "declaração de imposto de renda 2026",
            "imposto de renda 2026 como declarar",
            "orientações imposto de renda 2026",
        ],
        "exploratory_dengue_guidance" => &[
            "sintomas de dengue quando ir ao médico",
            "dengue sinais de alarme",
            "quando procurar médico por dengue",
            "sintomas graves de dengue",
        ],
        "current_bitcoin_price" => &[
            "preço do bitcoin hoje",
            "cotação do bitcoin agora",
            "bitcoin price today",
            "valor atual do bitcoin",
        ],
        "exploratory_local_restaurants" => &[
            "restaurantes japoneses perto de mim",
            "comida japonesa perto de mim",
            "restaurante japonês próximo",
            "sushi perto de mim",
        ],
        "exploratory_notebook_recommendation" => &[
            "melhor notebook para programar em 2026",
            "notebook para desenvolvimento de software 2026",
            "qual laptop comprar para programação",
            "best laptop for programming 2026",
        ],
        "current_brazil_president" => &[
            "quem é o presidente do Brasil?",
            "quem é o atual presidente do Brasil?",
            "atual presidente do Brasil",
            "who is the current president of Brazil?",
        ],
        _ => &[],
    }
}

#[derive(Debug, Serialize)]
struct ResultSummary {
    decision_rank: u32,
    retrieval_rank: u32,
    url: String,
    title: String,
    source: String,
    published_at: Option<i64>,
    crawled_at: Option<u64>,
    relevance_score: f64,
    decision_score: f64,
    uncertainty_mode: String,
    ignorance: f64,
    independence: f64,
    cash_log_bayes_factor: f64,
    evidence_signals: Vec<String>,
    evidence_lineage: Option<oi_types::EvidenceLineageMetadata>,
}

#[derive(Debug, Serialize)]
struct CaseReport {
    pass: usize,
    id: String,
    query: String,
    intent: String,
    search_ms: f64,
    answer_hydration_ms: f64,
    background_crawl_ms: f64,
    /// Compatibility total. Prefer the split fields above for diagnosis.
    crawl_ms: f64,
    crawled_pages: usize,
    answer_status: String,
    answer_summary: String,
    answer_warning: Option<String>,
    answer_claims: Vec<String>,
    answer_evidence_lineage: Vec<Option<oi_sdk::ClaimEvidenceLineage>>,
    answer_citations_per_claim: Vec<usize>,
    answer_citation_urls: Vec<Vec<String>>,
    poll_registry: Vec<oi_sdk::PesqEleRecord>,
    market_quotes: Vec<oi_sdk::MarketQuoteRecord>,
    search_passes: u32,
    hydrated_pages: usize,
    stage_trace: Option<oi_sdk::AnswerTrace>,
    sources: Vec<String>,
    quality_pass: bool,
    quality_failures: Vec<String>,
    results: Vec<ResultSummary>,
}

#[derive(Debug, Serialize)]
struct LiveReport {
    captured_at_unix_ms: u128,
    experiment: String,
    answer_mode: bool,
    real_embedder: String,
    data_dir: String,
    discovery_engine_override: Option<String>,
    initial_documents: usize,
    final_documents: usize,
    passes: usize,
    warmup_passes: usize,
    concurrency: usize,
    query_variants: bool,
    query_variant: Option<usize>,
    cases: usize,
    wall_ms: f64,
    throughput_qps: f64,
    quality_pass_rate: f64,
    search_p50_ms: f64,
    search_p95_ms: f64,
    search_p99_ms: f64,
    crawl_p50_ms: f64,
    crawl_p95_ms: f64,
    crawl_p99_ms: f64,
    answer_hydration_p50_ms: f64,
    answer_hydration_p95_ms: f64,
    answer_hydration_p99_ms: f64,
    background_crawl_p50_ms: f64,
    background_crawl_p95_ms: f64,
    background_crawl_p99_ms: f64,
    reports: Vec<CaseReport>,
}

pub fn run(
    data_dir: &str,
    engine_override: Option<&str>,
    passes: usize,
    concurrency: usize,
    warmup_passes: usize,
    use_query_variants: bool,
    query_variant: Option<usize>,
    top_k: usize,
    case_filter: Option<&str>,
    crawl: bool,
    answer_mode: bool,
    output: &str,
    summary_only: bool,
    report_path: Option<&str>,
) -> Result<(), String> {
    if passes == 0 || concurrency == 0 || top_k == 0 {
        return Err("passes, concurrency, and top_k must be positive".to_string());
    }
    if let Some(case_filter) = case_filter {
        if !CASES.iter().any(|case| case.id == case_filter) {
            return Err(format!("unknown live-search case: {case_filter}"));
        }
    }
    if query_variant.is_some() && case_filter.is_none() {
        return Err("--query-variant requires --case".to_string());
    }
    std::fs::create_dir_all(data_dir)
        .map_err(|error| format!("cannot create data dir {data_dir}: {error}"))?;
    let index_path = std::path::Path::new(data_dir).join("index");
    let search_engines = parse_engine_override(engine_override)?;
    let config = SdkConfig {
        local_index_path: Some(index_path.to_string_lossy().to_string()),
        // Match the production CLI: authenticated providers take priority
        // when explicitly configured in the environment. Their absence keeps
        // this benchmark on the credential-free path and is reported as such.
        brave_api_key: nonempty_env("BRAVE_API_KEY"),
        serper_api_key: nonempty_env("SERPER_API_KEY"),
        search_engines,
        crawl_on_query: true,
        crawl_on_query_max_urls: 3,
        stakes_path: Some(
            std::path::Path::new(data_dir)
                .join("stakes.json")
                .to_string_lossy()
                .to_string(),
        ),
        creator_ledger_path: Some(
            std::path::Path::new(data_dir)
                .join("creator_ledger.json")
                .to_string_lossy()
                .to_string(),
        ),
        ..Default::default()
    };
    let client = OpenSonarXClient::new(config);
    let embedder_name = client.embedder().name().to_string();
    if embedder_name.to_ascii_lowercase().contains("mock") {
        return Err(format!(
            "refusing live E2E with mock embedder '{embedder_name}'; run with --features onnx"
        ));
    }

    let initial_documents = client.indexed_document_count();
    let filters = QueryFilters {
        locale: Some("pt-BR".to_string()),
        gl: Some("br".to_string()),
        ..Default::default()
    };
    let selected_cases: Vec<LiveCase> = CASES
        .iter()
        .copied()
        .filter(|case| case_filter.is_none_or(|filter| case.id == filter))
        .collect();
    let mut selected_requests = Vec::new();
    for case in &selected_cases {
        if let Some(variant_index) = query_variant {
            let variants = query_variants(*case);
            let query = variants.get(variant_index).copied().ok_or_else(|| {
                format!(
                    "query variant {variant_index} is out of range for {} ({} variants)",
                    case.id,
                    variants.len()
                )
            })?;
            selected_requests.push((*case, query, variant_index));
        } else if use_query_variants {
            for (variant_index, query) in query_variants(*case).iter().copied().enumerate() {
                selected_requests.push((*case, query, variant_index));
            }
        } else {
            selected_requests.push((*case, case.query, 0));
        }
    }

    // Warm-up is never allowed to hide a quality failure. It exercises the
    // same answered-search and crawl path, but its latency is excluded so the
    // caller can measure steady-state separately from cold-start behavior.
    for warmup_pass in 1..=warmup_passes {
        for case in &selected_cases {
            let report = execute_case(
                &client,
                &filters,
                *case,
                case.query,
                0,
                warmup_pass,
                top_k,
                answer_mode,
            )?;
            if !report.quality_pass {
                return Err(format!(
                    "warm-up quality failure in {}: {:?}",
                    report.id, report.quality_failures
                ));
            }
            if crawl {
                client.process_crawl_queue();
            }
        }
    }

    let mut reports = Vec::with_capacity(selected_requests.len() * passes);
    let benchmark_started = Instant::now();

    for pass in 1..=passes {
        for batch in selected_requests.chunks(concurrency) {
            let mut batch_reports = if batch.len() == 1 {
                vec![execute_case(
                    &client,
                    &filters,
                    batch[0].0,
                    batch[0].1,
                    batch[0].2,
                    pass,
                    top_k,
                    answer_mode,
                )?]
            } else {
                std::thread::scope(|scope| {
                    let barrier = std::sync::Arc::new(std::sync::Barrier::new(batch.len()));
                    let client = &client;
                    let filters = &filters;
                    let handles: Vec<_> = batch
                        .iter()
                        .copied()
                        .map(|(case, query, variant_index)| {
                            let barrier = std::sync::Arc::clone(&barrier);
                            scope.spawn(move || {
                                barrier.wait();
                                execute_case(
                                    client,
                                    filters,
                                    case,
                                    query,
                                    variant_index,
                                    pass,
                                    top_k,
                                    answer_mode,
                                )
                            })
                        })
                        .collect();
                    handles
                        .into_iter()
                        .map(|handle| {
                            handle
                                .join()
                                .map_err(|_| "concurrent search worker panicked".to_string())?
                        })
                        .collect::<Result<Vec<_>, String>>()
                })?
            };

            let crawl_started = Instant::now();
            let crawled_pages = if crawl {
                client.process_crawl_queue()
            } else {
                0
            };
            let crawl_ms = crawl_started.elapsed().as_secs_f64() * 1_000.0;
            if let Some(first) = batch_reports.first_mut() {
                first.background_crawl_ms += crawl_ms;
                first.crawl_ms += crawl_ms;
                first.crawled_pages += crawled_pages;
            }
            reports.append(&mut batch_reports);
        }
    }
    let wall_ms = benchmark_started.elapsed().as_secs_f64() * 1_000.0;

    let mut search_latencies: Vec<f64> = reports.iter().map(|report| report.search_ms).collect();
    let mut crawl_latencies: Vec<f64> = reports.iter().map(|report| report.crawl_ms).collect();
    let mut answer_hydration_latencies: Vec<f64> = reports
        .iter()
        .map(|report| report.answer_hydration_ms)
        .collect();
    let mut background_crawl_latencies: Vec<f64> = reports
        .iter()
        .map(|report| report.background_crawl_ms)
        .collect();
    let passed = reports.iter().filter(|report| report.quality_pass).count();
    let request_count = reports.len();
    let report = LiveReport {
        captured_at_unix_ms: unix_time_ms(),
        experiment: if answer_mode {
            "live-answered-production-path-v1".to_string()
        } else {
            "live-production-path-v1".to_string()
        },
        answer_mode,
        real_embedder: embedder_name,
        data_dir: data_dir.to_string(),
        discovery_engine_override: engine_override.map(str::to_string),
        initial_documents,
        final_documents: client.indexed_document_count(),
        passes,
        warmup_passes,
        concurrency,
        query_variants: use_query_variants,
        query_variant,
        cases: request_count,
        wall_ms,
        throughput_qps: request_count as f64 / (wall_ms / 1_000.0).max(f64::EPSILON),
        quality_pass_rate: passed as f64 / request_count.max(1) as f64,
        search_p50_ms: percentile(&mut search_latencies, 0.50),
        search_p95_ms: percentile(&mut search_latencies, 0.95),
        search_p99_ms: percentile(&mut search_latencies, 0.99),
        crawl_p50_ms: percentile(&mut crawl_latencies, 0.50),
        crawl_p95_ms: percentile(&mut crawl_latencies, 0.95),
        crawl_p99_ms: percentile(&mut crawl_latencies, 0.99),
        answer_hydration_p50_ms: percentile(&mut answer_hydration_latencies, 0.50),
        answer_hydration_p95_ms: percentile(&mut answer_hydration_latencies, 0.95),
        answer_hydration_p99_ms: percentile(&mut answer_hydration_latencies, 0.99),
        background_crawl_p50_ms: percentile(&mut background_crawl_latencies, 0.50),
        background_crawl_p95_ms: percentile(&mut background_crawl_latencies, 0.95),
        background_crawl_p99_ms: percentile(&mut background_crawl_latencies, 0.99),
        reports,
    };

    let serialized = serde_json::to_string_pretty(&report)
        .map_err(|error| format!("cannot serialize live report: {error}"))?;
    if let Some(path) = report_path {
        if let Some(parent) = std::path::Path::new(path).parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| format!("cannot create report directory: {error}"))?;
            }
        }
        std::fs::write(path, &serialized)
            .map_err(|error| format!("cannot write live report {path}: {error}"))?;
    }

    match output {
        "json" => println!("{serialized}"),
        "table" => print_table(&report, summary_only),
        _ => return Err(format!("unsupported output format: {output}")),
    }
    Ok(())
}

fn unix_time_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

pub(crate) fn nonempty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

pub(crate) fn parse_engine_override(
    engine_override: Option<&str>,
) -> Result<Option<Vec<oi_sdk::config::SearchEngineConfig>>, String> {
    let Some(raw) = engine_override else {
        return Ok(None);
    };
    let mut configs = Vec::new();
    for (index, name) in raw
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .enumerate()
    {
        let canonical = name.to_ascii_lowercase();
        let api_key = match canonical.as_str() {
            "brave" => Some(
                nonempty_env("BRAVE_API_KEY")
                    .ok_or_else(|| "--engines brave requires BRAVE_API_KEY".to_string())?,
            ),
            "serper" => Some(
                nonempty_env("SERPER_API_KEY")
                    .ok_or_else(|| "--engines serper requires SERPER_API_KEY".to_string())?,
            ),
            "bing" | "brave-html" | "brave_html" | "duckduckgo" | "ddg" | "google" | "qwant"
            | "stract" | "yep" => None,
            _ => return Err(format!("unknown --engines adapter: {name}")),
        };
        configs.push(oi_sdk::config::SearchEngineConfig {
            name: canonical,
            api_key,
            enabled: true,
            priority: (index + 1) as u32,
        });
    }
    if configs.is_empty() {
        return Err("--engines requires at least one adapter".to_string());
    }
    Ok(Some(configs))
}

fn execute_case(
    client: &OpenSonarXClient,
    filters: &QueryFilters,
    case: LiveCase,
    query: &str,
    variant_index: usize,
    pass: usize,
    top_k: usize,
    answer_mode: bool,
) -> Result<CaseReport, String> {
    let started = Instant::now();
    let (
        results,
        sources,
        answer_status,
        answer_summary,
        answer_warning,
        answer_claims,
        answer_evidence_lineage,
        answer_citations_per_claim,
        answer_citation_urls,
        search_passes,
        hydrated_pages,
        poll_registry,
        market_quotes,
        stage_trace,
    ) = if answer_mode {
        let response = client
            .search_answered(query, top_k, filters)
            .map_err(|error| format!("{} answer search failed: {error}", case.id))?;
        let claim_texts = response
            .answer
            .claims
            .iter()
            .map(|claim| claim.text.clone())
            .collect();
        let citation_counts = response
            .answer
            .claims
            .iter()
            .map(|claim| claim.citations.len())
            .collect();
        let evidence_lineages = response
            .answer
            .claims
            .iter()
            .map(|claim| claim.evidence_lineage.clone())
            .collect();
        let citation_urls = response
            .answer
            .claims
            .iter()
            .map(|claim| {
                claim
                    .citations
                    .iter()
                    .map(|citation| citation.url.clone())
                    .collect()
            })
            .collect();
        (
            response.results,
            response.sources_used,
            response.answer.status,
            response.answer.summary,
            response.answer.warning,
            claim_texts,
            evidence_lineages,
            citation_counts,
            citation_urls,
            response.search_passes,
            response.hydrated_pages,
            response.poll_registry,
            response.market_quotes,
            Some(response.trace),
        )
    } else {
        let (results, sources) = client
            .search_blended(query, top_k, filters)
            .map_err(|error| format!("{} search failed: {error}", case.id))?;
        (
            results,
            sources,
            "not_run".to_string(),
            String::new(),
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            1,
            0,
            Vec::new(),
            Vec::new(),
            None,
        )
    };
    let search_ms = started.elapsed().as_secs_f64() * 1_000.0;
    let crawl_ms = stage_trace
        .as_ref()
        .map(|trace| trace.initial_hydration_ms + trace.corroboration_hydration_ms)
        .unwrap_or(0.0);
    let quality_failures = quality_failures(
        &case,
        search_ms,
        &results,
        answer_mode.then_some(answer_status.as_str()),
        &answer_summary,
        answer_warning.as_deref(),
        &answer_claims,
        &answer_evidence_lineage,
        &answer_citations_per_claim,
        &answer_citation_urls,
        &poll_registry,
        &market_quotes,
    );
    let summaries = results
        .iter()
        .map(|result| {
            let decision = result.decision.as_ref();
            ResultSummary {
                decision_rank: result.rank,
                retrieval_rank: decision
                    .map(|value| value.retrieval_rank)
                    .unwrap_or(result.rank),
                url: result.url.clone(),
                title: result.title.clone(),
                source: result.source.clone(),
                published_at: result.published_at,
                crawled_at: result.crawled_at,
                relevance_score: result.final_score,
                decision_score: decision
                    .map(|value| value.decision_score)
                    .unwrap_or(result.final_score),
                uncertainty_mode: decision
                    .map(|value| value.uncertainty_mode.clone())
                    .unwrap_or_else(|| "not_run".to_string()),
                ignorance: decision.map(|value| value.ignorance).unwrap_or(1.0),
                independence: decision.map(|value| value.independence).unwrap_or(1.0),
                cash_log_bayes_factor: decision
                    .map(|value| value.cash_log_bayes_factor)
                    .unwrap_or(0.0),
                evidence_signals: decision
                    .map(|value| value.evidence_signals.clone())
                    .unwrap_or_default(),
                evidence_lineage: decision.and_then(|value| value.evidence_lineage.clone()),
            }
        })
        .collect();

    Ok(CaseReport {
        pass,
        id: if query == case.query {
            case.id.to_string()
        } else {
            format!("{}#{}", case.id, variant_index + 1)
        },
        query: query.to_string(),
        intent: classify_query(query).to_string(),
        search_ms,
        answer_hydration_ms: crawl_ms,
        background_crawl_ms: 0.0,
        crawl_ms,
        crawled_pages: hydrated_pages,
        answer_status,
        answer_summary,
        answer_warning,
        answer_claims,
        answer_evidence_lineage,
        answer_citations_per_claim,
        answer_citation_urls,
        poll_registry,
        market_quotes,
        search_passes,
        hydrated_pages,
        stage_trace,
        sources,
        quality_pass: quality_failures.is_empty(),
        quality_failures,
        results: summaries,
    })
}

fn quality_failures(
    case: &LiveCase,
    search_ms: f64,
    results: &[oi_types::SearchResultItem],
    answer_status: Option<&str>,
    answer_summary: &str,
    answer_warning: Option<&str>,
    answer_claims: &[String],
    answer_evidence_lineage: &[Option<oi_sdk::ClaimEvidenceLineage>],
    answer_citations_per_claim: &[usize],
    answer_citation_urls: &[Vec<String>],
    poll_registry: &[oi_sdk::PesqEleRecord],
    market_quotes: &[oi_sdk::MarketQuoteRecord],
) -> Vec<String> {
    let mut failures = Vec::new();
    if let Some(answer_status) = answer_status {
        if answer_status != case.expected_answer_status {
            failures.push(format!(
                "answer_status:{answer_status}:expected:{}",
                case.expected_answer_status
            ));
        }
    }
    if case.expected_answer_status == "extractive" {
        if answer_claims.is_empty() {
            failures.push("extractive_answer_without_claim".to_string());
        }
        if answer_citations_per_claim.iter().any(|count| *count < 2) {
            failures.push("claim_without_two_citations".to_string());
        }
        if answer_claims.iter().any(|claim| claim.contains('#')) {
            failures.push("answer_claim_contains_markdown_heading".to_string());
        }
    }
    if case.id == "current_brazil_election_poll" {
        if poll_registry.is_empty() {
            failures.push("poll_answer_missing_official_pesqele_record".to_string());
        }
        if !answer_citation_urls
            .iter()
            .flatten()
            .any(|url| url.contains("pesqele-divulgacao.tse.jus.br/"))
        {
            failures.push("poll_answer_missing_pesqele_citation".to_string());
        }
        if !answer_warning.is_some_and(|warning| {
            warning.contains("uma única medição") && warning.contains("não valida os percentuais")
        }) {
            failures.push("poll_answer_missing_measurement_dependency_warning".to_string());
        }
        if answer_evidence_lineage.len() != answer_claims.len()
            || answer_evidence_lineage.iter().any(|lineage| {
                !lineage.as_ref().is_some_and(|lineage| {
                    lineage.identity_status == "official_registry_joined"
                        && lineage.underlying_unit_count == Some(1)
                        && lineage.reporting_source_families >= 2
                        && lineage.independence_status
                            == "registered_measurement_with_editorial_reports"
                        && lineage.known_independent_observations == Some(1)
                        && lineage.registry_metadata_records == 1
                        && lineage
                            .registry_does_not_verify
                            .iter()
                            .any(|scope| scope == "reported_percentages")
                })
            })
        {
            failures.push("poll_answer_missing_structured_measurement_lineage".to_string());
        }
        if answer_claims.len() > 2 {
            failures.push(format!("too_many_poll_claims:{}", answer_claims.len()));
        }
        let normalized_claims = answer_claims
            .iter()
            .map(|claim| claim.to_lowercase())
            .collect::<Vec<_>>();
        let has_first_round = normalized_claims.iter().any(|claim| {
            claim.contains("primeiro turno")
                || claim.contains("1º turno")
                || claim.contains("1o turno")
        });
        let has_second_round = normalized_claims.iter().any(|claim| {
            claim.contains("segundo turno")
                || claim.contains("2º turno")
                || claim.contains("2o turno")
        });
        if !has_first_round || !has_second_round {
            failures.push("poll_answer_missing_first_or_second_round_scenario".to_string());
        }
        let mut scenario_labels = std::collections::HashSet::new();
        for claim in answer_claims {
            let percent_count = claim.matches('%').count();
            if percent_count != 2 {
                failures.push(format!("poll_claim_percent_count:{percent_count}"));
            }
            if claim.contains('#') {
                failures.push("poll_claim_contains_markup".to_string());
            }
            let folded_claim = claim.to_lowercase();
            if folded_claim.contains("segundo turno")
                || folded_claim.contains("2º turno")
                || folded_claim.contains("2o turno")
            {
                let values = percent_values(claim);
                if values.len() != 2 || values.iter().sum::<f64>() < 60.0 {
                    failures.push("poll_runoff_values_implausible".to_string());
                }
            }
            if answer_claims.len() > 1 {
                let label = claim.split(':').next().unwrap_or_default().to_lowercase();
                if !label.contains("cenario")
                    && !label.contains("cenário")
                    && !label.contains("turno")
                {
                    failures.push("poll_claim_missing_scenario_context".to_string());
                }
                scenario_labels.insert(label);
            }
        }
        if answer_claims.len() > 1 && scenario_labels.len() != answer_claims.len() {
            failures.push("poll_claim_scenario_context_not_distinct".to_string());
        }
        let answered_unit_ids = answer_evidence_lineage
            .iter()
            .filter_map(Option::as_ref)
            .filter(|lineage| lineage.identity_status == "official_registry_joined")
            .flat_map(|lineage| lineage.underlying_unit_ids.iter())
            .collect::<std::collections::HashSet<_>>();
        for unit_id in answered_unit_ids {
            let unit_results = results
                .iter()
                .filter_map(|result| result.decision.as_ref())
                .filter(|decision| {
                    decision
                        .evidence_lineage
                        .as_ref()
                        .is_some_and(|lineage| lineage.unit_ref == *unit_id)
                })
                .collect::<Vec<_>>();
            let representative_count = unit_results
                .iter()
                .filter(|decision| {
                    decision
                        .evidence_lineage
                        .as_ref()
                        .is_some_and(|lineage| lineage.representative)
                })
                .count();
            if representative_count != 1 {
                failures.push(format!(
                    "poll_unit_representative_count:{unit_id}:{representative_count}"
                ));
            }
            if unit_results.iter().any(|decision| {
                decision
                    .evidence_lineage
                    .as_ref()
                    .is_some_and(|lineage| !lineage.representative)
                    && decision.independence.abs() > f64::EPSILON
            }) {
                failures.push(format!("poll_derived_report_counted_independent:{unit_id}"));
            }
        }
    }
    if case.id == "factual_anthropic_founders" {
        let folded = answer_summary.to_lowercase();
        if !folded.contains("lista parcial") || !folded.contains("lista") {
            failures.push("founder_completeness_not_exposed".to_string());
        }
        if !answer_claims.is_empty() {
            failures.push("partial_founder_list_emitted_as_claim".to_string());
        }
        let relevant_surfaces = results
            .iter()
            .filter(|result| {
                oi_sdk::answer::factual_discovery_surface_matches_relation(case.query, result)
            })
            .count();
        if relevant_surfaces < 2 {
            failures.push(format!(
                "founder_auditable_result_depth:{relevant_surfaces}"
            ));
        }
        if results.first().is_some_and(|result| {
            !oi_sdk::answer::factual_public_surface_matches_relation(case.query, result)
        }) {
            failures.push("founder_top1_not_relation_specific".to_string());
        }
    }
    if case.id == "factual_pix_creator" {
        if !answer_citation_urls
            .iter()
            .flatten()
            .any(|url| url.contains("bcb.gov.br/"))
        {
            failures.push("pix_answer_missing_primary_bcb_citation".to_string());
        }
        if answer_evidence_lineage.len() != answer_claims.len()
            || answer_evidence_lineage.iter().any(|lineage| {
                !lineage.as_ref().is_some_and(|lineage| {
                    lineage.underlying_unit_kind == "factual_relation_claim"
                        && lineage.identity_status == "normalized_claim_signature_only"
                        && lineage.underlying_unit_ids.len() == 1
                        && lineage.underlying_unit_count.is_none()
                        && lineage.reporting_source_families >= 2
                        && lineage.claim_text_citations >= 2
                        && lineage.independence_status == "unknown_editorial_derivation"
                        && lineage.known_independent_observations.is_none()
                        && !lineage.unresolved_dependencies.is_empty()
                        && lineage.registry_metadata_records == 0
                })
            })
        {
            failures.push("factual_answer_missing_structured_claim_lineage".to_string());
        }
    }
    if case.id == "diligence_gama" {
        for result in results.iter().take(5) {
            let surface =
                format!("{} {} {}", result.title, result.snippet, result.url).to_lowercase();
            if !surface.contains("google") && !surface.contains("monashees") {
                failures.push(format!("diligence_entity_context_mismatch:{}", result.url));
            }
        }
    }
    if case.id == "tool_python_compiler" {
        let Some(tool_top) = results.first() else {
            return vec!["no_results".to_string()];
        };
        let surface = format!("{} {} {}", tool_top.title, tool_top.snippet, tool_top.url)
            .to_ascii_lowercase();
        if !surface.contains("python")
            || !["compiler", "compilador", "editor", "ide", "interpreter"]
                .iter()
                .any(|term| surface.contains(term))
        {
            failures.push(format!("tool_destination_mismatch:{}", tool_top.url));
        }
        if results.len() < 3 {
            failures.push(format!("tool_result_depth:{}", results.len()));
        }
        let domains = results
            .iter()
            .take(3)
            .filter_map(|result| {
                result
                    .url
                    .split("//")
                    .nth(1)
                    .and_then(|rest| rest.split('/').next())
            })
            .collect::<std::collections::HashSet<_>>();
        if domains.len() < 3 {
            failures.push(format!("tool_top3_domain_diversity:{}", domains.len()));
        }
    }
    if case.id == "current_bitcoin_price" {
        if answer_claims.len() != 1 {
            failures.push(format!("live_price_claim_count:{}", answer_claims.len()));
        }
        if market_quotes.len() < 2
            || market_quotes
                .iter()
                .map(|quote| quote.provider.as_str())
                .collect::<std::collections::HashSet<_>>()
                .len()
                < 2
        {
            failures.push(format!(
                "live_price_structured_provider_depth:{}",
                market_quotes.len()
            ));
        }
        if !answer_summary.contains("faixa observada")
            || !answer_summary.contains("fontes estruturadas")
        {
            failures.push("live_price_missing_bounded_quote_surface".to_string());
        }
        if !answer_warning.is_some_and(|warning| {
            warning.contains("não recomendação financeira")
                && warning.contains("não foi tratada como observações independentes")
        }) {
            failures.push("live_price_missing_dependency_warning".to_string());
        }
        if answer_evidence_lineage.len() != 1
            || !answer_evidence_lineage[0].as_ref().is_some_and(|lineage| {
                lineage.underlying_unit_kind == "market_quote_snapshot"
                    && lineage.identity_status == "asset_currency_timestamp_joined"
                    && lineage.underlying_unit_count == Some(2)
                    && lineage.reporting_source_families == 2
                    && lineage.known_independent_observations.is_none()
                    && lineage.independence_status == "cross_provider_dependency_unresolved"
            })
        {
            failures.push("live_price_missing_structured_lineage".to_string());
        }
    }
    if case.id == "current_bitcoin_price" && results.is_empty() {
        // A typed quote has citations of its own and does not require an HTML
        // result to masquerade as evidence. Other cases still require a
        // public result slate below.
        return failures;
    }
    let Some(top) = results.first() else {
        return vec!["no_results".to_string()];
    };
    if matches!(
        case.id,
        "exploratory_quantum_definition"
            | "exploratory_tax_guidance"
            | "exploratory_dengue_guidance"
            | "exploratory_local_restaurants"
            | "exploratory_notebook_recommendation"
    ) && search_ms > 5_000.0
    {
        failures.push(format!("exploratory_latency_slo_ms:{search_ms:.1}"));
    }
    if case.id == "exploratory_quantum_definition" {
        let surface = fold_search_surface(&format!("{} {}", top.title, top.url));
        if !surface.contains("quant") || !surface.contains("comput") {
            failures.push(format!("quantum_top1_topic_mismatch:{}", top.url));
        }
        if results.len() < 3 {
            failures.push(format!("quantum_result_depth:{}", results.len()));
        }
    }
    if case.id == "exploratory_tax_guidance"
        && !top.decision.as_ref().is_some_and(|decision| {
            decision
                .evidence_signals
                .iter()
                .any(|signal| signal == "query_relative_primary_authority")
        })
    {
        failures.push("tax_top1_missing_query_relative_authority".to_string());
    }
    if case.id == "exploratory_dengue_guidance" {
        if !answer_warning.is_some_and(|warning| {
            warning.contains("sem validação clínica")
                && warning.contains("não substituem avaliação médica")
        }) {
            failures.push("health_answer_missing_safety_warning".to_string());
        }
        if !top.decision.as_ref().is_some_and(|decision| {
            decision
                .evidence_signals
                .iter()
                .any(|signal| signal == "query_relative_primary_authority")
        }) {
            failures.push("health_top1_missing_query_relative_authority".to_string());
        }
    }
    if case.id == "exploratory_local_restaurants" {
        if !answer_warning
            .is_some_and(|warning| warning.contains("localização") && warning.contains("distância"))
        {
            failures.push("local_search_missing_geolocation_warning".to_string());
        }
        let surface = format!("{} {}", top.title, top.url).to_lowercase();
        if !["japon", "restaurant", "sushi"]
            .iter()
            .any(|term| surface.contains(term))
        {
            failures.push(format!("local_search_top1_topic_mismatch:{}", top.url));
        }
    }
    if case.id == "exploratory_notebook_recommendation" {
        if !answer_warning.is_some_and(|warning| {
            warning.contains("comissões de afiliados")
                && warning.contains("independência editorial")
        }) {
            failures.push("recommendation_missing_commercial_dependency_warning".to_string());
        }
        let surface = format!("{} {}", top.title, top.url).to_lowercase();
        if !surface.contains("notebook") && !surface.contains("laptop") {
            failures.push(format!("notebook_top1_topic_mismatch:{}", top.url));
        }
        if top.decision.as_ref().is_some_and(|decision| {
            decision
                .evidence_signals
                .iter()
                .any(|signal| signal == "merchant_first_party_surface")
        }) {
            failures.push(format!("notebook_merchant_ranked_top1:{}", top.url));
        }
        if results.len() < 3 {
            failures.push(format!("notebook_result_depth:{}", results.len()));
        }
        let domains = results
            .iter()
            .take(3)
            .filter_map(|result| {
                result
                    .url
                    .split("//")
                    .nth(1)
                    .and_then(|rest| rest.split('/').next())
            })
            .collect::<std::collections::HashSet<_>>();
        if domains.len() < 3 {
            failures.push(format!("notebook_top3_domain_diversity:{}", domains.len()));
        }
        if results.iter().take(5).any(|result| {
            !result.decision.as_ref().is_some_and(|decision| {
                decision
                    .evidence_signals
                    .iter()
                    .any(|signal| signal == "commercial_independence_unverified")
            })
        }) {
            failures.push("recommendation_missing_commercial_dependency_signal".to_string());
        }
    }
    if case.id == "current_brazil_president" {
        let official_citation = answer_citation_urls.iter().flatten().any(|url| {
            url.contains("gov.br/planalto/pt-br/acesso-a-informacao/institucional/principais-cargos-e-respectivos-ocupantes")
                || url.contains("gov.br/planalto/pt-br/conheca-a-presidencia/biografia-do-presidente")
        });
        let independent_citation = answer_citation_urls.iter().flatten().any(|url| {
            !url.contains("gov.br/planalto")
                && !url.contains("wikipedia.org")
                && !url.contains("youtube.com")
                && !url.contains("linkedin.com")
                && !url.contains("lula.com.br")
        });
        if !official_citation {
            failures.push("office_answer_missing_exact_primary_record".to_string());
        }
        if !independent_citation {
            failures.push("office_answer_missing_independent_editorial_citation".to_string());
        }
        if answer_claims.len() != 1
            || answer_evidence_lineage.len() != 1
            || !answer_evidence_lineage[0].as_ref().is_some_and(|lineage| {
                lineage.underlying_unit_kind == "current_office_holder_claim"
                    && lineage.identity_status
                        == "official_primary_record_and_editorial_corroboration"
                    && lineage.reporting_source_families >= 2
            })
        {
            failures.push("office_answer_missing_structured_lineage".to_string());
        }
        if answer_summary.contains(" PT.") {
            failures.push("office_answer_party_suffix_in_name".to_string());
        }
        if search_ms > 8_000.0 {
            failures.push(format!("office_latency_slo_ms:{search_ms:.1}"));
        }
    }
    if case.id == "current_generic_ai_news" {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        match top.published_at {
            Some(published_at) if (0..=7 * 86_400).contains(&(now - published_at)) => {}
            Some(published_at) => failures.push(format!(
                "current_news_top1_stale_days:{}",
                (now - published_at).max(0) / 86_400
            )),
            None => failures.push("current_news_top1_date_unknown".to_string()),
        }
        if results.len() < 3 {
            failures.push(format!(
                "current_news_insufficient_result_depth:{}",
                results.len()
            ));
        }
        for result in results.iter().take(5) {
            match result.published_at {
                Some(published_at) if (0..=7 * 86_400).contains(&(now - published_at)) => {}
                Some(published_at) => failures.push(format!(
                    "current_news_top5_stale_days:{}:{}",
                    (now - published_at).max(0) / 86_400,
                    result.url
                )),
                None => failures.push(format!("current_news_top5_date_unknown:{}", result.url)),
            }
        }
        let top_surface = format!("{} {}", top.title, top.url).to_ascii_lowercase();
        if [
            "advertorial",
            "businesswire.com",
            "globenewswire",
            "newsfilecorp.com",
            "newswire",
            "press-release",
            "prnewswire.com",
            "sponsored",
        ]
        .iter()
        .any(|marker| top_surface.contains(marker))
        {
            failures.push("current_news_top1_press_release".to_string());
        }
    }
    if !case.top1_any.is_empty()
        && !case
            .top1_any
            .iter()
            .any(|needle| url_matches_expected_prefix(&top.url, needle))
    {
        failures.push(format!("top1_unexpected:{}", top.url));
    }
    if case.top1_none.iter().any(|needle| top.url.contains(needle)) {
        failures.push(format!("top1_forbidden:{}", top.url));
    }
    for result in results.iter().take(5) {
        if case
            .forbidden_top5
            .iter()
            .any(|needle| result.url.contains(needle))
        {
            failures.push(format!("forbidden_top5:{}", result.url));
        }
        let Some(decision) = result.decision.as_ref() else {
            failures.push(format!("decision_missing:{}", result.url));
            continue;
        };
        if decision.uncertainty_mode != case.expected_uncertainty {
            failures.push(format!(
                "uncertainty_mode:{}:{}",
                decision.uncertainty_mode, result.url
            ));
        }
        if decision.cash_log_bayes_factor.abs() > f64::EPSILON {
            failures.push(format!("unexpected_cash_effect:{}", result.url));
        }
        if case.expected_uncertainty == "evidential_only" && decision.p_reliable.is_some() {
            failures.push(format!("fabricated_probability:{}", result.url));
        }
    }
    failures.sort();
    failures.dedup();
    failures
}

fn percent_values(text: &str) -> Vec<f64> {
    let mut values = Vec::new();
    let mut current = String::new();
    for character in text.chars() {
        if character.is_ascii_digit() || matches!(character, ',' | '.') {
            current.push(character);
        } else if character == '%' {
            if let Ok(value) = current.replace(',', ".").parse::<f64>() {
                values.push(value);
            }
            current.clear();
        } else {
            current.clear();
        }
    }
    values
}

fn url_matches_expected_prefix(url: &str, expected: &str) -> bool {
    fn normalize(value: &str) -> &str {
        let without_scheme = value
            .strip_prefix("https://")
            .or_else(|| value.strip_prefix("http://"))
            .unwrap_or(value);
        without_scheme
            .strip_prefix("www.")
            .unwrap_or(without_scheme)
    }
    normalize(url).starts_with(normalize(expected))
}

fn fold_search_surface(value: &str) -> String {
    value
        .chars()
        .map(|character| match character.to_ascii_lowercase() {
            'Á' | 'À' | 'Â' | 'Ã' | 'Ä' | 'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
            'É' | 'È' | 'Ê' | 'Ë' | 'é' | 'è' | 'ê' | 'ë' => 'e',
            'Í' | 'Ì' | 'Î' | 'Ï' | 'í' | 'ì' | 'î' | 'ï' => 'i',
            'Ó' | 'Ò' | 'Ô' | 'Õ' | 'Ö' | 'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
            'Ú' | 'Ù' | 'Û' | 'Ü' | 'ú' | 'ù' | 'û' | 'ü' => 'u',
            'Ç' | 'ç' => 'c',
            other => other,
        })
        .collect()
}

fn percentile(values: &mut [f64], quantile: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(f64::total_cmp);
    let index = ((values.len() - 1) as f64 * quantile).ceil() as usize;
    values[index]
}

fn print_table(report: &LiveReport, summary_only: bool) {
    println!(
        "live production path | answer_mode={} | model={} | docs {} -> {} | requests={} concurrency={} warmup_passes={} query_variants={} | quality={:.1}%",
        report.answer_mode,
        report.real_embedder,
        report.initial_documents,
        report.final_documents,
        report.cases,
        report.concurrency,
        report.warmup_passes,
        report.query_variants,
        report.quality_pass_rate * 100.0
    );
    println!(
        "wall={:.1}ms throughput={:.1}qps | search p50={:.1}ms p95={:.1}ms p99={:.1}ms | answer_hydration p50={:.1}ms p95={:.1}ms p99={:.1}ms | background_crawl p50={:.1}ms p95={:.1}ms p99={:.1}ms | combined_crawl p99={:.1}ms",
        report.wall_ms,
        report.throughput_qps,
        report.search_p50_ms,
        report.search_p95_ms,
        report.search_p99_ms,
        report.answer_hydration_p50_ms,
        report.answer_hydration_p95_ms,
        report.answer_hydration_p99_ms,
        report.background_crawl_p50_ms,
        report.background_crawl_p95_ms,
        report.background_crawl_p99_ms,
        report.crawl_p99_ms,
    );
    for case in report
        .reports
        .iter()
        .filter(|case| !summary_only || !case.quality_pass)
    {
        let top = case
            .results
            .first()
            .map(|result| result.url.as_str())
            .unwrap_or("-");
        println!(
            "{} pass={} {:<34} intent={:<15} search={:>7.1}ms hydrate={:>7.1}ms background_crawl={:>7.1}ms passes={} hydrated={} answer={} sources={} top1={}{}",
            if case.quality_pass { "PASS" } else { "FAIL" },
            case.pass,
            case.id,
            case.intent,
            case.search_ms,
            case.answer_hydration_ms,
            case.background_crawl_ms,
            case.search_passes,
            case.hydrated_pages,
            case.answer_status,
            case.sources.join("+"),
            top,
            if case.quality_failures.is_empty() {
                String::new()
            } else {
                format!(" failures={:?}", case.quality_failures)
            }
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_explicit_keyless_failover_order() {
        let configs = parse_engine_override(Some("brave-html, duckduckgo"))
            .unwrap()
            .unwrap();
        assert_eq!(configs.len(), 2);
        assert_eq!(configs[0].name, "brave-html");
        assert_eq!(configs[0].priority, 1);
        assert_eq!(configs[1].name, "duckduckgo");
        assert_eq!(configs[1].priority, 2);
    }

    #[test]
    fn rejects_empty_or_unknown_engine_override() {
        assert!(parse_engine_override(Some(" ")).is_err());
        assert!(parse_engine_override(Some("imaginary")).is_err());
        assert!(parse_engine_override(None).unwrap().is_none());
    }
}
