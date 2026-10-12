use oi_sdk::{GatewayClient, OpenSonarXClient, SdkConfig};
use oi_types::error::ExitCode;
use oi_types::traits::SearchIndex;

use super::{
    CliConfig, DisputeCommands, DomainCommands, IndexCommands, JuryCommands, NetworkCommands,
    SeedCommands, SentinelCommands, WalletCommands, YieldCommands,
};
use crate::config_file::ConfigFile;
use crate::output;

/// Dual-mode client: local (web3) or gateway (web2).
pub enum ClientMode {
    Local(Box<OpenSonarXClient>),
    Gateway(GatewayClient),
}

/// Build a local SDK client from the global CLI config.
/// This is the original `build_client()` — kept as-is, just renamed.
fn build_local_client(config: &CliConfig) -> OpenSonarXClient {
    // Load the canonical local index (shared by seed, sentinel, and the node)
    // so ingested content is searchable; without this, local search silently
    // returns external results only.
    let seed_index_path =
        crate::config_file::ConfigFile::index_dir().map(|p| p.to_string_lossy().to_string());
    let sdk_config = SdkConfig {
        bootstrap_peers: config.bootstrap_peers.clone(),
        wallet_path: config.wallet_path.clone(),
        local_index_path: seed_index_path,
        network: config.network.clone(),
        brave_api_key: std::env::var("BRAVE_API_KEY").ok(),
        serper_api_key: std::env::var("SERPER_API_KEY").ok(),
        search_engines: config.search_engines.clone(),
        // Self-building index: external results surfaced by a query get crawled
        // and ingested after results are shown (see crawl-on-query in oi-sdk).
        crawl_on_query: true,
        crawl_on_query_max_urls: 3,
        stakes_path: crate::config_file::ConfigFile::config_dir()
            .map(|d| d.join("stakes.json").to_string_lossy().to_string()),
        creator_ledger_path: crate::config_file::ConfigFile::config_dir()
            .map(|d| d.join("creator_ledger.json").to_string_lossy().to_string()),
        answer_model: answer_model_from_env(),
        answer_calibration_path: std::env::var("OI_ANSWER_CALIBRATION")
            .ok()
            .filter(|path| !path.trim().is_empty()),
    };
    let client = OpenSonarXClient::new(sdk_config);
    // With OI_EXTRACT_CLAIMS=1 the same model reads each newly indexed page
    // for claims, in the background.
    let extract_claims = std::env::var("OI_EXTRACT_CLAIMS")
        .is_ok_and(|value| matches!(value.trim(), "1" | "true" | "yes"));
    match answer_model_from_env() {
        Some(model) if extract_claims => {
            client.with_passage_claim_extractor(std::sync::Arc::new(oi_sdk::ChatModel::new(model)))
        }
        _ => client,
    }
}

/// The model behind `--answer`, from `OI_ANSWER_MODEL`. `OI_ANSWER_PROVIDER`
/// selects where it is served: `ollama` (default, a local server) or
/// `openrouter` (hosted; reads `OPENROUTER_API_KEY`). Optional:
/// `OI_ANSWER_MODEL_URL` and `OI_ANSWER_MODEL_THINK` (`low`/`medium`/`high`
/// for models with a reasoning setting). Unset = no model: `--answer`
/// returns the slate.
fn answer_model_from_env() -> Option<oi_sdk::AnswerModelConfig> {
    let model = std::env::var("OI_ANSWER_MODEL").ok()?;
    let model = model.trim();
    if model.is_empty() {
        return None;
    }
    let hosted = std::env::var("OI_ANSWER_PROVIDER")
        .is_ok_and(|provider| provider.trim().eq_ignore_ascii_case("openrouter"));
    let mut config = if hosted {
        oi_sdk::AnswerModelConfig::openrouter(model)
    } else {
        oi_sdk::AnswerModelConfig::new(model)
    };
    if let Ok(url) = std::env::var("OI_ANSWER_MODEL_URL")
        && !url.trim().is_empty()
    {
        config.base_url = Some(url.trim().to_string());
    }
    config.think = std::env::var("OI_ANSWER_MODEL_THINK")
        .ok()
        .map(|think| think.trim().to_string())
        .filter(|think| !think.is_empty());
    Some(config)
}

/// Build a client in the appropriate mode.
///
/// Priority:
/// 1. `--wallet` flag or config file wallet -> Local (web3)
/// 2. API key in config -> Gateway (web2)
/// 3. Open mode (no wallet, no API key) -> Local (wallet-less)
/// 4. Neither -> error
fn build_client_mode(config: &CliConfig) -> Result<ClientMode, ExitCode> {
    if config.wallet_path.is_some() {
        return Ok(ClientMode::Local(Box::new(build_local_client(config))));
    }

    if let Some(api_key) = &config.api_key {
        let base_url = config
            .gateway_url
            .clone()
            .unwrap_or_else(|| "https://api.opensonarx.com".to_string());
        let client =
            GatewayClient::new(base_url, Some(api_key.clone()), config.timeout_secs * 1000);
        return Ok(ClientMode::Gateway(client));
    }

    // Open mode: run locally without a wallet
    if config.open_mode {
        return Ok(ClientMode::Local(Box::new(build_local_client(config))));
    }

    output::print_err(
        "No auth configured. Run `oi login` for web2 mode, provide `--wallet` for web3 mode, or use `--open` for open mode.",
        &config.format,
    );
    Err(ExitCode::UserError)
}

/// Guard that rejects gateway mode for wallet-only commands.
fn require_local(config: &CliConfig, command_name: &str) -> Result<OpenSonarXClient, ExitCode> {
    if config.wallet_path.is_none() && config.api_key.is_some() {
        output::print_err(
            &format!(
                "The `{command_name}` command requires a wallet. \
                 Use `--wallet` or create one with `oi wallet create`."
            ),
            &config.format,
        );
        return Err(ExitCode::UserError);
    }
    Ok(build_local_client(config))
}

/// Guard that rejects commands not available in open mode.
fn reject_in_open_mode(config: &CliConfig, command_name: &str) -> Option<ExitCode> {
    if config.open_mode {
        output::print_err(
            &format!("`{command_name}` is not available in open mode"),
            &config.format,
        );
        return Some(ExitCode::UserError);
    }
    None
}

// ── Search ──────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub fn search(
    config: &CliConfig,
    query: &str,
    limit: u32,
    min_stake: Option<u32>,
    freshness: Option<String>,
    types: Option<Vec<String>>,
    shards: Option<Vec<String>>,
    entities: Option<Vec<String>>,
    content_kinds: Option<Vec<String>>,
    locale: Option<String>,
    gl: Option<String>,
    site: Option<String>,
    min_trust: Option<String>,
    node: Option<String>,
    explain: bool,
    answer: bool,
    claims: bool,
) -> ExitCode {
    if query.trim().is_empty() {
        output::print_err("Query must not be empty", &config.format);
        return ExitCode::UserError;
    }
    if let Some(ref t) = min_trust
        && !["unverified", "neutral", "trusted", "highly_trusted"].contains(&t.as_str())
    {
        output::print_err(
            &format!(
                "Invalid --min-trust '{t}'. Valid: unverified, neutral, trusted, highly_trusted"
            ),
            &config.format,
        );
        return ExitCode::UserError;
    }

    let filters = oi_types::QueryFilters {
        min_stake_usd: min_stake,
        freshness,
        types: types.unwrap_or_default(),
        shards: shards.unwrap_or_default(),
        entities: entities.unwrap_or_default(),
        content_kinds: content_kinds.unwrap_or_default(),
        locale,
        gl,
        site,
        min_trust,
        ..Default::default()
    };

    // If --node is provided, query the P2P node's HTTP API directly
    if let Some(node_url) = node {
        if answer {
            return answer_via_node(config, &node_url, query, limit, &filters, explain);
        }
        return search_via_node(config, &node_url, query, limit, &filters);
    }

    let (client, is_anonymous) = match build_client_mode(config) {
        Ok(c) => (c, false),
        Err(_) => {
            let base_url = config
                .gateway_url
                .clone()
                .unwrap_or_else(|| "https://api.opensonarx.com".to_string());
            (
                ClientMode::Gateway(GatewayClient::new(
                    base_url,
                    None,
                    config.timeout_secs * 1000,
                )),
                true,
            )
        }
    };

    if claims && let ClientMode::Local(local) = &client {
        print_stored_claims(local, query);
    }

    if answer {
        return search_with_answer(
            config,
            &client,
            query,
            limit,
            &filters,
            explain,
            is_anonymous,
        );
    }

    let search_started = std::time::Instant::now();
    let result = match &client {
        ClientMode::Local(c) => {
            if let Some(corrected) = c.correct_query(query) {
                eprintln!("(showing results for: {corrected})");
            }
            c.search_blended(query, limit as usize, &filters)
        }
        ClientMode::Gateway(c) => c.search(query, limit as usize, &filters).map(|items| {
            let mut sources = Vec::new();
            for item in &items {
                if !sources.contains(&item.source) {
                    sources.push(item.source.clone());
                }
            }
            (items, sources)
        }),
    };
    let search_ms = search_started.elapsed().as_secs_f64() * 1_000.0;

    match result {
        Ok((items, sources)) => {
            if explain {
                let local_results = items
                    .iter()
                    .filter(|item| item.source == "local" || item.source == "cached")
                    .count();
                let web_results = items.len().saturating_sub(local_results);
                let evidence_modes: Vec<&str> = items
                    .iter()
                    .filter_map(|item| item.decision.as_ref())
                    .map(|decision| decision.uncertainty_mode.as_str())
                    .fold(Vec::new(), |mut modes, mode| {
                        if !modes.contains(&mode) {
                            modes.push(mode);
                        }
                        modes
                    });
                let decision_moves = items
                    .iter()
                    .filter_map(|item| item.decision.as_ref())
                    .filter(|decision| decision.retrieval_rank != decision.decision_rank)
                    .count();
                let average_ignorance = if evidence_modes.as_slice() == ["not_applicable"] {
                    "n/a".to_string()
                } else if items.is_empty() {
                    "0.000".to_string()
                } else {
                    format!(
                        "{:.3}",
                        items
                            .iter()
                            .filter_map(|item| item.decision.as_ref())
                            .map(|decision| decision.ignorance)
                            .sum::<f64>()
                            / items.len() as f64
                    )
                };
                let candidate_depth = match &client {
                    ClientMode::Local(_) => {
                        oi_sdk::retrieval_candidate_depth(limit as usize).to_string()
                    }
                    ClientMode::Gateway(_) => "gateway".to_string(),
                };
                eprintln!(
                    "(path: candidates={} sources={} local={} web={} evidence={} decision_moves={} avg_ignorance={average_ignorance} search_ms={search_ms:.1})",
                    candidate_depth,
                    sources.join("+"),
                    local_results,
                    web_results,
                    evidence_modes.join("+"),
                    decision_moves,
                );
            }
            if items.is_empty() {
                output::print_ok(&format!("No results found for: {query}"), &config.format);
            } else {
                output::print_list(
                    &items,
                    &config.format,
                    &[
                        "RANK",
                        "RETR",
                        "REL",
                        "DEC",
                        "UNCERTAINTY",
                        "BOND",
                        "URL",
                        "SOURCE",
                    ],
                    |r| {
                        let decision = r.decision.as_ref();
                        vec![
                            r.rank.to_string(),
                            decision
                                .map(|value| value.retrieval_rank.to_string())
                                .unwrap_or_else(|| r.rank.to_string()),
                            format!("{:.3}", r.final_score),
                            decision
                                .map(|value| format!("{:.3}", value.decision_score))
                                .unwrap_or_else(|| "-".to_string()),
                            decision
                                .map(|value| value.uncertainty_mode.clone())
                                .unwrap_or_else(|| "not_run".to_string()),
                            bond_label(r),
                            r.url.clone(),
                            r.source.clone(),
                        ]
                    },
                );
                if items
                    .iter()
                    .any(|r| r.source != "local" && r.source != "cached")
                {
                    eprintln!(
                        "(some results are live web discoveries; claim-bearing queries keep elevated ignorance until retained evidence is available)"
                    );
                }
            }
            if is_anonymous {
                eprintln!("(anonymous mode - 10 queries/day, run `oi login` for 100/day)");
            }
            // Crawl-on-query: index the external results this search surfaced so
            // the local index grows with every query. Runs after results are
            // printed so the user never waits on it for their answer.
            if let ClientMode::Local(c) = &client {
                let pending = c.pending_crawl_count();
                if pending > 0 {
                    eprintln!("(learning: indexing {pending} new sources from this search...)");
                    let crawl_started = std::time::Instant::now();
                    let indexed = c.process_crawl_queue();
                    let crawl_ms = crawl_started.elapsed().as_secs_f64() * 1_000.0;
                    if indexed > 0 {
                        eprintln!("(learned: {indexed} pages added to the local index)");
                    }
                    if explain {
                        eprintln!(
                            "(path: crawl_queued={pending} crawl_indexed={indexed} crawl_ms={crawl_ms:.1})"
                        );
                    }
                }
                read_new_pages_for_claims(c);
            }
            ExitCode::Success
        }
        Err(e) => {
            output::print_err(&format!("Search failed: {e}"), &config.format);
            e.exit_code()
        }
    }
}

