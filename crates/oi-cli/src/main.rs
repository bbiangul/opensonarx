mod commands;
mod config_file;
mod node_api;
mod output;
mod seed;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "oi",
    about = "OpenSonarX CLI -- decentralized search protocol",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Output format: json, table, ndjson
    #[arg(short = 'o', long = "output", default_value = "table", global = true)]
    format: OutputFormat,

    /// Wallet keypair path
    #[arg(short = 'w', long = "wallet", global = true)]
    wallet: Option<String>,

    /// Network: mainnet, devnet, localnet
    #[arg(
        short = 'n',
        long = "network",
        default_value = "mainnet",
        global = true
    )]
    network: String,

    /// Solana RPC URL override
    #[arg(long = "rpc", global = true)]
    rpc: Option<String>,

    /// Bootstrap peer multiaddrs
    #[arg(long = "peers", global = true)]
    peers: Option<Vec<String>>,

    /// Request timeout in seconds
    #[arg(short = 't', long = "timeout", default_value = "30", global = true)]
    timeout: u64,

    /// Suppress non-essential output
    #[arg(short = 'q', long = "quiet", global = true)]
    quiet: bool,

    /// Verbose logging
    #[arg(short = 'v', long = "verbose", global = true)]
    verbose: bool,

    /// Skip confirmation prompts
    #[arg(long = "yes", global = true)]
    yes: bool,

    /// Open mode: disable staking/settlement (crypto-optional)
    #[arg(long = "open", global = true)]
    open: bool,
}

#[derive(Clone, Debug, clap::ValueEnum)]
pub enum OutputFormat {
    Table,
    Json,
    Ndjson,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Search the OpenSonarX network
    Search {
        /// Search query text
        query: String,
        /// Maximum results to return
        #[arg(short = 'k', long, default_value = "5")]
        limit: u32,
        /// Minimum stake in USD
        #[arg(long)]
        min_stake: Option<u32>,
        /// Freshness filter: 1h, 1d, 1w, 1m, 1y
        #[arg(long)]
        freshness: Option<String>,
        /// Content type filter: text, image, pdf
        #[arg(long = "type")]
        types: Option<Vec<String>>,
        /// Force specific shard(s)
        #[arg(long)]
        shard: Option<Vec<String>>,
        /// Filter by entity (e.g., youtube.com/c/3blue1brown)
        #[arg(long)]
        entity: Option<Vec<String>>,
        /// Filter by content kind: video, post, comment
        #[arg(long)]
        content_kind: Option<Vec<String>>,
        /// Locale hint for external engines (e.g., en, pt-BR)
        #[arg(long)]
        locale: Option<String>,
        /// Country/region code for geo-scoped search (e.g., us, br)
        #[arg(long)]
        gl: Option<String>,
        /// Domain scope filter (e.g., nytimes.com)
        #[arg(long)]
        site: Option<String>,
        /// Minimum trust verdict: unverified, neutral, trusted, highly_trusted
        /// (filters by skin-in-the-game; never affects ranking order)
        #[arg(long = "min-trust")]
        min_trust: Option<String>,
        /// Query a running P2P node's HTTP API (e.g., http://localhost:8001)
        #[arg(long)]
        node: Option<String>,
        /// Print a concise execution path and phase latencies to stderr
        #[arg(long)]
        explain: bool,
        /// Answer from retained evidence using the model in OI_ANSWER_MODEL; without one, returns the slate
        #[arg(long)]
        answer: bool,
    },
    /// Submit feedback on a search result
    Feedback {
        /// Result ID to give feedback on
        result_id: String,
        /// Whether the result was useful
        #[arg(long)]
        useful: bool,
        /// Original query that produced the result (improves data moat)
        #[arg(long)]
        query: Option<String>,
        /// URL of the result
        #[arg(long)]
        result_url: Option<String>,
        /// Rank position of the result (0-indexed)
        #[arg(long)]
        result_rank: Option<u32>,
    },
    /// Show aggregate feedback statistics (data moat metrics)
    FeedbackStats,
    /// Stake $TRUTH tokens for a domain
    Stake {
        /// Domain to stake for
        domain: String,
        /// Amount in $TRUTH
        #[arg(long)]
        amount: u64,
    },
    /// Begin unstaking a domain (7-day cooldown)
    Unstake {
        /// Domain to unstake
        domain: String,
    },
    /// Manage domain verification and curation
    Domain {
        #[command(subcommand)]
        action: DomainCommands,
    },
    /// Open or manage disputes
    Dispute {
        #[command(subcommand)]
        action: DisputeCommands,
    },
    /// Jury participation
    Jury {
        #[command(subcommand)]
        action: JuryCommands,
    },
    /// View yield and earnings
    Yield {
        #[command(subcommand)]
        action: YieldCommands,
    },
    /// Network operations
    Network {
        #[command(subcommand)]
        action: NetworkCommands,
    },
    /// Manage wallet
    Wallet {
        #[command(subcommand)]
        action: WalletCommands,
    },
    /// Log in with email (web2 mode)
    Login {
        /// Email address for magic link login
        email: Option<String>,
        /// Verify a magic link token to complete login
        #[arg(long)]
        verify: Option<String>,
        /// Gateway URL override
        #[arg(long, default_value = "https://api.opensonarx.com")]
        gateway: String,
    },
    /// Log out and clear stored credentials
    Logout,
    /// Trust-check a URL (stake lookup + risk)
    Verify {
        /// URL to verify
        url: String,
    },
    /// Sentinel node operations (crawl daemon)
    Sentinel {
        #[command(subcommand)]
        action: SentinelCommands,
    },
    /// Show auth mode, account info, quota
    Status,
    /// Open gateway API docs
    Docs,
    /// Show most-queried domains/entities leaderboard
    Leaderboard {
        /// Maximum entries to show
        #[arg(short = 'k', long, default_value = "20")]
        limit: u32,
    },
    /// Manage the local search index
    Index {
        #[command(subcommand)]
        action: IndexCommands,
    },
    /// Seed the local index with documentation content
    Seed {
        #[command(subcommand)]
        action: SeedCommands,
    },
}

