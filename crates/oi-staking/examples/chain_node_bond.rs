//! A node operator's bond on a Solana cluster: post it, read it, slash it.
//!
//! ```text
//! chain_node_bond init  --rpc <url> --program <id> --keypair <authority>
//! chain_node_bond join  --rpc <url> --program <id> --authority <keypair> --operator <keypair> --amount <units>
//! chain_node_bond show  --rpc <url> --program <id> --wallet <pubkey>[,<pubkey>...] [--mirror <file>]
//! chain_node_bond slash --rpc <url> --program <id> --keypair <authority> --wallet <pubkey> --evidence <hex>
//! ```
//!
//! `init` sets the protocol up on a cluster where the program has not been
//! initialized: it creates the test token mint and the vault and records the
//! caller as the protocol authority. It does nothing when already done.
//!
//! `join` is what an operator does before its node is accepted by others: the
//! operator receives test tokens and locks them with the program's
//! `StakeSentinel` instruction, in an account at the PDA
//! `["sentinel", operator]`. On a test cluster the authority hands out the
//! tokens and the fees; on a real one the operator would bring its own.
//!
//! `show` reads operators' bond accounts. With `--mirror` it writes them to a
//! file a node reads to decide whose content it accepts.
//!
//! `slash` sends `SlashSentinel` for the operator's whole remaining bond, with
//! a memo carrying the hash of the evidence. **Only the protocol authority
//! can send it.** The program takes the authority's word; it does not check
//! the evidence. Making this step permissionless is not built.
//!
//! The four instructions are encoded by hand, like `chain_bond.rs` does for
//! `Stake`; `tests/integration/solana_e2e.rs` checks the encoding against the
//! program's own enum.

use solana_client::rpc_client::RpcClient;
use solana_sdk::commitment_config::CommitmentConfig;
use solana_sdk::instruction::{AccountMeta, Instruction};
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::{Keypair, Signature, Signer, read_keypair_file};
use solana_sdk::transaction::Transaction;
// solana-sdk 2.x re-exports these under deprecated paths.
#[allow(deprecated)]
use solana_sdk::{system_instruction, system_program, sysvar};
use std::collections::HashMap;
use std::str::FromStr;

const TOKEN_PROGRAM: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const ASSOCIATED_TOKEN_PROGRAM: &str = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL";
const MEMO_PROGRAM: &str = "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr";
/// Sizes of an SPL token mint and token account.
const MINT_LEN: usize = 82;
const TOKEN_ACCOUNT_LEN: usize = 165;
/// What an operator needs for the fees and rent of joining.
const OPERATOR_LAMPORTS: u64 = 20_000_000;

/// The protocol parameters `init` writes. They are the ones the program's
/// tests use; on a test cluster nothing depends on their size.
const MIN_STAKE: u64 = 1_000_000;
const COOLDOWN_SECS: i64 = 604_800;
const DISPUTE_BOND: u64 = 10_000_000;
const SLASH_PCT: u8 = 10;
const REWARD_RATE: u64 = 100;
const JURY_MIN_STAKE: u64 = 5_000_000;

fn initialize_data() -> Vec<u8> {
    let mut data = vec![3u8];
    data.extend(MIN_STAKE.to_le_bytes());
    data.extend(COOLDOWN_SECS.to_le_bytes());
    data.extend(DISPUTE_BOND.to_le_bytes());
    data.push(SLASH_PCT);
    data.extend(REWARD_RATE.to_le_bytes());
    data.extend(JURY_MIN_STAKE.to_le_bytes());
    data
}

fn amount_data(variant: u8, amount: u64) -> Vec<u8> {
    let mut data = vec![variant];
    data.extend(amount.to_le_bytes());
    data
}

fn slash_data(operator: &Pubkey, amount: u64) -> Vec<u8> {
    let mut data = vec![29u8];
    data.extend(operator.to_bytes());
    data.extend(amount.to_le_bytes());
    data
}

/// The fields of the on-chain protocol config this client needs, read at
/// their fixed offsets: a flag, the authority, the mint, the minimum stake,
/// three more numbers, the blacklist authority, then the vault.
struct Config {
    authority: Pubkey,
    mint: Pubkey,
    min_stake: u64,
    vault: Pubkey,
}

fn parse_config(data: &[u8]) -> Option<Config> {
    let key = |at: usize| {
        Some(Pubkey::new_from_array(
            data.get(at..at + 32)?.try_into().ok()?,
        ))
    };
    if *data.first()? != 1 {
        return None;
    }
    Some(Config {
        authority: key(1)?,
        mint: key(33)?,
        min_stake: u64::from_le_bytes(data.get(65..73)?.try_into().ok()?),
        vault: key(138)?,
    })
}