fn search_with_answer(
    config: &CliConfig,
    client: &ClientMode,
    query: &str,
    limit: u32,
    filters: &oi_types::QueryFilters,
    explain: bool,
    is_anonymous: bool,
) -> ExitCode {
    let started = std::time::Instant::now();
    let response = match client {
        ClientMode::Local(client) => client.search_answered(query, limit as usize, filters),
        ClientMode::Gateway(client) => {
            client
                .search(query, limit as usize, filters)
                .map(|results| {
                    let mut sources_used = Vec::new();
                    for result in &results {
                        if !sources_used.contains(&result.source) {
                            sources_used.push(result.source.clone());
                        }
                    }
                    // The gateway returns a slate; no model reads it here.
                    let (answer, _) =
                        oi_sdk::answer::results_only(&oi_sdk::AnswerPolicy::default());
                    oi_sdk::AnsweredSearch {
                        query: query.to_string(),
                        answer,
                        results,
                        sources_used,
                        hydrated_pages: 0,
                        poll_registry: Vec::new(),
                        trace: oi_sdk::AnswerTrace::default(),
                    }
                })
        }
    };
    let elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0;

    match response {
        Ok(response) => {
            if explain {
                eprintln!(
                    "(path: intent={} answer_status={} model={} passages={} rejected_quotes={} hydrated={} sources={} uncertainty={} avg_ignorance={} total_ms={elapsed_ms:.1})",
                    response.answer.intent,
                    response.answer.status,
                    response.trace.extractor_model.as_deref().unwrap_or("none"),
                    response.trace.retained_passages,
                    response.trace.rejected_quotes,
                    response.hydrated_pages,
                    response.sources_used.join("+"),
                    response.answer.uncertainty_mode,
                    response
                        .answer
                        .average_ignorance
                        .map(|value| format!("{value:.3}"))
                        .unwrap_or_else(|| "n/a".to_string()),
                );
            }
            print_answered_search(config, &response);
            if is_anonymous {
                eprintln!("(anonymous mode - 10 queries/day, run `oi login` for 100/day)");
            }
            // Keep the ordinary flywheel for non-claim queries and any
            // secondary discoveries not needed by the answer itself.
            if let ClientMode::Local(client) = client
                && client.pending_crawl_count() > 0
            {
                let _ = client.process_crawl_queue();
            }
            if let ClientMode::Local(client) = client {
                read_new_pages_for_claims(client);
            }
            ExitCode::Success
        }
        Err(error) => {
            output::print_err(&format!("Search answer failed: {error}"), &config.format);
            error.exit_code()
        }
    }
}

/// The bond behind a result's source, as read from the stake registry, in
/// the registry's base units (lamports for a bond read from Solana). "-" when
/// nobody has bonded anything to it.
fn bond_label(result: &oi_types::SearchResultItem) -> String {
    if result.stake_truth == 0 {
        return "-".to_string();
    }
    result.stake_truth.to_string()
}

/// Read the pages this search added to the index for claims. Runs after
/// results are printed; each page costs model calls.
fn read_new_pages_for_claims(client: &OpenSonarXClient) {
    let pending = client.pending_claim_count();
    if pending == 0 {
        return;
    }
    eprintln!("(reading {pending} new pages for claims...)");
    let started = std::time::Instant::now();
    let before = client.claim_count();
    client.process_claim_queue();
    let after = client.claim_count();
    eprintln!(
        "(stored {} verified claims in {:.1} s; {after} in the index)",
        after - before,
        started.elapsed().as_secs_f64(),
    );
}

/// What the claims stored at ingestion say about a query, with the time the
/// lookup took. Nothing here calls a model.
fn print_stored_claims(client: &OpenSonarXClient, query: &str) {
    if client.claim_count() == 0 {
        println!("CLAIMS [none stored]\n");
        return;
    }
    let started = std::time::Instant::now();
    let signal = match client.corroboration(query) {
        Ok(signal) => signal,
        Err(error) => {
            println!("CLAIMS [lookup failed: {error}]\n");
            return;
        }
    };
    let elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0;
    let policy = client.claim_policy();
    // The count is of source families holding a claim close to the nearest
    // one. Closeness is embedding similarity: the listed claims are similar,
    // which is not the same as saying one thing, so they are shown rather
    // than summarized as agreement.
    let state = match signal.supporting_families {
        0 => "no stored claim close enough".to_string(),
        1 => "similar claims from 1 source family".to_string(),
        families => format!("similar claims from {families} source families"),
    };
    println!(
        "CLAIMS [{state} | {elapsed_ms:.1} ms | {} stored]",
        client.claim_count()
    );
    if signal.supporting_families > 0 {
        let mut families = std::collections::BTreeSet::new();
        for claim in signal.matches.iter().filter(|claim| {
            claim.query_similarity >= policy.min_query_similarity
                && claim.head_similarity >= policy.min_claim_similarity
        }) {
            // One line per source family: the first, closest claim from it.
            if families.insert(claim.source_family.clone()) {
                println!("  {}: {}", claim.source_family, claim.claim);
                println!("      Quote: {}", claim.quote);
            }
        }
    }
    println!();
}

fn print_answered_search(config: &CliConfig, response: &oi_sdk::AnsweredSearch) {
    output::print_item(response, &config.format, |response| {
        println!(
            "EVIDENCE [{} | {}]",
            response.answer.status, response.answer.uncertainty_mode
        );
        println!("{}", response.answer.summary);
        if let Some(warning) = &response.answer.warning {
            println!("Warning: {warning}");
        }
        for (index, claim) in response.answer.claims.iter().enumerate() {
            println!("Claim {} [{}]: {}", index + 1, claim.status, claim.text);
            for citation in &claim.citations {
                println!(
                    "  [{} | {}] {} — {}",
                    citation.decision_rank, citation.evidence_role, citation.title, citation.url
                );
                if let Some(excerpt) = &citation.evidence_excerpt
                    && excerpt != &claim.text
                {
                    println!("      Quote: {excerpt}");
                }
            }
        }
        println!();
        output::print_list(
            &response.results,
            &config.format,
            &[
                "RANK",
                "RETR",
                "REL",
                "DEC",
                "UNCERTAINTY",
                "BOND",
                "URL",
                "SOURCE",
            ],
            |result| {
                let decision = result.decision.as_ref();
                let bond = bond_label(result);
                vec![
                    result.rank.to_string(),
                    decision
                        .map(|value| value.retrieval_rank.to_string())
                        .unwrap_or_else(|| result.rank.to_string()),
                    format!("{:.3}", result.final_score),
                    decision
                        .map(|value| format!("{:.3}", value.decision_score))
                        .unwrap_or_else(|| "-".to_string()),
                    decision
                        .map(|value| value.uncertainty_mode.clone())
                        .unwrap_or_else(|| "not_run".to_string()),
                    bond,
                    result.url.clone(),
                    result.source.clone(),
                ]
            },
        );
    });
}

/// Query a running P2P node's HTTP API.
fn search_via_node(
    config: &CliConfig,
    node_url: &str,
    query: &str,
    limit: u32,
    filters: &oi_types::QueryFilters,
) -> ExitCode {
    let url = format!("{}/search", node_url.trim_end_matches('/'));
    let body = serde_json::json!({
        "query": query,
        "limit": limit,
        "filters": filters,
    });
    let body_bytes = serde_json::to_vec(&body).unwrap();

    let response = ureq::post(&url)
        .content_type("application/json")
        .send(&body_bytes[..]);

    let response = match response {
        Ok(r) => r,
        Err(e) => {
            output::print_err(
                &format!("Failed to reach node at {node_url}: {e}"),
                &config.format,
            );
            return ExitCode::UserError;
        }
    };

    let body = match response.into_body().read_to_vec() {
        Ok(b) => b,
        Err(e) => {
            output::print_err(&format!("Failed to read response: {e}"), &config.format);
            return ExitCode::UserError;
        }
    };

    let items: Vec<oi_types::SearchResultItem> = match serde_json::from_slice(&body) {
        Ok(i) => i,
        Err(e) => {
            output::print_err(&format!("Failed to parse response: {e}"), &config.format);
            return ExitCode::UserError;
        }
    };

    if items.is_empty() {
        output::print_ok(&format!("No results found for: {query}"), &config.format);
    } else {
        output::print_list(
            &items,
            &config.format,
            &["RANK", "SCORE", "URL", "SOURCE"],
            |r| {
                vec![
                    r.rank.to_string(),
                    format!("{:.3}", r.final_score),
                    r.url.clone(),
                    r.source.clone(),
                ]
            },
        );
    }
    ExitCode::Success
}

fn answer_via_node(
    config: &CliConfig,
    node_url: &str,
    query: &str,
    limit: u32,
    filters: &oi_types::QueryFilters,
    explain: bool,
) -> ExitCode {
    let url = format!("{}/answer", node_url.trim_end_matches('/'));
    let body = serde_json::json!({
        "query": query,
        "limit": limit,
        "filters": filters,
    });
    let started = std::time::Instant::now();
    let response = ureq::post(&url)
        .content_type("application/json")
        .send(&serde_json::to_vec(&body).unwrap()[..]);
    let response = match response {
        Ok(response) => response,
        Err(error) => {
            output::print_err(
                &format!("Failed to reach node at {node_url}: {error}"),
                &config.format,
            );
            return ExitCode::UserError;
        }
    };
    let body = match response.into_body().read_to_vec() {
        Ok(body) => body,
        Err(error) => {
            output::print_err(
                &format!("Failed to read answer response: {error}"),
                &config.format,
            );
            return ExitCode::UserError;
        }
    };
    let answer: oi_sdk::AnsweredSearch = match serde_json::from_slice(&body) {
        Ok(answer) => answer,
        Err(error) => {
            output::print_err(
                &format!("Failed to parse answer response: {error}"),
                &config.format,
            );
            return ExitCode::UserError;
        }
    };
    if explain {
        eprintln!(
            "(path: node_answer intent={} status={} hydrated={} sources={} total_ms={:.1})",
            answer.answer.intent,
            answer.answer.status,
            answer.hydrated_pages,
            answer.sources_used.join("+"),
            started.elapsed().as_secs_f64() * 1_000.0,
        );
    }
    print_answered_search(config, &answer);
    ExitCode::Success
}

// ── Feedback ────────────────────────────────────────────────────────────────