#[derive(Subcommand)]
pub enum IndexCommands {
    /// Export the index to a portable tar.gz archive
    Export {
        /// Output file path (default: opensonarx-export.tar.gz)
        #[arg(short, long)]
        output: Option<String>,
        /// Filter by shard ID
        #[arg(long)]
        shard: Option<String>,
    },
    /// Import an index from a tar.gz archive
    Import {
        /// Path to tar.gz archive
        path: String,
        /// Merge with existing index (default: true)
        #[arg(long, default_value = "true")]
        merge: bool,
    },
    /// Show index statistics
    Info,
    /// Re-embed every indexed chunk with the current embedding model
    /// (required after an embedder upgrade — old vectors are meaningless)
    Reembed {
        /// Embedding batch size
        #[arg(long, default_value = "64")]
        batch: usize,
    },
}

#[derive(Subcommand)]
pub enum SeedCommands {
    /// Crawl all URLs from a sitemap.xml
    Sitemap {
        /// Sitemap URL (supports .xml and .xml.gz)
        url: String,
        /// Number of concurrent crawl workers
        #[arg(short, long, default_value = "4")]
        concurrency: usize,
    },
    /// Crawl URLs from a text file (one per line)
    Urls {
        /// Path to text file with URLs
        file: String,
        /// Number of concurrent crawl workers
        #[arg(short, long, default_value = "4")]
        concurrency: usize,
    },
    /// Seed from a package registry
    Registry {
        #[command(subcommand)]
        source: RegistrySource,
    },
    /// Crawl a site by following links from a start URL
    Crawl {
        /// Start URL to spider from
        url: String,
        /// Maximum link-follow depth
        #[arg(short, long, default_value = "3")]
        depth: usize,
        /// Maximum number of pages to crawl
        #[arg(short, long, default_value = "1000")]
        max_pages: usize,
        /// Number of concurrent crawl workers
        #[arg(short, long, default_value = "4")]
        concurrency: usize,
    },
    /// Bootstrap a fresh index with a curated starter corpus (~80 high-value pages)
    Starter {
        /// Number of concurrent crawl workers
        #[arg(short, long, default_value = "4")]
        concurrency: usize,
    },
}

#[derive(Subcommand)]
pub enum RegistrySource {
    /// Top N crates from crates.io → docs.rs
    DocsRs {
        /// Number of top crates by downloads
        #[arg(long, default_value = "100")]
        top: usize,
        /// Number of concurrent crawl workers
        #[arg(short, long, default_value = "4")]
        concurrency: usize,
    },
    /// All English MDN Web Docs
    Mdn {
        /// Number of concurrent crawl workers
        #[arg(short, long, default_value = "4")]
        concurrency: usize,
    },
}

