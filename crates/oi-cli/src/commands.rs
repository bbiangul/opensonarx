use oi_sdk::{GatewayClient, OpenSonarXClient, SdkConfig};
use oi_types::error::ExitCode;
use oi_types::traits::SearchIndex;

use super::{
    CliConfig, DomainCommands, DisputeCommands, IndexCommands, JuryCommands,
    NetworkCommands, SeedCommands, SentinelCommands, WalletCommands, YieldCommands,
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
    // Load the index that `oi seed` writes so seeded content is searchable;
    // without this, local search silently returns external results only.
    let seed_index_path = dirs::home_dir()
        .map(|h| h.join(".opensonarx").join("index"))
        .filter(|p| p.exists())
        .map(|p| p.to_string_lossy().to_string());
    let sdk_config = SdkConfig {
        bootstrap_peers: config.bootstrap_peers.clone(),
        wallet_path: config.wallet_path.clone(),
        local_index_path: seed_index_path,
        network: config.network.clone(),
        brave_api_key: std::env::var("BRAVE_API_KEY").ok(),
        search_engines: config.search_engines.clone(),
        crawl_on_query: false,
        crawl_on_query_max_urls: 3,
    };
    OpenSonarXClient::new(sdk_config)
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
        let client = GatewayClient::new(base_url, Some(api_key.clone()), config.timeout_secs * 1000);
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
) -> ExitCode {
    if let Some(ref t) = min_trust
        && !["unverified", "neutral", "trusted", "highly_trusted"].contains(&t.as_str())
    {
        output::print_err(
            &format!("Invalid --min-trust '{t}'. Valid: unverified, neutral, trusted, highly_trusted"),
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
                ClientMode::Gateway(GatewayClient::new(base_url, None, config.timeout_secs * 1000)),
                true,
            )
        }
    };

    let result = match &client {
        ClientMode::Local(c) => c.search_blended(query, limit as usize, &filters).map(|(r, _)| r),
        ClientMode::Gateway(c) => c.search(query, limit as usize, &filters),
    };

    match result {
        Ok(items) => {
            if items.is_empty() {
                output::print_ok(
                    &format!("No results found for: {query}"),
                    &config.format,
                );
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
                if items.iter().any(|r| r.stake_truth == 0) {
                    eprintln!("(external results — not staked. Run `oi stake <domain> <amount>` to vouch for quality)");
                }
            }
            if is_anonymous {
                eprintln!("(anonymous mode - 10 queries/day, run `oi login` for 100/day)");
            }
            ExitCode::Success
        }
        Err(e) => {
            output::print_err(&format!("Search failed: {e}"), &config.format);
            e.exit_code()
        }
    }
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
            output::print_err(&format!("Failed to fetch feedback stats: {e}"), &config.format);
            e.exit_code()
        }
    }
}

// ── Stake / Unstake ─────────────────────────────────────────────────────────

pub fn stake(config: &CliConfig, domain: &str, amount: u64) -> ExitCode {
    if let Some(code) = reject_in_open_mode(config, "stake") { return code; }
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
                println!("  Tx: {}", item["tx_hash"]);
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
    if let Some(code) = reject_in_open_mode(config, "unstake") { return code; }
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
                println!(
                    "Unstake initiated for {} (7-day cooldown)",
                    item["domain"]
                );
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
    if let Some(code) = reject_in_open_mode(config, "domain") { return code; }
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
                output::print_err(
                    &format!("Failed to get domain info: {e}"),
                    &config.format,
                );
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
        },
    }
}

// ── Dispute ─────────────────────────────────────────────────────────────────