pub fn feedback(
    config: &CliConfig,
    result_id: &str,
    useful: bool,
    query: Option<&str>,
    result_url: Option<&str>,
    result_rank: Option<u32>,
) -> ExitCode {
    let client = match build_client_mode(config) {
        Ok(c) => c,
        Err(code) => return code,
    };

    let result = match &client {
        ClientMode::Local(c) => c.feedback(result_id, useful),
        ClientMode::Gateway(c) => {
            c.feedback_with_context(result_id, useful, query, result_url, result_rank)
        }
    };

    match result {
        Ok(()) => {
            output::print_ok(
                &format!("Feedback submitted: result={result_id} useful={useful}"),
                &config.format,
            );
            ExitCode::Success
        }
        Err(e) => {
            output::print_err(&format!("Feedback failed: {e}"), &config.format);
            e.exit_code()
        }
    }
}

pub fn feedback_stats(config: &CliConfig) -> ExitCode {
    let client = match build_client_mode(config) {
        Ok(c) => c,
        Err(code) => return code,
    };

    let result = match &client {
        ClientMode::Local(c) => c.feedback_stats(),
        ClientMode::Gateway(c) => c.feedback_stats(),
    };

    match result {
        Ok(stats) => {
            let info = serde_json::json!({
                "total_signals": stats.total_signals,
                "positive_signals": stats.positive_signals,
                "negative_signals": stats.negative_signals,
                "unique_queries": stats.unique_queries,
                "top_helpful_domains": stats.top_helpful_domains.len(),
            });
            output::print_item(&info, &config.format, |item| {
                println!("Feedback Data Moat Statistics:");
                println!("  Total signals:    {}", item["total_signals"]);
                println!("  Positive:         {}", item["positive_signals"]);
                println!("  Negative:         {}", item["negative_signals"]);
                println!("  Unique queries:   {}", item["unique_queries"]);
            });
            if !stats.top_helpful_domains.is_empty() {
                output::print_list(
                    &stats.top_helpful_domains,
                    &config.format,
                    &["DOMAIN", "HELPFUL", "UNHELPFUL", "SCORE"],
                    |d| {
                        vec![
                            d.domain.clone(),
                            d.helpful_count.to_string(),
                            d.unhelpful_count.to_string(),
                            format!("{:.2}", d.score),
                        ]
                    },
                );
            }
            ExitCode::Success
        }
        Err(e) => {
            output::print_err(
                &format!("Failed to fetch feedback stats: {e}"),
                &config.format,
            );
            e.exit_code()
        }
    }
}

// ── Stake / Unstake ─────────────────────────────────────────────────────────

pub fn stake(config: &CliConfig, domain: &str, amount: u64) -> ExitCode {
    if let Some(code) = reject_in_open_mode(config, "stake") {
        return code;
    }
    // Validate entity ref format before sending to chain
    if let Err(e) = oi_types::entity::EntityRef::parse(domain) {
        output::print_err(&format!("Invalid entity ref: {e}"), &config.format);
        return ExitCode::UserError;
    }

    let client = match require_local(config, "stake") {
        Ok(c) => c,
        Err(code) => return code,
    };

    match client.stake(domain, amount) {
        Ok(tx_hash) => {
            let info = serde_json::json!({
                "action": "stake",
                "domain": domain,
                "amount": amount,
                "tx_hash": tx_hash,
                "network": config.network,
            });
            output::print_item(&info, &config.format, |item| {
                println!("Staked {} $TRUTH for {}", item["amount"], item["domain"]);
                // The SDK records the bond in the local registry; a
                // `stub_tx_` reference is not a chain transaction.
                match item["tx_hash"].as_str() {
                    Some(reference) if reference.starts_with("stub_tx_") => {
                        println!("  Recorded in the local registry (no on-chain transaction)")
                    }
                    _ => println!("  Tx: {}", item["tx_hash"]),
                }
            });
            ExitCode::Success
        }
        Err(e) => {
            output::print_err(&format!("Stake failed: {e}"), &config.format);
            e.exit_code()
        }
    }
}

pub fn unstake(config: &CliConfig, domain: &str) -> ExitCode {
    if let Some(code) = reject_in_open_mode(config, "unstake") {
        return code;
    }
    if let Err(e) = oi_types::entity::EntityRef::parse(domain) {
        output::print_err(&format!("Invalid entity ref: {e}"), &config.format);
        return ExitCode::UserError;
    }

    let client = match require_local(config, "unstake") {
        Ok(c) => c,
        Err(code) => return code,
    };

    match client.unstake(domain) {
        Ok(tx_hash) => {
            let info = serde_json::json!({
                "action": "unstake",
                "domain": domain,
                "tx_hash": tx_hash,
                "cooldown_days": 7,
            });
            output::print_item(&info, &config.format, |item| {
                println!("Unstake initiated for {} (7-day cooldown)", item["domain"]);
                println!("  Tx: {}", item["tx_hash"]);
            });
            ExitCode::Success
        }
        Err(e) => {
            output::print_err(&format!("Unstake failed: {e}"), &config.format);
            e.exit_code()
        }
    }
}

// ── Domain ──────────────────────────────────────────────────────────────────

pub fn domain(config: &CliConfig, action: DomainCommands) -> ExitCode {
    if let Some(code) = reject_in_open_mode(config, "domain") {
        return code;
    }
    let client = match require_local(config, "domain") {
        Ok(c) => c,
        Err(code) => return code,
    };

    match action {
        DomainCommands::Verify { domain } => {
            output::print_ok(
                &format!(
                    "Checking DNS TXT record for {domain}...\n\
                     Expected record: TXT \"opensonarx-verify=<your-wallet-address>\"\n\
                     Add this record to {domain} and re-run to complete verification."
                ),
                &config.format,
            );
            ExitCode::Success
        }
        DomainCommands::List => match client.list_stakes() {
            Ok(stakes) => {
                output::print_list(
                    &stakes,
                    &config.format,
                    &["DOMAIN", "AMOUNT", "USD", "VERIFIED", "REPUTATION"],
                    |s| {
                        vec![
                            s.domain.clone(),
                            s.amount_lamports.to_string(),
                            format!("${:.2}", s.amount_usd),
                            if s.dns_verified { "yes" } else { "no" }.to_string(),
                            format!("{:.2}", s.reputation),
                        ]
                    },
                );
                ExitCode::Success
            }
            Err(e) => {
                output::print_err(&format!("Failed to list domains: {e}"), &config.format);
                e.exit_code()
            }
        },
        DomainCommands::Info { domain } => match client.get_stake(&domain) {
            Ok(Some(info)) => {
                output::print_item(&info, &config.format, |item| {
                    println!("Domain: {}", item.domain);
                    println!("  Staked:     {} lamports", item.amount_lamports);
                    println!("  USD value:  ${:.2}", item.amount_usd);
                    println!("  Verified:   {}", item.dns_verified);
                    println!("  Reputation: {:.2}", item.reputation);
                    println!("  Wallet:     {}", item.wallet);
                });
                ExitCode::Success
            }
            Ok(None) => {
                let info = serde_json::json!({
                    "domain": domain,
                    "status": "not staked",
                });
                output::print_item(&info, &config.format, |item| {
                    println!("Domain: {}", item["domain"]);
                    println!("  Status: {}", item["status"]);
                });
                ExitCode::Success
            }
            Err(e) => {
                output::print_err(&format!("Failed to get domain info: {e}"), &config.format);
                e.exit_code()
            }
        },
        DomainCommands::Curate { domain, amount } => {
            if let Err(e) = oi_types::entity::EntityRef::parse(&domain) {
                output::print_err(&format!("Invalid entity ref: {e}"), &config.format);
                return ExitCode::UserError;
            }
            match client.stake(&domain, amount) {
                Ok(tx_hash) => {
                    let info = serde_json::json!({
                        "action": "curate",
                        "domain": domain,
                        "amount": amount,
                        "tx_hash": tx_hash,
                    });
                    output::print_item(&info, &config.format, |item| {
                        println!("Curated {} with {} $TRUTH", item["domain"], item["amount"]);
                        println!("  Tx: {}", item["tx_hash"]);
                    });
                    ExitCode::Success
                }
                Err(e) => {
                    output::print_err(&format!("Curation failed: {e}"), &config.format);
                    e.exit_code()
                }
            }
        }
    }
}

// ── Dispute ─────────────────────────────────────────────────────────────────

pub fn dispute(config: &CliConfig, action: DisputeCommands) -> ExitCode {
    if let Some(code) = reject_in_open_mode(config, "dispute") {
        return code;
    }
    match action {
        DisputeCommands::Open {
            domain,
            bond,
            reason,
            fiat,
        } => {
            // Fiat path: pay with credit card via gateway
            if let Some(payment_method_id) = fiat {
                let client = match build_client_mode(config) {
                    Ok(ClientMode::Gateway(c)) => c,
                    Ok(ClientMode::Local(_)) => {
                        output::print_err(
                            "The --fiat flag requires gateway mode. Use `oi login` first.",
                            &config.format,
                        );
                        return ExitCode::UserError;
                    }
                    Err(code) => return code,
                };

                match client.open_fiat_dispute(&domain, &reason, &payment_method_id) {
                    Ok(resp) => {
                        let info = serde_json::json!({
                            "action": "open_fiat_dispute",
                            "bond_id": resp.bond_id,
                            "domain": resp.domain,
                            "amount_cents": resp.amount_cents,
                            "bond_truth_lamports": resp.bond_truth_lamports,
                            "status": resp.status,
                        });
                        output::print_item(&info, &config.format, |item| {
                            println!("Fiat dispute opened against {}", item["domain"]);
                            println!("  Bond ID:  {}", item["bond_id"]);
                            println!(
                                "  Amount:   ${:.2}",
                                item["amount_cents"].as_u64().unwrap_or(0) as f64 / 100.0
                            );
                            println!("  $TRUTH:   {} lamports", item["bond_truth_lamports"]);
                            println!("  Status:   {}", item["status"]);
                        });
                        ExitCode::Success
                    }
                    Err(e) => {
                        output::print_err(
                            &format!("Failed to open fiat dispute: {e}"),
                            &config.format,
                        );
                        e.exit_code()
                    }
                }
            } else {
                // Web3 path: direct on-chain bond
                let info = serde_json::json!({
                    "action": "open_dispute",
                    "domain": domain,
                    "bond_truth": bond,
                    "reason": reason,
                    "status": "submitted",
                });
                output::print_item(&info, &config.format, |item| {
                    println!("Opening dispute against {}", item["domain"]);
                    println!("  Bond:   {} $TRUTH", item["bond_truth"]);
                    println!("  Reason: {}", item["reason"]);
                    println!("  Status: {}", item["status"]);
                });
                ExitCode::Success
            }
        }
        DisputeCommands::List => {
            let client = match build_client_mode(config) {
                Ok(c) => c,
                Err(code) => return code,
            };

            let disputes = match &client {
                ClientMode::Gateway(c) => match c.list_disputes() {
                    Ok(d) => d,
                    Err(e) => {
                        output::print_err(&format!("Failed to list disputes: {e}"), &config.format);
                        return e.exit_code();
                    }
                },
                ClientMode::Local(c) => c.list_disputes().unwrap_or_default(),
            };

            output::print_list(
                &disputes,
                &config.format,
                &["DOMAIN", "REPORTER", "BOND", "STATUS", "VOTES_G", "VOTES_I"],
                |d| {
                    vec![
                        d.domain.clone(),
                        d.reporter.clone(),
                        d.bond.to_string(),
                        d.status.clone(),
                        d.votes_guilty.to_string(),
                        d.votes_innocent.to_string(),
                    ]
                },
            );
            ExitCode::Success
        }
        DisputeCommands::Info { dispute_id } => {
            let client = match build_client_mode(config) {
                Ok(c) => c,
                Err(code) => return code,
            };

            match &client {
                ClientMode::Gateway(c) => match c.get_dispute(&dispute_id) {
                    Ok(resp) => {
                        output::print_item(&resp.dispute, &config.format, |d| {
                            println!("Dispute: {}:{}", d.domain, d.status);
                            println!("  Reporter: {}", d.reporter);
                            println!("  Bond:     {} $TRUTH", d.bond);
                            println!(
                                "  Votes:    {} guilty / {} innocent",
                                d.votes_guilty, d.votes_innocent
                            );
                        });
                        if let Some(bond) = &resp.fiat_bond {
                            println!(
                                "  Fiat Bond: {} (${:.2})",
                                bond.bond_id,
                                bond.amount_cents as f64 / 100.0
                            );
                            println!("  Status:    {:?}", bond.status);
                        }
                        ExitCode::Success
                    }
                    Err(e) => {
                        output::print_err(&format!("Failed to get dispute: {e}"), &config.format);
                        e.exit_code()
                    }
                },
                ClientMode::Local(_) => {
                    let info = serde_json::json!({
                        "dispute_id": dispute_id,
                        "status": "not found",
                    });
                    output::print_item(&info, &config.format, |item| {
                        println!("Dispute {}: {}", item["dispute_id"], item["status"]);
                    });
                    ExitCode::UserError
                }
            }
        }
    }
}