#[derive(Subcommand)]
pub enum DomainCommands {
    /// Verify DNS TXT record for a domain
    Verify { domain: String },
    /// List all staked domains
    List,
    /// Show domain info
    Info { domain: String },
    /// Curate (stake for someone else's domain)
    Curate {
        domain: String,
        #[arg(long)]
        amount: u64,
    },
}

#[derive(Subcommand)]
pub enum DisputeCommands {
    /// Open a dispute against a domain
    Open {
        domain: String,
        /// Bond amount in $TRUTH (web3 mode)
        #[arg(long, default_value = "0")]
        bond: u64,
        /// Reason for dispute
        #[arg(long)]
        reason: String,
        /// Pay bond with credit card via Stripe (gateway mode only)
        #[arg(long)]
        fiat: Option<String>,
    },
    /// List active disputes
    List,
    /// Show dispute details
    Info { dispute_id: String },
}

#[derive(Subcommand)]
pub enum JuryCommands {
    /// Register as an eligible juror
    Register,
    /// Vote on an active dispute
    Vote {
        dispute_id: String,
        /// Vote: uphold or reject
        #[arg(long)]
        verdict: String,
    },
    /// Check jury eligibility
    Status,
}

#[derive(Subcommand)]
pub enum YieldCommands {
    /// Show current yield/earnings
    Show,
    /// Show yield history
    History,
    /// Creator earnings: how often agents used your content, accrued credits
    Creators,
}

#[derive(Subcommand)]
pub enum NetworkCommands {
    /// Run an OpenSonarX node
    Node {
        /// Port to listen on
        #[arg(short, long, default_value = "4001")]
        port: u16,
        /// Bootstrap peer address
        #[arg(short, long)]
        bootstrap: Option<Vec<String>>,
        /// Enable integrated sentinel crawl daemon
        #[arg(long)]
        sentinel: bool,
        /// Comma-separated domains to crawl (requires --sentinel)
        #[arg(long, value_delimiter = ',')]
        domains: Option<Vec<String>>,
        /// Comma-separated shard IDs (omit for automatic topic-based assignment)
        #[arg(long, value_delimiter = ',')]
        shards: Option<Vec<String>>,
        /// Sentinel crawl concurrency
        #[arg(long, default_value = "4")]
        concurrency: usize,
        /// Enable HTTP API on this port (e.g., 8001)
        #[arg(long)]
        http_port: Option<u16>,
    },
    /// Show network status
    Status,
    /// List connected peers
    Peers,
}

#[derive(Subcommand)]
pub enum SentinelCommands {
    /// Start the Sentinel crawl daemon
    Start {
        /// Number of concurrent crawl workers
        #[arg(short, long, default_value = "4")]
        concurrency: usize,
        /// Comma-separated domains to monitor
        #[arg(short, long, value_delimiter = ',')]
        domains: Option<Vec<String>>,
        /// RSS/Atom feed URLs to monitor (comma-separated)
        #[arg(short, long, value_delimiter = ',')]
        feeds: Option<Vec<String>>,
        /// Feed poll interval in seconds (default 300)
        #[arg(long, default_value = "300")]
        poll_secs: u64,
    },
    /// Show Sentinel status and crawl stats
    Status,
    /// Subscribe to an RSS/Atom feed for monitoring
    AddFeed {
        /// Feed URL (RSS or Atom)
        url: String,
    },
    /// Add a domain to monitor
    AddDomain {
        /// Domain to monitor
        domain: String,
        /// Stake amount hint (for priority)
        #[arg(long, default_value = "0")]
        stake: u64,
    },
}

#[derive(Subcommand)]
pub enum WalletCommands {
    /// Create a new wallet
    Create,
    /// Show wallet balance
    Balance,
    /// Show wallet address
    Address,
    /// Import existing wallet from keypair
    Import {
        /// Path to keypair file
        path: String,
    },
}