/// (staked, slashed) of a sentinel account: a flag, the operator, the amount,
/// two timestamps, a 16-byte reward index, then the slashed amount.
fn parse_bond(data: &[u8]) -> Option<(u64, u64)> {
    if *data.first()? != 1 {
        return None;
    }
    let number = |at: usize| Some(u64::from_le_bytes(data.get(at..at + 8)?.try_into().ok()?));
    Some((number(33)?, number(73)?))
}

struct Chain {
    rpc: RpcClient,
    program: Pubkey,
    token_program: Pubkey,
}

impl Chain {
    fn config_address(&self) -> Pubkey {
        Pubkey::find_program_address(&[b"config"], &self.program).0
    }

    fn bond_address(&self, operator: &Pubkey) -> Pubkey {
        Pubkey::find_program_address(&[b"sentinel", operator.as_ref()], &self.program).0
    }

    fn token_address(&self, wallet: &Pubkey, mint: &Pubkey) -> Pubkey {
        Pubkey::find_program_address(
            &[wallet.as_ref(), self.token_program.as_ref(), mint.as_ref()],
            &Pubkey::from_str(ASSOCIATED_TOKEN_PROGRAM).expect("constant"),
        )
        .0
    }

    fn config(&self) -> Option<Config> {
        parse_config(&self.rpc.get_account_data(&self.config_address()).ok()?)
    }

    fn bond(&self, operator: &Pubkey) -> Option<(u64, u64)> {
        parse_bond(
            &self
                .rpc
                .get_account_data(&self.bond_address(operator))
                .ok()?,
        )
    }

    fn send(
        &self,
        what: &str,
        instructions: &[Instruction],
        payer: &Keypair,
        signers: &[&Keypair],
    ) -> Result<Signature, String> {
        let blockhash = self
            .rpc
            .get_latest_blockhash()
            .map_err(|error| format!("latest blockhash: {error}"))?;
        let transaction = Transaction::new_signed_with_payer(
            instructions,
            Some(&payer.pubkey()),
            signers,
            blockhash,
        );
        self.rpc
            .send_and_confirm_transaction(&transaction)
            .map_err(|error| format!("{what}: {error}"))
    }
}

fn flags(args: &[String]) -> Result<HashMap<String, String>, String> {
    let mut map = HashMap::new();
    let mut rest = args.iter();
    while let Some(flag) = rest.next() {
        let name = flag
            .strip_prefix("--")
            .ok_or_else(|| format!("unexpected argument {flag:?}"))?;
        let value = rest
            .next()
            .ok_or_else(|| format!("--{name} needs a value"))?;
        map.insert(name.to_string(), value.clone());
    }
    Ok(map)
}

fn required<'a>(flags: &'a HashMap<String, String>, name: &str) -> Result<&'a str, String> {
    flags
        .get(name)
        .map(String::as_str)
        .ok_or_else(|| format!("--{name} is required"))
}

fn keypair(flags: &HashMap<String, String>, name: &str) -> Result<Keypair, String> {
    read_keypair_file(required(flags, name)?).map_err(|error| format!("--{name}: {error}"))
}

fn init(chain: &Chain, authority: &Keypair) -> Result<(), String> {
    if let Some(config) = chain.config() {
        println!("protocol already initialized");
        println!("  authority:  {}", config.authority);
        println!("  token mint: {}", config.mint);
        println!("  min stake:  {}", config.min_stake);
        return Ok(());
    }
    let (mint, vault) = (Keypair::new(), Keypair::new());
    let rent = |len: usize| {
        chain
            .rpc
            .get_minimum_balance_for_rent_exemption(len)
            .map_err(|error| format!("rent: {error}"))
    };
    let create = |account: &Keypair, len: usize| -> Result<Instruction, String> {
        Ok(system_instruction::create_account(
            &authority.pubkey(),
            &account.pubkey(),
            rent(len)?,
            len as u64,
            &chain.token_program,
        ))
    };
    let initialize = Instruction {
        program_id: chain.program,
        accounts: vec![
            AccountMeta::new(authority.pubkey(), true),
            AccountMeta::new(chain.config_address(), false),
            AccountMeta::new(mint.pubkey(), false),
            AccountMeta::new(vault.pubkey(), false),
            AccountMeta::new_readonly(chain.token_program, false),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new_readonly(sysvar::rent::id(), false),
        ],
        data: initialize_data(),
    };
    let signature = chain.send(
        "initialize",
        &[
            create(&mint, MINT_LEN)?,
            create(&vault, TOKEN_ACCOUNT_LEN)?,
            initialize,
        ],
        authority,
        &[authority, &mint, &vault],
    )?;
    println!("protocol initialized");
    println!("  authority:  {}", authority.pubkey());
    println!("  token mint: {}", mint.pubkey());
    println!("  min stake:  {MIN_STAKE}");
    println!("  signature:  {signature}");
    Ok(())
}