// ── Jury ────────────────────────────────────────────────────────────────────

pub fn jury(config: &CliConfig, action: JuryCommands) -> ExitCode {
    if let Some(code) = reject_in_open_mode(config, "jury") {
        return code;
    }
    match action {
        JuryCommands::Register => {
            let info = serde_json::json!({
                "action": "jury_register",
                "status": "submitted",
                "message": "Registration request submitted. Eligibility depends on stake amount and history.",
            });
            output::print_item(&info, &config.format, |item| {
                println!("Registering as juror...");
                println!("  Status: {}", item["status"]);
                println!("  {}", item["message"]);
            });
            ExitCode::Success
        }
        JuryCommands::Vote {
            dispute_id,
            verdict,
        } => {
            let info = serde_json::json!({
                "action": "jury_vote",
                "dispute_id": dispute_id,
                "verdict": verdict,
                "status": "submitted",
            });
            output::print_item(&info, &config.format, |item| {
                println!(
                    "Voting on dispute {}: {}",
                    item["dispute_id"], item["verdict"]
                );
                println!("  Status: {}", item["status"]);
            });
            ExitCode::Success
        }
        JuryCommands::Status => {
            let info = serde_json::json!({
                "eligible": false,
                "reason": "No active stake meets the minimum juror threshold",
            });
            output::print_item(&info, &config.format, |item| {
                println!("Jury eligible: {}", item["eligible"]);
                println!("  Reason: {}", item["reason"]);
            });
            ExitCode::UserError
        }
    }
}

// ── Yield ───────────────────────────────────────────────────────────────────

pub fn yield_cmd(config: &CliConfig, action: YieldCommands) -> ExitCode {
    // Creator earnings are local usage data — viewable in every mode.
    // On-chain yield (Show/History) still requires a wallet.
    if !matches!(action, YieldCommands::Creators)
        && let Some(code) = reject_in_open_mode(config, "yield")
    {
        return code;
    }
    match action {
        YieldCommands::Show => {
            let earnings = oi_sdk::OpenSonarXClient::get_earnings();
            let info = if let Some(snap) = earnings {
                serde_json::json!({
                    "accrued_truth": snap.total_earned_lamports,
                    "queries_served": snap.queries_served,
                    "pending_tickets": snap.pending_ticket_count,
                    "pending_truth": snap.pending_value_lamports,
                })
            } else {
                serde_json::json!({
                    "accrued_truth": 0,
                    "queries_served": 0,
                    "pending_tickets": 0,
                    "pending_truth": 0,
                })
            };
            output::print_item(&info, &config.format, |item| {
                println!("Node Earnings:");
                println!(
                    "  Earned:         {} lamports $TRUTH",
                    item["accrued_truth"]
                );
                println!("  Queries served: {}", item["queries_served"]);
                println!("  Pending tickets: {}", item["pending_tickets"]);
                println!("  Pending value:  {} lamports", item["pending_truth"]);
            });
            ExitCode::Success
        }
        YieldCommands::Creators => {
            // Compatibility command name; these are non-payable exposure
            // records, not licensed consumption or creator earnings.
            let client = build_local_client(config);
            let (queries, rows) = client.creator_earnings();
            if matches!(
                config.format,
                crate::output::Format::Json | crate::output::Format::Ndjson
            ) {
                let doc = serde_json::json!({
                    "record_kind": "non_payable_exposure_telemetry",
                    "monetary_value": false,
                    "settlement_eligible": false,
                    "queries_attributed": queries,
                    "creators": rows.iter().map(|(d, u)| serde_json::json!({
                        "domain": d,
                        "served": u.served,
                        "top3": u.top3,
                        "credits": u.credits,
                        "stake_backed": u.stake_backed,
                        "last_used": u.last_used,
                    })).collect::<Vec<_>>(),
                });
                println!("{}", serde_json::to_string_pretty(&doc).unwrap_or_default());
                return ExitCode::Success;
            }
            if rows.is_empty() {
                println!("No creator usage recorded yet. Serve some searches first.");
                return ExitCode::Success;
            }
            println!("Creator exposure ({queries} queries attributed):");
            println!(
                "Credits are non-monetary telemetry; no payment or licensed consumption is recorded."
            );
            println!(
                "{:<40} {:>7} {:>6} {:>8}  STAKED",
                "DOMAIN", "SERVED", "TOP-3", "CREDITS"
            );
            for (d, u) in rows.iter().take(25) {
                println!(
                    "{:<40} {:>7} {:>6} {:>8}  {}",
                    d,
                    u.served,
                    u.top3,
                    u.credits,
                    if u.stake_backed { "yes" } else { "-" }
                );
            }
            ExitCode::Success
        }
        YieldCommands::History => {
            let history: Vec<oi_types::RewardInfo> = Vec::new();
            output::print_list(
                &history,
                &config.format,
                &["DOMAIN", "STAKER", "ACCRUED", "CLAIMED"],
                |r| {
                    vec![
                        r.domain.clone(),
                        r.staker.clone(),
                        r.accrued.to_string(),
                        r.claimed.to_string(),
                    ]
                },
            );
            ExitCode::Success
        }
    }
}

// ── Network ─────────────────────────────────────────────────────────────────

/// The shard names behind `--shards` when a routing file is in use: each
/// entry is a cluster number of that routing, or one of its shard names.
fn learned_shard_ids(
    routing: &oi_types::shard::LearnedRouting,
    chosen: &[String],
) -> Result<Vec<String>, String> {
    let known = routing.shard_ids();
    chosen
        .iter()
        .map(|entry| {
            let entry = entry.trim();
            entry
                .parse::<usize>()
                .ok()
                .and_then(|cluster| known.get(cluster).cloned())
                .or_else(|| known.iter().find(|id| *id == entry).cloned())
                .ok_or_else(|| {
                    format!(
                        "--shards {entry:?} is not a shard of routing {} (it has clusters 0 to {})",
                        routing.version,
                        known.len().saturating_sub(1)
                    )
                })
        })
        .collect()
}