/// Global configuration derived from CLI flags + config file.
pub struct CliConfig {
    pub format: output::Format,
    pub wallet_path: Option<String>,
    pub network: String,
    pub rpc_url: String,
    pub bootstrap_peers: Vec<String>,
    pub timeout_secs: u64,
    /// Gateway URL (from config file or CLI flag)
    pub gateway_url: Option<String>,
    /// API key (from config file)
    pub api_key: Option<String>,
    /// Configured search engines
    pub search_engines: Option<Vec<oi_sdk::config::SearchEngineConfig>>,
    /// Open mode: disable staking/settlement (crypto-optional)
    pub open_mode: bool,
}

fn main() {
    let cli = Cli::parse();

    if cli.verbose {
        // Logs MUST go to stderr: stdout carries machine-readable output
        // (-o json/ndjson) and a single stray WARN corrupts it for agents.
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(std::io::stderr)
            .init();
    } else if !cli.quiet {
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::WARN)
            .with_writer(std::io::stderr)
            .init();
    }

    // Load persistent config from ~/.opensonarx/config.toml
    let file_config = config_file::ConfigFile::load();

    let rpc_url = cli.rpc.clone().unwrap_or_else(|| {
        file_config
            .network
            .rpc_url
            .clone()
            .unwrap_or_else(|| match cli.network.as_str() {
                "devnet" => "https://api.devnet.solana.com".to_string(),
                "localnet" => "http://localhost:8899".to_string(),
                _ => "https://api.mainnet-beta.solana.com".to_string(),
            })
    });

    let format = match cli.format {
        OutputFormat::Table => output::Format::Table,
        OutputFormat::Json => output::Format::Json,
        OutputFormat::Ndjson => output::Format::Ndjson,
    };

    // CLI flags override config file values
    let wallet_path = cli.wallet.clone().or(file_config.wallet.path.clone());

    let bootstrap_peers = cli
        .peers
        .clone()
        .unwrap_or_else(|| file_config.network.bootstrap_peers.clone());

    let search_engines = if file_config.search.engines.is_empty() {
        None
    } else {
        Some(file_config.search.engines.clone())
    };

    // Default to open mode when no wallet and no API key are configured
    let open_mode = cli.open || (wallet_path.is_none() && file_config.auth.api_key.is_none());

    let config = CliConfig {
        format,
        wallet_path,
        network: cli.network.clone(),
        rpc_url,
        bootstrap_peers,
        timeout_secs: cli.timeout,
        gateway_url: file_config.auth.gateway_url.clone(),
        api_key: file_config.auth.api_key.clone(),
        search_engines,
        open_mode,
    };

    let exit_code = match cli.command {
        Commands::Search {
            query,
            limit,
            min_stake,
            freshness,
            types,
            shard,
            entity,
            content_kind,
            locale,
            gl,
            site,
            min_trust,
            node,
            explain,
            answer,
        } => commands::search(
            &config,
            &query,
            limit,
            min_stake,
            freshness,
            types,
            shard,
            entity,
            content_kind,
            locale,
            gl,
            site,
            min_trust,
            node,
            explain,
            answer,
        ),
        Commands::Feedback {
            result_id,
            useful,
            query,
            result_url,
            result_rank,
        } => commands::feedback(
            &config,
            &result_id,
            useful,
            query.as_deref(),
            result_url.as_deref(),
            result_rank,
        ),
        Commands::FeedbackStats => commands::feedback_stats(&config),
        Commands::Stake { domain, amount } => commands::stake(&config, &domain, amount),
        Commands::Unstake { domain } => commands::unstake(&config, &domain),
        Commands::Domain { action } => commands::domain(&config, action),
        Commands::Dispute { action } => commands::dispute(&config, action),
        Commands::Jury { action } => commands::jury(&config, action),
        Commands::Yield { action } => commands::yield_cmd(&config, action),
        Commands::Network { action } => commands::network(&config, action, cli.verbose),
        Commands::Wallet { action } => commands::wallet(&config, action),
        Commands::Login {
            email,
            verify,
            gateway,
        } => commands::login(&file_config, &config, email, verify, &gateway),
        Commands::Logout => commands::logout(&config),
        Commands::Sentinel { action } => commands::sentinel(&config, action),
        Commands::Verify { url } => commands::verify(&config, &url),
        Commands::Status => commands::status(&config),
        Commands::Docs => commands::docs(&config),
        Commands::Leaderboard { limit } => commands::leaderboard(&config, limit),
        Commands::Index { action } => commands::index(&config, action),
        Commands::Seed { action } => commands::seed(&config, action, cli.verbose),
    };

    std::process::exit(exit_code as i32);
}