fn join(chain: &Chain, authority: &Keypair, operator: &Keypair, amount: u64) -> Result<(), String> {
    let config = chain
        .config()
        .ok_or("the protocol is not initialized on this cluster; run init first")?;
    if chain.bond(&operator.pubkey()).is_some() {
        return Err(format!("{} already has a bond account", operator.pubkey()));
    }
    let tokens = chain.token_address(&operator.pubkey(), &config.mint);
    // The authority pays the operator's fees, opens its token account and
    // hands it the test tokens.
    let fund =
        system_instruction::transfer(&authority.pubkey(), &operator.pubkey(), OPERATOR_LAMPORTS);
    let open_token_account = Instruction {
        program_id: Pubkey::from_str(ASSOCIATED_TOKEN_PROGRAM).expect("constant"),
        accounts: vec![
            AccountMeta::new(authority.pubkey(), true),
            AccountMeta::new(tokens, false),
            AccountMeta::new_readonly(operator.pubkey(), false),
            AccountMeta::new_readonly(config.mint, false),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new_readonly(chain.token_program, false),
        ],
        // CreateIdempotent
        data: vec![1],
    };
    let mint = Instruction {
        program_id: chain.program,
        accounts: vec![
            AccountMeta::new_readonly(authority.pubkey(), true),
            AccountMeta::new(chain.config_address(), false),
            AccountMeta::new(config.mint, false),
            AccountMeta::new(tokens, false),
            AccountMeta::new_readonly(chain.token_program, false),
        ],
        data: amount_data(4, amount),
    };
    chain.send(
        "fund operator",
        &[fund, open_token_account, mint],
        authority,
        &[authority],
    )?;

    let stake = Instruction {
        program_id: chain.program,
        accounts: vec![
            AccountMeta::new(operator.pubkey(), true),
            AccountMeta::new(tokens, false),
            AccountMeta::new(config.vault, false),
            AccountMeta::new(chain.bond_address(&operator.pubkey()), false),
            AccountMeta::new(chain.config_address(), false),
            AccountMeta::new_readonly(chain.token_program, false),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new_readonly(sysvar::clock::id(), false),
        ],
        data: amount_data(24, amount),
    };
    let signature = chain.send("stake", &[stake], operator, &[operator])?;
    println!("operator bonded {amount} units of the test token");
    println!("  operator:     {}", operator.pubkey());
    println!("  bond account: {}", chain.bond_address(&operator.pubkey()));
    println!("  signature:    {signature}");
    Ok(())
}

fn show(chain: &Chain, wallets: &str, mirror: Option<&String>) -> Result<(), String> {
    let min_stake = chain.config().map_or(0, |config| config.min_stake);
    let mut operators = serde_json::Map::new();
    for wallet in wallets.split(',').map(str::trim).filter(|w| !w.is_empty()) {
        let key =
            Pubkey::from_str(wallet).map_err(|error| format!("--wallet {wallet}: {error}"))?;
        let (staked, slashed) = chain.bond(&key).unwrap_or((0, 0));
        let standing = staked.saturating_sub(slashed);
        println!("operator {wallet}");
        println!("  bond account: {}", chain.bond_address(&key));
        println!("  staked:       {staked}");
        println!("  slashed:      {slashed}");
        println!("  standing:     {standing}  (minimum to be accepted: {min_stake})");
        operators.insert(
            wallet.to_string(),
            serde_json::json!({ "staked": staked, "slashed": slashed }),
        );
    }
    if let Some(path) = mirror {
        // Operators read earlier stay in the file; the ones read now replace
        // their previous entry.
        let mut all = std::fs::read_to_string(path)
            .ok()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
            .and_then(|value| value["operators"].as_object().cloned())
            .unwrap_or_default();
        all.extend(operators);
        let body = serde_json::json!({
            "program": chain.program.to_string(),
            "min_stake": min_stake,
            "operators": all,
        });
        // Written whole and renamed, so a node never reads half a file.
        let temporary = format!("{path}.tmp");
        std::fs::write(
            &temporary,
            serde_json::to_string_pretty(&body).map_err(|error| error.to_string())?,
        )
        .and_then(|_| std::fs::rename(&temporary, path))
        .map_err(|error| format!("cannot write {path}: {error}"))?;
        println!("mirrored to {path}");
    }
    Ok(())
}