pub fn network(config: &CliConfig, action: NetworkCommands, verbose: bool) -> ExitCode {
    match action {
        NetworkCommands::Node {
            port,
            bootstrap,
            sentinel,
            domains,
            shards,
            concurrency,
            http_port,
            min_replicas,
            sync_secs,
            routing,
        } => {
            // A routing file replaces the built-in topics. Loaded before
            // anything starts so a bad file stops the node instead of
            // letting it announce shards nobody else understands.
            let learned = match routing
                .as_deref()
                .map(|path| oi_types::shard::LearnedRouting::load(std::path::Path::new(path)))
            {
                Some(Ok(routing)) => Some(routing),
                Some(Err(error)) => {
                    output::print_err(&format!("Routing file: {error}"), &config.format);
                    return ExitCode::UserError;
                }
                None => None,
            };
            let shards = match (&learned, shards) {
                (Some(routing), Some(chosen)) => match learned_shard_ids(routing, &chosen) {
                    Ok(ids) => Some(ids),
                    Err(error) => {
                        output::print_err(&error, &config.format);
                        return ExitCode::UserError;
                    }
                },
                (Some(routing), None) => Some(routing.shard_ids()),
                (None, shards) => shards,
            };
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let bootstrap_peers = if let Some(peers) = bootstrap {
                    peers
                } else if !config.bootstrap_peers.is_empty() {
                    config.bootstrap_peers.clone()
                } else {
                    Vec::new()
                };
                // Under the data dir, like the index: two nodes on one
                // machine must not share (and overwrite) one peer table.
                let routing_table_path = crate::config_file::ConfigFile::config_dir()
                    .map(|dir| dir.join("routing_table.json").to_string_lossy().to_string());

                let net_config = oi_network::config::NetworkConfig {
                    listen_port: port,
                    bootstrap_peers,
                    routing_table_path,
                    min_replicas_per_shard: min_replicas,
                    replication_check_interval_secs: sync_secs.max(1),
                    ..Default::default()
                };

                // Create a hybrid index for this node
                let dim = 384; // Standard MiniLM dimension
                let index_path = crate::config_file::ConfigFile::index_dir().expect("home dir");
                let storage = oi_index::Storage::new(&index_path).ok();
                let index = if let Some(ref s) = storage
                    && s.segment_exists("hnsw.bin")
                {
                    match oi_index::HybridIndex::load(s) {
                        Ok(idx) => {
                            println!("  Loaded index: {} docs", idx.doc_count());
                            idx
                        }
                        Err(e) => {
                            println!("  Failed to load index ({e}), starting fresh");
                            oi_index::HybridIndex::new(dim)
                        }
                    }
                } else {
                    oi_index::HybridIndex::new(dim)
                };
                // Attach the persisted stake state so trust verdicts compute
                // on node-served results — without this, staked domains read
                // "unverified" via the node and min_trust filters everything.
                let node_stake_registry =
                    std::sync::Arc::new(oi_staking::InMemoryStakeRegistry::new());
                if let Some(stakes_path) = crate::config_file::ConfigFile::config_dir()
                    .map(|d| d.join("stakes.json"))
                    .filter(|p| p.exists())
                {
                    oi_sdk::OpenSonarXClient::load_stakes_from(
                        &stakes_path.to_string_lossy(),
                        &node_stake_registry,
                    );
                }
                let index = index.with_stake_registry(node_stake_registry.clone()
                    as std::sync::Arc<dyn oi_types::traits::StakeRegistry>);
                let index = std::sync::Arc::new(index);

                // Shard mode: explicit --shards override, or auto-discover via topic router
                let shard_override = shards.is_some();
                let shard_ids = shards.unwrap_or_else(|| vec!["topic-general".into()]);

                let mut node =
                    oi_network::Node::new(net_config).with_index(index.clone(), shard_ids.clone());
                let node_routing_table = node.routing_table();
                println!("Starting OpenSonarX node...");
                println!("  Peer ID:  {}", node.peer_id);
                println!("  Listen:   /ip4/0.0.0.0/tcp/{port}");
                println!("  Network:  {}", config.network);
                println!(
                    "  Shards:   {} {}",
                    shard_ids.join(", "),
                    if shard_override { "(manual)" } else { "(auto)" }
                );
                println!("  Index:    {} docs", index.doc_count());

                // Build topic router for sentinel classification
                let topic_router = if let Some(routing) = &learned {
                    let embedder = oi_sdk::OpenSonarXClient::default_embedder();
                    if embedder.name() != routing.embedder {
                        eprintln!(
                            "  Routing file is for embedder {:?}; this node embeds with {:?}",
                            routing.embedder,
                            embedder.name()
                        );
                        return;
                    }
                    let router = routing.router();
                    match index.reassign_shards(&router) {
                        Ok(moved) => println!(
                            "  Routing:  learned, version {} ({} shards; {moved} stored chunks reassigned)",
                            routing.version,
                            routing.centroids.len()
                        ),
                        Err(e) => eprintln!("  Routing: could not reassign stored chunks ({e})"),
                    }
                    Some(std::sync::Arc::new(router))
                } else if sentinel && !shard_override {
                    let embedder = oi_sdk::OpenSonarXClient::default_embedder();
                    let descriptions: Vec<&str> = oi_types::BUILTIN_TOPICS
                        .iter()
                        .map(|t| t.description)
                        .collect();
                    match embedder.embed_text_batch(&descriptions) {
                        Ok(embeddings) => {
                            let centroids: Vec<(String, Vec<f32>)> = oi_types::BUILTIN_TOPICS
                                .iter()
                                .zip(embeddings)
                                .map(|(t, e)| (t.shard_id.to_string(), e))
                                .collect();
                            let router = oi_types::TopicRouter::new(centroids);
                            println!(
                                "  Topic router: initialized ({} topics)",
                                oi_types::BUILTIN_TOPICS.len()
                            );
                            Some(std::sync::Arc::new(router))
                        }
                        Err(e) => {
                            eprintln!(
                                "  Topic router: failed to embed topics ({e}), using default shard"
                            );
                            None
                        }
                    }
                } else {
                    None
                };

                // Attach topic router to the node for smart query fanout
                if let Some(ref router) = topic_router {
                    node = node.with_topic_router(router.clone());
                }

                if sentinel {
                    println!("  Sentinel: enabled (concurrency={concurrency})");
                    if let Some(ref doms) = domains {
                        println!("  Domains:  {}", doms.join(", "));
                    }
                }
                if node.config.bootstrap_peers.is_empty() {
                    println!(
                        "  No bootstrap peers configured — running in local discovery mode (mDNS)."
                    );
                    println!(
                        "  To join the network: oi network node --bootstrap /ip4/<peer>/tcp/4001"
                    );
                } else {
                    println!("  Bootstrap peers:");
                    for p in &node.config.bootstrap_peers {
                        println!("    - {p}");
                    }
                }

                match node.start().await {
                    Ok(handle) => {
                        println!("  Node running, peer ID: {}", handle.peer_id);

                        // Spawn HTTP API if --http-port is set. It serves the
                        // full blended search over the node's shared index.
                        if let Some(hp) = http_port {
                            let api_handle = handle.clone();
                            let api_index = index.clone();
                            let api_sdk_config = SdkConfig {
                                brave_api_key: std::env::var("BRAVE_API_KEY").ok(),
                                serper_api_key: std::env::var("SERPER_API_KEY").ok(),
                                search_engines: config.search_engines.clone(),
                                crawl_on_query: true,
                                crawl_on_query_max_urls: 3,
                                stakes_path: crate::config_file::ConfigFile::config_dir()
                                    .map(|d| d.join("stakes.json").to_string_lossy().to_string()),
                                creator_ledger_path: crate::config_file::ConfigFile::config_dir()
                                    .map(|d| {
                                        d.join("creator_ledger.json").to_string_lossy().to_string()
                                    }),
                                answer_model: answer_model_from_env(),
                                answer_calibration_path: std::env::var("OI_ANSWER_CALIBRATION")
                                    .ok()
                                    .filter(|path| !path.trim().is_empty()),
                                ..Default::default()
                            };
                            let api_index_path = crate::config_file::ConfigFile::index_dir();
                            // With a learned routing, the pages this node
                            // learns are classified by it, and the node
                            // keeps the chunks of the shards it serves.
                            let api_context = crate::node_api::NodeContext {
                                shards: shard_ids.clone(),
                                routing_version: learned
                                    .as_ref()
                                    .map(|routing| routing.version.clone()),
                                routing: learned
                                    .as_ref()
                                    .and(topic_router.clone())
                                    .map(|router| (router, shard_ids.clone())),
                                routing_table: node_routing_table.clone(),
                                stake_registry: node_stake_registry.clone(),
                                stakes_path: crate::config_file::ConfigFile::config_dir()
                                    .map(|d| d.join("stakes.json")),
                            };
                            tokio::spawn(async move {
                                if let Err(e) = crate::node_api::run_node_api(
                                    hp,
                                    api_handle,
                                    api_index,
                                    api_index_path,
                                    api_sdk_config,
                                    api_context,
                                )
                                .await
                                {
                                    eprintln!("HTTP API error: {e}");
                                }
                            });
                        }

                        if sentinel {
                            run_integrated_sentinel(
                                handle,
                                index.clone(),
                                &storage,
                                shard_ids,
                                domains,
                                concurrency,
                                verbose,
                                topic_router,
                                shard_override,
                            )
                            .await;
                        } else {
                            // No sentinel — just wait for Ctrl+C
                            tokio::signal::ctrl_c().await.ok();
                            println!("\nShutting down...");
                            handle.shutdown();
                            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                            if let Some(ref s) = storage {
                                if let Err(e) = index.save(s) {
                                    eprintln!("Failed to save index: {e}");
                                } else {
                                    println!("  Index saved ({} docs)", index.doc_count());
                                }
                            }
                        }
                    }
                    Err(e) => {
                        output::print_err(&format!("Node failed to start: {e}"), &config.format);
                    }
                }
            });
            ExitCode::Success
        }
        NetworkCommands::LearnRouting { shards, output } => {
            let Some(index_path) = crate::config_file::ConfigFile::index_dir() else {
                output::print_err("No data directory", &config.format);
                return ExitCode::UserError;
            };
            let index = match oi_index::Storage::new(&index_path)
                .map_err(|e| e.to_string())
                .and_then(|storage| {
                    oi_index::HybridIndex::load(&storage).map_err(|e| e.to_string())
                }) {
                Ok(index) => index,
                Err(error) => {
                    output::print_err(
                        &format!("Cannot load the index at {}: {error}", index_path.display()),
                        &config.format,
                    );
                    return ExitCode::UserError;
                }
            };
            let vectors = index.chunk_vectors().unwrap_or_default();
            let embedder = oi_sdk::OpenSonarXClient::default_embedder();
            let Some(routing) =
                oi_types::shard::LearnedRouting::learn(&vectors, shards, embedder.name())
            else {
                output::print_err(
                    "The index holds no pages to learn a routing from",
                    &config.format,
                );
                return ExitCode::UserError;
            };
            let path = output
                .map(std::path::PathBuf::from)
                .or_else(|| {
                    crate::config_file::ConfigFile::config_dir().map(|d| d.join("routing.json"))
                })
                .unwrap_or_else(|| std::path::PathBuf::from("routing.json"));
            if let Err(error) = routing.save(&path) {
                output::print_err(
                    &format!("Cannot write {}: {error}", path.display()),
                    &config.format,
                );
                return ExitCode::UserError;
            }
            let router = routing.router();
            let mut sizes: std::collections::BTreeMap<String, usize> =
                std::collections::BTreeMap::new();
            for vector in &vectors {
                *sizes
                    .entry(router.classify(vector).to_string())
                    .or_default() += 1;
            }
            println!("Routing learned from {} stored chunks", vectors.len());
            println!("  Version:  {}", routing.version);
            println!("  Embedder: {}", routing.embedder);
            println!("  File:     {}", path.display());
            for shard in routing.shard_ids() {
                println!(
                    "  {shard}: {} chunks",
                    sizes.get(&shard).copied().unwrap_or(0)
                );
            }
            println!(
                "Every node of the network must start with this same file: oi network node --routing {}",
                path.display()
            );
            ExitCode::Success
        }
        NetworkCommands::Status => {
            let info = serde_json::json!({
                "connected": false,
                "network": config.network,
                "rpc_url": config.rpc_url,
                "peers": 0,
            });
            output::print_item(&info, &config.format, |item| {
                println!("Network status:");
                println!("  Connected: {}", item["connected"]);
                println!("  Network:   {}", item["network"]);
                println!("  RPC:       {}", item["rpc_url"]);
                println!("  Peers:     {}", item["peers"]);
            });
            ExitCode::Success
        }
        NetworkCommands::Peers => {
            let peers: Vec<oi_types::PeerInfo> = Vec::new();
            output::print_list(
                &peers,
                &config.format,
                &["PEER_ID", "ADDRESS", "SHARDS", "VECTORS", "UPTIME"],
                |p| {
                    vec![
                        p.peer_id.clone(),
                        p.addr.clone(),
                        p.shard_ids.len().to_string(),
                        p.vectors_stored.to_string(),
                        format!("{:.1}%", p.uptime_pct),
                    ]
                },
            );
            ExitCode::Success
        }
    }
}

// ── Integrated Sentinel ─────────────────────────────────────────────────────

/// Run the integrated sentinel crawl daemon inside the node process.
///
/// Crawled content is embedded, ingested into the SHARED `Arc<HybridIndex>`
/// (the same index the query handler reads), and announced to the P2P gossip
/// network. On Ctrl+C the index is saved to disk.
#[allow(clippy::too_many_arguments)]
async fn run_integrated_sentinel(
    handle: oi_network::NodeHandle,
    index: std::sync::Arc<oi_index::HybridIndex>,
    storage: &Option<oi_index::Storage>,
    shard_ids: Vec<String>,
    domains: Option<Vec<String>>,
    concurrency: usize,
    verbose: bool,
    topic_router: Option<std::sync::Arc<oi_types::TopicRouter>>,
    shard_override: bool,
) {
    use oi_crawl::daemon::{CrawlDaemon, CrawlEvent, DaemonConfig, MonitoredDomain};
    use oi_types::traits::SearchIndex;

    let embedder = oi_sdk::OpenSonarXClient::default_embedder();
    let sentinel_id = handle.peer_id.to_string();
    let default_shard = shard_ids
        .first()
        .cloned()
        .unwrap_or_else(|| "topic-general".into());

    let daemon_config = DaemonConfig {
        concurrency,
        ..Default::default()
    };
    let (daemon, mut event_rx) = CrawlDaemon::new(daemon_config);
    let daemon = std::sync::Arc::new(daemon);

    // Add initial monitored domains
    if let Some(ref doms) = domains {
        let monitored: Vec<MonitoredDomain> = doms
            .iter()
            .map(|d| MonitoredDomain {
                domain: d.clone(),
                stake_amount: 0,
                feed_urls: Vec::new(),
                hub_urls: Vec::new(),
            })
            .collect();
        daemon.add_domains(monitored).await;
    }

    // Spawn event handler: embed → classify → ingest → announce
    let index_for_events = index.clone();
    let handle_for_events = handle.clone();
    let sentinel_id_clone = sentinel_id.clone();
    let default_shard_clone = default_shard.clone();
    let verbose_events = verbose;
    let discovered_shards: std::sync::Arc<std::sync::Mutex<std::collections::HashSet<String>>> =
        std::sync::Arc::new(std::sync::Mutex::new(shard_ids.iter().cloned().collect()));
    let discovered_shards_clone = discovered_shards.clone();
    tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            let (url, domain, content_hash, chunk_count, mut chunks, is_update) = match event {
                CrawlEvent::ContentDiscovered {
                    url,
                    domain,
                    content_hash,
                    chunk_count,
                    chunks,
                } => (url, domain, content_hash, chunk_count, chunks, false),
                CrawlEvent::ContentUpdated {
                    url,
                    domain,
                    new_hash,
                    chunk_count,
                    chunks,
                    ..
                } => (url, domain, new_hash, chunk_count, chunks, true),
            };

            // Embed each chunk that doesn't already have an embedding
            for chunk in &mut chunks {
                if chunk.embedding.is_none()
                    && let Ok(emb) = embedder.embed_text(&chunk.content)
                {
                    chunk.embedding = Some(emb);
                }
            }

            // Classify each chunk into a topic shard (if topic router is available)
            let mut content_shard = default_shard_clone.clone();
            if let Some(ref router) = topic_router {
                for chunk in &mut chunks {
                    if let Some(ref embedding) = chunk.embedding {
                        let shard = router.classify(embedding);
                        if let Some(ref mut meta) = chunk.meta {
                            meta.shard_id = Some(shard.to_string());
                        }
                        // Use the first chunk's shard for the content announcement
                        if content_shard == default_shard_clone {
                            content_shard = shard.to_string();
                        }
                    }
                }
            }

            // Ingest into the shared index
            match index_for_events.ingest(&chunks) {
                Ok(()) => {
                    if verbose_events {
                        println!(
                            "[SENTINEL] Indexed {} ({}) — {} chunks [{}]",
                            url, domain, chunk_count, content_shard
                        );
                    }
                }
                Err(e) => {
                    eprintln!("[SENTINEL] Failed to ingest {}: {}", url, e);
                    continue;
                }
            }

            // Dynamic shard discovery: announce new shards to the network
            if !shard_override {
                let is_new = {
                    let mut shards = discovered_shards_clone.lock().unwrap();
                    shards.insert(content_shard.clone())
                };
                if is_new {
                    if verbose_events {
                        println!("[SENTINEL] Discovered new topic shard: {content_shard}");
                    }
                    let current: Vec<String> = discovered_shards_clone
                        .lock()
                        .unwrap()
                        .iter()
                        .cloned()
                        .collect();
                    handle_for_events.announce_shards(current);
                }
            }

            // Announce to gossip network
            handle_for_events.announce_content(
                sentinel_id_clone.clone(),
                url,
                domain,
                content_hash,
                chunk_count as u32,
                content_shard,
                is_update,
                String::new(),
            );
        }
    });

    // Run the crawl daemon
    let (shutdown_tx, shutdown_rx) = tokio::sync::mpsc::channel(1);
    let daemon_handle = tokio::spawn(daemon.clone().run(shutdown_rx));

    // Wait for Ctrl+C
    tokio::signal::ctrl_c().await.ok();
    println!("\nShutting down...");

    // Signal the P2P event loop to flush routing table + earnings
    handle.shutdown();
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Save index to disk
    if let Some(s) = storage {
        if let Err(e) = index.save(s) {
            eprintln!("Failed to save index: {e}");
        } else {
            println!("  Index saved ({} docs)", index.doc_count());
        }
    }

    let _ = shutdown_tx.send(()).await;
    let _ = daemon_handle.await;

    let stats = daemon.stats().await;
    println!("Sentinel stats:");
    println!("  URLs crawled:  {}", stats.urls_crawled);
    println!("  URLs indexed:  {}", stats.urls_indexed);
    println!("  URLs rejected: {}", stats.urls_rejected);
    println!("  Errors:        {}", stats.errors);
    println!("  Docs in index: {}", index.doc_count());
}