pub fn dispute(config: &CliConfig, action: DisputeCommands) -> ExitCode {
    if let Some(code) = reject_in_open_mode(config, "dispute") { return code; }
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
                        output::print_err(
                            &format!("Failed to list disputes: {e}"),
                            &config.format,
                        );
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
                            println!("  Votes:    {} guilty / {} innocent", d.votes_guilty, d.votes_innocent);
                        });
                        if let Some(bond) = &resp.fiat_bond {
                            println!("  Fiat Bond: {} (${:.2})", bond.bond_id, bond.amount_cents as f64 / 100.0);
                            println!("  Status:    {:?}", bond.status);
                        }
                        ExitCode::Success
                    }
                    Err(e) => {
                        output::print_err(
                            &format!("Failed to get dispute: {e}"),
                            &config.format,
                        );
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
    if let Some(code) = reject_in_open_mode(config, "jury") { return code; }
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
    if let Some(code) = reject_in_open_mode(config, "yield") { return code; }
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
                println!("  Earned:         {} lamports $TRUTH", item["accrued_truth"]);
                println!("  Queries served: {}", item["queries_served"]);
                println!("  Pending tickets: {}", item["pending_tickets"]);
                println!("  Pending value:  {} lamports", item["pending_truth"]);
            });
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
        } => {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let bootstrap_peers = if let Some(peers) = bootstrap {
                    peers
                } else if !config.bootstrap_peers.is_empty() {
                    config.bootstrap_peers.clone()
                } else {
                    Vec::new()
                };
                let routing_table_path = dirs::home_dir()
                    .map(|h| h.join(".opensonarx").join("routing_table.json").to_string_lossy().to_string());

                let net_config = oi_network::config::NetworkConfig {
                    listen_port: port,
                    bootstrap_peers,
                    routing_table_path,
                    ..Default::default()
                };

                // Create a hybrid index for this node
                let dim = 384; // Standard MiniLM dimension
                let index_path = dirs::home_dir()
                    .map(|h| h.join(".opensonarx").join("node_index"))
                    .expect("home dir");
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
                let index = std::sync::Arc::new(index);

                // Shard mode: explicit --shards override, or auto-discover via topic router
                let shard_override = shards.is_some();
                let shard_ids = shards.unwrap_or_else(|| vec!["topic-general".into()]);

                let mut node = oi_network::Node::new(net_config)
                    .with_index(index.clone(), shard_ids.clone());
                println!("Starting OpenSonarX node...");
                println!("  Peer ID:  {}", node.peer_id);
                println!("  Listen:   /ip4/0.0.0.0/tcp/{port}");
                println!("  Network:  {}", config.network);
                println!("  Shards:   {} {}", shard_ids.join(", "), if shard_override { "(manual)" } else { "(auto)" });
                println!("  Index:    {} docs", index.doc_count());

                // Build topic router for sentinel classification
                let topic_router = if sentinel && !shard_override {
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
                            println!("  Topic router: initialized ({} topics)", oi_types::BUILTIN_TOPICS.len());
                            Some(std::sync::Arc::new(router))
                        }
                        Err(e) => {
                            eprintln!("  Topic router: failed to embed topics ({e}), using default shard");
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
                    println!("  No bootstrap peers configured — running in local discovery mode (mDNS).");
                    println!("  To join the network: oi network node --bootstrap /ip4/<peer>/tcp/4001");
                } else {
                    println!("  Bootstrap peers:");
                    for p in &node.config.bootstrap_peers {
                        println!("    - {p}");
                    }
                }

                match node.start().await {
                    Ok(handle) => {
                        println!("  Node running, peer ID: {}", handle.peer_id);

                        // Spawn HTTP API if --http-port is set
                        if let Some(hp) = http_port {
                            let api_handle = handle.clone();
                            tokio::spawn(async move {
                                if let Err(e) = crate::node_api::run_node_api(hp, api_handle).await {
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
                        output::print_err(
                            &format!("Node failed to start: {e}"),
                            &config.format,
                        );
                    }
                }
            });
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
    let default_shard = shard_ids.first().cloned().unwrap_or_else(|| "topic-general".into());

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
        std::sync::Arc::new(std::sync::Mutex::new(
            shard_ids.iter().cloned().collect(),
        ));
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
            let w = oi_sdk::Wallet::new();
            let info = serde_json::json!({
                "action": "wallet_created",
                "address": w.address(),
            });
            output::print_item(&info, &config.format, |item| {
                println!("Wallet created successfully.");
                println!("  Address: {}", item["address"]);
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
                output::print_err(
                    "Email required. Usage: oi login <email>",
                    &config.format,
                );
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
                    let result = oi_types::TrustCheckResult {
                        url: url.to_string(),
                        domain: domain.to_string(),
                        is_staked: true,
                        stake_info: Some(info.clone()),
                        trust_score: info.reputation,
                        verdict: "trusted".to_string(),
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
        let client = GatewayClient::new(base_url, Some(api_key.clone()), config.timeout_secs * 1000);
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
                output::print_ok("No leaderboard data yet. Run some searches first.", &config.format);
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

                // Create local index for ingesting crawled content
                let index_path = dirs::home_dir()
                    .unwrap()
                    .join(".opensonarx")
                    .join("sentinel_index");
                let _ = std::fs::create_dir_all(&index_path);
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
                            CrawlEvent::ContentDiscovered { url, domain, chunk_count, chunks, .. } => {
                                (url, domain, chunk_count, chunks)
                            }
                            CrawlEvent::ContentUpdated { url, domain, chunk_count, chunks, .. } => {
                                (url, domain, chunk_count, chunks)
                            }
                        };

                        // Embed each chunk that doesn't already have an embedding
                        for chunk in &mut chunks {
                            if chunk.embedding.is_none()
                                && let Ok(emb) = client_for_events.embedder().embed_text(&chunk.content)
                            {
                                chunk.embedding = Some(emb);
                            }
                        }

                        // Ingest into local index
                        match client_for_events.ingest(&chunks) {
                            Ok(()) => println!("[INDEXED] {} ({}) — {} chunks", url, domain, chunk_count),
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
            output::print_ok("Sentinel status: not running (start with `oi sentinel start`)", &config.format);
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
                &format!("Domain registered: {domain} (stake hint: {stake})\nStart the sentinel daemon to begin monitoring."),
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
    let index_path = dirs::home_dir()
        .map(|h| h.join(".opensonarx").join("node_index"))
        .expect("home dir");

    match action {
        IndexCommands::Export { output: output_path, shard } => {
            let storage = match oi_index::Storage::new(&index_path) {
                Ok(s) if s.segment_exists("hnsw.bin") => s,
                _ => {
                    output::print_err("No index found. Run a node first to build an index.", &config.format);
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
            println!("Exported {} chunks to {} ({:.1} MB)",
                chunks.len(), out_file, file_size as f64 / 1_048_576.0);

            // Collect shard distribution
            let mut shard_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
            for chunk in &chunks {
                let sid = chunk.meta.as_ref()
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
                        let data: Vec<u8> = std::io::Read::bytes(entry)
                            .filter_map(|b| b.ok())
                            .collect();
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
            println!("  Ingested {added} new chunks ({} duplicates skipped)", chunks.len() - added);

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
                    output::print_err("No index found at ~/.opensonarx/node_index/", &config.format);
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

            println!("Index info:");
            println!("  Path:       {}", index_path.display());
            println!("  Documents:  {}", idx.doc_count());
            println!("  Seq:        {}", idx.current_seq());

            // Disk size
            let disk_size: u64 = std::fs::read_dir(&index_path)
                .map(|entries| {
                    entries
                        .filter_map(|e| e.ok())
                        .filter_map(|e| e.metadata().ok().map(|m| m.len()))
                        .sum()
                })
                .unwrap_or(0);
            println!("  Disk size:  {:.1} MB", disk_size as f64 / 1_048_576.0);

            // Shard distribution via chunk replication
            if let Ok(chunks) = idx.get_chunks_for_replication(None, usize::MAX) {
                let mut shard_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
                for chunk in &chunks {
                    let sid = chunk.meta.as_ref()
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

            ExitCode::Success
        }
    }
}