fn slash(chain: &Chain, authority: &Keypair, wallet: &str, evidence: &str) -> Result<(), String> {
    let operator = Pubkey::from_str(wallet).map_err(|error| format!("--wallet: {error}"))?;
    let (staked, slashed) = chain
        .bond(&operator)
        .ok_or_else(|| format!("{wallet} has no bond account"))?;
    let standing = staked.saturating_sub(slashed);
    if standing == 0 {
        println!("nothing left to slash for {wallet}");
        return Ok(());
    }
    let memo = Instruction {
        program_id: Pubkey::from_str(MEMO_PROGRAM).expect("constant"),
        accounts: vec![],
        data: format!("opensonarx refused claim, evidence sha256 {evidence}").into_bytes(),
    };
    let instruction = Instruction {
        program_id: chain.program,
        accounts: vec![
            AccountMeta::new_readonly(authority.pubkey(), true),
            AccountMeta::new(chain.bond_address(&operator), false),
            AccountMeta::new_readonly(chain.config_address(), false),
        ],
        data: slash_data(&operator, standing),
    };
    let signature = chain.send("slash", &[memo, instruction], authority, &[authority])?;
    println!("slashed {standing} units from {wallet}");
    println!("  bond account: {}", chain.bond_address(&operator));
    println!("  evidence:     {evidence}");
    println!("  signature:    {signature}");
    Ok(())
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (command, rest) = args
        .split_first()
        .ok_or("usage: chain_node_bond <init|join|show|slash> --rpc <url> --program <id> ...")?;
    let flags = flags(rest)?;
    let chain = Chain {
        rpc: RpcClient::new_with_commitment(
            required(&flags, "rpc")?.to_string(),
            CommitmentConfig::confirmed(),
        ),
        program: Pubkey::from_str(required(&flags, "program")?)
            .map_err(|error| format!("--program: {error}"))?,
        token_program: Pubkey::from_str(TOKEN_PROGRAM).expect("constant"),
    };
    match command.as_str() {
        "init" => init(&chain, &keypair(&flags, "keypair")?),
        "join" => join(
            &chain,
            &keypair(&flags, "authority")?,
            &keypair(&flags, "operator")?,
            required(&flags, "amount")?
                .parse()
                .map_err(|error| format!("--amount: {error}"))?,
        ),
        "show" => show(&chain, required(&flags, "wallet")?, flags.get("mirror")),
        "slash" => slash(
            &chain,
            &keypair(&flags, "keypair")?,
            required(&flags, "wallet")?,
            required(&flags, "evidence")?,
        ),
        other => Err(format!(
            "unknown command {other:?}; use init, join, show or slash"
        )),
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("chain_node_bond: {error}");
        std::process::exit(2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_fields_are_read_at_their_offsets() {
        let mut config = vec![0u8; 400];
        config[0] = 1;
        config[1..33].copy_from_slice(&[2u8; 32]);
        config[33..65].copy_from_slice(&[3u8; 32]);
        config[65..73].copy_from_slice(&1_000_000u64.to_le_bytes());
        config[138..170].copy_from_slice(&[4u8; 32]);
        let parsed = parse_config(&config).unwrap();
        assert_eq!(parsed.authority, Pubkey::new_from_array([2u8; 32]));
        assert_eq!(parsed.mint, Pubkey::new_from_array([3u8; 32]));
        assert_eq!(parsed.min_stake, 1_000_000);
        assert_eq!(parsed.vault, Pubkey::new_from_array([4u8; 32]));
        assert!(parse_config(&[0u8; 400]).is_none(), "not initialized");

        let mut bond = vec![0u8; 82];
        bond[0] = 1;
        bond[33..41].copy_from_slice(&5_000u64.to_le_bytes());
        bond[73..81].copy_from_slice(&1_200u64.to_le_bytes());
        assert_eq!(parse_bond(&bond), Some((5_000, 1_200)));
        assert!(parse_bond(&[0u8; 82]).is_none());
    }

    #[test]
    fn instruction_bytes_start_with_the_variant() {
        assert_eq!(initialize_data()[0], 3);
        assert_eq!(initialize_data().len(), 1 + 8 + 8 + 8 + 1 + 8 + 8);
        assert_eq!(
            amount_data(24, 9),
            [&[24u8][..], &9u64.to_le_bytes()].concat()
        );
        let data = slash_data(&Pubkey::new_from_array([7u8; 32]), 9);
        assert_eq!(data[0], 29);
        assert_eq!(&data[1..33], &[7u8; 32]);
        assert_eq!(&data[33..], &9u64.to_le_bytes());
    }
}