// ── Wallet ──────────────────────────────────────────────────────────────────

pub fn wallet(config: &CliConfig, action: WalletCommands) -> ExitCode {
    match action {
        WalletCommands::Create => {
            // Persist the keypair seed and register it in config — a wallet
            // that only lives in process memory is unusable for staking.
            let Some(dir) = crate::config_file::ConfigFile::config_dir() else {
                output::print_err("Cannot determine home directory", &config.format);
                return ExitCode::UserError;
            };
            let key_path = dir.join("wallet.key");
            if key_path.exists() {
                output::print_err(
                    &format!(
                        "Wallet already exists at {}. Remove it first to create a new one.",
                        key_path.display()
                    ),
                    &config.format,
                );
                return ExitCode::UserError;
            }
            let _ = std::fs::create_dir_all(&dir);
            let seed: [u8; 32] = rand::random();
            let seed_hex: String = seed.iter().map(|b| format!("{b:02x}")).collect();
            if let Err(e) = std::fs::write(&key_path, seed_hex) {
                output::print_err(&format!("Failed to write wallet key: {e}"), &config.format);
                return ExitCode::UserError;
            }
            let w = match oi_sdk::Wallet::from_path(key_path.to_str().unwrap_or_default()) {
                Ok(w) => w,
                Err(e) => {
                    output::print_err(&format!("Failed to load new wallet: {e}"), &config.format);
                    return ExitCode::UserError;
                }
            };
            let mut file_config = crate::config_file::ConfigFile::load();
            file_config.wallet.path = Some(key_path.to_string_lossy().to_string());
            if let Err(e) = file_config.save() {
                output::print_err(&format!("Failed to update config: {e}"), &config.format);
                return ExitCode::UserError;
            }
            let info = serde_json::json!({
                "action": "wallet_created",
                "address": w.address(),
                "keypair_path": key_path.to_string_lossy(),
            });
            output::print_item(&info, &config.format, |item| {
                println!("Wallet created successfully.");
                println!("  Address: {}", item["address"]);
                println!(
                    "  Keypair: {} (registered in config.toml)",
                    item["keypair_path"]
                );
            });
            ExitCode::Success
        }
        WalletCommands::Balance => {
            let address = match &config.wallet_path {
                Some(path) => match oi_sdk::Wallet::from_path(path) {
                    Ok(w) => w.address().to_string(),
                    Err(e) => {
                        output::print_err(
                            &format!("Failed to load wallet from {path}: {e}"),
                            &config.format,
                        );
                        return ExitCode::UserError;
                    }
                },
                None => "(no wallet configured)".to_string(),
            };
            let info = serde_json::json!({
                "address": address,
                "balance_truth": 0,
                "balance_sol": 0.0,
            });
            output::print_item(&info, &config.format, |item| {
                println!("Wallet: {}", item["address"]);
                println!("  $TRUTH: {}", item["balance_truth"]);
                println!("  SOL:    {}", item["balance_sol"]);
            });
            ExitCode::Success
        }
        WalletCommands::Address => {
            let address = match &config.wallet_path {
                Some(path) => match oi_sdk::Wallet::from_path(path) {
                    Ok(w) => w.address().to_string(),
                    Err(e) => {
                        output::print_err(
                            &format!("Failed to load wallet from {path}: {e}"),
                            &config.format,
                        );
                        return ExitCode::UserError;
                    }
                },
                None => {
                    output::print_err(
                        "No wallet configured. Use -w <path> to specify a keypair.",
                        &config.format,
                    );
                    return ExitCode::UserError;
                }
            };
            output::print_ok(&address, &config.format);
            ExitCode::Success
        }
        WalletCommands::Import { path } => match oi_sdk::Wallet::from_path(&path) {
            Ok(w) => {
                let info = serde_json::json!({
                    "action": "wallet_imported",
                    "path": path,
                    "address": w.address(),
                });
                output::print_item(&info, &config.format, |item| {
                    println!("Wallet imported from {}", item["path"]);
                    println!("  Address: {}", item["address"]);
                });
                ExitCode::Success
            }
            Err(e) => {
                output::print_err(
                    &format!("Failed to import wallet from {path}: {e}"),
                    &config.format,
                );
                ExitCode::UserError
            }
        },
    }
}

// ── Login ───────────────────────────────────────────────────────────────────

pub fn login(
    file_config: &ConfigFile,
    config: &CliConfig,
    email: Option<String>,
    verify_token: Option<String>,
    gateway_url: &str,
) -> ExitCode {
    // If --verify <token> is provided, complete the magic link flow.
    if let Some(token) = verify_token {
        return login_verify(file_config, config, &token, gateway_url);
    }

    // Otherwise, request a new magic link.
    let email = match email {
        Some(e) => e,
        None => {
            if let Some(e) = &file_config.auth.email {
                e.clone()
            } else {
                output::print_err("Email required. Usage: oi login <email>", &config.format);
                return ExitCode::UserError;
            }
        }
    };

    output::print_ok(
        &format!("Requesting magic link for {email}..."),
        &config.format,
    );

    if let Err(e) = oi_sdk::auth::request_magic_link(gateway_url, &email) {
        output::print_err(&format!("Login failed: {e}"), &config.format);
        return e.exit_code();
    }

    output::print_ok(
        "Magic link sent! Check your email and run:\n  oi login --verify <token>",
        &config.format,
    );

    // Save email + gateway so --verify can find them later.
    let mut new_config = file_config.clone();
    new_config.auth.email = Some(email);
    new_config.auth.gateway_url = Some(gateway_url.to_string());
    if let Err(e) = new_config.save() {
        output::print_err(
            &format!("Warning: failed to save config: {e}"),
            &config.format,
        );
    }

    ExitCode::Success
}

/// Complete login by verifying a magic link token and saving the API key.
fn login_verify(
    file_config: &ConfigFile,
    config: &CliConfig,
    token: &str,
    gateway_url: &str,
) -> ExitCode {
    output::print_ok("Verifying token...", &config.format);

    match oi_sdk::auth::verify_magic_link(gateway_url, token) {
        Ok(auth) => {
            let mut new_config = file_config.clone();
            new_config.auth.api_key = Some(auth.api_key);
            new_config.auth.email = Some(auth.user.email.clone());
            new_config.auth.gateway_url = Some(gateway_url.to_string());

            if let Err(e) = new_config.save() {
                output::print_err(
                    &format!("Login succeeded but failed to save credentials: {e}"),
                    &config.format,
                );
                return ExitCode::UserError;
            }

            output::print_ok(
                &format!(
                    "Logged in as {}. Quota: {}/{} queries/day.",
                    auth.user.email,
                    auth.quota.remaining(),
                    auth.quota.max_queries_per_day,
                ),
                &config.format,
            );
            ExitCode::Success
        }
        Err(e) => {
            output::print_err(&format!("Verification failed: {e}"), &config.format);
            e.exit_code()
        }
    }
}

// ── Logout ──────────────────────────────────────────────────────────────────

pub fn logout(config: &CliConfig) -> ExitCode {
    let mut file_config = ConfigFile::load();
    file_config.auth.api_key = None;
    file_config.auth.email = None;

    match file_config.save() {
        Ok(()) => {
            output::print_ok("Logged out. Credentials cleared.", &config.format);
            ExitCode::Success
        }
        Err(e) => {
            output::print_err(&format!("Failed to clear credentials: {e}"), &config.format);
            ExitCode::UserError
        }
    }
}

// ── Verify ──────────────────────────────────────────────────────────────────

pub fn verify(config: &CliConfig, url: &str) -> ExitCode {
    let client = match build_client_mode(config) {
        Ok(c) => c,
        Err(code) => return code,
    };

    match &client {
        ClientMode::Gateway(c) => match c.verify_url(url) {
            Ok(result) => {
                output::print_item(&result, &config.format, |r| {
                    println!("URL:     {}", r.url);
                    println!("Domain:  {}", r.domain);
                    println!("Staked:  {}", r.is_staked);
                    println!("Trust:   {:.2}", r.trust_score);
                    println!("Verdict: {}", r.verdict);
                });
                ExitCode::Success
            }
            Err(e) => {
                output::print_err(&format!("Verify failed: {e}"), &config.format);
                e.exit_code()
            }
        },
        ClientMode::Local(c) => {
            let domain = url
                .split("//")
                .nth(1)
                .unwrap_or(url)
                .split('/')
                .next()
                .unwrap_or(url);

            match c.get_stake(domain) {
                Ok(Some(info)) => {
                    // Same verdict rule as search results — a hardcoded
                    // "trusted" here contradicted the "neutral" shown in
                    // search for the same domain.
                    let verdict = oi_index::ranking::compute_verdict(
                        info.amount_usd,
                        info.reputation,
                        0, // reputation votes: no feedback provider in local mode
                        info.disputed,
                        info.dns_verified,
                    );
                    let result = oi_types::TrustCheckResult {
                        url: url.to_string(),
                        domain: domain.to_string(),
                        is_staked: true,
                        stake_info: Some(info.clone()),
                        trust_score: info.reputation,
                        verdict: verdict.to_string(),
                    };
                    output::print_item(&result, &config.format, |r| {
                        println!("URL:     {}", r.url);
                        println!("Domain:  {}", r.domain);
                        println!("Staked:  {}", r.is_staked);
                        println!("Trust:   {:.2}", r.trust_score);
                        println!("Verdict: {}", r.verdict);
                    });
                    ExitCode::Success
                }
                Ok(None) => {
                    let result = oi_types::TrustCheckResult {
                        url: url.to_string(),
                        domain: domain.to_string(),
                        is_staked: false,
                        stake_info: None,
                        trust_score: 0.0,
                        verdict: "unverified".to_string(),
                    };
                    output::print_item(&result, &config.format, |r| {
                        println!("URL:     {}", r.url);
                        println!("Domain:  {}", r.domain);
                        println!("Staked:  {}", r.is_staked);
                        println!("Verdict: {}", r.verdict);
                    });
                    ExitCode::Success
                }
                Err(e) => {
                    output::print_err(&format!("Verify failed: {e}"), &config.format);
                    e.exit_code()
                }
            }
        }
    }
}

