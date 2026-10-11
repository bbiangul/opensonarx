//! Bond SOL to a domain on a Solana cluster and read the bond back.
//!
//! ```text
//! cargo run -p oi-staking --features solana --example chain_bond -- \
//!     stake --rpc <url> --program <id> --keypair <file> --domain example.com --lamports 10000000
//! cargo run -p oi-staking --features solana --example chain_bond -- \
//!     show --rpc <url> --program <id> --domain example.com [--mirror <stakes.json>]
//! ```
//!
//! `stake` sends the program's `Stake` instruction: the payer funds a stake
//! account at the PDA `["stake", domain]` holding the bonded lamports.
//! `show` reads that account. With `--mirror` it also writes the bond into a
//! local stake registry file, so `oi search` shows it next to results from
//! that domain. The mirrored record is marked as not ownership-verified: a
//! bond says someone put money behind a domain, not that they own it.

use oi_staking::solana::SolanaStakeRegistry;
use oi_types::traits::StakeRegistry;
use oi_types::StakeInfo;
use solana_client::rpc_client::RpcClient;
use solana_sdk::commitment_config::CommitmentConfig;
use solana_sdk::instruction::{AccountMeta, Instruction};
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::{read_keypair_file, Signer};
use solana_sdk::transaction::Transaction;
use solana_sdk::{system_program, sysvar};
use std::collections::HashMap;
use std::str::FromStr;

/// Borsh encoding of `StakingInstruction::Stake { domain, amount }`, the
/// program's first instruction variant.
fn stake_instruction_data(domain: &str, lamports: u64) -> Vec<u8> {
    let mut data = vec![0u8];
    data.extend((domain.len() as u32).to_le_bytes());
    data.extend(domain.as_bytes());
    data.extend(lamports.to_le_bytes());
    data
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

fn mirror(path: &str, info: &StakeInfo) -> Result<(), String> {
    let mut stakes: Vec<StakeInfo> = match std::fs::read_to_string(path) {
        Ok(raw) => serde_json::from_str(&raw).map_err(|error| format!("{path}: {error}"))?,
        Err(_) => Vec::new(),
    };
    stakes.retain(|stake| stake.domain != info.domain);
    stakes.push(info.clone());
    let body = serde_json::to_string_pretty(&stakes).map_err(|error| error.to_string())?;
    std::fs::write(path, body).map_err(|error| format!("cannot write {path}: {error}"))
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (command, rest) = args
        .split_first()
        .ok_or("usage: chain_bond <stake|show> --rpc <url> --program <id> --domain <name> ...")?;
    let flags = flags(rest)?;
    let rpc_url = required(&flags, "rpc")?;
    let program_id = Pubkey::from_str(required(&flags, "program")?)
        .map_err(|error| format!("--program: {error}"))?;
    let domain = required(&flags, "domain")?;
    let (stake_account, _) =
        Pubkey::find_program_address(&[b"stake", domain.as_bytes()], &program_id);

    match command.as_str() {
        "stake" => {
            let lamports: u64 = required(&flags, "lamports")?
                .parse()
                .map_err(|error| format!("--lamports: {error}"))?;
            let payer = read_keypair_file(required(&flags, "keypair")?)
                .map_err(|error| format!("--keypair: {error}"))?;
            let rpc = RpcClient::new_with_commitment(
                rpc_url.to_string(),
                CommitmentConfig::confirmed(),
            );
            let instruction = Instruction {
                program_id,
                accounts: vec![
                    AccountMeta::new(payer.pubkey(), true),
                    AccountMeta::new(stake_account, false),
                    AccountMeta::new_readonly(system_program::id(), false),
                    AccountMeta::new_readonly(sysvar::clock::id(), false),
                ],
                data: stake_instruction_data(domain, lamports),
            };
            let blockhash = rpc
                .get_latest_blockhash()
                .map_err(|error| format!("latest blockhash: {error}"))?;
            let transaction = Transaction::new_signed_with_payer(
                &[instruction],
                Some(&payer.pubkey()),
                &[&payer],
                blockhash,
            );
            let signature = rpc
                .send_and_confirm_transaction(&transaction)
                .map_err(|error| format!("stake transaction: {error}"))?;
            println!("bonded {lamports} lamports to {domain}");
            println!("  staker:        {}", payer.pubkey());
            println!("  stake account: {stake_account}");
            println!("  signature:     {signature}");
            Ok(())
        }
        "show" => {
            let registry = SolanaStakeRegistry::new(rpc_url, program_id);
            let Some(mut info) = registry
                .get_stake(domain)
                .map_err(|error| error.to_string())?
            else {
                println!("no bond on {domain} (stake account {stake_account})");
                return Ok(());
            };
            // Nothing on chain proves the staker controls the domain.
            info.dns_verified = false;
            println!("bond on {domain}");
            println!("  stake account: {stake_account}");
            println!("  staker:        {}", info.wallet);
            println!("  lamports:      {}", info.amount_lamports);
            println!("  staked at:     {}", info.staked_at);
            if let Some(path) = flags.get("mirror") {
                mirror(path, &info)?;
                println!("  mirrored to:   {path}");
            }
            Ok(())
        }
        other => Err(format!("unknown command {other:?}; use stake or show")),
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("chain_bond: {error}");
        std::process::exit(2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stake_instruction_is_variant_zero_with_a_borsh_string_and_amount() {
        let data = stake_instruction_data("a.io", 7);
        assert_eq!(data[0], 0);
        assert_eq!(&data[1..5], &4u32.to_le_bytes());
        assert_eq!(&data[5..9], b"a.io");
        assert_eq!(&data[9..], &7u64.to_le_bytes());
    }

    #[test]
    fn flags_come_in_pairs() {
        let parsed = flags(&["--rpc".to_string(), "http://x".to_string()]).unwrap();
        assert_eq!(parsed["rpc"], "http://x");
        assert!(flags(&["--rpc".to_string()]).is_err());
        assert!(flags(&["rpc".to_string()]).is_err());
    }
}