// ── Status ──────────────────────────────────────────────────────────────────

pub fn status(config: &CliConfig) -> ExitCode {
    let mode = if config.wallet_path.is_some() {
        "web3 (wallet)"
    } else if config.api_key.is_some() {
        "web2 (gateway)"
    } else {
        "not configured"
    };

    let file_config = ConfigFile::load();

    let info = serde_json::json!({
        "mode": mode,
        "email": file_config.auth.email,
        "wallet": config.wallet_path,
        "gateway_url": config.gateway_url,
        "network": config.network,
        "has_api_key": config.api_key.is_some(),
    });
    output::print_item(&info, &config.format, |item| {
        println!("OpenSonarX Status");
        println!("  Mode:    {}", item["mode"]);
        if let Some(email) = item["email"].as_str() {
            println!("  Email:   {email}");
        }
        if let Some(wallet) = item["wallet"].as_str() {
            println!("  Wallet:  {wallet}");
        }
        if let Some(gw) = item["gateway_url"].as_str() {
            println!("  Gateway: {gw}");
        }
        println!("  Network: {}", item["network"]);
        println!(
            "  API Key: {}",
            if item["has_api_key"].as_bool().unwrap_or(false) {
                "configured"
            } else {
                "none"
            }
        );
    });

    if let Some(api_key) = &config.api_key {
        let base_url = config
            .gateway_url
            .clone()
            .unwrap_or_else(|| "https://api.opensonarx.com".to_string());
        let client =
            GatewayClient::new(base_url, Some(api_key.clone()), config.timeout_secs * 1000);
        match client.quota() {
            Ok(quota) => {
                println!(
                    "  Quota:   {}/{} queries today",
                    quota.queries_today, quota.max_queries_per_day
                );
            }
            Err(_) => {
                println!("  Quota:   (unable to fetch)");
            }
        }
    }

    ExitCode::Success
}

// ── Docs ────────────────────────────────────────────────────────────────────

pub fn docs(config: &CliConfig) -> ExitCode {
    let base_url = config
        .gateway_url
        .clone()
        .unwrap_or_else(|| "https://api.opensonarx.com".to_string());
    let docs_url = format!("{base_url}/v1/docs/html");
    output::print_ok(
        &format!("OpenSonarX Gateway API docs: {docs_url}"),
        &config.format,
    );
    ExitCode::Success
}

// ── Leaderboard ─────────────────────────────────────────────────────────

pub fn leaderboard(config: &CliConfig, limit: u32) -> ExitCode {
    let client = match build_client_mode(config) {
        Ok(c) => c,
        Err(code) => return code,
    };

    let result = match &client {
        ClientMode::Local(c) => c.top_queried(limit as usize),
        ClientMode::Gateway(c) => c.leaderboard(limit as usize),
    };

    match result {
        Ok(entries) => {
            if entries.is_empty() {
                output::print_ok(
                    "No leaderboard data yet. Run some searches first.",
                    &config.format,
                );
            } else {
                output::print_list(
                    &entries,
                    &config.format,
                    &["RANK", "IDENTIFIER", "QUERIES", "STAKE", "TYPE"],
                    |e| {
                        let entry_type = if e.is_entity {
                            match &e.content_kind {
                                Some(kind) => format!("entity ({kind})"),
                                None => "entity".to_string(),
                            }
                        } else {
                            "domain".to_string()
                        };
                        vec![
                            String::new(), // rank filled by print_list position
                            e.identifier.clone(),
                            e.query_count.to_string(),
                            if e.stake_truth > 0 {
                                format!("{} (${:.2})", e.stake_truth, e.stake_usd)
                            } else {
                                "none".to_string()
                            },
                            entry_type,
                        ]
                    },
                );
            }
            ExitCode::Success
        }
        Err(e) => {
            output::print_err(&format!("Failed to fetch leaderboard: {e}"), &config.format);
            e.exit_code()
        }
    }
}

pub fn sentinel(config: &CliConfig, action: SentinelCommands) -> ExitCode {
    match action {
        SentinelCommands::Start {
            concurrency,
            domains,
            feeds,
            poll_secs,
        } => {
            if concurrency == 0 {
                output::print_err("Concurrency must be at least 1", &config.format);
                return ExitCode::UserError;
            }

            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                use oi_crawl::daemon::{CrawlDaemon, CrawlEvent, DaemonConfig, MonitoredDomain};
                use std::sync::Arc;

                let daemon_config = DaemonConfig {
                    concurrency,
                    feed_poll_interval: std::time::Duration::from_secs(poll_secs.max(30)),
                    ..Default::default()
                };

                let (daemon, mut event_rx) = CrawlDaemon::new(daemon_config);
                let daemon = Arc::new(daemon);

                // Add initial domains
                if let Some(domains) = domains {
                    let monitored: Vec<MonitoredDomain> = domains
                        .iter()
                        .map(|d| MonitoredDomain {
                            domain: d.clone(),
                            stake_amount: 0,
                            feed_urls: Vec::new(),
                            hub_urls: Vec::new(),
                        })
                        .collect();
                    daemon.add_domains(monitored).await;
                }

                // Direct feed subscriptions: one monitored entry per feed,
                // domain derived from the feed URL. No hub URLs — the feed
                // itself is the discovery surface.
                if let Some(feeds) = feeds {
                    let monitored: Vec<MonitoredDomain> = feeds
                        .iter()
                        .map(|f| MonitoredDomain {
                            domain: f
                                .strip_prefix("https://")
                                .or_else(|| f.strip_prefix("http://"))
                                .and_then(|rest| rest.split('/').next())
                                .unwrap_or(f)
                                .to_string(),
                            stake_amount: 0,
                            feed_urls: vec![f.clone()],
                            hub_urls: vec![f.clone()],
                        })
                        .collect();
                    for m in &monitored {
                        println!("  Feed:      {} ({})", m.feed_urls[0], m.domain);
                    }
                    daemon.add_domains(monitored).await;
                }

                // Create local index for ingesting crawled content
                let index_path = crate::config_file::ConfigFile::index_dir().expect("home dir");
                let sdk_config = SdkConfig {
                    local_index_path: Some(index_path.to_str().unwrap().to_string()),
                    ..Default::default()
                };
                let client = Arc::new(OpenSonarXClient::new(sdk_config));

                println!("Sentinel crawl daemon starting...");
                println!("  Concurrency: {concurrency}");
                println!("  Network:     {}", config.network);
                println!("  Index:       {}", index_path.display());

                // Spawn event handler: embed chunks and ingest into local index
                let client_for_events = client.clone();
                tokio::spawn(async move {
                    while let Some(event) = event_rx.recv().await {
                        let (url, domain, chunk_count, mut chunks) = match event {
                            CrawlEvent::ContentDiscovered {
                                url,
                                domain,
                                chunk_count,
                                chunks,
                                ..
                            } => (url, domain, chunk_count, chunks),
                            CrawlEvent::ContentUpdated {
                                url,
                                domain,
                                chunk_count,
                                chunks,
                                ..
                            } => (url, domain, chunk_count, chunks),
                        };

                        // Embed each chunk that doesn't already have an embedding
                        for chunk in &mut chunks {
                            if chunk.embedding.is_none()
                                && let Ok(emb) =
                                    client_for_events.embedder().embed_text(&chunk.content)
                            {
                                chunk.embedding = Some(emb);
                            }
                        }

                        // Ingest into local index and persist immediately —
                        // sentinel content must survive the process (and be
                        // visible to searches in other processes).
                        match client_for_events.ingest(&chunks) {
                            Ok(()) => {
                                println!("[INDEXED] {} ({}) — {} chunks", url, domain, chunk_count);
                                if let Err(e) = client_for_events.save_index() {
                                    println!("[ERROR] Failed to save index: {e}");
                                }
                            }
                            Err(e) => println!("[ERROR] Failed to ingest {}: {}", url, e),
                        }
                    }
                });

                // Run daemon until Ctrl+C
                let (shutdown_tx, shutdown_rx) = tokio::sync::mpsc::channel(1);
                let daemon_handle = tokio::spawn(daemon.clone().run(shutdown_rx));

                tokio::signal::ctrl_c().await.ok();
                println!("\nShutting down...");
                let _ = shutdown_tx.send(()).await;
                let _ = daemon_handle.await;

                let stats = daemon.stats().await;
                match client.save_index() {
                    Ok(_) => println!("Index saved."),
                    Err(e) => println!("[ERROR] Final index save failed: {e}"),
                }
                println!("Final stats:");
                println!("  URLs crawled:   {}", stats.urls_crawled);
                println!("  URLs indexed:   {}", stats.urls_indexed);
                println!("  URLs rejected:  {}", stats.urls_rejected);
                println!("  Errors:         {}", stats.errors);
                println!("  Docs in index:  {}", client.index().doc_count());
            });
            ExitCode::Success
        }
        SentinelCommands::Status => {
            output::print_ok(
                "Sentinel status: not running (start with `oi sentinel start`)",
                &config.format,
            );
            ExitCode::Success
        }
        SentinelCommands::AddFeed { url } => {
            output::print_ok(
                &format!("Feed registered: {url}\nStart the sentinel daemon to begin monitoring."),
                &config.format,
            );
            ExitCode::Success
        }
        SentinelCommands::AddDomain { domain, stake } => {
            output::print_ok(
                &format!(
                    "Domain registered: {domain} (stake hint: {stake})\nStart the sentinel daemon to begin monitoring."
                ),
                &config.format,
            );
            ExitCode::Success
        }
    }
}

// ── Seed ────────────────────────────────────────────────────────────────────

pub fn seed(config: &CliConfig, action: SeedCommands, verbose: bool) -> ExitCode {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(crate::seed::run_seed(config, action, verbose))
}

// ── Index import/export ─────────────────────────────────────────────────────

pub fn index(config: &CliConfig, action: IndexCommands) -> ExitCode {
    let index_path = crate::config_file::ConfigFile::index_dir().expect("home dir");

    match action {
        IndexCommands::Export {
            output: output_path,
            shard,
        } => {
            let storage = match oi_index::Storage::new(&index_path) {
                Ok(s) if s.segment_exists("hnsw.bin") => s,
                _ => {
                    output::print_err(
                        "No index found. Run a node first to build an index.",
                        &config.format,
                    );
                    return ExitCode::UserError;
                }
            };

            let idx = match oi_index::HybridIndex::load(&storage) {
                Ok(i) => i,
                Err(e) => {
                    output::print_err(&format!("Failed to load index: {e}"), &config.format);
                    return ExitCode::UserError;
                }
            };

            let out_file = output_path.unwrap_or_else(|| "opensonarx-export.tar.gz".to_string());

            // Get chunks for export
            let shard_filter = shard.as_deref();
            let chunks = match idx.get_chunks_for_replication(shard_filter, usize::MAX) {
                Ok(c) => c,
                Err(e) => {
                    output::print_err(&format!("Failed to read chunks: {e}"), &config.format);
                    return ExitCode::UserError;
                }
            };

            if chunks.is_empty() {
                output::print_err("No chunks to export.", &config.format);
                return ExitCode::UserError;
            }

            // Serialize chunks to JSON, then write a tar.gz archive
            let json = match serde_json::to_vec(&chunks) {
                Ok(j) => j,
                Err(e) => {
                    output::print_err(&format!("Serialization failed: {e}"), &config.format);
                    return ExitCode::UserError;
                }
            };

            let file = match std::fs::File::create(&out_file) {
                Ok(f) => f,
                Err(e) => {
                    output::print_err(&format!("Failed to create {out_file}: {e}"), &config.format);
                    return ExitCode::UserError;
                }
            };

            let enc = flate2::write::GzEncoder::new(file, flate2::Compression::default());
            let mut archive = tar::Builder::new(enc);

            let mut header = tar::Header::new_gnu();
            header.set_size(json.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            if let Err(e) = archive.append_data(&mut header, "chunks.json", &json[..]) {
                output::print_err(&format!("Archive write failed: {e}"), &config.format);
                return ExitCode::UserError;
            }

            if let Err(e) = archive.finish() {
                output::print_err(&format!("Archive finalize failed: {e}"), &config.format);
                return ExitCode::UserError;
            }

            let file_size = std::fs::metadata(&out_file).map(|m| m.len()).unwrap_or(0);
            println!(
                "Exported {} chunks to {} ({:.1} MB)",
                chunks.len(),
                out_file,
                file_size as f64 / 1_048_576.0
            );

            // Collect shard distribution
            let mut shard_counts: std::collections::HashMap<String, usize> =
                std::collections::HashMap::new();
            for chunk in &chunks {
                let sid = chunk
                    .meta
                    .as_ref()
                    .and_then(|m| m.shard_id.as_deref())
                    .unwrap_or("unclassified");
                *shard_counts.entry(sid.to_string()).or_default() += 1;
            }
            for (sid, count) in &shard_counts {
                println!("  {sid}: {count} chunks");
            }

            ExitCode::Success
        }

        IndexCommands::Import { path, merge } => {
            // Open and decompress the archive
            let file = match std::fs::File::open(&path) {
                Ok(f) => f,
                Err(e) => {
                    output::print_err(&format!("Failed to open {path}: {e}"), &config.format);
                    return ExitCode::UserError;
                }
            };

            let dec = flate2::read::GzDecoder::new(file);
            let mut archive = tar::Archive::new(dec);

            // Find chunks.json in the archive
            let mut chunks_json: Option<Vec<u8>> = None;
            if let Ok(entries) = archive.entries() {
                for entry in entries.flatten() {
                    if let Ok(path) = entry.path()
                        && path.to_str() == Some("chunks.json")
                    {
                        let data: Vec<u8> =
                            std::io::Read::bytes(entry).filter_map(|b| b.ok()).collect();
                        chunks_json = Some(data);
                        break;
                    }
                }
            }

            let json_data = match chunks_json {
                Some(d) => d,
                None => {
                    output::print_err("Archive does not contain chunks.json", &config.format);
                    return ExitCode::UserError;
                }
            };

            let chunks: Vec<oi_types::ContentChunk> = match serde_json::from_slice(&json_data) {
                Ok(c) => c,
                Err(e) => {
                    output::print_err(&format!("Failed to parse chunks.json: {e}"), &config.format);
                    return ExitCode::UserError;
                }
            };

            println!("Read {} chunks from {path}", chunks.len());

            // Load or create the target index
            let dim = 384;
            let _ = std::fs::create_dir_all(&index_path);
            let storage = match oi_index::Storage::new(&index_path) {
                Ok(s) => s,
                Err(e) => {
                    output::print_err(&format!("Failed to open storage: {e}"), &config.format);
                    return ExitCode::UserError;
                }
            };

            let idx = if merge && storage.segment_exists("hnsw.bin") {
                match oi_index::HybridIndex::load(&storage) {
                    Ok(i) => {
                        println!("  Merging into existing index ({} docs)", i.doc_count());
                        i
                    }
                    Err(e) => {
                        println!("  Failed to load existing index ({e}), starting fresh");
                        oi_index::HybridIndex::new(dim)
                    }
                }
            } else {
                println!("  Creating new index");
                oi_index::HybridIndex::new(dim)
            };

            let before = idx.doc_count();
            if let Err(e) = idx.ingest(&chunks) {
                output::print_err(&format!("Ingest failed: {e}"), &config.format);
                return ExitCode::UserError;
            }

            let added = idx.doc_count() - before;
            println!(
                "  Ingested {added} new chunks ({} duplicates skipped)",
                chunks.len() - added
            );

            if let Err(e) = idx.save(&storage) {
                output::print_err(&format!("Failed to save index: {e}"), &config.format);
                return ExitCode::UserError;
            }

            println!("  Index saved ({} total docs)", idx.doc_count());
            ExitCode::Success
        }

        IndexCommands::Info => {
            let storage = match oi_index::Storage::new(&index_path) {
                Ok(s) if s.segment_exists("hnsw.bin") => s,
                _ => {
                    output::print_err("No index found at ~/.opensonarx/index/", &config.format);
                    return ExitCode::UserError;
                }
            };

            let idx = match oi_index::HybridIndex::load(&storage) {
                Ok(i) => i,
                Err(e) => {
                    output::print_err(&format!("Failed to load index: {e}"), &config.format);
                    return ExitCode::UserError;
                }
            };

            let table_mode = matches!(config.format, crate::output::Format::Table);
            if table_mode {
                println!("Index info:");
                println!("  Path:       {}", index_path.display());
                println!("  Documents:  {}", idx.doc_count());
                println!("  Seq:        {}", idx.current_seq());
            }

            // Disk size
            let disk_size: u64 = std::fs::read_dir(&index_path)
                .map(|entries| {
                    entries
                        .filter_map(|e| e.ok())
                        .filter_map(|e| e.metadata().ok().map(|m| m.len()))
                        .sum()
                })
                .unwrap_or(0);
            if table_mode {
                println!("  Disk size:  {:.1} MB", disk_size as f64 / 1_048_576.0);
            }

            // Shard distribution via chunk replication
            if table_mode && let Ok(chunks) = idx.get_chunks_for_replication(None, usize::MAX) {
                let mut shard_counts: std::collections::HashMap<String, usize> =
                    std::collections::HashMap::new();
                for chunk in &chunks {
                    let sid = chunk
                        .meta
                        .as_ref()
                        .and_then(|m| m.shard_id.as_deref())
                        .unwrap_or("unclassified");
                    *shard_counts.entry(sid.to_string()).or_default() += 1;
                }
                if !shard_counts.is_empty() {
                    println!("  Shards:");
                    let mut sorted: Vec<_> = shard_counts.into_iter().collect();
                    sorted.sort_by(|a, b| b.1.cmp(&a.1));
                    for (sid, count) in sorted {
                        println!("    {sid}: {count}");
                    }
                }
            }

            if table_mode && !idx.embedder_name().is_empty() {
                println!("  Embedder:   {}", idx.embedder_name());
            }

            // Health audit: cross-store consistency + content quality
            let a = idx.audit();
            let consistent = a.hnsw_count == a.bm25_count
                && a.bm25_count == a.meta_count
                && a.meta_without_bm25 == 0;
            let mut domains: Vec<_> = a.per_domain.into_iter().collect();
            domains.sort_by(|x, y| y.1.cmp(&x.1));

            if matches!(
                config.format,
                crate::output::Format::Json | crate::output::Format::Ndjson
            ) {
                // Machine-readable health for agents/monitoring.
                let info = serde_json::json!({
                    "path": index_path.to_string_lossy(),
                    "documents": idx.doc_count(),
                    "seq": idx.current_seq(),
                    "embedder": idx.embedder_name(),
                    "health": {
                        "consistent": consistent,
                        "hnsw": a.hnsw_count,
                        "bm25": a.bm25_count,
                        "metadata": a.meta_count,
                        "avg_quality": a.avg_quality,
                        "missing_title": a.missing_title,
                        "missing_url": a.missing_url,
                        "orphan_dedup_hashes": a.orphan_hashes,
                    },
                    "top_domains": domains.iter().take(8)
                        .map(|(d, c)| serde_json::json!({"domain": d, "chunks": c}))
                        .collect::<Vec<_>>(),
                });
                println!(
                    "{}",
                    serde_json::to_string_pretty(&info).unwrap_or_default()
                );
                return ExitCode::Success;
            }

            println!("  Health:");
            println!(
                "    hnsw/bm25/meta: {}/{}/{}{}",
                a.hnsw_count,
                a.bm25_count,
                a.meta_count,
                if consistent {
                    " (consistent)"
                } else {
                    " (INCONSISTENT!)"
                }
            );
            println!("    avg quality:    {:.3}", a.avg_quality);
            println!(
                "    missing title/url: {}/{}",
                a.missing_title, a.missing_url
            );
            println!("    orphan dedup hashes: {}", a.orphan_hashes);
            println!("  Top domains:");
            for (d, c) in domains.into_iter().take(8) {
                println!("    {c:>5}  {d}");
            }

            ExitCode::Success
        }
        IndexCommands::Reembed { batch } => {
            let storage = match oi_index::Storage::new(&index_path) {
                Ok(s) if s.segment_exists("hnsw.bin") => s,
                _ => {
                    output::print_err("No index found at ~/.opensonarx/index/", &config.format);
                    return ExitCode::UserError;
                }
            };
            let old = match oi_index::HybridIndex::load(&storage) {
                Ok(i) => i,
                Err(e) => {
                    output::print_err(&format!("Failed to load index: {e}"), &config.format);
                    return ExitCode::UserError;
                }
            };
            let embedder = oi_sdk::OpenSonarXClient::default_embedder();
            println!(
                "Re-embedding {} docs: '{}' -> '{}'",
                old.doc_count(),
                if old.embedder_name().is_empty() {
                    "(unrecorded)"
                } else {
                    old.embedder_name()
                },
                embedder.name()
            );

            let chunks = match old.chunks_since(None, 0, usize::MAX) {
                Ok(c) => c,
                Err(e) => {
                    output::print_err(&format!("Failed to read chunks: {e}"), &config.format);
                    return ExitCode::UserError;
                }
            };

            let fresh_config = oi_index::hybrid::HybridConfig {
                embedder_name: embedder.name().to_string(),
                ..Default::default()
            };
            let new_index = oi_index::HybridIndex::with_config(embedder.dimension(), fresh_config);

            use oi_types::traits::SearchIndex as _;
            let start = std::time::Instant::now();
            let batch_size = batch.max(1);
            let mut done = 0usize;
            for group in chunks.chunks(batch_size) {
                let texts: Vec<&str> = group.iter().map(|c| c.content.as_str()).collect();
                let embeddings = match embedder.embed_text_batch(&texts) {
                    Ok(e) => e,
                    Err(e) => {
                        output::print_err(&format!("Embedding failed: {e}"), &config.format);
                        return ExitCode::UserError;
                    }
                };
                let reembedded: Vec<oi_types::ContentChunk> = group
                    .iter()
                    .zip(embeddings)
                    .map(|(c, emb)| oi_types::ContentChunk {
                        chunk_id: c.chunk_id.clone(),
                        content: c.content.clone(),
                        embedding: Some(emb),
                        seq: 0,
                        meta: c.meta.clone(),
                    })
                    .collect();
                if let Err(e) = new_index.ingest(&reembedded) {
                    output::print_err(&format!("Ingest failed: {e}"), &config.format);
                    return ExitCode::UserError;
                }
                done += group.len();
                if done % 5000 < batch_size {
                    let rate = done as f64 / start.elapsed().as_secs_f64().max(0.001);
                    println!("  {done} re-embedded ({rate:.0} docs/s)");
                }
            }

            // Old index stays intact on disk until this save succeeds.
            if let Err(e) = new_index.save(&storage) {
                output::print_err(
                    &format!("Failed to save re-embedded index: {e}"),
                    &config.format,
                );
                return ExitCode::UserError;
            }
            println!(
                "Done: {} docs re-embedded with '{}' in {:.1}s (replication seqs reset)",
                new_index.doc_count(),
                embedder.name(),
                start.elapsed().as_secs_f64()
            );
            ExitCode::Success
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shards_of_a_routing_are_given_by_number_or_by_name() {
        let routing = oi_types::shard::LearnedRouting::from_centroids(
            "test-embedder",
            vec![vec![1.0, 0.0], vec![0.0, 1.0], vec![0.6, 0.8]],
        );
        let named = routing.shard_id(2);
        assert_eq!(
            learned_shard_ids(&routing, &["0".to_string(), named.clone()]).unwrap(),
            [routing.shard_id(0), named]
        );
        // A cluster the routing does not have, or a shard of another routing.
        assert!(learned_shard_ids(&routing, &["3".to_string()]).is_err());
        assert!(learned_shard_ids(&routing, &["learned-000000000000-0".to_string()]).is_err());
    }
}
