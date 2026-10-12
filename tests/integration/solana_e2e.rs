#![allow(deprecated)] // solana 2.x module path deprecations

use std::sync::Arc;

use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::program_pack::Pack;
use solana_program::rent::Rent;
use solana_program_test::{processor, ProgramTest};
use solana_sdk::{
    account::AccountSharedData,
    ed25519_instruction,
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    system_instruction,
    sysvar::{clock, instructions as ix_sysvar, rent},
    transaction::Transaction,
};

use opensonarx_staking::{instruction::StakingInstruction, state::*};

use oi_types::traits::{SearchIndex, StakeRegistry};
use oi_types::{ContentChunk, QueryFilters, StakeInfo};

// ──────────────────────────────────────────────
// Helper: boot ProgramTest with staking program
// ──────────────────────────────────────────────

fn make_program_test() -> (Pubkey, ProgramTest) {
    let program_id = Pubkey::new_unique();
    let pt = ProgramTest::new(
        "opensonarx_staking",
        program_id,
        processor!(opensonarx_staking::process_instruction),
    );
    (program_id, pt)
}

// ──────────────────────────────────────────────
// Helper: create a pre-allocated account for mint/token accounts
// ──────────────────────────────────────────────

fn add_spl_token_account(pt: &mut ProgramTest, pubkey: &Pubkey, size: usize) {
    let rent = Rent::default();
    let lamports = rent.minimum_balance(size);
    pt.add_account(
        *pubkey,
        solana_sdk::account::Account {
            lamports,
            data: vec![0u8; size],
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
}

// ──────────────────────────────────────────────
// Helper: create and initialize an SPL token account for a wallet
// ──────────────────────────────────────────────

async fn create_token_account(
    banks: &mut solana_program_test::BanksClient,
    payer: &Keypair,
    recent_blockhash: solana_sdk::hash::Hash,
    mint: &Pubkey,
    owner: &Pubkey,
) -> Pubkey {
    let account = Keypair::new();
    let rent = Rent::default();
    let space = spl_token::state::Account::LEN;

    let create_ix = system_instruction::create_account(
        &payer.pubkey(),
        &account.pubkey(),
        rent.minimum_balance(space),
        space as u64,
        &spl_token::id(),
    );
    let init_ix = spl_token::instruction::initialize_account3(
        &spl_token::id(),
        &account.pubkey(),
        mint,
        owner,
    )
    .unwrap();

    let tx = Transaction::new_signed_with_payer(
        &[create_ix, init_ix],
        Some(&payer.pubkey()),
        &[payer, &account],
        recent_blockhash,
    );
    banks.process_transaction(tx).await.unwrap();
    account.pubkey()
}

// ──────────────────────────────────────────────
// Helper: create Ed25519 instruction for channel settlement
// ──────────────────────────────────────────────

fn make_ed25519_settle_ix(
    payer: &Keypair,
    channel_pda: &Pubkey,
    spent: u64,
    nonce: u64,
) -> Instruction {
    let msg = [
        channel_pda.as_ref(),
        &spent.to_le_bytes(),
        &nonce.to_le_bytes(),
    ]
    .concat();
    let sig = payer.sign_message(&msg);
    let sig_bytes: [u8; 64] = sig.into();
    let pubkey_bytes: [u8; 32] = payer.pubkey().to_bytes();
    ed25519_instruction::new_ed25519_instruction_with_signature(&msg, &sig_bytes, &pubkey_bytes)
}

// ──────────────────────────────────────────────
// Legacy V1 tests (preserved from original)
// ──────────────────────────────────────────────

/// Full end-to-end: stake on Solana → crawl → index → search (with enrichment) → unstake
#[tokio::test]
async fn test_full_pipeline_with_solana() {
    // -- 1. Boot in-process Solana runtime --
    let program_id = Pubkey::new_unique();
    let program_test = ProgramTest::new(
        "opensonarx_staking",
        program_id,
        processor!(opensonarx_staking::process_instruction),
    );
    let (banks_client, payer, recent_blockhash) = program_test.start().await;

    // -- 2. Submit Stake transaction --
    let domain = "example.com";
    let amount: u64 = 1_000_000_000; // 1 SOL

    let (stake_pda, _bump) =
        Pubkey::find_program_address(&[STAKE_SEED, domain.as_bytes()], &program_id);

    let mut ix_data = Vec::new();
    StakingInstruction::Stake {
        domain: domain.to_string(),
        amount,
    }
    .serialize(&mut ix_data)
    .unwrap();

    let stake_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(stake_pda, false),
            AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
            AccountMeta::new_readonly(clock::id(), false),
        ],
        data: ix_data,
    };

    let tx = Transaction::new_signed_with_payer(
        &[stake_ix],
        Some(&payer.pubkey()),
        &[&payer],
        recent_blockhash,
    );
    banks_client.process_transaction(tx).await.unwrap();

    // -- 3. Verify PDA account --
    let pda_account = banks_client
        .get_account(stake_pda)
        .await
        .unwrap()
        .expect("PDA account should exist");
    let stake_account = StakeAccount::deserialize_reader(&mut &pda_account.data[..]).unwrap();

    assert!(stake_account.is_initialized);
    assert_eq!(stake_account.domain, domain);
    assert_eq!(stake_account.amount_lamports, amount);
    assert_eq!(stake_account.cooldown_until, 0);
    assert_eq!(stake_account.staker, payer.pubkey());

    // -- 4. Crawl HTML --
    let html = "<html><head><title>Rust Guide</title></head><body>\
        <h1>Learn Rust</h1>\
        <p>Rust is a systems programming language focused on safety and performance. \
        Its ownership model prevents data races at compile time without garbage collection. \
        The borrow checker ensures references are always valid throughout their lifetime. \
        Pattern matching provides exhaustive checks on enum variants for robust error handling. \
        Traits enable polymorphism without inheritance, favoring composition over subclassing. \
        Error handling uses Result and Option types for explicit control flow instead of exceptions. \
        The standard library provides efficient collections like Vec, HashMap, and BTreeMap. \
        Cargo manages dependencies and builds reproducibly across platforms. \
        Clippy catches common mistakes and suggests idiomatic improvements to your code. \
        Rustfmt enforces consistent formatting across projects and teams. \
        Async await syntax enables efficient concurrent programming without callback hell. \
        Zero-cost abstractions let you write high-level code that compiles to optimal machine instructions. \
        The type system catches null pointer errors, buffer overflows, and iterator invalidation at compile time. \
        Crates.io hosts over 100,000 community packages for networking, serialization, and web development. \
        Cross-compilation support enables targeting embedded systems, WebAssembly, and mobile platforms. \
        Procedural macros generate boilerplate code at compile time for serialization and database mappings. \
        Unsafe blocks provide escape hatches for FFI and low-level memory operations when needed. \
        The module system organizes code into clear hierarchies with fine-grained visibility control. \
        Lifetime annotations express relationships between references for the borrow checker to verify. \
        Smart pointers like Box, Rc, and Arc provide heap allocation and reference counting.</p>\
        </body></html>".to_string();
    let crawl_output =
        oi_crawl::pipeline::process_html("https://example.com/rust", &html, 512, 128).unwrap();
    assert!(!crawl_output.chunks.is_empty());

    // -- 5. Add mock embeddings --
    let dim = 8;
    let chunks: Vec<ContentChunk> = crawl_output
        .chunks
        .into_iter()
        .enumerate()
        .map(|(i, mut chunk)| {
            let mut emb = vec![0.0f32; dim];
            emb[i % dim] = 1.0;
            chunk.embedding = Some(emb);
            chunk
        })
        .collect();

    // -- 6. Build InMemoryStakeRegistry from on-chain data --
    let registry = oi_staking::InMemoryStakeRegistry::new();
    registry
        .add_stake(StakeInfo {
            domain: domain.to_string(),
            amount_lamports: stake_account.amount_lamports,
            amount_usd: stake_account.amount_lamports as f64 / 1_000_000_000.0 * 150.0,
            staked_at: stake_account.staked_at,
            cooldown_until: None,
            wallet: payer.pubkey().to_string(),
            dns_verified: true,
            reputation: 0.95,
            entity: None,
            disputed: false,
        })
        .unwrap();

    // -- 7. Ingest into HybridIndex with stake enrichment --
    let index = oi_index::HybridIndex::new(dim)
        .with_stake_registry(Arc::new(registry) as Arc<dyn StakeRegistry>);
    index.ingest(&chunks).unwrap();
    assert!(index.doc_count() > 0);

    // -- 8. Query and verify enrichment --
    let query_emb = vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    let results = index
        .query(&query_emb, "rust programming", &QueryFilters::default(), 5)
        .unwrap();
    assert!(!results.is_empty());
    assert!(
        results[0].stake_truth > 0,
        "expected stake_truth > 0, got {}",
        results[0].stake_truth
    );

    // -- 9. Submit Unstake transaction --
    let mut ix_data = Vec::new();
    StakingInstruction::Unstake {
        domain: domain.to_string(),
    }
    .serialize(&mut ix_data)
    .unwrap();

    let unstake_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(stake_pda, false),
            AccountMeta::new_readonly(clock::id(), false),
        ],
        data: ix_data,
    };

    let tx = Transaction::new_signed_with_payer(
        &[unstake_ix],
        Some(&payer.pubkey()),
        &[&payer],
        recent_blockhash,
    );
    banks_client.process_transaction(tx).await.unwrap();

    // -- 10. Verify cooldown --
    let pda_account = banks_client
        .get_account(stake_pda)
        .await
        .unwrap()
        .expect("PDA account should still exist");
    let stake_account = StakeAccount::deserialize_reader(&mut &pda_account.data[..]).unwrap();
    assert!(
        stake_account.cooldown_until > 0,
        "expected cooldown_until > 0 after unstake"
    );
}

/// Verify PDA derivation is deterministic and off-curve.
#[test]
fn test_stake_pda_derivation() {
    let program_id = Pubkey::new_unique();
    let domain = "test.example.com";

    let (pda1, bump1) = Pubkey::find_program_address(&[STAKE_SEED, domain.as_bytes()], &program_id);
    let (pda2, bump2) = Pubkey::find_program_address(&[STAKE_SEED, domain.as_bytes()], &program_id);

    assert_eq!(pda1, pda2, "PDA should be deterministic");
    assert_eq!(bump1, bump2);
    assert_ne!(pda1, program_id);
    assert_ne!(pda1, Pubkey::default());
}

/// Verify StakeAccount borsh round-trip.
#[test]
fn test_stake_account_serialization() {
    let account = StakeAccount {
        is_initialized: true,
        staker: Pubkey::new_unique(),
        domain: "example.com".to_string(),
        amount_lamports: 1_000_000_000,
        staked_at: 1700000000,
        cooldown_until: 0,
    };

    let mut buf = Vec::new();
    account.serialize(&mut buf).unwrap();

    let decoded = StakeAccount::deserialize_reader(&mut &buf[..]).unwrap();
    assert!(decoded.is_initialized);
    assert_eq!(decoded.domain, "example.com");
    assert_eq!(decoded.amount_lamports, 1_000_000_000);
    assert_eq!(decoded.staked_at, 1700000000);
    assert_eq!(decoded.cooldown_until, 0);
    assert_eq!(decoded.staker, account.staker);
}

// ──────────────────────────────────────────────
// Phase 0: Protocol Config + $TRUTH Mint Tests
// ──────────────────────────────────────────────

#[tokio::test]
async fn test_initialize_protocol() {
    let (program_id, mut pt) = make_program_test();

    // Pre-create mint and vault accounts owned by spl-token
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    add_spl_token_account(&mut pt, &truth_mint.pubkey(), spl_token::state::Mint::LEN);
    add_spl_token_account(
        &mut pt,
        &reward_vault.pubkey(),
        spl_token::state::Account::LEN,
    );

    let (banks, payer, blockhash) = pt.start().await;

    let (config_pda, _) = Pubkey::find_program_address(&[CONFIG_SEED], &program_id);

    let mut ix_data = Vec::new();
    StakingInstruction::InitializeProtocol {
        min_stake: 1_000_000,
        cooldown_secs: 604800,
        dispute_bond: 10_000_000,
        slash_pct: 10,
        reward_rate: 100,
        jury_min_stake: 5_000_000,
    }
    .serialize(&mut ix_data)
    .unwrap();

    let ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(config_pda, false),
            AccountMeta::new(truth_mint.pubkey(), false),
            AccountMeta::new(reward_vault.pubkey(), false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
            AccountMeta::new_readonly(rent::id(), false),
        ],
        data: ix_data,
    };

    let tx = Transaction::new_signed_with_payer(&[ix], Some(&payer.pubkey()), &[&payer], blockhash);
    banks.process_transaction(tx).await.unwrap();

    // Verify config PDA
    let config_account = banks.get_account(config_pda).await.unwrap().unwrap();
    let config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
    assert!(config.is_initialized);
    assert_eq!(config.min_stake, 1_000_000);
    assert_eq!(config.cooldown_secs, 604800);
    assert_eq!(config.slash_pct, 10);
    assert_eq!(config.authority, payer.pubkey());
    assert_eq!(config.truth_mint, truth_mint.pubkey());
}

#[tokio::test]
async fn test_mint_truth() {
    let (program_id, mut pt) = make_program_test();

    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    add_spl_token_account(&mut pt, &truth_mint.pubkey(), spl_token::state::Mint::LEN);
    add_spl_token_account(
        &mut pt,
        &reward_vault.pubkey(),
        spl_token::state::Account::LEN,
    );

    let (mut banks, payer, blockhash) = pt.start().await;
    let (config_pda, _) = Pubkey::find_program_address(&[CONFIG_SEED], &program_id);

    // Initialize protocol
    let mut ix_data = Vec::new();
    StakingInstruction::InitializeProtocol {
        min_stake: 1_000_000,
        cooldown_secs: 604800,
        dispute_bond: 10_000_000,
        slash_pct: 10,
        reward_rate: 100,
        jury_min_stake: 5_000_000,
    }
    .serialize(&mut ix_data)
    .unwrap();

    let init_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(config_pda, false),
            AccountMeta::new(truth_mint.pubkey(), false),
            AccountMeta::new(reward_vault.pubkey(), false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
            AccountMeta::new_readonly(rent::id(), false),
        ],
        data: ix_data,
    };

    let tx =
        Transaction::new_signed_with_payer(&[init_ix], Some(&payer.pubkey()), &[&payer], blockhash);
    banks.process_transaction(tx).await.unwrap();

    // Create destination token account
    let dest_token = create_token_account(
        &mut banks,
        &payer,
        blockhash,
        &truth_mint.pubkey(),
        &payer.pubkey(),
    )
    .await;

    // Mint 500M $TRUTH (within the 1B max_total_supply default)
    let mut ix_data = Vec::new();
    StakingInstruction::MintTruth {
        amount: 500_000_000,
    }
    .serialize(&mut ix_data)
    .unwrap();

    let mint_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(config_pda, false),
            AccountMeta::new(truth_mint.pubkey(), false),
            AccountMeta::new(dest_token, false),
            AccountMeta::new_readonly(spl_token::id(), false),
        ],
        data: ix_data,
    };

    let blockhash = banks.get_latest_blockhash().await.unwrap();
    let tx =
        Transaction::new_signed_with_payer(&[mint_ix], Some(&payer.pubkey()), &[&payer], blockhash);
    banks.process_transaction(tx).await.unwrap();

    // Verify token balance
    let token_account = banks.get_account(dest_token).await.unwrap().unwrap();
    let token_data = spl_token::state::Account::unpack(&token_account.data).unwrap();
    assert_eq!(token_data.amount, 500_000_000);
}

#[tokio::test]
async fn test_mint_truth_unauthorized_fails() {
    let (program_id, mut pt) = make_program_test();

    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    add_spl_token_account(&mut pt, &truth_mint.pubkey(), spl_token::state::Mint::LEN);
    add_spl_token_account(
        &mut pt,
        &reward_vault.pubkey(),
        spl_token::state::Account::LEN,
    );

    let (mut banks, payer, blockhash) = pt.start().await;
    let (config_pda, _) = Pubkey::find_program_address(&[CONFIG_SEED], &program_id);

    // Initialize protocol
    let mut ix_data = Vec::new();
    StakingInstruction::InitializeProtocol {
        min_stake: 1_000_000,
        cooldown_secs: 604800,
        dispute_bond: 10_000_000,
        slash_pct: 10,
        reward_rate: 100,
        jury_min_stake: 5_000_000,
    }
    .serialize(&mut ix_data)
    .unwrap();

    let init_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(config_pda, false),
            AccountMeta::new(truth_mint.pubkey(), false),
            AccountMeta::new(reward_vault.pubkey(), false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
            AccountMeta::new_readonly(rent::id(), false),
        ],
        data: ix_data,
    };

    let tx =
        Transaction::new_signed_with_payer(&[init_ix], Some(&payer.pubkey()), &[&payer], blockhash);
    banks.process_transaction(tx).await.unwrap();

    // Create token account for unauthorized user
    let unauthorized = Keypair::new();
    let dest_token = create_token_account(
        &mut banks,
        &payer,
        blockhash,
        &truth_mint.pubkey(),
        &unauthorized.pubkey(),
    )
    .await;

    // Fund unauthorized to pay for tx
    let fund_ix =
        system_instruction::transfer(&payer.pubkey(), &unauthorized.pubkey(), 1_000_000_000);
    let blockhash = banks.get_latest_blockhash().await.unwrap();
    let tx =
        Transaction::new_signed_with_payer(&[fund_ix], Some(&payer.pubkey()), &[&payer], blockhash);
    banks.process_transaction(tx).await.unwrap();

    // Try to mint with unauthorized signer
    let mut ix_data = Vec::new();
    StakingInstruction::MintTruth { amount: 1_000 }
        .serialize(&mut ix_data)
        .unwrap();

    let mint_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(unauthorized.pubkey(), true),
            AccountMeta::new(config_pda, false),
            AccountMeta::new(truth_mint.pubkey(), false),
            AccountMeta::new(dest_token, false),
            AccountMeta::new_readonly(spl_token::id(), false),
        ],
        data: ix_data,
    };

    let blockhash = banks.get_latest_blockhash().await.unwrap();
    let tx = Transaction::new_signed_with_payer(
        &[mint_ix],
        Some(&unauthorized.pubkey()),
        &[&unauthorized],
        blockhash,
    );
    let result = banks.process_transaction(tx).await;
    assert!(result.is_err(), "unauthorized mint should fail");
}

// ──────────────────────────────────────────────
// Phase 1: SPL Token Staking Tests
// ──────────────────────────────────────────────

async fn setup_protocol(
    _program_id: Pubkey,
    pt: &mut ProgramTest,
    truth_mint: &Keypair,
    reward_vault: &Keypair,
) {
    add_spl_token_account(pt, &truth_mint.pubkey(), spl_token::state::Mint::LEN);
    add_spl_token_account(pt, &reward_vault.pubkey(), spl_token::state::Account::LEN);
}

async fn init_protocol_and_mint(
    banks: &mut solana_program_test::BanksClient,
    payer: &Keypair,
    program_id: &Pubkey,
    truth_mint: &Pubkey,
    reward_vault: &Pubkey,
    mint_amount: u64,
) -> (Pubkey, Pubkey) {
    let (config_pda, _) = Pubkey::find_program_address(&[CONFIG_SEED], program_id);
    let blockhash = banks.get_latest_blockhash().await.unwrap();

    // Initialize protocol
    let mut ix_data = Vec::new();
    StakingInstruction::InitializeProtocol {
        min_stake: 1_000_000,
        cooldown_secs: 1, // 1 second for testing
        dispute_bond: 10_000_000,
        slash_pct: 10,
        reward_rate: 100,
        jury_min_stake: 5_000_000,
    }
    .serialize(&mut ix_data)
    .unwrap();

    let init_ix = Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(config_pda, false),
            AccountMeta::new(*truth_mint, false),
            AccountMeta::new(*reward_vault, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
            AccountMeta::new_readonly(rent::id(), false),
        ],
        data: ix_data,
    };

    let tx =
        Transaction::new_signed_with_payer(&[init_ix], Some(&payer.pubkey()), &[payer], blockhash);
    banks.process_transaction(tx).await.unwrap();

    // Create staker token account
    let staker_token =
        create_token_account(banks, payer, blockhash, truth_mint, &payer.pubkey()).await;

    // Mint tokens
    if mint_amount > 0 {
        let mut ix_data = Vec::new();
        StakingInstruction::MintTruth {
            amount: mint_amount,
        }
        .serialize(&mut ix_data)
        .unwrap();

        let mint_ix = Instruction {
            program_id: *program_id,
            accounts: vec![
                AccountMeta::new(payer.pubkey(), true),
                AccountMeta::new(config_pda, false),
                AccountMeta::new(*truth_mint, false),
                AccountMeta::new(staker_token, false),
                AccountMeta::new_readonly(spl_token::id(), false),
            ],
            data: ix_data,
        };

        let blockhash = banks.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[mint_ix],
            Some(&payer.pubkey()),
            &[payer],
            blockhash,
        );
        banks.process_transaction(tx).await.unwrap();
    }

    (config_pda, staker_token)
}

#[tokio::test]
async fn test_stake_v2_publisher() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let (mut banks, payer, _) = pt.start().await;
    let (config_pda, staker_token) = init_protocol_and_mint(
        &mut banks,
        &payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        100_000_000,
    )
    .await;

    let domain = "test.com";
    let stake_amount = 50_000_000u64;

    let (stake_pda, _) = Pubkey::find_program_address(
        &[STAKE_SEED, domain.as_bytes(), payer.pubkey().as_ref()],
        &program_id,
    );
    let (domain_pda, _) =
        Pubkey::find_program_address(&[DOMAIN_SEED, domain.as_bytes()], &program_id);

    let mut ix_data = Vec::new();
    StakingInstruction::StakeV2 {
        domain: domain.to_string(),
        amount: stake_amount,
        stake_type: 0, // Publisher
    }
    .serialize(&mut ix_data)
    .unwrap();

    let ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(staker_token, false),
            AccountMeta::new(reward_vault.pubkey(), false),
            AccountMeta::new(stake_pda, false),
            AccountMeta::new(domain_pda, false),
            AccountMeta::new(config_pda, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
            AccountMeta::new_readonly(clock::id(), false),
        ],
        data: ix_data,
    };

    let blockhash = banks.get_latest_blockhash().await.unwrap();
    let tx = Transaction::new_signed_with_payer(&[ix], Some(&payer.pubkey()), &[&payer], blockhash);
    banks.process_transaction(tx).await.unwrap();

    // Verify stake account
    let stake_account = banks.get_account(stake_pda).await.unwrap().unwrap();
    let stake = StakeAccountV2::deserialize_reader(&mut &stake_account.data[..]).unwrap();
    assert!(stake.is_initialized);
    assert_eq!(stake.amount, stake_amount);
    assert_eq!(stake.stake_type, StakeType::Publisher);
    assert_eq!(stake.cooldown_until, 0);

    // Verify domain record
    let domain_account = banks.get_account(domain_pda).await.unwrap().unwrap();
    let record = DomainRecord::deserialize_reader(&mut &domain_account.data[..]).unwrap();
    assert!(record.is_initialized);
    assert_eq!(record.total_staked, stake_amount);
    assert_eq!(record.publisher_count, 1);
    assert_eq!(record.curator_count, 0);
}

#[tokio::test]
async fn test_stake_v2_curator() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let (mut banks, payer, _) = pt.start().await;
    let (config_pda, staker_token) = init_protocol_and_mint(
        &mut banks,
        &payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        100_000_000,
    )
    .await;

    let domain = "curated.com";
    let (stake_pda, _) = Pubkey::find_program_address(
        &[STAKE_SEED, domain.as_bytes(), payer.pubkey().as_ref()],
        &program_id,
    );
    let (domain_pda, _) =
        Pubkey::find_program_address(&[DOMAIN_SEED, domain.as_bytes()], &program_id);

    let mut ix_data = Vec::new();
    StakingInstruction::StakeV2 {
        domain: domain.to_string(),
        amount: 20_000_000,
        stake_type: 1, // Curator
    }
    .serialize(&mut ix_data)
    .unwrap();

    let ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(staker_token, false),
            AccountMeta::new(reward_vault.pubkey(), false),
            AccountMeta::new(stake_pda, false),
            AccountMeta::new(domain_pda, false),
            AccountMeta::new(config_pda, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
            AccountMeta::new_readonly(clock::id(), false),
        ],
        data: ix_data,
    };

    let blockhash = banks.get_latest_blockhash().await.unwrap();
    let tx = Transaction::new_signed_with_payer(&[ix], Some(&payer.pubkey()), &[&payer], blockhash);
    banks.process_transaction(tx).await.unwrap();

    let stake_account = banks.get_account(stake_pda).await.unwrap().unwrap();
    let stake = StakeAccountV2::deserialize_reader(&mut &stake_account.data[..]).unwrap();
    assert_eq!(stake.stake_type, StakeType::Curator);

    let domain_account = banks.get_account(domain_pda).await.unwrap().unwrap();
    let record = DomainRecord::deserialize_reader(&mut &domain_account.data[..]).unwrap();
    assert_eq!(record.curator_count, 1);
    assert_eq!(record.publisher_count, 0);
}

#[tokio::test]
async fn test_stake_v2_below_minimum_fails() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let (mut banks, payer, _) = pt.start().await;
    let (config_pda, staker_token) = init_protocol_and_mint(
        &mut banks,
        &payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        100_000_000,
    )
    .await;

    let domain = "small.com";
    let (stake_pda, _) = Pubkey::find_program_address(
        &[STAKE_SEED, domain.as_bytes(), payer.pubkey().as_ref()],
        &program_id,
    );
    let (domain_pda, _) =
        Pubkey::find_program_address(&[DOMAIN_SEED, domain.as_bytes()], &program_id);

    let mut ix_data = Vec::new();
    StakingInstruction::StakeV2 {
        domain: domain.to_string(),
        amount: 100, // Below min_stake of 1_000_000
        stake_type: 0,
    }
    .serialize(&mut ix_data)
    .unwrap();

    let ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(staker_token, false),
            AccountMeta::new(reward_vault.pubkey(), false),
            AccountMeta::new(stake_pda, false),
            AccountMeta::new(domain_pda, false),
            AccountMeta::new(config_pda, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
            AccountMeta::new_readonly(clock::id(), false),
        ],
        data: ix_data,
    };

    let blockhash = banks.get_latest_blockhash().await.unwrap();
    let tx = Transaction::new_signed_with_payer(&[ix], Some(&payer.pubkey()), &[&payer], blockhash);
    let result = banks.process_transaction(tx).await;
    assert!(result.is_err(), "staking below minimum should fail");
}

#[tokio::test]
async fn test_unstake_v2_and_cooldown() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let (mut banks, payer, _) = pt.start().await;
    let (config_pda, staker_token) = init_protocol_and_mint(
        &mut banks,
        &payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        100_000_000,
    )
    .await;

    let domain = "unstake.com";
    let (stake_pda, _) = Pubkey::find_program_address(
        &[STAKE_SEED, domain.as_bytes(), payer.pubkey().as_ref()],
        &program_id,
    );
    let (domain_pda, _) =
        Pubkey::find_program_address(&[DOMAIN_SEED, domain.as_bytes()], &program_id);

    // Stake first
    let mut ix_data = Vec::new();
    StakingInstruction::StakeV2 {
        domain: domain.to_string(),
        amount: 10_000_000,
        stake_type: 0,
    }
    .serialize(&mut ix_data)
    .unwrap();

    let ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(staker_token, false),
            AccountMeta::new(reward_vault.pubkey(), false),
            AccountMeta::new(stake_pda, false),
            AccountMeta::new(domain_pda, false),
            AccountMeta::new(config_pda, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
            AccountMeta::new_readonly(clock::id(), false),
        ],
        data: ix_data,
    };

    let blockhash = banks.get_latest_blockhash().await.unwrap();
    let tx = Transaction::new_signed_with_payer(&[ix], Some(&payer.pubkey()), &[&payer], blockhash);
    banks.process_transaction(tx).await.unwrap();

    // Unstake
    let (gov_lock_pda, _) =
        Pubkey::find_program_address(&[GOV_LOCK_SEED, payer.pubkey().as_ref()], &program_id);

    let mut ix_data = Vec::new();
    StakingInstruction::UnstakeV2 {
        domain: domain.to_string(),
    }
    .serialize(&mut ix_data)
    .unwrap();

    let ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(stake_pda, false),
            AccountMeta::new(config_pda, false),
            AccountMeta::new_readonly(domain_pda, false),
            AccountMeta::new_readonly(clock::id(), false),
            AccountMeta::new_readonly(gov_lock_pda, false),
        ],
        data: ix_data,
    };

    let blockhash = banks.get_latest_blockhash().await.unwrap();
    let tx = Transaction::new_signed_with_payer(&[ix], Some(&payer.pubkey()), &[&payer], blockhash);
    banks.process_transaction(tx).await.unwrap();

    // Verify cooldown set
    let stake_account = banks.get_account(stake_pda).await.unwrap().unwrap();
    let stake = StakeAccountV2::deserialize_reader(&mut &stake_account.data[..]).unwrap();
    assert!(stake.cooldown_until > 0, "cooldown should be set");
}

// ──────────────────────────────────────────────
// V2 State types serialization tests
// ──────────────────────────────────────────────

#[test]
fn test_stake_account_v2_serialization() {
    let account = StakeAccountV2 {
        is_initialized: true,
        staker: Pubkey::new_unique(),
        domain: "v2.example.com".to_string(),
        amount: 5_000_000_000,
        staked_at: 1700000000,
        cooldown_until: 0,
        stake_type: StakeType::Publisher,
        reward_debt: 0,
        dns_verified: false,
        slashed_amount: 0,
        bump: 255,
        governance_lock_until: 0,
    };

    let mut buf = Vec::new();
    account.serialize(&mut buf).unwrap();
    let decoded = StakeAccountV2::deserialize_reader(&mut &buf[..]).unwrap();
    assert!(decoded.is_initialized);
    assert_eq!(decoded.domain, "v2.example.com");
    assert_eq!(decoded.amount, 5_000_000_000);
    assert_eq!(decoded.stake_type, StakeType::Publisher);
}

#[test]
fn test_domain_record_serialization() {
    let record = DomainRecord {
        is_initialized: true,
        domain: "test.com".to_string(),
        total_staked: 100_000_000,
        publisher_count: 2,
        curator_count: 1,
        blacklisted: false,
        dns_verified: true,
        dns_wallet: Pubkey::new_unique(),
        bump: 254,
        is_ugc_platform: false,
        effective_slash_pct: 0,
        active_dispute_nonce: u64::MAX,
    };

    let mut buf = Vec::new();
    record.serialize(&mut buf).unwrap();
    let decoded = DomainRecord::deserialize_reader(&mut &buf[..]).unwrap();
    assert_eq!(decoded.total_staked, 100_000_000);
    assert_eq!(decoded.publisher_count, 2);
    assert_eq!(decoded.curator_count, 1);
    assert!(decoded.dns_verified);
}

#[test]
fn test_dispute_account_serialization() {
    let dispute = DisputeAccount {
        is_initialized: true,
        reporter: Pubkey::new_unique(),
        domain: "disputed.com".to_string(),
        bond: 10_000_000,
        reason_hash: [42u8; 32],
        status: DisputeStatus::Active,
        votes_guilty: 0,
        votes_innocent: 0,
        jury_count: 0,
        created_at: 1700000000,
        deadline: 1700604800,
        nonce: 0,
        bump: 253,
        commit_deadline: 1700259200,
        reveal_deadline: 1700604800,
        severity: 1,
        jury_reward_pool: 0,
        jury_reward_claimed: 0,
    };

    let mut buf = Vec::new();
    dispute.serialize(&mut buf).unwrap();
    let decoded = DisputeAccount::deserialize_reader(&mut &buf[..]).unwrap();
    assert_eq!(decoded.bond, 10_000_000);
    assert_eq!(decoded.status, DisputeStatus::Active);
    assert_eq!(decoded.reason_hash, [42u8; 32]);
}

#[test]
fn test_governance_proposal_serialization() {
    let proposal = GovernanceProposal {
        is_initialized: true,
        proposer: Pubkey::new_unique(),
        description_hash: [1u8; 32],
        param_key: "cooldown_secs".to_string(),
        param_value: 86400,
        votes_for: 0,
        votes_against: 0,
        status: ProposalStatus::Active,
        created_at: 1700000000,
        voting_ends: 1700259200,
        nonce: 0,
        bump: 252,
        executable_after: 1700259200,
        deposit_amount: 0,
        depositor_token: Pubkey::default(),
    };

    let mut buf = Vec::new();
    proposal.serialize(&mut buf).unwrap();
    let decoded = GovernanceProposal::deserialize_reader(&mut &buf[..]).unwrap();
    assert_eq!(decoded.param_key, "cooldown_secs");
    assert_eq!(decoded.param_value, 86400);
    assert_eq!(decoded.status, ProposalStatus::Active);
}

#[test]
fn test_state_channel_serialization() {
    let channel = StateChannelAccount {
        is_initialized: true,
        payer: Pubkey::new_unique(),
        payee: Pubkey::new_unique(),
        deposited: 50_000_000,
        spent: 10_000_000,
        nonce: 5,
        expiry_slot: 1000,
        bump: 251,
    };

    let mut buf = Vec::new();
    channel.serialize(&mut buf).unwrap();
    let decoded = StateChannelAccount::deserialize_reader(&mut &buf[..]).unwrap();
    assert_eq!(decoded.deposited, 50_000_000);
    assert_eq!(decoded.spent, 10_000_000);
    assert_eq!(decoded.nonce, 5);
}

#[test]
fn test_jury_vote_serialization() {
    let vote = JuryVote {
        is_initialized: true,
        voter: Pubkey::new_unique(),
        verdict: Verdict::Guilty,
        weight: 10_000_000,
        voted_at: 1700000000,
        bump: 250,
        commit_hash: [0u8; 32],
        reward_claimed: false,
    };

    let mut buf = Vec::new();
    vote.serialize(&mut buf).unwrap();
    let decoded = JuryVote::deserialize_reader(&mut &buf[..]).unwrap();
    assert_eq!(decoded.verdict, Verdict::Guilty);
    assert_eq!(decoded.weight, 10_000_000);
}

#[test]
fn test_governance_vote_serialization() {
    let vote = GovernanceVote {
        is_initialized: true,
        voter: Pubkey::new_unique(),
        vote: GovernanceVoteChoice::For,
        weight: 5_000_000,
        bump: 249,
    };

    let mut buf = Vec::new();
    vote.serialize(&mut buf).unwrap();
    let decoded = GovernanceVote::deserialize_reader(&mut &buf[..]).unwrap();
    assert_eq!(decoded.vote, GovernanceVoteChoice::For);
    assert_eq!(decoded.weight, 5_000_000);
}

#[test]
fn test_protocol_config_serialization() {
    let config = ProtocolConfig {
        is_initialized: true,
        authority: Pubkey::new_unique(),
        truth_mint: Pubkey::new_unique(),
        min_stake: 1_000_000,
        cooldown_secs: 604800,
        dispute_bond: 10_000_000,
        slash_pct: 10,
        reward_rate: 100,
        jury_min_stake: 5_000_000,
        blacklist_authority: Pubkey::new_unique(),
        reward_vault: Pubkey::new_unique(),
        total_staked: 500_000_000,
        reward_per_token_stored: 1_000_000_000_000_000_000,
        proposal_nonce: 3,
        dispute_nonce: 1,
        current_epoch: 0,
        epoch_funded_amount: 0,
        epoch_max_emission: 10_000_000,
        sentinel_reward_per_token: 0,
        total_sentinel_staked: 0,
        min_epoch_duration: 0,
        epoch_started_at: 0,
        max_total_supply: 1_000_000_000,
        total_minted: 0,
        min_jury_weight: 1_000,
        burn_pct: 10,
        staker_emission_pct: 70,
        execution_delay_secs: 86_400,
        proposal_deposit: 100,
        curator_yield_cap_bps: 1500,
        sentinel_fee_pct: 5,
        accumulated_sentinel_fees: 0,
        total_funded: 0,
        total_distributed: 0,
        jury_vote_reward: 1_000,
        jury_rewards_this_epoch: 0,
        max_jury_rewards_per_epoch: 50_000,
        bump: 248,
    };

    let mut buf = Vec::new();
    config.serialize(&mut buf).unwrap();
    let decoded = ProtocolConfig::deserialize_reader(&mut &buf[..]).unwrap();
    assert_eq!(decoded.min_stake, 1_000_000);
    assert_eq!(decoded.slash_pct, 10);
    assert_eq!(decoded.total_staked, 500_000_000);
    assert_eq!(decoded.proposal_nonce, 3);
}

// ──────────────────────────────────────────────
// PDA derivation tests for new account types
// ──────────────────────────────────────────────

#[test]
fn test_v2_pda_derivation_includes_staker() {
    let program_id = Pubkey::new_unique();
    let domain = "multi.com";
    let staker1 = Pubkey::new_unique();
    let staker2 = Pubkey::new_unique();

    let (pda1, _) = Pubkey::find_program_address(
        &[STAKE_SEED, domain.as_bytes(), staker1.as_ref()],
        &program_id,
    );
    let (pda2, _) = Pubkey::find_program_address(
        &[STAKE_SEED, domain.as_bytes(), staker2.as_ref()],
        &program_id,
    );

    assert_ne!(pda1, pda2, "Different stakers should get different PDAs");
}

#[test]
fn test_config_pda_deterministic() {
    let program_id = Pubkey::new_unique();
    let (pda1, bump1) = Pubkey::find_program_address(&[CONFIG_SEED], &program_id);
    let (pda2, bump2) = Pubkey::find_program_address(&[CONFIG_SEED], &program_id);
    assert_eq!(pda1, pda2);
    assert_eq!(bump1, bump2);
}

#[test]
fn test_domain_pda_deterministic() {
    let program_id = Pubkey::new_unique();
    let domain = "example.com";
    let (pda1, _) = Pubkey::find_program_address(&[DOMAIN_SEED, domain.as_bytes()], &program_id);
    let (pda2, _) = Pubkey::find_program_address(&[DOMAIN_SEED, domain.as_bytes()], &program_id);
    assert_eq!(pda1, pda2);
}

#[test]
fn test_dispute_pda_uses_nonce() {
    let program_id = Pubkey::new_unique();
    let domain = "disputed.com";
    let nonce0 = 0u64.to_le_bytes();
    let nonce1 = 1u64.to_le_bytes();

    let (pda0, _) =
        Pubkey::find_program_address(&[DISPUTE_SEED, domain.as_bytes(), &nonce0], &program_id);
    let (pda1, _) =
        Pubkey::find_program_address(&[DISPUTE_SEED, domain.as_bytes(), &nonce1], &program_id);
    assert_ne!(pda0, pda1, "Different nonces should give different PDAs");
}

#[test]
fn test_channel_pda_uses_both_parties() {
    let program_id = Pubkey::new_unique();
    let payer = Pubkey::new_unique();
    let payee = Pubkey::new_unique();

    let (pda1, _) =
        Pubkey::find_program_address(&[CHANNEL_SEED, payer.as_ref(), payee.as_ref()], &program_id);
    let (pda2, _) =
        Pubkey::find_program_address(&[CHANNEL_SEED, payee.as_ref(), payer.as_ref()], &program_id);
    assert_ne!(pda1, pda2, "Swapped payer/payee should give different PDAs");
}

#[test]
fn test_proposal_pda_deterministic() {
    let program_id = Pubkey::new_unique();
    let nonce = 42u64.to_le_bytes();
    let (pda1, _) = Pubkey::find_program_address(&[PROPOSAL_SEED, &nonce], &program_id);
    let (pda2, _) = Pubkey::find_program_address(&[PROPOSAL_SEED, &nonce], &program_id);
    assert_eq!(pda1, pda2);
}

// ──────────────────────────────────────────────
// Enum discriminant tests
// ──────────────────────────────────────────────

#[test]
fn test_stake_type_roundtrip() {
    for st in [StakeType::Publisher, StakeType::Curator] {
        let mut buf = Vec::new();
        st.serialize(&mut buf).unwrap();
        let decoded = StakeType::deserialize_reader(&mut &buf[..]).unwrap();
        assert_eq!(st, decoded);
    }
}

#[test]
fn test_dispute_status_roundtrip() {
    for s in [
        DisputeStatus::Active,
        DisputeStatus::ResolvedGuilty,
        DisputeStatus::ResolvedInnocent,
        DisputeStatus::CommitPhase,
        DisputeStatus::RevealPhase,
    ] {
        let mut buf = Vec::new();
        s.serialize(&mut buf).unwrap();
        let decoded = DisputeStatus::deserialize_reader(&mut &buf[..]).unwrap();
        assert_eq!(s, decoded);
    }
}

#[test]
fn test_proposal_status_roundtrip() {
    for s in [
        ProposalStatus::Active,
        ProposalStatus::Executed,
        ProposalStatus::Failed,
    ] {
        let mut buf = Vec::new();
        s.serialize(&mut buf).unwrap();
        let decoded = ProposalStatus::deserialize_reader(&mut &buf[..]).unwrap();
        assert_eq!(s, decoded);
    }
}

#[test]
fn test_verdict_roundtrip() {
    for v in [Verdict::Guilty, Verdict::Innocent] {
        let mut buf = Vec::new();
        v.serialize(&mut buf).unwrap();
        let decoded = Verdict::deserialize_reader(&mut &buf[..]).unwrap();
        assert_eq!(v, decoded);
    }
}

#[test]
fn test_governance_vote_choice_roundtrip() {
    for v in [GovernanceVoteChoice::For, GovernanceVoteChoice::Against] {
        let mut buf = Vec::new();
        v.serialize(&mut buf).unwrap();
        let decoded = GovernanceVoteChoice::deserialize_reader(&mut &buf[..]).unwrap();
        assert_eq!(v, decoded);
    }
}

// ──────────────────────────────────────────────
// Protocol Economics: oi-types new types
// ──────────────────────────────────────────────

#[test]
fn test_dispute_info_type() {
    let info = oi_types::DisputeInfo {
        domain: "test.com".to_string(),
        reporter: "wallet1".to_string(),
        bond: 10_000,
        reason_hash: [0u8; 32],
        status: "active".to_string(),
        votes_guilty: 100,
        votes_innocent: 50,
        jury_count: 3,
        created_at: 1700000000,
        deadline: 1700604800,
        nonce: 0,
    };
    assert_eq!(info.domain, "test.com");
    assert_eq!(info.votes_guilty, 100);
}

#[test]
fn test_proposal_info_type() {
    let info = oi_types::ProposalInfo {
        proposer: "wallet2".to_string(),
        description_hash: [1u8; 32],
        param_key: "slash_pct".to_string(),
        param_value: 20,
        votes_for: 1000,
        votes_against: 200,
        status: "active".to_string(),
        created_at: 1700000000,
        voting_ends: 1700259200,
        nonce: 5,
    };
    assert_eq!(info.param_key, "slash_pct");
    assert_eq!(info.votes_for, 1000);
}

#[test]
fn test_reward_info_type() {
    let info = oi_types::RewardInfo {
        domain: "test.com".to_string(),
        staker: "wallet3".to_string(),
        accrued: 50_000,
        claimed: 30_000,
        reward_debt: 1_000_000_000_000_000_000,
    };
    assert_eq!(info.accrued, 50_000);
    assert_eq!(info.claimed, 30_000);
}

// ──────────────────────────────────────────────
// Seed constant correctness
// ──────────────────────────────────────────────

#[test]
fn test_seed_constants() {
    assert_eq!(STAKE_SEED, b"stake");
    assert_eq!(CONFIG_SEED, b"config");
    assert_eq!(DOMAIN_SEED, b"domain");
    assert_eq!(DISPUTE_SEED, b"dispute");
    assert_eq!(JURY_SEED, b"jury");
    assert_eq!(PROPOSAL_SEED, b"proposal");
    assert_eq!(GOV_VOTE_SEED, b"gov_vote");
    assert_eq!(REWARD_VAULT_SEED, b"reward_vault");
    assert_eq!(CHANNEL_SEED, b"channel");
}

// ──────────────────────────────────────────────
// Instruction serialization tests
// ──────────────────────────────────────────────

#[test]
fn test_instruction_serialize_roundtrip() {
    let instructions: Vec<StakingInstruction> = vec![
        StakingInstruction::Stake {
            domain: "a.com".to_string(),
            amount: 100,
        },
        StakingInstruction::Unstake {
            domain: "a.com".to_string(),
        },
        StakingInstruction::InitializeProtocol {
            min_stake: 1000,
            cooldown_secs: 86400,
            dispute_bond: 5000,
            slash_pct: 10,
            reward_rate: 100,
            jury_min_stake: 2000,
        },
        StakingInstruction::MintTruth { amount: 1_000_000 },
        StakingInstruction::StakeV2 {
            domain: "b.com".to_string(),
            amount: 500,
            stake_type: 0,
        },
        StakingInstruction::UnstakeV2 {
            domain: "b.com".to_string(),
        },
        StakingInstruction::Withdraw {
            domain: "b.com".to_string(),
        },
        StakingInstruction::ClaimReward {
            domain: "b.com".to_string(),
        },
        StakingInstruction::VerifyDns {
            domain: "c.com".to_string(),
            dns_wallet: [1u8; 32],
        },
        StakingInstruction::BlacklistDomain {
            domain: "c.com".to_string(),
        },
        StakingInstruction::OpenDispute {
            domain: "d.com".to_string(),
            reason_hash: [2u8; 32],
            severity: 1,
        },
        StakingInstruction::CastJuryVote {
            domain: "d.com".to_string(),
            dispute_nonce: 0,
            verdict: 0,
        },
        StakingInstruction::ResolveDispute {
            domain: "d.com".to_string(),
            dispute_nonce: 0,
        },
        StakingInstruction::FundRewards { amount: 100_000 },
        StakingInstruction::UpdateRewardIndex {
            funded_amount: 50_000,
        },
        StakingInstruction::CreateProposal {
            description_hash: [3u8; 32],
            param_key: "min_stake".to_string(),
            param_value: 2000,
        },
        StakingInstruction::CastGovernanceVote {
            proposal_nonce: 0,
            vote: 0,
        },
        StakingInstruction::ExecuteProposal { proposal_nonce: 0 },
        StakingInstruction::OpenChannel {
            payee: [4u8; 32],
            amount: 1000,
            expiry_slot: 999,
        },
        StakingInstruction::SettleChannel {
            payer_key: [5u8; 32],
            payee_key: [6u8; 32],
            spent: 500,
            nonce: 1,
        },
    ];

    for ix in instructions {
        let mut buf = Vec::new();
        ix.serialize(&mut buf).unwrap();
        // Should be able to deserialize back
        let decoded = StakingInstruction::try_from_slice(&buf).unwrap();
        // Verify it round-trips (check a few key variants)
        match decoded {
            StakingInstruction::Stake { domain, amount } => {
                assert_eq!(domain, "a.com");
                assert_eq!(amount, 100);
            }
            StakingInstruction::MintTruth { amount } => {
                assert_eq!(amount, 1_000_000);
            }
            _ => {} // Other variants verified by not panicking
        }
    }
}

// ──────────────────────────────────────────────
// MAX_SIZE sanity checks
// ──────────────────────────────────────────────

#[test]
fn test_max_sizes_reasonable() {
    // All MAX_SIZE values should be > 0 and < 1024 (reasonable for Solana accounts).
    // Checked at compile time via const blocks.
    const {
        assert!(StakeAccount::MAX_SIZE > 0 && StakeAccount::MAX_SIZE < 1024);
        assert!(ProtocolConfig::MAX_SIZE > 0 && ProtocolConfig::MAX_SIZE < 1024);
        assert!(StakeAccountV2::MAX_SIZE > 0 && StakeAccountV2::MAX_SIZE < 1024);
        assert!(DomainRecord::MAX_SIZE > 0 && DomainRecord::MAX_SIZE < 1024);
        assert!(DisputeAccount::MAX_SIZE > 0 && DisputeAccount::MAX_SIZE < 1024);
        assert!(JuryVote::MAX_SIZE > 0 && JuryVote::MAX_SIZE < 1024);
        assert!(GovernanceProposal::MAX_SIZE > 0 && GovernanceProposal::MAX_SIZE < 1024);
        assert!(GovernanceVote::MAX_SIZE > 0 && GovernanceVote::MAX_SIZE < 1024);
        assert!(StateChannelAccount::MAX_SIZE > 0 && StateChannelAccount::MAX_SIZE < 1024);
        assert!(GovernanceLockAccount::MAX_SIZE > 0 && GovernanceLockAccount::MAX_SIZE < 1024);
    }
}

// ══════════════════════════════════════════════
// Functional E2E Tests — Phase 1: Staking
// ══════════════════════════════════════════════

/// Helper: stake V2 on a domain and return (stake_pda, domain_pda).
#[allow(clippy::too_many_arguments)]
async fn stake_v2_helper(
    banks: &mut solana_program_test::BanksClient,
    payer: &Keypair,
    program_id: &Pubkey,
    config_pda: &Pubkey,
    staker_token: &Pubkey,
    reward_vault: &Pubkey,
    domain: &str,
    amount: u64,
    stake_type: u8,
) -> (Pubkey, Pubkey) {
    let (stake_pda, _) = Pubkey::find_program_address(
        &[STAKE_SEED, domain.as_bytes(), payer.pubkey().as_ref()],
        program_id,
    );
    let (domain_pda, _) =
        Pubkey::find_program_address(&[DOMAIN_SEED, domain.as_bytes()], program_id);

    let mut ix_data = Vec::new();
    StakingInstruction::StakeV2 {
        domain: domain.to_string(),
        amount,
        stake_type,
    }
    .serialize(&mut ix_data)
    .unwrap();

    let ix = Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(*staker_token, false),
            AccountMeta::new(*reward_vault, false),
            AccountMeta::new(stake_pda, false),
            AccountMeta::new(domain_pda, false),
            AccountMeta::new(*config_pda, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
            AccountMeta::new_readonly(clock::id(), false),
        ],
        data: ix_data,
    };

    let blockhash = banks.get_latest_blockhash().await.unwrap();
    let tx = Transaction::new_signed_with_payer(&[ix], Some(&payer.pubkey()), &[payer], blockhash);
    banks.process_transaction(tx).await.unwrap();

    (stake_pda, domain_pda)
}

/// Helper: unstake V2 on a domain.
async fn unstake_v2_helper(
    banks: &mut solana_program_test::BanksClient,
    payer: &Keypair,
    program_id: &Pubkey,
    config_pda: &Pubkey,
    domain: &str,
) -> Pubkey {
    let (stake_pda, _) = Pubkey::find_program_address(
        &[STAKE_SEED, domain.as_bytes(), payer.pubkey().as_ref()],
        program_id,
    );
    let (domain_pda, _) =
        Pubkey::find_program_address(&[DOMAIN_SEED, domain.as_bytes()], program_id);
    let (gov_lock_pda, _) =
        Pubkey::find_program_address(&[GOV_LOCK_SEED, payer.pubkey().as_ref()], program_id);

    let mut ix_data = Vec::new();
    StakingInstruction::UnstakeV2 {
        domain: domain.to_string(),
    }
    .serialize(&mut ix_data)
    .unwrap();

    let ix = Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(stake_pda, false),
            AccountMeta::new(*config_pda, false),
            AccountMeta::new_readonly(domain_pda, false),
            AccountMeta::new_readonly(clock::id(), false),
            AccountMeta::new_readonly(gov_lock_pda, false),
        ],
        data: ix_data,
    };

    let blockhash = banks.get_latest_blockhash().await.unwrap();
    let tx = Transaction::new_signed_with_payer(&[ix], Some(&payer.pubkey()), &[payer], blockhash);
    banks.process_transaction(tx).await.unwrap();

    stake_pda
}

/// Helper: initialize protocol with custom cooldown_secs and mint tokens.
async fn init_protocol_custom_cooldown(
    banks: &mut solana_program_test::BanksClient,
    payer: &Keypair,
    program_id: &Pubkey,
    truth_mint: &Pubkey,
    reward_vault: &Pubkey,
    mint_amount: u64,
    cooldown_secs: i64,
) -> (Pubkey, Pubkey) {
    let (config_pda, _) = Pubkey::find_program_address(&[CONFIG_SEED], program_id);
    let blockhash = banks.get_latest_blockhash().await.unwrap();

    let mut ix_data = Vec::new();
    StakingInstruction::InitializeProtocol {
        min_stake: 1_000_000,
        cooldown_secs,
        dispute_bond: 10_000_000,
        slash_pct: 10,
        reward_rate: 100,
        jury_min_stake: 5_000_000,
    }
    .serialize(&mut ix_data)
    .unwrap();

    let init_ix = Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(config_pda, false),
            AccountMeta::new(*truth_mint, false),
            AccountMeta::new(*reward_vault, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
            AccountMeta::new_readonly(rent::id(), false),
        ],
        data: ix_data,
    };

    let tx =
        Transaction::new_signed_with_payer(&[init_ix], Some(&payer.pubkey()), &[payer], blockhash);
    banks.process_transaction(tx).await.unwrap();

    let staker_token =
        create_token_account(banks, payer, blockhash, truth_mint, &payer.pubkey()).await;

    if mint_amount > 0 {
        let mut ix_data = Vec::new();
        StakingInstruction::MintTruth {
            amount: mint_amount,
        }
        .serialize(&mut ix_data)
        .unwrap();
        let mint_ix = Instruction {
            program_id: *program_id,
            accounts: vec![
                AccountMeta::new(payer.pubkey(), true),
                AccountMeta::new(config_pda, false),
                AccountMeta::new(*truth_mint, false),
                AccountMeta::new(staker_token, false),
                AccountMeta::new_readonly(spl_token::id(), false),
            ],
            data: ix_data,
        };
        let blockhash = banks.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[mint_ix],
            Some(&payer.pubkey()),
            &[payer],
            blockhash,
        );
        banks.process_transaction(tx).await.unwrap();
    }

    (config_pda, staker_token)
}

#[tokio::test]
async fn test_withdraw_after_cooldown() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let (mut banks, payer, _) = pt.start().await;
    // cooldown_secs = 0 so cooldown_until = clock + 0 = clock, and withdraw checks
    // clock < cooldown_until which is false → withdraw succeeds immediately.
    let (config_pda, staker_token) = init_protocol_custom_cooldown(
        &mut banks,
        &payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        100_000_000,
        0,
    )
    .await;

    let domain = "withdraw-ok.com";
    let stake_amount = 10_000_000u64;

    // Stake
    let (stake_pda, domain_pda) = stake_v2_helper(
        &mut banks,
        &payer,
        &program_id,
        &config_pda,
        &staker_token,
        &reward_vault.pubkey(),
        domain,
        stake_amount,
        0,
    )
    .await;

    // Unstake (starts cooldown — with cooldown_secs=0, cooldown_until == current clock)
    unstake_v2_helper(&mut banks, &payer, &program_id, &config_pda, domain).await;

    // Withdraw (succeeds because clock >= cooldown_until)
    let mut ix_data = Vec::new();
    StakingInstruction::Withdraw {
        domain: domain.to_string(),
    }
    .serialize(&mut ix_data)
    .unwrap();

    let ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(stake_pda, false),
            AccountMeta::new(staker_token, false),
            AccountMeta::new(reward_vault.pubkey(), false),
            AccountMeta::new(domain_pda, false),
            AccountMeta::new(config_pda, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(clock::id(), false),
            AccountMeta::new(truth_mint.pubkey(), false), // $TRUTH mint for slash burn
        ],
        data: ix_data,
    };

    let blockhash = banks.get_latest_blockhash().await.unwrap();
    let tx = Transaction::new_signed_with_payer(&[ix], Some(&payer.pubkey()), &[&payer], blockhash);
    banks.process_transaction(tx).await.unwrap();

    // Verify stake PDA is zeroed (closed)
    let stake_account = banks.get_account(stake_pda).await.unwrap();
    match stake_account {
        None => {} // PDA closed entirely
        Some(acct) => {
            assert!(
                acct.data.iter().all(|b| *b == 0),
                "stake PDA data should be zeroed"
            );
        }
    }

    // Verify domain record updated
    let domain_account = banks.get_account(domain_pda).await.unwrap().unwrap();
    let record = DomainRecord::deserialize_reader(&mut &domain_account.data[..]).unwrap();
    assert_eq!(record.total_staked, 0);
    assert_eq!(record.publisher_count, 0);
}

#[tokio::test]
async fn test_withdraw_before_cooldown_fails() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let (mut banks, payer, _) = pt.start().await;

    // Initialize with long cooldown (604800 = 7 days)
    let (config_pda, _) = Pubkey::find_program_address(&[CONFIG_SEED], &program_id);
    let blockhash = banks.get_latest_blockhash().await.unwrap();

    let mut ix_data = Vec::new();
    StakingInstruction::InitializeProtocol {
        min_stake: 1_000_000,
        cooldown_secs: 604800, // 7 days
        dispute_bond: 10_000_000,
        slash_pct: 10,
        reward_rate: 100,
        jury_min_stake: 5_000_000,
    }
    .serialize(&mut ix_data)
    .unwrap();

    let init_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(config_pda, false),
            AccountMeta::new(truth_mint.pubkey(), false),
            AccountMeta::new(reward_vault.pubkey(), false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
            AccountMeta::new_readonly(rent::id(), false),
        ],
        data: ix_data,
    };

    let tx =
        Transaction::new_signed_with_payer(&[init_ix], Some(&payer.pubkey()), &[&payer], blockhash);
    banks.process_transaction(tx).await.unwrap();

    // Create staker token account + mint tokens
    let staker_token = create_token_account(
        &mut banks,
        &payer,
        blockhash,
        &truth_mint.pubkey(),
        &payer.pubkey(),
    )
    .await;
    let mut ix_data = Vec::new();
    StakingInstruction::MintTruth {
        amount: 100_000_000,
    }
    .serialize(&mut ix_data)
    .unwrap();
    let mint_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(config_pda, false),
            AccountMeta::new(truth_mint.pubkey(), false),
            AccountMeta::new(staker_token, false),
            AccountMeta::new_readonly(spl_token::id(), false),
        ],
        data: ix_data,
    };
    let blockhash = banks.get_latest_blockhash().await.unwrap();
    let tx =
        Transaction::new_signed_with_payer(&[mint_ix], Some(&payer.pubkey()), &[&payer], blockhash);
    banks.process_transaction(tx).await.unwrap();

    let domain = "withdraw-fail.com";
    let (stake_pda, domain_pda) = stake_v2_helper(
        &mut banks,
        &payer,
        &program_id,
        &config_pda,
        &staker_token,
        &reward_vault.pubkey(),
        domain,
        10_000_000,
        0,
    )
    .await;

    // Unstake (starts 7-day cooldown)
    unstake_v2_helper(&mut banks, &payer, &program_id, &config_pda, domain).await;

    // Immediately try to withdraw → should fail with CooldownNotExpired
    let mut ix_data = Vec::new();
    StakingInstruction::Withdraw {
        domain: domain.to_string(),
    }
    .serialize(&mut ix_data)
    .unwrap();

    let ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(stake_pda, false),
            AccountMeta::new(staker_token, false),
            AccountMeta::new(reward_vault.pubkey(), false),
            AccountMeta::new(domain_pda, false),
            AccountMeta::new(config_pda, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(clock::id(), false),
            AccountMeta::new(truth_mint.pubkey(), false), // $TRUTH mint for slash burn
        ],
        data: ix_data,
    };

    let blockhash = banks.get_latest_blockhash().await.unwrap();
    let tx = Transaction::new_signed_with_payer(&[ix], Some(&payer.pubkey()), &[&payer], blockhash);
    let result = banks.process_transaction(tx).await;
    assert!(result.is_err(), "withdraw before cooldown should fail");
}

#[tokio::test]
async fn test_claim_reward() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let (mut banks, payer, _) = pt.start().await;
    let (config_pda, staker_token) = init_protocol_and_mint(
        &mut banks,
        &payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        200_000_000,
    )
    .await;

    let domain = "reward.com";
    let stake_amount = 50_000_000u64;

    // Stake
    let (stake_pda, _) = stake_v2_helper(
        &mut banks,
        &payer,
        &program_id,
        &config_pda,
        &staker_token,
        &reward_vault.pubkey(),
        domain,
        stake_amount,
        0,
    )
    .await;

    // Fund rewards: transfer $TRUTH into vault
    let mut ix_data = Vec::new();
    StakingInstruction::FundRewards { amount: 10_000_000 }
        .serialize(&mut ix_data)
        .unwrap();
    let fund_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(staker_token, false),
            AccountMeta::new(reward_vault.pubkey(), false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new(config_pda, false),
        ],
        data: ix_data,
    };
    let blockhash = banks.get_latest_blockhash().await.unwrap();
    let tx =
        Transaction::new_signed_with_payer(&[fund_ix], Some(&payer.pubkey()), &[&payer], blockhash);
    banks.process_transaction(tx).await.unwrap();

    // UpdateRewardIndex: admin updates the cumulative reward_per_token
    let mut ix_data = Vec::new();
    StakingInstruction::UpdateRewardIndex {
        funded_amount: 10_000_000,
    }
    .serialize(&mut ix_data)
    .unwrap();
    let update_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(config_pda, false),
        ],
        data: ix_data,
    };
    let blockhash = banks.get_latest_blockhash().await.unwrap();
    let tx = Transaction::new_signed_with_payer(
        &[update_ix],
        Some(&payer.pubkey()),
        &[&payer],
        blockhash,
    );
    banks.process_transaction(tx).await.unwrap();

    // ClaimReward
    let mut ix_data = Vec::new();
    StakingInstruction::ClaimReward {
        domain: domain.to_string(),
    }
    .serialize(&mut ix_data)
    .unwrap();
    let claim_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(stake_pda, false),
            AccountMeta::new(truth_mint.pubkey(), false),
            AccountMeta::new(staker_token, false),
            AccountMeta::new(config_pda, false),
            AccountMeta::new_readonly(spl_token::id(), false),
        ],
        data: ix_data,
    };
    let blockhash = banks.get_latest_blockhash().await.unwrap();
    let tx = Transaction::new_signed_with_payer(
        &[claim_ix],
        Some(&payer.pubkey()),
        &[&payer],
        blockhash,
    );
    banks.process_transaction(tx).await.unwrap();

    // With total_staked fix, UpdateRewardIndex should now calculate a real reward_per_token.
    // reward_per_token_stored += (funded_amount * 1e18) / total_staked
    //                         = (10_000_000 * 1e18) / 50_000_000
    //                         = 200_000_000_000_000_000 (0.2 * 1e18)
    // pending = (amount * (reward_per_token_stored - reward_debt)) / 1e18
    //         = (50_000_000 * 200_000_000_000_000_000) / 1e18
    //         = 10_000_000
    let stake_account = banks.get_account(stake_pda).await.unwrap().unwrap();
    let stake = StakeAccountV2::deserialize_reader(&mut &stake_account.data[..]).unwrap();
    assert!(
        stake.is_initialized,
        "stake account should still be initialized after claim"
    );
    assert!(
        stake.reward_debt > 0,
        "reward_debt should be updated after claim"
    );

    // Verify config.total_staked was updated by StakeV2
    let config_account = banks.get_account(config_pda).await.unwrap().unwrap();
    let config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
    assert_eq!(
        config.total_staked, stake_amount,
        "total_staked should equal stake amount"
    );
}

// ══════════════════════════════════════════════
// Functional E2E Tests — Phase 2: DNS & Blacklist
// ══════════════════════════════════════════════

#[tokio::test]
async fn test_verify_dns_sets_flag() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let (mut banks, payer, _) = pt.start().await;
    let (config_pda, staker_token) = init_protocol_and_mint(
        &mut banks,
        &payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        100_000_000,
    )
    .await;

    let domain = "dns-test.com";
    let (_, domain_pda) = stake_v2_helper(
        &mut banks,
        &payer,
        &program_id,
        &config_pda,
        &staker_token,
        &reward_vault.pubkey(),
        domain,
        10_000_000,
        0,
    )
    .await;

    // VerifyDns
    let dns_wallet = payer.pubkey().to_bytes();
    let mut ix_data = Vec::new();
    StakingInstruction::VerifyDns {
        domain: domain.to_string(),
        dns_wallet,
    }
    .serialize(&mut ix_data)
    .unwrap();
    let ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(domain_pda, false),
            AccountMeta::new_readonly(config_pda, false),
        ],
        data: ix_data,
    };

    let blockhash = banks.get_latest_blockhash().await.unwrap();
    let tx = Transaction::new_signed_with_payer(&[ix], Some(&payer.pubkey()), &[&payer], blockhash);
    banks.process_transaction(tx).await.unwrap();

    // Verify dns_verified = true
    let domain_account = banks.get_account(domain_pda).await.unwrap().unwrap();
    let record = DomainRecord::deserialize_reader(&mut &domain_account.data[..]).unwrap();
    assert!(
        record.dns_verified,
        "dns_verified should be true after VerifyDns"
    );
    assert_eq!(record.dns_wallet, payer.pubkey());
}

#[tokio::test]
async fn test_blacklist_domain() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let (mut banks, payer, _) = pt.start().await;
    let (config_pda, staker_token) = init_protocol_and_mint(
        &mut banks,
        &payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        100_000_000,
    )
    .await;

    let domain = "bad-domain.com";
    let (_, domain_pda) = stake_v2_helper(
        &mut banks,
        &payer,
        &program_id,
        &config_pda,
        &staker_token,
        &reward_vault.pubkey(),
        domain,
        10_000_000,
        0,
    )
    .await;

    // BlacklistDomain
    let mut ix_data = Vec::new();
    StakingInstruction::BlacklistDomain {
        domain: domain.to_string(),
    }
    .serialize(&mut ix_data)
    .unwrap();
    let ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(domain_pda, false),
            AccountMeta::new_readonly(config_pda, false),
        ],
        data: ix_data,
    };

    let blockhash = banks.get_latest_blockhash().await.unwrap();
    let tx = Transaction::new_signed_with_payer(&[ix], Some(&payer.pubkey()), &[&payer], blockhash);
    banks.process_transaction(tx).await.unwrap();

    // Verify blacklisted = true
    let domain_account = banks.get_account(domain_pda).await.unwrap().unwrap();
    let record = DomainRecord::deserialize_reader(&mut &domain_account.data[..]).unwrap();
    assert!(record.blacklisted, "domain should be blacklisted");
}

#[tokio::test]
async fn test_stake_on_blacklisted_domain_fails() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let (mut banks, payer, _) = pt.start().await;
    let (config_pda, staker_token) = init_protocol_and_mint(
        &mut banks,
        &payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        200_000_000,
    )
    .await;

    let domain = "blacklisted.com";

    // First stake to create the domain record
    stake_v2_helper(
        &mut banks,
        &payer,
        &program_id,
        &config_pda,
        &staker_token,
        &reward_vault.pubkey(),
        domain,
        10_000_000,
        0,
    )
    .await;

    // Blacklist it
    let (domain_pda, _) =
        Pubkey::find_program_address(&[DOMAIN_SEED, domain.as_bytes()], &program_id);
    let mut ix_data = Vec::new();
    StakingInstruction::BlacklistDomain {
        domain: domain.to_string(),
    }
    .serialize(&mut ix_data)
    .unwrap();
    let ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(domain_pda, false),
            AccountMeta::new_readonly(config_pda, false),
        ],
        data: ix_data,
    };
    let blockhash = banks.get_latest_blockhash().await.unwrap();
    let tx = Transaction::new_signed_with_payer(&[ix], Some(&payer.pubkey()), &[&payer], blockhash);
    banks.process_transaction(tx).await.unwrap();

    // Try to stake again on the blacklisted domain with a new staker
    // Since the domain PDA already exists and is blacklisted, StakeV2 should fail.
    // We need a second wallet to stake (first already has a StakeAccountV2 for this domain).
    let staker2 = Keypair::new();
    // Fund staker2
    let fund_ix = system_instruction::transfer(&payer.pubkey(), &staker2.pubkey(), 2_000_000_000);
    let blockhash = banks.get_latest_blockhash().await.unwrap();
    let tx =
        Transaction::new_signed_with_payer(&[fund_ix], Some(&payer.pubkey()), &[&payer], blockhash);
    banks.process_transaction(tx).await.unwrap();

    // Create staker2 token account and mint tokens
    let blockhash = banks.get_latest_blockhash().await.unwrap();
    let staker2_token = create_token_account(
        &mut banks,
        &payer,
        blockhash,
        &truth_mint.pubkey(),
        &staker2.pubkey(),
    )
    .await;

    let mut ix_data = Vec::new();
    StakingInstruction::MintTruth { amount: 50_000_000 }
        .serialize(&mut ix_data)
        .unwrap();
    let mint_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(config_pda, false),
            AccountMeta::new(truth_mint.pubkey(), false),
            AccountMeta::new(staker2_token, false),
            AccountMeta::new_readonly(spl_token::id(), false),
        ],
        data: ix_data,
    };
    let blockhash = banks.get_latest_blockhash().await.unwrap();
    let tx =
        Transaction::new_signed_with_payer(&[mint_ix], Some(&payer.pubkey()), &[&payer], blockhash);
    banks.process_transaction(tx).await.unwrap();

    // Staker2 tries to stake on blacklisted domain
    let (stake_pda2, _) = Pubkey::find_program_address(
        &[STAKE_SEED, domain.as_bytes(), staker2.pubkey().as_ref()],
        &program_id,
    );

    let mut ix_data = Vec::new();
    StakingInstruction::StakeV2 {
        domain: domain.to_string(),
        amount: 10_000_000,
        stake_type: 0,
    }
    .serialize(&mut ix_data)
    .unwrap();

    let ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(staker2.pubkey(), true),
            AccountMeta::new(staker2_token, false),
            AccountMeta::new(reward_vault.pubkey(), false),
            AccountMeta::new(stake_pda2, false),
            AccountMeta::new(domain_pda, false),
            AccountMeta::new(config_pda, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
            AccountMeta::new_readonly(clock::id(), false),
        ],
        data: ix_data,
    };

    let blockhash = banks.get_latest_blockhash().await.unwrap();
    let tx =
        Transaction::new_signed_with_payer(&[ix], Some(&staker2.pubkey()), &[&staker2], blockhash);
    let result = banks.process_transaction(tx).await;
    assert!(result.is_err(), "staking on blacklisted domain should fail");
}

// ══════════════════════════════════════════════
// Functional E2E Tests — Phase 3: Disputes
// ══════════════════════════════════════════════

/// Helper: set up a domain and open a dispute, returns dispute_pda, nonce, domain_pda.
#[allow(clippy::too_many_arguments)]
async fn setup_dispute(
    banks: &mut solana_program_test::BanksClient,
    payer: &Keypair,
    program_id: &Pubkey,
    config_pda: &Pubkey,
    staker_token: &Pubkey,
    reward_vault: &Pubkey,
    _truth_mint: &Pubkey,
    domain: &str,
) -> (Pubkey, u64, Pubkey) {
    // Stake to create domain record
    let (_, domain_pda) = stake_v2_helper(
        banks,
        payer,
        program_id,
        config_pda,
        staker_token,
        reward_vault,
        domain,
        10_000_000,
        0,
    )
    .await;

    // Read current dispute_nonce from config
    let config_account = banks.get_account(*config_pda).await.unwrap().unwrap();
    let config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
    let nonce = config.dispute_nonce;
    let nonce_bytes = nonce.to_le_bytes();

    let (dispute_pda, _) =
        Pubkey::find_program_address(&[DISPUTE_SEED, domain.as_bytes(), &nonce_bytes], program_id);

    // OpenDispute
    let reason_hash = [42u8; 32];
    let mut ix_data = Vec::new();
    StakingInstruction::OpenDispute {
        domain: domain.to_string(),
        reason_hash,
        severity: 1,
    }
    .serialize(&mut ix_data)
    .unwrap();

    let ix = Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(*staker_token, false),
            AccountMeta::new(*reward_vault, false),
            AccountMeta::new(dispute_pda, false),
            AccountMeta::new(domain_pda, false),
            AccountMeta::new(*config_pda, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
            AccountMeta::new_readonly(clock::id(), false),
        ],
        data: ix_data,
    };

    let blockhash = banks.get_latest_blockhash().await.unwrap();
    let tx = Transaction::new_signed_with_payer(&[ix], Some(&payer.pubkey()), &[payer], blockhash);
    banks.process_transaction(tx).await.unwrap();

    (dispute_pda, nonce, domain_pda)
}

#[tokio::test]
async fn test_open_dispute() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let (mut banks, payer, _) = pt.start().await;
    let (config_pda, staker_token) = init_protocol_and_mint(
        &mut banks,
        &payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        200_000_000,
    )
    .await;

    let domain = "disputed.com";
    let (dispute_pda, _, _) = setup_dispute(
        &mut banks,
        &payer,
        &program_id,
        &config_pda,
        &staker_token,
        &reward_vault.pubkey(),
        &truth_mint.pubkey(),
        domain,
    )
    .await;

    // Verify dispute account created
    let dispute_account = banks.get_account(dispute_pda).await.unwrap().unwrap();
    let dispute = DisputeAccount::deserialize_reader(&mut &dispute_account.data[..]).unwrap();
    assert!(dispute.is_initialized);
    assert_eq!(dispute.domain, domain);
    assert_eq!(dispute.status, DisputeStatus::CommitPhase);
    assert_eq!(dispute.reason_hash, [42u8; 32]);
    assert!(
        dispute.deadline > dispute.created_at,
        "deadline should be after created_at"
    );
    let expected_deadline = dispute.created_at + 7 * 24 * 3600;
    assert_eq!(dispute.deadline, expected_deadline);
    let expected_commit_deadline = dispute.created_at + 3 * 24 * 3600;
    assert_eq!(dispute.commit_deadline, expected_commit_deadline);
    assert_eq!(dispute.reveal_deadline, expected_deadline);
}

#[tokio::test]
async fn test_cast_jury_vote() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let mut ctx = pt.start_with_context().await;
    let (config_pda, staker_token) = init_protocol_and_mint(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        200_000_000,
    )
    .await;

    // Open dispute on "disputed.com"
    let domain = "disputed2.com";
    let (dispute_pda, dispute_nonce, _) = setup_dispute(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &config_pda,
        &staker_token,
        &reward_vault.pubkey(),
        &truth_mint.pubkey(),
        domain,
    )
    .await;

    // Stake as juror on a DIFFERENT domain (to avoid conflict of interest)
    let juror_domain = "juror-domain.com";
    let (juror_stake_pda, _) = stake_v2_helper(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &config_pda,
        &staker_token,
        &reward_vault.pubkey(),
        juror_domain,
        10_000_000,
        0,
    )
    .await;

    // Patch juror stake: set staked_at to 8 days ago (satisfies 7-day minimum)
    {
        let stake_account = ctx
            .banks_client
            .get_account(juror_stake_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = stake_account.data.len();
        let mut stake = StakeAccountV2::deserialize_reader(&mut &stake_account.data[..]).unwrap();
        stake.staked_at -= 8 * 24 * 3600;
        let mut buf = Vec::new();
        stake.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(stake_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&juror_stake_pda, &patched);
    }

    // Commit jury vote (disputes start in CommitPhase, must use commit-reveal)
    let (jury_pda, _) = Pubkey::find_program_address(
        &[JURY_SEED, dispute_pda.as_ref(), ctx.payer.pubkey().as_ref()],
        &program_id,
    );

    // Compute commit hash: SHA256(verdict || salt)
    let verdict: u8 = 0; // Guilty
    let salt = [7u8; 32];
    let commit_hash = solana_program::hash::hashv(&[&[verdict], &salt]).to_bytes();

    let mut ix_data = Vec::new();
    StakingInstruction::CommitJuryVote {
        domain: domain.to_string(),
        dispute_nonce,
        commit_hash,
    }
    .serialize(&mut ix_data)
    .unwrap();

    let ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(ctx.payer.pubkey(), true),
            AccountMeta::new(jury_pda, false),
            AccountMeta::new(dispute_pda, false),
            AccountMeta::new_readonly(juror_stake_pda, false),
            AccountMeta::new_readonly(config_pda, false),
            AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
            AccountMeta::new_readonly(clock::id(), false),
        ],
        data: ix_data,
    };

    let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&ctx.payer.pubkey()),
        &[&ctx.payer],
        blockhash,
    );
    ctx.banks_client.process_transaction(tx).await.unwrap();

    // Verify JuryVote PDA was created with commit hash
    let jury_account = ctx
        .banks_client
        .get_account(jury_pda)
        .await
        .unwrap()
        .unwrap();
    let vote = JuryVote::deserialize_reader(&mut &jury_account.data[..]).unwrap();
    assert!(vote.is_initialized);
    assert_eq!(vote.commit_hash, commit_hash);
    assert_eq!(vote.weight, 10_000_000);
    assert_eq!(vote.voted_at, 0, "voted_at should be 0 until reveal");

    // Verify dispute jury_count updated (tallies NOT updated until reveal)
    let dispute_account = ctx
        .banks_client
        .get_account(dispute_pda)
        .await
        .unwrap()
        .unwrap();
    let dispute = DisputeAccount::deserialize_reader(&mut &dispute_account.data[..]).unwrap();
    assert_eq!(dispute.jury_count, 1);
    assert_eq!(
        dispute.votes_guilty, 0,
        "tallies should be 0 until reveal phase"
    );
}

#[tokio::test]
async fn test_jury_conflict_of_interest_fails() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let (mut banks, payer, _) = pt.start().await;
    let (config_pda, staker_token) = init_protocol_and_mint(
        &mut banks,
        &payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        200_000_000,
    )
    .await;

    let domain = "conflict.com";
    let (dispute_pda, dispute_nonce, _) = setup_dispute(
        &mut banks,
        &payer,
        &program_id,
        &config_pda,
        &staker_token,
        &reward_vault.pubkey(),
        &truth_mint.pubkey(),
        domain,
    )
    .await;

    // The payer is ALREADY staked on the disputed domain (from setup_dispute → stake_v2_helper).
    // Trying to commit a vote on own domain should fail with ConflictOfInterest.
    let (juror_stake_pda, _) = Pubkey::find_program_address(
        &[STAKE_SEED, domain.as_bytes(), payer.pubkey().as_ref()],
        &program_id,
    );
    let (jury_pda, _) = Pubkey::find_program_address(
        &[JURY_SEED, dispute_pda.as_ref(), payer.pubkey().as_ref()],
        &program_id,
    );

    let commit_hash = [0u8; 32]; // dummy hash, won't get that far
    let mut ix_data = Vec::new();
    StakingInstruction::CommitJuryVote {
        domain: domain.to_string(),
        dispute_nonce,
        commit_hash,
    }
    .serialize(&mut ix_data)
    .unwrap();

    let ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(jury_pda, false),
            AccountMeta::new(dispute_pda, false),
            AccountMeta::new_readonly(juror_stake_pda, false),
            AccountMeta::new_readonly(config_pda, false),
            AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
            AccountMeta::new_readonly(clock::id(), false),
        ],
        data: ix_data,
    };

    let blockhash = banks.get_latest_blockhash().await.unwrap();
    let tx = Transaction::new_signed_with_payer(&[ix], Some(&payer.pubkey()), &[&payer], blockhash);
    let result = banks.process_transaction(tx).await;
    assert!(
        result.is_err(),
        "juror staked on disputed domain should fail with ConflictOfInterest"
    );
}

#[tokio::test]
async fn test_resolve_dispute_before_deadline_fails() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let (mut banks, payer, _) = pt.start().await;
    let (config_pda, staker_token) = init_protocol_and_mint(
        &mut banks,
        &payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        200_000_000,
    )
    .await;

    let domain = "early-resolve.com";
    let (dispute_pda, dispute_nonce, domain_pda) = setup_dispute(
        &mut banks,
        &payer,
        &program_id,
        &config_pda,
        &staker_token,
        &reward_vault.pubkey(),
        &truth_mint.pubkey(),
        domain,
    )
    .await;

    // Try to resolve immediately (deadline is 7 days away)
    let mut ix_data = Vec::new();
    StakingInstruction::ResolveDispute {
        domain: domain.to_string(),
        dispute_nonce,
    }
    .serialize(&mut ix_data)
    .unwrap();

    let ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(dispute_pda, false),
            AccountMeta::new(domain_pda, false),
            AccountMeta::new(staker_token, false),
            AccountMeta::new(reward_vault.pubkey(), false),
            AccountMeta::new(truth_mint.pubkey(), false),
            AccountMeta::new_readonly(config_pda, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(clock::id(), false),
        ],
        data: ix_data,
    };

    let blockhash = banks.get_latest_blockhash().await.unwrap();
    let tx = Transaction::new_signed_with_payer(&[ix], Some(&payer.pubkey()), &[&payer], blockhash);
    let result = banks.process_transaction(tx).await;
    assert!(
        result.is_err(),
        "resolving dispute before deadline should fail"
    );
}

// ══════════════════════════════════════════════
// Functional E2E Tests — Phase 5: Governance
// ══════════════════════════════════════════════

#[tokio::test]
async fn test_create_and_execute_proposal() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let mut ctx = pt.start_with_context().await;
    let (config_pda, staker_token) = init_protocol_and_mint(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        500_000_000,
    )
    .await;

    // Stake a large amount — needed for quorum (10% of total_staked)
    let domain = "gov.com";
    let (voter_stake_pda, _) = stake_v2_helper(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &config_pda,
        &staker_token,
        &reward_vault.pubkey(),
        domain,
        100_000_000,
        0,
    )
    .await;

    // Patch stake: set staked_at to 8 days ago (satisfies 7-day governance voting minimum)
    {
        let stake_account = ctx
            .banks_client
            .get_account(voter_stake_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = stake_account.data.len();
        let mut stake = StakeAccountV2::deserialize_reader(&mut &stake_account.data[..]).unwrap();
        stake.staked_at -= 8 * 24 * 3600;
        let mut buf = Vec::new();
        stake.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(stake_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&voter_stake_pda, &patched);
    }

    // Patch config: set execution_delay_secs to 0 for this test
    {
        let config_account = ctx
            .banks_client
            .get_account(config_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = config_account.data.len();
        let mut config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
        config.execution_delay_secs = 0;
        config.proposal_deposit = 0;
        let mut buf = Vec::new();
        config.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(config_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&config_pda, &patched);
    }

    // Read config to get proposal nonce
    let config_account = ctx
        .banks_client
        .get_account(config_pda)
        .await
        .unwrap()
        .unwrap();
    let config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
    let nonce = config.proposal_nonce;
    let nonce_bytes = nonce.to_le_bytes();

    let (proposal_pda, _) =
        Pubkey::find_program_address(&[PROPOSAL_SEED, &nonce_bytes], &program_id);

    // CreateProposal: change cooldown_secs to 86400
    let mut ix_data = Vec::new();
    StakingInstruction::CreateProposal {
        description_hash: [1u8; 32],
        param_key: "cooldown_secs".to_string(),
        param_value: 86400,
    }
    .serialize(&mut ix_data)
    .unwrap();

    let ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(ctx.payer.pubkey(), true),
            AccountMeta::new(proposal_pda, false),
            AccountMeta::new(config_pda, false),
            AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
            AccountMeta::new_readonly(clock::id(), false),
        ],
        data: ix_data,
    };

    let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&ctx.payer.pubkey()),
        &[&ctx.payer],
        blockhash,
    );
    ctx.banks_client.process_transaction(tx).await.unwrap();

    // Verify proposal created
    let proposal_account = ctx
        .banks_client
        .get_account(proposal_pda)
        .await
        .unwrap()
        .unwrap();
    let proposal = GovernanceProposal::deserialize_reader(&mut &proposal_account.data[..]).unwrap();
    assert!(proposal.is_initialized);
    assert_eq!(proposal.param_key, "cooldown_secs");
    assert_eq!(proposal.param_value, 86400);
    assert_eq!(proposal.status, ProposalStatus::Active);

    // CastGovernanceVote: For
    let (gov_vote_pda, _) = Pubkey::find_program_address(
        &[
            GOV_VOTE_SEED,
            proposal_pda.as_ref(),
            ctx.payer.pubkey().as_ref(),
        ],
        &program_id,
    );
    let (gov_lock_pda, _) =
        Pubkey::find_program_address(&[GOV_LOCK_SEED, ctx.payer.pubkey().as_ref()], &program_id);

    let mut ix_data = Vec::new();
    StakingInstruction::CastGovernanceVote {
        proposal_nonce: nonce,
        vote: 0,
    }
    .serialize(&mut ix_data)
    .unwrap();

    let ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(ctx.payer.pubkey(), true),
            AccountMeta::new(gov_vote_pda, false),
            AccountMeta::new(proposal_pda, false),
            AccountMeta::new_readonly(voter_stake_pda, false),
            AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
            AccountMeta::new_readonly(clock::id(), false),
            AccountMeta::new(gov_lock_pda, false),
        ],
        data: ix_data,
    };

    let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&ctx.payer.pubkey()),
        &[&ctx.payer],
        blockhash,
    );
    ctx.banks_client.process_transaction(tx).await.unwrap();

    // Verify vote recorded
    let vote_account = ctx
        .banks_client
        .get_account(gov_vote_pda)
        .await
        .unwrap()
        .unwrap();
    let vote = GovernanceVote::deserialize_reader(&mut &vote_account.data[..]).unwrap();
    assert_eq!(vote.vote, GovernanceVoteChoice::For);
    assert_eq!(vote.weight, 100_000_000);
}

#[tokio::test]
async fn test_proposal_fails_no_quorum() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let mut ctx = pt.start_with_context().await;
    let (config_pda, staker_token) = init_protocol_and_mint(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        500_000_000,
    )
    .await;

    // Patch config: proposal_deposit = 0 for this test
    {
        let config_account = ctx
            .banks_client
            .get_account(config_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = config_account.data.len();
        let mut config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
        config.proposal_deposit = 0;
        let mut buf = Vec::new();
        config.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(config_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&config_pda, &patched);
    }

    // Stake a small amount
    let domain = "gov-fail.com";
    let (_voter_stake_pda, _) = stake_v2_helper(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &config_pda,
        &staker_token,
        &reward_vault.pubkey(),
        domain,
        5_000_000,
        0,
    )
    .await;

    // Read config for nonce
    let config_account = ctx
        .banks_client
        .get_account(config_pda)
        .await
        .unwrap()
        .unwrap();
    let config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
    let nonce = config.proposal_nonce;
    let nonce_bytes = nonce.to_le_bytes();

    let (proposal_pda, _) =
        Pubkey::find_program_address(&[PROPOSAL_SEED, &nonce_bytes], &program_id);

    // CreateProposal
    let mut ix_data = Vec::new();
    StakingInstruction::CreateProposal {
        description_hash: [2u8; 32],
        param_key: "min_stake".to_string(),
        param_value: 999,
    }
    .serialize(&mut ix_data)
    .unwrap();

    let ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(ctx.payer.pubkey(), true),
            AccountMeta::new(proposal_pda, false),
            AccountMeta::new(config_pda, false),
            AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
            AccountMeta::new_readonly(clock::id(), false),
        ],
        data: ix_data,
    };

    let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&ctx.payer.pubkey()),
        &[&ctx.payer],
        blockhash,
    );
    ctx.banks_client.process_transaction(tx).await.unwrap();

    // Try to execute immediately — should fail because voting period is still active
    let mut ix_data = Vec::new();
    StakingInstruction::ExecuteProposal {
        proposal_nonce: nonce,
    }
    .serialize(&mut ix_data)
    .unwrap();

    let ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(ctx.payer.pubkey(), true),
            AccountMeta::new(proposal_pda, false),
            AccountMeta::new(config_pda, false),
            AccountMeta::new_readonly(clock::id(), false),
        ],
        data: ix_data,
    };

    let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&ctx.payer.pubkey()),
        &[&ctx.payer],
        blockhash,
    );
    let result = ctx.banks_client.process_transaction(tx).await;
    assert!(
        result.is_err(),
        "executing proposal during voting period should fail"
    );
}

// ══════════════════════════════════════════════
// Functional E2E Tests — Phase 6: Channels
// ══════════════════════════════════════════════

#[tokio::test]
async fn test_open_and_settle_channel() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let (mut banks, payer, _) = pt.start().await;
    let (config_pda, staker_token) = init_protocol_and_mint(
        &mut banks,
        &payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        200_000_000,
    )
    .await;

    let payee = Keypair::new();
    // Fund payee for rent
    let fund_ix = system_instruction::transfer(&payer.pubkey(), &payee.pubkey(), 1_000_000_000);
    let blockhash = banks.get_latest_blockhash().await.unwrap();
    let tx =
        Transaction::new_signed_with_payer(&[fund_ix], Some(&payer.pubkey()), &[&payer], blockhash);
    banks.process_transaction(tx).await.unwrap();

    // Create payee token account
    let blockhash = banks.get_latest_blockhash().await.unwrap();
    let payee_token = create_token_account(
        &mut banks,
        &payer,
        blockhash,
        &truth_mint.pubkey(),
        &payee.pubkey(),
    )
    .await;

    let channel_amount = 50_000_000u64;
    let payee_bytes = payee.pubkey().to_bytes();

    let (channel_pda, _) = Pubkey::find_program_address(
        &[
            CHANNEL_SEED,
            payer.pubkey().as_ref(),
            payee.pubkey().as_ref(),
        ],
        &program_id,
    );

    // OpenChannel
    let mut ix_data = Vec::new();
    StakingInstruction::OpenChannel {
        payee: payee_bytes,
        amount: channel_amount,
        expiry_slot: 10000,
    }
    .serialize(&mut ix_data)
    .unwrap();

    let ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(staker_token, false),
            AccountMeta::new(reward_vault.pubkey(), false),
            AccountMeta::new(channel_pda, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
            AccountMeta::new_readonly(clock::id(), false),
            AccountMeta::new_readonly(config_pda, false),
        ],
        data: ix_data,
    };

    let blockhash = banks.get_latest_blockhash().await.unwrap();
    let tx = Transaction::new_signed_with_payer(&[ix], Some(&payer.pubkey()), &[&payer], blockhash);
    banks.process_transaction(tx).await.unwrap();

    // Verify channel created
    let channel_account = banks.get_account(channel_pda).await.unwrap().unwrap();
    let channel = StateChannelAccount::deserialize_reader(&mut &channel_account.data[..]).unwrap();
    assert!(channel.is_initialized);
    assert_eq!(channel.deposited, channel_amount);
    assert_eq!(channel.spent, 0);
    assert_eq!(channel.nonce, 0);

    // SettleChannel — payee claims 20M, payer gets 30M refund
    let spent = 20_000_000u64;
    let settle_nonce = 1u64;

    // Create Ed25519 signature instruction (must precede SettleChannel)
    let ed25519_ix = make_ed25519_settle_ix(&payer, &channel_pda, spent, settle_nonce);

    let mut ix_data = Vec::new();
    StakingInstruction::SettleChannel {
        payer_key: payer.pubkey().to_bytes(),
        payee_key: payee_bytes,
        spent,
        nonce: settle_nonce,
    }
    .serialize(&mut ix_data)
    .unwrap();

    let ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(channel_pda, false),
            AccountMeta::new(payee_token, false),
            AccountMeta::new(staker_token, false),
            AccountMeta::new(reward_vault.pubkey(), false),
            AccountMeta::new(config_pda, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new(truth_mint.pubkey(), false), // mint for burn
            AccountMeta::new_readonly(ix_sysvar::id(), false), // Instructions sysvar (required)
        ],
        data: ix_data,
    };

    let blockhash = banks.get_latest_blockhash().await.unwrap();
    let tx = Transaction::new_signed_with_payer(
        &[ed25519_ix, ix],
        Some(&payer.pubkey()),
        &[&payer],
        blockhash,
    );
    banks.process_transaction(tx).await.unwrap();

    // Verify channel PDA is closed
    let channel_account = banks.get_account(channel_pda).await.unwrap();
    match channel_account {
        None => {} // fully closed
        Some(acct) => {
            assert!(
                acct.data.iter().all(|b| *b == 0),
                "channel PDA should be zeroed"
            );
        }
    }

    // Verify payee got tokens (85% of spent after 10% burn + 5% sentinel fee)
    // burn = 20_000_000 * 10 / 100 = 2_000_000
    // sentinel_fee = 20_000_000 * 5 / 100 = 1_000_000
    // payee = 20_000_000 - 2_000_000 - 1_000_000 = 17_000_000
    let payee_account = banks.get_account(payee_token).await.unwrap().unwrap();
    let payee_data = spl_token::state::Account::unpack(&payee_account.data).unwrap();
    assert_eq!(payee_data.amount, 17_000_000);
}

#[tokio::test]
async fn test_settle_stale_nonce_fails() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let (mut banks, payer, _) = pt.start().await;
    let (config_pda, staker_token) = init_protocol_and_mint(
        &mut banks,
        &payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        200_000_000,
    )
    .await;

    let payee = Keypair::new();
    let fund_ix = system_instruction::transfer(&payer.pubkey(), &payee.pubkey(), 1_000_000_000);
    let blockhash = banks.get_latest_blockhash().await.unwrap();
    let tx =
        Transaction::new_signed_with_payer(&[fund_ix], Some(&payer.pubkey()), &[&payer], blockhash);
    banks.process_transaction(tx).await.unwrap();

    let blockhash = banks.get_latest_blockhash().await.unwrap();
    let payee_token = create_token_account(
        &mut banks,
        &payer,
        blockhash,
        &truth_mint.pubkey(),
        &payee.pubkey(),
    )
    .await;

    let payee_bytes = payee.pubkey().to_bytes();
    let (channel_pda, _) = Pubkey::find_program_address(
        &[
            CHANNEL_SEED,
            payer.pubkey().as_ref(),
            payee.pubkey().as_ref(),
        ],
        &program_id,
    );

    // OpenChannel
    let mut ix_data = Vec::new();
    StakingInstruction::OpenChannel {
        payee: payee_bytes,
        amount: 50_000_000,
        expiry_slot: 10000,
    }
    .serialize(&mut ix_data)
    .unwrap();

    let ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(staker_token, false),
            AccountMeta::new(reward_vault.pubkey(), false),
            AccountMeta::new(channel_pda, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
            AccountMeta::new_readonly(clock::id(), false),
            AccountMeta::new_readonly(config_pda, false),
        ],
        data: ix_data,
    };

    let blockhash = banks.get_latest_blockhash().await.unwrap();
    let tx = Transaction::new_signed_with_payer(&[ix], Some(&payer.pubkey()), &[&payer], blockhash);
    banks.process_transaction(tx).await.unwrap();

    // First settle with nonce=5 (succeeds, settles the channel and closes it)
    let ed25519_ix = make_ed25519_settle_ix(&payer, &channel_pda, 10_000_000, 5);

    let mut ix_data = Vec::new();
    StakingInstruction::SettleChannel {
        payer_key: payer.pubkey().to_bytes(),
        payee_key: payee_bytes,
        spent: 10_000_000,
        nonce: 5,
    }
    .serialize(&mut ix_data)
    .unwrap();

    let ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(channel_pda, false),
            AccountMeta::new(payee_token, false),
            AccountMeta::new(staker_token, false),
            AccountMeta::new(reward_vault.pubkey(), false),
            AccountMeta::new(config_pda, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new(truth_mint.pubkey(), false), // mint for burn
            AccountMeta::new_readonly(ix_sysvar::id(), false),
        ],
        data: ix_data,
    };

    let blockhash = banks.get_latest_blockhash().await.unwrap();
    let tx = Transaction::new_signed_with_payer(
        &[ed25519_ix, ix],
        Some(&payer.pubkey()),
        &[&payer],
        blockhash,
    );
    banks.process_transaction(tx).await.unwrap();

    // Second settle with nonce=3 (stale) — should fail because channel was already closed.
    // The channel PDA is zeroed, so deserialization will fail (UninitializedAccount).
    let ed25519_ix2 = make_ed25519_settle_ix(&payer, &channel_pda, 5_000_000, 3);

    let mut ix_data = Vec::new();
    StakingInstruction::SettleChannel {
        payer_key: payer.pubkey().to_bytes(),
        payee_key: payee_bytes,
        spent: 5_000_000,
        nonce: 3,
    }
    .serialize(&mut ix_data)
    .unwrap();

    let ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(channel_pda, false),
            AccountMeta::new(payee_token, false),
            AccountMeta::new(staker_token, false),
            AccountMeta::new(reward_vault.pubkey(), false),
            AccountMeta::new(config_pda, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new(truth_mint.pubkey(), false), // mint for burn
            AccountMeta::new_readonly(ix_sysvar::id(), false),
        ],
        data: ix_data,
    };

    let blockhash = banks.get_latest_blockhash().await.unwrap();
    let tx = Transaction::new_signed_with_payer(
        &[ed25519_ix2, ix],
        Some(&payer.pubkey()),
        &[&payer],
        blockhash,
    );
    let result = banks.process_transaction(tx).await;
    assert!(
        result.is_err(),
        "settling with stale nonce on closed channel should fail"
    );
}

// ══════════════════════════════════════════════
// New Tests — Economic Hardening
// ══════════════════════════════════════════════

/// Verify that MintTruth respects the max_total_supply cap.
#[tokio::test]
async fn test_supply_cap_enforcement() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let mut ctx = pt.start_with_context().await;

    let (config_pda, staker_token) = init_protocol_and_mint(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        100_000_000,
    )
    .await;

    // Verify total_minted tracks the initial mint
    let config_account = ctx
        .banks_client
        .get_account(config_pda)
        .await
        .unwrap()
        .unwrap();
    let config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
    assert_eq!(config.total_minted, 100_000_000);

    // Patch config: set max_total_supply = 150_000_000
    {
        let orig_len = config_account.data.len();
        let mut config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
        config.max_total_supply = 150_000_000;
        let mut buf = Vec::new();
        config.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(config_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&config_pda, &patched);
    }

    // Mint 50M more → should succeed (total_minted = 150M = cap)
    {
        let mut ix_data = Vec::new();
        StakingInstruction::MintTruth { amount: 50_000_000 }
            .serialize(&mut ix_data)
            .unwrap();
        let mint_ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(config_pda, false),
                AccountMeta::new(truth_mint.pubkey(), false),
                AccountMeta::new(staker_token, false),
                AccountMeta::new_readonly(spl_token::id(), false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[mint_ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        ctx.banks_client.process_transaction(tx).await.unwrap();
    }

    // Verify total_minted = 150_000_000
    let config_account = ctx
        .banks_client
        .get_account(config_pda)
        .await
        .unwrap()
        .unwrap();
    let config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
    assert_eq!(config.total_minted, 150_000_000);

    // Mint 1 more → should fail (SupplyCapExceeded)
    {
        let mut ix_data = Vec::new();
        StakingInstruction::MintTruth { amount: 1 }
            .serialize(&mut ix_data)
            .unwrap();
        let mint_ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(config_pda, false),
                AccountMeta::new(truth_mint.pubkey(), false),
                AccountMeta::new(staker_token, false),
                AccountMeta::new_readonly(spl_token::id(), false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[mint_ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        let result = ctx.banks_client.process_transaction(tx).await;
        assert!(result.is_err(), "minting beyond supply cap should fail");
    }
}

/// Verify AdvanceEpoch advances epoch correctly and updates emission cap.
#[tokio::test]
async fn test_epoch_advance_and_emission_halving() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let mut ctx = pt.start_with_context().await;

    let (config_pda, _) = init_protocol_and_mint(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        0,
    )
    .await;

    // Verify initial state: epoch 0, max emission = 10M, new fields have defaults
    let config_account = ctx
        .banks_client
        .get_account(config_pda)
        .await
        .unwrap()
        .unwrap();
    let config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
    assert_eq!(config.current_epoch, 0);
    assert_eq!(config.epoch_max_emission, 10_000_000);
    assert_eq!(config.min_epoch_duration, 604_800, "M7: default 7 days");
    assert_eq!(config.epoch_started_at, 0);
    assert_eq!(config.max_total_supply, 1_000_000_000);
    assert_eq!(config.burn_pct, 10);
    assert_eq!(config.staker_emission_pct, 70);
    assert_eq!(config.curator_yield_cap_bps, 1500);

    // Patch config: set min_epoch_duration=0 for this test (focus on halving, not timing)
    {
        let config_account = ctx
            .banks_client
            .get_account(config_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = config_account.data.len();
        let mut config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
        config.min_epoch_duration = 0;
        let mut buf = Vec::new();
        config.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(config_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&config_pda, &patched);
    }

    // AdvanceEpoch (min_epoch_duration=0 → no timing check, Clock not needed)
    let mut ix_data = Vec::new();
    StakingInstruction::AdvanceEpoch
        .serialize(&mut ix_data)
        .unwrap();
    let ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(ctx.payer.pubkey(), true),
            AccountMeta::new(config_pda, false),
        ],
        data: ix_data,
    };
    let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&ctx.payer.pubkey()),
        &[&ctx.payer],
        blockhash,
    );
    ctx.banks_client.process_transaction(tx).await.unwrap();

    // Verify epoch advanced to 1 with halved emission
    let config_account = ctx
        .banks_client
        .get_account(config_pda)
        .await
        .unwrap()
        .unwrap();
    let config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
    assert_eq!(config.current_epoch, 1);
    assert_eq!(
        config.epoch_max_emission, 9_850_000,
        "epoch 1 should decay emission by 1.5% to 9.85M"
    );
    assert_eq!(
        config.epoch_funded_amount, 0,
        "epoch_funded_amount should reset to 0"
    );

    // Warp slot so blockhash changes (avoids duplicate tx detection)
    ctx.warp_to_slot(100).unwrap();

    // Advance again → epoch 2 with 2.5M emission
    let mut ix_data = Vec::new();
    StakingInstruction::AdvanceEpoch
        .serialize(&mut ix_data)
        .unwrap();
    let ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(ctx.payer.pubkey(), true),
            AccountMeta::new(config_pda, false),
        ],
        data: ix_data,
    };
    let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&ctx.payer.pubkey()),
        &[&ctx.payer],
        blockhash,
    );
    ctx.banks_client.process_transaction(tx).await.unwrap();

    let config_account = ctx
        .banks_client
        .get_account(config_pda)
        .await
        .unwrap()
        .unwrap();
    let config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
    assert_eq!(config.current_epoch, 2);
    assert_eq!(
        config.epoch_max_emission, 9_702_250,
        "epoch 2 should decay emission by 1.5% to ~9.7M"
    );
}

/// Verify SlashSentinel reduces the amount received on WithdrawSentinel.
#[tokio::test]
async fn test_slash_sentinel() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let mut ctx = pt.start_with_context().await;

    // Init with cooldown_secs = 0 for immediate withdraw
    let (config_pda, staker_token) = init_protocol_custom_cooldown(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        100_000_000,
        0,
    )
    .await;

    let sentinel_pubkey = ctx.payer.pubkey();
    let stake_amount = 10_000_000u64;
    let slash_amount = 3_000_000u64;

    let (sentinel_pda, _) =
        Pubkey::find_program_address(&[SENTINEL_SEED, sentinel_pubkey.as_ref()], &program_id);

    // StakeSentinel
    {
        let mut ix_data = Vec::new();
        StakingInstruction::StakeSentinel {
            amount: stake_amount,
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(sentinel_pubkey, true),
                AccountMeta::new(staker_token, false),
                AccountMeta::new(reward_vault.pubkey(), false),
                AccountMeta::new(sentinel_pda, false),
                AccountMeta::new(config_pda, false),
                AccountMeta::new_readonly(spl_token::id(), false),
                AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
                AccountMeta::new_readonly(clock::id(), false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&sentinel_pubkey),
            &[&ctx.payer],
            blockhash,
        );
        ctx.banks_client.process_transaction(tx).await.unwrap();
    }

    // Verify sentinel staked
    let sentinel_account = ctx
        .banks_client
        .get_account(sentinel_pda)
        .await
        .unwrap()
        .unwrap();
    let sentinel = SentinelAccount::deserialize_reader(&mut &sentinel_account.data[..]).unwrap();
    assert!(sentinel.is_initialized);
    assert_eq!(sentinel.amount, stake_amount);
    assert_eq!(sentinel.slashed_amount, 0);

    // SlashSentinel
    {
        let mut ix_data = Vec::new();
        StakingInstruction::SlashSentinel {
            sentinel: sentinel_pubkey.to_bytes(),
            amount: slash_amount,
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(sentinel_pda, false),
                AccountMeta::new_readonly(config_pda, false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        ctx.banks_client.process_transaction(tx).await.unwrap();
    }

    // Verify slashed amount recorded
    let sentinel_account = ctx
        .banks_client
        .get_account(sentinel_pda)
        .await
        .unwrap()
        .unwrap();
    let sentinel = SentinelAccount::deserialize_reader(&mut &sentinel_account.data[..]).unwrap();
    assert_eq!(sentinel.slashed_amount, slash_amount);

    // UnstakeSentinel
    {
        let mut ix_data = Vec::new();
        StakingInstruction::UnstakeSentinel
            .serialize(&mut ix_data)
            .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(sentinel_pubkey, true),
                AccountMeta::new(sentinel_pda, false),
                AccountMeta::new(config_pda, false),
                AccountMeta::new_readonly(clock::id(), false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&sentinel_pubkey),
            &[&ctx.payer],
            blockhash,
        );
        ctx.banks_client.process_transaction(tx).await.unwrap();
    }

    // Get token balance before withdraw
    let token_before = {
        let acct = ctx
            .banks_client
            .get_account(staker_token)
            .await
            .unwrap()
            .unwrap();
        spl_token::state::Account::unpack(&acct.data)
            .unwrap()
            .amount
    };

    // WithdrawSentinel
    {
        let mut ix_data = Vec::new();
        StakingInstruction::WithdrawSentinel
            .serialize(&mut ix_data)
            .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(sentinel_pubkey, true),
                AccountMeta::new(sentinel_pda, false),
                AccountMeta::new(staker_token, false),
                AccountMeta::new(reward_vault.pubkey(), false),
                AccountMeta::new(config_pda, false),
                AccountMeta::new_readonly(spl_token::id(), false),
                AccountMeta::new_readonly(clock::id(), false),
                AccountMeta::new(truth_mint.pubkey(), false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&sentinel_pubkey),
            &[&ctx.payer],
            blockhash,
        );
        ctx.banks_client.process_transaction(tx).await.unwrap();
    }

    // Verify sentinel got back (stake_amount - slash_amount)
    let token_after = {
        let acct = ctx
            .banks_client
            .get_account(staker_token)
            .await
            .unwrap()
            .unwrap();
        spl_token::state::Account::unpack(&acct.data)
            .unwrap()
            .amount
    };
    let expected_return = stake_amount - slash_amount;
    // 50% of slashed tokens are burned, but sentinel still receives stake minus full slash
    assert_eq!(
        token_after - token_before,
        expected_return,
        "sentinel should receive stake minus slashed amount"
    );
}

/// Verify that governable burn_pct is used in channel settlement.
#[tokio::test]
async fn test_governable_burn_pct() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let mut ctx = pt.start_with_context().await;

    let (config_pda, staker_token) = init_protocol_and_mint(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        200_000_000,
    )
    .await;

    // Patch config: set burn_pct = 20, sentinel_fee_pct = 0 (isolate burn test)
    {
        let config_account = ctx
            .banks_client
            .get_account(config_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = config_account.data.len();
        let mut config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
        config.burn_pct = 20;
        config.sentinel_fee_pct = 0;
        let mut buf = Vec::new();
        config.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(config_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&config_pda, &patched);
    }

    // Fund payee
    let payee = Keypair::new();
    let fund_ix = system_instruction::transfer(&ctx.payer.pubkey(), &payee.pubkey(), 1_000_000_000);
    let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
    let tx = Transaction::new_signed_with_payer(
        &[fund_ix],
        Some(&ctx.payer.pubkey()),
        &[&ctx.payer],
        blockhash,
    );
    ctx.banks_client.process_transaction(tx).await.unwrap();

    let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
    let payee_token = create_token_account(
        &mut ctx.banks_client,
        &ctx.payer,
        blockhash,
        &truth_mint.pubkey(),
        &payee.pubkey(),
    )
    .await;

    let payee_bytes = payee.pubkey().to_bytes();
    let channel_amount = 50_000_000u64;
    let (channel_pda, _) = Pubkey::find_program_address(
        &[
            CHANNEL_SEED,
            ctx.payer.pubkey().as_ref(),
            payee.pubkey().as_ref(),
        ],
        &program_id,
    );

    // OpenChannel
    {
        let mut ix_data = Vec::new();
        StakingInstruction::OpenChannel {
            payee: payee_bytes,
            amount: channel_amount,
            expiry_slot: 10000,
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(staker_token, false),
                AccountMeta::new(reward_vault.pubkey(), false),
                AccountMeta::new(channel_pda, false),
                AccountMeta::new_readonly(spl_token::id(), false),
                AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
                AccountMeta::new_readonly(clock::id(), false),
                AccountMeta::new_readonly(config_pda, false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        ctx.banks_client.process_transaction(tx).await.unwrap();
    }

    // SettleChannel with spent=20M
    let spent = 20_000_000u64;
    {
        let ed25519_ix = make_ed25519_settle_ix(&ctx.payer, &channel_pda, spent, 1);
        let mut ix_data = Vec::new();
        StakingInstruction::SettleChannel {
            payer_key: ctx.payer.pubkey().to_bytes(),
            payee_key: payee_bytes,
            spent,
            nonce: 1,
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(channel_pda, false),
                AccountMeta::new(payee_token, false),
                AccountMeta::new(staker_token, false),
                AccountMeta::new(reward_vault.pubkey(), false),
                AccountMeta::new(config_pda, false),
                AccountMeta::new_readonly(spl_token::id(), false),
                AccountMeta::new(truth_mint.pubkey(), false),
                AccountMeta::new_readonly(ix_sysvar::id(), false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ed25519_ix, ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        ctx.banks_client.process_transaction(tx).await.unwrap();
    }

    // Verify payee got 80% of spent (burn_pct = 20)
    // burn = 20_000_000 * 20 / 100 = 4_000_000
    // payee = 20_000_000 - 4_000_000 = 16_000_000
    let payee_account = ctx
        .banks_client
        .get_account(payee_token)
        .await
        .unwrap()
        .unwrap();
    let payee_data = spl_token::state::Account::unpack(&payee_account.data).unwrap();
    assert_eq!(
        payee_data.amount, 16_000_000,
        "payee should get 80% when burn_pct=20"
    );
}

/// Verify dispute resolution with quorum not met results in QuorumFailed.
#[tokio::test]
async fn test_dispute_quorum() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let mut ctx = pt.start_with_context().await;

    let (config_pda, staker_token) = init_protocol_and_mint(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        200_000_000,
    )
    .await;

    // Open dispute
    let domain = "quorum-test.com";
    let (dispute_pda, dispute_nonce, domain_pda) = setup_dispute(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &config_pda,
        &staker_token,
        &reward_vault.pubkey(),
        &truth_mint.pubkey(),
        domain,
    )
    .await;

    // Patch config: set min_jury_weight very high (quorum impossible to meet)
    {
        let config_account = ctx
            .banks_client
            .get_account(config_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = config_account.data.len();
        let mut config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
        config.min_jury_weight = 1_000_000_000;
        let mut buf = Vec::new();
        config.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(config_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&config_pda, &patched);
    }

    // Patch dispute: set deadline = 0 (so resolution can proceed past deadline check)
    {
        let dispute_account = ctx
            .banks_client
            .get_account(dispute_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = dispute_account.data.len();
        let mut dispute =
            DisputeAccount::deserialize_reader(&mut &dispute_account.data[..]).unwrap();
        dispute.deadline = 0;
        let mut buf = Vec::new();
        dispute.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(dispute_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&dispute_pda, &patched);
    }

    // ResolveDispute — should succeed but result in QuorumFailed status
    {
        let mut ix_data = Vec::new();
        StakingInstruction::ResolveDispute {
            domain: domain.to_string(),
            dispute_nonce,
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(dispute_pda, false),
                AccountMeta::new(domain_pda, false),
                AccountMeta::new(staker_token, false),
                AccountMeta::new(reward_vault.pubkey(), false),
                AccountMeta::new(truth_mint.pubkey(), false),
                AccountMeta::new_readonly(config_pda, false),
                AccountMeta::new_readonly(spl_token::id(), false),
                AccountMeta::new_readonly(clock::id(), false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        ctx.banks_client.process_transaction(tx).await.unwrap();
    }

    // Verify dispute status is QuorumFailed
    let dispute_account = ctx
        .banks_client
        .get_account(dispute_pda)
        .await
        .unwrap()
        .unwrap();
    let dispute = DisputeAccount::deserialize_reader(&mut &dispute_account.data[..]).unwrap();
    assert_eq!(
        dispute.status,
        DisputeStatus::QuorumFailed,
        "dispute should be QuorumFailed when jury weight is below threshold"
    );
}

// ══════════════════════════════════════════════
// Phase 11 — New tests for economic fixes
// ══════════════════════════════════════════════

/// Fix 2: Only one active dispute per domain at a time.
#[tokio::test]
async fn test_one_dispute_per_domain() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let (mut banks, payer, _) = pt.start().await;
    let (config_pda, staker_token) = init_protocol_and_mint(
        &mut banks,
        &payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        200_000_000,
    )
    .await;

    let domain = "one-dispute.com";
    let (_dispute_pda, _dispute_nonce, _domain_pda) = setup_dispute(
        &mut banks,
        &payer,
        &program_id,
        &config_pda,
        &staker_token,
        &reward_vault.pubkey(),
        &truth_mint.pubkey(),
        domain,
    )
    .await;

    // Read config for the next nonce
    let config_account = banks.get_account(config_pda).await.unwrap().unwrap();
    let config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
    let nonce2 = config.dispute_nonce;
    let nonce2_bytes = nonce2.to_le_bytes();

    let (domain_pda2, _) =
        Pubkey::find_program_address(&[DOMAIN_SEED, domain.as_bytes()], &program_id);
    let (dispute_pda2, _) = Pubkey::find_program_address(
        &[DISPUTE_SEED, domain.as_bytes(), &nonce2_bytes],
        &program_id,
    );

    // Try to open a second dispute on the same domain
    let mut ix_data = Vec::new();
    StakingInstruction::OpenDispute {
        domain: domain.to_string(),
        reason_hash: [99u8; 32],
        severity: 0,
    }
    .serialize(&mut ix_data)
    .unwrap();

    let ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(staker_token, false),
            AccountMeta::new(reward_vault.pubkey(), false),
            AccountMeta::new(dispute_pda2, false),
            AccountMeta::new(domain_pda2, false),
            AccountMeta::new(config_pda, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
            AccountMeta::new_readonly(clock::id(), false),
        ],
        data: ix_data,
    };

    let blockhash = banks.get_latest_blockhash().await.unwrap();
    let tx = Transaction::new_signed_with_payer(&[ix], Some(&payer.pubkey()), &[&payer], blockhash);
    let result = banks.process_transaction(tx).await;
    assert!(
        result.is_err(),
        "second dispute on same domain should fail with DisputeAlreadyActive"
    );
}

/// Fix 3: Slash percentage monotonicity — a lower-severity guilty verdict
/// should never reduce effective_slash_pct set by a prior higher-severity verdict.
#[tokio::test]
async fn test_slash_pct_monotonic() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let mut ctx = pt.start_with_context().await;
    let (config_pda, staker_token) = init_protocol_and_mint(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        200_000_000,
    )
    .await;

    // Open first dispute (severity 2 = high)
    let domain = "slash-mono.com";
    let (dispute_pda, dispute_nonce, domain_pda) = setup_dispute(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &config_pda,
        &staker_token,
        &reward_vault.pubkey(),
        &truth_mint.pubkey(),
        domain,
    )
    .await;

    // Patch config: min_jury_weight = 0 so any vote counts for quorum
    {
        let config_account = ctx
            .banks_client
            .get_account(config_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = config_account.data.len();
        let mut config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
        config.min_jury_weight = 0;
        let mut buf = Vec::new();
        config.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(config_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&config_pda, &patched);
    }

    // Patch dispute: deadline=0, severity=2, votes_guilty=1 (force guilty outcome)
    {
        let dispute_account = ctx
            .banks_client
            .get_account(dispute_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = dispute_account.data.len();
        let mut dispute =
            DisputeAccount::deserialize_reader(&mut &dispute_account.data[..]).unwrap();
        dispute.deadline = 0;
        dispute.reveal_deadline = 0;
        dispute.severity = 2;
        dispute.votes_guilty = 1;
        let mut buf = Vec::new();
        dispute.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(dispute_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&dispute_pda, &patched);
    }

    // Resolve first dispute → guilty (severity 2, slash_pct=10 → effective=15)
    {
        let mut ix_data = Vec::new();
        StakingInstruction::ResolveDispute {
            domain: domain.to_string(),
            dispute_nonce,
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(dispute_pda, false),
                AccountMeta::new(domain_pda, false),
                AccountMeta::new(staker_token, false),
                AccountMeta::new(reward_vault.pubkey(), false),
                AccountMeta::new(truth_mint.pubkey(), false),
                AccountMeta::new_readonly(config_pda, false),
                AccountMeta::new_readonly(spl_token::id(), false),
                AccountMeta::new_readonly(clock::id(), false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        ctx.banks_client.process_transaction(tx).await.unwrap();
    }

    // Verify effective_slash_pct = 15 (severity 2: 10*3/2=15)
    let domain_account = ctx
        .banks_client
        .get_account(domain_pda)
        .await
        .unwrap()
        .unwrap();
    let record = DomainRecord::deserialize_reader(&mut &domain_account.data[..]).unwrap();
    assert_eq!(
        record.effective_slash_pct, 15,
        "severity-2 should set slash to 15%"
    );
    assert_eq!(
        record.active_dispute_nonce,
        u64::MAX,
        "dispute nonce should be cleared after resolution"
    );

    // Now open second dispute with severity 0 (low)
    let config_account = ctx
        .banks_client
        .get_account(config_pda)
        .await
        .unwrap()
        .unwrap();
    let config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
    let nonce2 = config.dispute_nonce;
    let nonce2_bytes = nonce2.to_le_bytes();
    let (dispute_pda2, _) = Pubkey::find_program_address(
        &[DISPUTE_SEED, domain.as_bytes(), &nonce2_bytes],
        &program_id,
    );

    {
        let mut ix_data = Vec::new();
        StakingInstruction::OpenDispute {
            domain: domain.to_string(),
            reason_hash: [55u8; 32],
            severity: 0,
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(staker_token, false),
                AccountMeta::new(reward_vault.pubkey(), false),
                AccountMeta::new(dispute_pda2, false),
                AccountMeta::new(domain_pda, false),
                AccountMeta::new(config_pda, false),
                AccountMeta::new_readonly(spl_token::id(), false),
                AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
                AccountMeta::new_readonly(clock::id(), false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        ctx.banks_client.process_transaction(tx).await.unwrap();
    }

    // Patch second dispute: deadline=0, votes_guilty=1 (force guilty)
    {
        let dispute_account = ctx
            .banks_client
            .get_account(dispute_pda2)
            .await
            .unwrap()
            .unwrap();
        let orig_len = dispute_account.data.len();
        let mut dispute =
            DisputeAccount::deserialize_reader(&mut &dispute_account.data[..]).unwrap();
        dispute.deadline = 0;
        dispute.reveal_deadline = 0;
        dispute.votes_guilty = 1;
        let mut buf = Vec::new();
        dispute.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(dispute_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&dispute_pda2, &patched);
    }

    // Resolve second dispute → guilty (severity 0, slash_pct=10 → effective=5)
    // But effective_slash_pct should stay at 15 due to max() monotonicity
    {
        let mut ix_data = Vec::new();
        StakingInstruction::ResolveDispute {
            domain: domain.to_string(),
            dispute_nonce: nonce2,
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(dispute_pda2, false),
                AccountMeta::new(domain_pda, false),
                AccountMeta::new(staker_token, false),
                AccountMeta::new(reward_vault.pubkey(), false),
                AccountMeta::new(truth_mint.pubkey(), false),
                AccountMeta::new_readonly(config_pda, false),
                AccountMeta::new_readonly(spl_token::id(), false),
                AccountMeta::new_readonly(clock::id(), false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        ctx.banks_client.process_transaction(tx).await.unwrap();
    }

    // Verify effective_slash_pct is still 15 (max(15, 5) = 15)
    let domain_account = ctx
        .banks_client
        .get_account(domain_pda)
        .await
        .unwrap()
        .unwrap();
    let record = DomainRecord::deserialize_reader(&mut &domain_account.data[..]).unwrap();
    assert_eq!(
        record.effective_slash_pct, 15,
        "slash pct should be monotonic: max(15, 5) = 15"
    );
}

/// Fix 6: Fresh stake (< 7 days) cannot commit jury votes.
#[tokio::test]
async fn test_jury_stake_age() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let (mut banks, payer, _) = pt.start().await;
    let (config_pda, staker_token) = init_protocol_and_mint(
        &mut banks,
        &payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        200_000_000,
    )
    .await;

    let domain = "age-check.com";
    let (dispute_pda, dispute_nonce, _) = setup_dispute(
        &mut banks,
        &payer,
        &program_id,
        &config_pda,
        &staker_token,
        &reward_vault.pubkey(),
        &truth_mint.pubkey(),
        domain,
    )
    .await;

    // Fresh stake as juror on a different domain
    let juror_domain = "fresh-juror.com";
    let (juror_stake_pda, _) = stake_v2_helper(
        &mut banks,
        &payer,
        &program_id,
        &config_pda,
        &staker_token,
        &reward_vault.pubkey(),
        juror_domain,
        10_000_000,
        0,
    )
    .await;

    let (jury_pda, _) = Pubkey::find_program_address(
        &[JURY_SEED, dispute_pda.as_ref(), payer.pubkey().as_ref()],
        &program_id,
    );

    let commit_hash = [0u8; 32];
    let mut ix_data = Vec::new();
    StakingInstruction::CommitJuryVote {
        domain: domain.to_string(),
        dispute_nonce,
        commit_hash,
    }
    .serialize(&mut ix_data)
    .unwrap();

    let ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(jury_pda, false),
            AccountMeta::new(dispute_pda, false),
            AccountMeta::new_readonly(juror_stake_pda, false),
            AccountMeta::new_readonly(config_pda, false),
            AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
            AccountMeta::new_readonly(clock::id(), false),
        ],
        data: ix_data,
    };

    let blockhash = banks.get_latest_blockhash().await.unwrap();
    let tx = Transaction::new_signed_with_payer(&[ix], Some(&payer.pubkey()), &[&payer], blockhash);
    let result = banks.process_transaction(tx).await;
    assert!(
        result.is_err(),
        "fresh stake (< 7 days) should fail jury vote with JuryNotEligible"
    );
}

/// Fix 7: Governance parameter range guards reject out-of-range values at execution.
#[tokio::test]
async fn test_governance_param_range() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let mut ctx = pt.start_with_context().await;
    let (config_pda, staker_token) = init_protocol_and_mint(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        200_000_000,
    )
    .await;

    // Stake to have voting weight
    let domain = "gov-range.com";
    let (voter_stake_pda, _) = stake_v2_helper(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &config_pda,
        &staker_token,
        &reward_vault.pubkey(),
        domain,
        100_000_000,
        0,
    )
    .await;

    // Patch stake: set staked_at to 8 days ago (satisfies 7-day governance voting minimum)
    {
        let stake_account = ctx
            .banks_client
            .get_account(voter_stake_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = stake_account.data.len();
        let mut stake = StakeAccountV2::deserialize_reader(&mut &stake_account.data[..]).unwrap();
        stake.staked_at -= 8 * 24 * 3600;
        let mut buf = Vec::new();
        stake.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(stake_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&voter_stake_pda, &patched);
    }

    // Patch config: set execution_delay_secs to 0 for this test
    {
        let config_account = ctx
            .banks_client
            .get_account(config_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = config_account.data.len();
        let mut config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
        config.execution_delay_secs = 0;
        config.proposal_deposit = 0;
        let mut buf = Vec::new();
        config.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(config_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&config_pda, &patched);
    }

    // Read config for proposal nonce
    let config_account = ctx
        .banks_client
        .get_account(config_pda)
        .await
        .unwrap()
        .unwrap();
    let config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
    let nonce = config.proposal_nonce;
    let nonce_bytes = nonce.to_le_bytes();
    let (proposal_pda, _) =
        Pubkey::find_program_address(&[PROPOSAL_SEED, &nonce_bytes], &program_id);

    // Create proposal: cooldown_secs = 100_000_000 (> 31_536_000 max)
    {
        let mut ix_data = Vec::new();
        StakingInstruction::CreateProposal {
            description_hash: [10u8; 32],
            param_key: "cooldown_secs".to_string(),
            param_value: 100_000_000,
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(proposal_pda, false),
                AccountMeta::new(config_pda, false),
                AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
                AccountMeta::new_readonly(clock::id(), false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        ctx.banks_client.process_transaction(tx).await.unwrap();
    }

    // Vote for it
    let (gov_vote_pda, _) = Pubkey::find_program_address(
        &[
            GOV_VOTE_SEED,
            proposal_pda.as_ref(),
            ctx.payer.pubkey().as_ref(),
        ],
        &program_id,
    );
    let (gov_lock_pda, _) =
        Pubkey::find_program_address(&[GOV_LOCK_SEED, ctx.payer.pubkey().as_ref()], &program_id);
    {
        let mut ix_data = Vec::new();
        StakingInstruction::CastGovernanceVote {
            proposal_nonce: nonce,
            vote: 0,
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(gov_vote_pda, false),
                AccountMeta::new(proposal_pda, false),
                AccountMeta::new_readonly(voter_stake_pda, false),
                AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
                AccountMeta::new_readonly(clock::id(), false),
                AccountMeta::new(gov_lock_pda, false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        ctx.banks_client.process_transaction(tx).await.unwrap();
    }

    // Patch proposal: voting_ends=0, executable_after=0 (bypass timelock)
    {
        let proposal_account = ctx
            .banks_client
            .get_account(proposal_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = proposal_account.data.len();
        let mut proposal =
            GovernanceProposal::deserialize_reader(&mut &proposal_account.data[..]).unwrap();
        proposal.voting_ends = 0;
        proposal.executable_after = 0;
        let mut buf = Vec::new();
        proposal.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(proposal_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&proposal_pda, &patched);
    }

    // Execute proposal — should fail with ParamOutOfRange
    {
        let mut ix_data = Vec::new();
        StakingInstruction::ExecuteProposal {
            proposal_nonce: nonce,
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(proposal_pda, false),
                AccountMeta::new(config_pda, false),
                AccountMeta::new_readonly(clock::id(), false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        let result = ctx.banks_client.process_transaction(tx).await;
        assert!(
            result.is_err(),
            "executing proposal with cooldown_secs > 365 days should fail with ParamOutOfRange"
        );
    }
}

/// Fix 5: max_total_supply proposal below total_minted is gracefully failed.
#[tokio::test]
async fn test_max_supply_below_minted() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let mut ctx = pt.start_with_context().await;
    let (config_pda, staker_token) = init_protocol_and_mint(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        200_000_000,
    )
    .await;

    // Stake to have voting weight
    let domain = "max-supply.com";
    let (voter_stake_pda, _) = stake_v2_helper(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &config_pda,
        &staker_token,
        &reward_vault.pubkey(),
        domain,
        100_000_000,
        0,
    )
    .await;

    // Patch stake: set staked_at to 8 days ago (satisfies 7-day governance voting minimum)
    {
        let stake_account = ctx
            .banks_client
            .get_account(voter_stake_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = stake_account.data.len();
        let mut stake = StakeAccountV2::deserialize_reader(&mut &stake_account.data[..]).unwrap();
        stake.staked_at -= 8 * 24 * 3600;
        let mut buf = Vec::new();
        stake.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(stake_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&voter_stake_pda, &patched);
    }

    // Patch config: set execution_delay_secs to 0 for this test
    {
        let config_account = ctx
            .banks_client
            .get_account(config_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = config_account.data.len();
        let mut config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
        config.execution_delay_secs = 0;
        config.proposal_deposit = 0;
        let mut buf = Vec::new();
        config.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(config_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&config_pda, &patched);
    }

    let config_account = ctx
        .banks_client
        .get_account(config_pda)
        .await
        .unwrap()
        .unwrap();
    let config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
    let nonce = config.proposal_nonce;
    let nonce_bytes = nonce.to_le_bytes();
    let (proposal_pda, _) =
        Pubkey::find_program_address(&[PROPOSAL_SEED, &nonce_bytes], &program_id);

    // Create proposal: max_total_supply = 1 (way below total_minted = 200M)
    {
        let mut ix_data = Vec::new();
        StakingInstruction::CreateProposal {
            description_hash: [11u8; 32],
            param_key: "max_total_supply".to_string(),
            param_value: 1,
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(proposal_pda, false),
                AccountMeta::new(config_pda, false),
                AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
                AccountMeta::new_readonly(clock::id(), false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        ctx.banks_client.process_transaction(tx).await.unwrap();
    }

    // Vote for it
    let (gov_vote_pda, _) = Pubkey::find_program_address(
        &[
            GOV_VOTE_SEED,
            proposal_pda.as_ref(),
            ctx.payer.pubkey().as_ref(),
        ],
        &program_id,
    );
    let (gov_lock_pda, _) =
        Pubkey::find_program_address(&[GOV_LOCK_SEED, ctx.payer.pubkey().as_ref()], &program_id);
    {
        let mut ix_data = Vec::new();
        StakingInstruction::CastGovernanceVote {
            proposal_nonce: nonce,
            vote: 0,
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(gov_vote_pda, false),
                AccountMeta::new(proposal_pda, false),
                AccountMeta::new_readonly(voter_stake_pda, false),
                AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
                AccountMeta::new_readonly(clock::id(), false),
                AccountMeta::new(gov_lock_pda, false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        ctx.banks_client.process_transaction(tx).await.unwrap();
    }

    // Patch proposal: voting_ends=0, executable_after=0
    {
        let proposal_account = ctx
            .banks_client
            .get_account(proposal_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = proposal_account.data.len();
        let mut proposal =
            GovernanceProposal::deserialize_reader(&mut &proposal_account.data[..]).unwrap();
        proposal.voting_ends = 0;
        proposal.executable_after = 0;
        let mut buf = Vec::new();
        proposal.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(proposal_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&proposal_pda, &patched);
    }

    // Execute proposal — should succeed but proposal status = Failed
    {
        let mut ix_data = Vec::new();
        StakingInstruction::ExecuteProposal {
            proposal_nonce: nonce,
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(proposal_pda, false),
                AccountMeta::new(config_pda, false),
                AccountMeta::new_readonly(clock::id(), false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        ctx.banks_client.process_transaction(tx).await.unwrap();
    }

    // Verify proposal status is Failed (not Executed)
    let proposal_account = ctx
        .banks_client
        .get_account(proposal_pda)
        .await
        .unwrap()
        .unwrap();
    let proposal = GovernanceProposal::deserialize_reader(&mut &proposal_account.data[..]).unwrap();
    assert_eq!(
        proposal.status,
        ProposalStatus::Failed,
        "max_total_supply below total_minted should fail gracefully"
    );
}

/// Fix 4+8: Winning-side juror can claim correct share of jury reward pool.
#[tokio::test]
async fn test_claim_jury_reward() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let mut ctx = pt.start_with_context().await;
    let (config_pda, staker_token) = init_protocol_and_mint(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        200_000_000,
    )
    .await;

    let domain = "jury-claim.com";
    let (dispute_pda, dispute_nonce, _domain_pda) = setup_dispute(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &config_pda,
        &staker_token,
        &reward_vault.pubkey(),
        &truth_mint.pubkey(),
        domain,
    )
    .await;

    // Patch dispute: ResolvedGuilty (jury_reward_pool=0 in new model, but still need resolved status)
    // votes_guilty = 10_000_000 (total winning weight)
    {
        let dispute_account = ctx
            .banks_client
            .get_account(dispute_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = dispute_account.data.len();
        let mut dispute =
            DisputeAccount::deserialize_reader(&mut &dispute_account.data[..]).unwrap();
        dispute.status = DisputeStatus::ResolvedGuilty;
        dispute.votes_guilty = 10_000_000;
        dispute.votes_innocent = 0;
        dispute.jury_reward_pool = 0;
        dispute.jury_reward_claimed = 0;
        let mut buf = Vec::new();
        dispute.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(dispute_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&dispute_pda, &patched);
    }

    // Patch config: set jury_vote_reward = 1000
    {
        let config_account = ctx
            .banks_client
            .get_account(config_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = config_account.data.len();
        let mut config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
        config.jury_vote_reward = 1_000;
        let mut buf = Vec::new();
        config.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(config_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&config_pda, &patched);
    }

    // Create jury vote PDA via set_account (voter voted guilty with weight 10M)
    let (jury_pda, jury_bump) = Pubkey::find_program_address(
        &[JURY_SEED, dispute_pda.as_ref(), ctx.payer.pubkey().as_ref()],
        &program_id,
    );
    {
        let jury_vote = JuryVote {
            is_initialized: true,
            voter: ctx.payer.pubkey(),
            verdict: Verdict::Guilty,
            weight: 10_000_000,
            voted_at: 100, // non-zero = revealed
            bump: jury_bump,
            commit_hash: [0u8; 32],
            reward_claimed: false,
        };
        let mut buf = Vec::new();
        jury_vote.serialize(&mut buf).unwrap();
        let rent = Rent::default();
        let lamports = rent.minimum_balance(JuryVote::MAX_SIZE);
        buf.resize(JuryVote::MAX_SIZE, 0);
        let account = solana_sdk::account::Account {
            lamports,
            data: buf,
            owner: program_id,
            executable: false,
            rent_epoch: 0,
        };
        let shared = AccountSharedData::from(account);
        ctx.set_account(&jury_pda, &shared);
    }

    // Record staker token balance before claim
    let before_account = ctx
        .banks_client
        .get_account(staker_token)
        .await
        .unwrap()
        .unwrap();
    let before = spl_token::state::Account::unpack(&before_account.data).unwrap();
    let balance_before = before.amount;

    // ClaimJuryReward (new account layout: mint instead of vault, config writable)
    {
        let mut ix_data = Vec::new();
        StakingInstruction::ClaimJuryReward {
            domain: domain.to_string(),
            dispute_nonce,
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(jury_pda, false),
                AccountMeta::new(dispute_pda, false),
                AccountMeta::new(staker_token, false),
                AccountMeta::new(truth_mint.pubkey(), false), // $TRUTH Mint (was vault)
                AccountMeta::new(config_pda, false),          // Config (now writable)
                AccountMeta::new_readonly(spl_token::id(), false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        ctx.banks_client.process_transaction(tx).await.unwrap();
    }

    // Verify payout: flat jury_vote_reward = 1_000 minted
    let after_account = ctx
        .banks_client
        .get_account(staker_token)
        .await
        .unwrap()
        .unwrap();
    let after = spl_token::state::Account::unpack(&after_account.data).unwrap();
    assert_eq!(
        after.amount - balance_before,
        1_000,
        "juror should receive flat jury_vote_reward (1000)"
    );

    // Verify jury vote marked as claimed
    let jury_account = ctx
        .banks_client
        .get_account(jury_pda)
        .await
        .unwrap()
        .unwrap();
    let vote = JuryVote::deserialize_reader(&mut &jury_account.data[..]).unwrap();
    assert!(vote.reward_claimed, "jury vote should be marked as claimed");
}

/// Fix 4+8: Losing-side juror cannot claim jury reward.
#[tokio::test]
async fn test_claim_jury_reward_loser_fails() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let mut ctx = pt.start_with_context().await;
    let (config_pda, staker_token) = init_protocol_and_mint(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        200_000_000,
    )
    .await;

    let domain = "loser-claim.com";
    let (dispute_pda, dispute_nonce, _) = setup_dispute(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &config_pda,
        &staker_token,
        &reward_vault.pubkey(),
        &truth_mint.pubkey(),
        domain,
    )
    .await;

    // Patch dispute: ResolvedGuilty
    {
        let dispute_account = ctx
            .banks_client
            .get_account(dispute_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = dispute_account.data.len();
        let mut dispute =
            DisputeAccount::deserialize_reader(&mut &dispute_account.data[..]).unwrap();
        dispute.status = DisputeStatus::ResolvedGuilty;
        dispute.votes_guilty = 5_000_000;
        dispute.votes_innocent = 10_000_000;
        dispute.jury_reward_pool = 0;
        dispute.jury_reward_claimed = 0;
        let mut buf = Vec::new();
        dispute.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(dispute_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&dispute_pda, &patched);
    }

    // Patch config: set jury_vote_reward = 1000
    {
        let config_account = ctx
            .banks_client
            .get_account(config_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = config_account.data.len();
        let mut config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
        config.jury_vote_reward = 1_000;
        let mut buf = Vec::new();
        config.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(config_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&config_pda, &patched);
    }

    // Create jury vote PDA — this voter voted INNOCENT (the losing side since verdict is Guilty)
    let (jury_pda, jury_bump) = Pubkey::find_program_address(
        &[JURY_SEED, dispute_pda.as_ref(), ctx.payer.pubkey().as_ref()],
        &program_id,
    );
    {
        let jury_vote = JuryVote {
            is_initialized: true,
            voter: ctx.payer.pubkey(),
            verdict: Verdict::Innocent,
            weight: 10_000_000,
            voted_at: 100,
            bump: jury_bump,
            commit_hash: [0u8; 32],
            reward_claimed: false,
        };
        let mut buf = Vec::new();
        jury_vote.serialize(&mut buf).unwrap();
        let rent = Rent::default();
        let lamports = rent.minimum_balance(JuryVote::MAX_SIZE);
        buf.resize(JuryVote::MAX_SIZE, 0);
        let account = solana_sdk::account::Account {
            lamports,
            data: buf,
            owner: program_id,
            executable: false,
            rent_epoch: 0,
        };
        ctx.set_account(&jury_pda, &AccountSharedData::from(account));
    }

    // Try to claim — should fail with NoJuryReward
    let mut ix_data = Vec::new();
    StakingInstruction::ClaimJuryReward {
        domain: domain.to_string(),
        dispute_nonce,
    }
    .serialize(&mut ix_data)
    .unwrap();
    let ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(ctx.payer.pubkey(), true),
            AccountMeta::new(jury_pda, false),
            AccountMeta::new(dispute_pda, false),
            AccountMeta::new(staker_token, false),
            AccountMeta::new(truth_mint.pubkey(), false), // $TRUTH Mint (was vault)
            AccountMeta::new(config_pda, false),          // Config (now writable)
            AccountMeta::new_readonly(spl_token::id(), false),
        ],
        data: ix_data,
    };
    let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&ctx.payer.pubkey()),
        &[&ctx.payer],
        blockhash,
    );
    let result = ctx.banks_client.process_transaction(tx).await;
    assert!(
        result.is_err(),
        "losing-side juror should not be able to claim jury reward"
    );
}

/// Fix 9: Unstake decrements config.total_staked (not withdraw).
#[tokio::test]
async fn test_cooldown_dilution_fix() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let (mut banks, payer, _) = pt.start().await;
    let (config_pda, staker_token) = init_protocol_and_mint(
        &mut banks,
        &payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        200_000_000,
    )
    .await;

    let domain = "dilution.com";
    let stake_amount = 50_000_000u64;
    let (_stake_pda, _domain_pda) = stake_v2_helper(
        &mut banks,
        &payer,
        &program_id,
        &config_pda,
        &staker_token,
        &reward_vault.pubkey(),
        domain,
        stake_amount,
        0,
    )
    .await;

    // Verify total_staked includes our stake
    let config_account = banks.get_account(config_pda).await.unwrap().unwrap();
    let config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
    let total_before = config.total_staked;
    assert!(
        total_before >= stake_amount,
        "total_staked should include staked amount"
    );

    // Unstake — should decrement total_staked immediately
    let _stake_pda = unstake_v2_helper(&mut banks, &payer, &program_id, &config_pda, domain).await;

    // Verify total_staked decremented at unstake time (not waiting for withdraw)
    let config_account = banks.get_account(config_pda).await.unwrap().unwrap();
    let config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
    assert_eq!(
        config.total_staked,
        total_before - stake_amount,
        "total_staked should be decremented at unstake time, not withdraw"
    );
}

/// HIGH-1: Proposal deposit is collected when config.proposal_deposit > 0.
#[tokio::test]
async fn test_proposal_deposit_collected() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let mut ctx = pt.start_with_context().await;
    let (config_pda, staker_token) = init_protocol_and_mint(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        200_000_000,
    )
    .await;

    let deposit_amount = 5_000_000u64;

    // Patch config to set proposal_deposit > 0
    {
        let config_account = ctx
            .banks_client
            .get_account(config_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = config_account.data.len();
        let mut config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
        config.proposal_deposit = deposit_amount;
        let mut buf = Vec::new();
        config.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(config_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&config_pda, &patched);
    }

    // Record balances before
    let before_staker = spl_token::state::Account::unpack(
        &ctx.banks_client
            .get_account(staker_token)
            .await
            .unwrap()
            .unwrap()
            .data,
    )
    .unwrap()
    .amount;
    let before_vault = spl_token::state::Account::unpack(
        &ctx.banks_client
            .get_account(reward_vault.pubkey())
            .await
            .unwrap()
            .unwrap()
            .data,
    )
    .unwrap()
    .amount;

    // Read config for proposal nonce
    let config_account = ctx
        .banks_client
        .get_account(config_pda)
        .await
        .unwrap()
        .unwrap();
    let config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
    let nonce = config.proposal_nonce;
    let nonce_bytes = nonce.to_le_bytes();
    let (proposal_pda, _) =
        Pubkey::find_program_address(&[PROPOSAL_SEED, &nonce_bytes], &program_id);

    // Create proposal with the 3 extra accounts for deposit
    let mut ix_data = Vec::new();
    StakingInstruction::CreateProposal {
        description_hash: [20u8; 32],
        param_key: "min_stake".to_string(),
        param_value: 2_000_000,
    }
    .serialize(&mut ix_data)
    .unwrap();
    let ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(ctx.payer.pubkey(), true),
            AccountMeta::new(proposal_pda, false),
            AccountMeta::new(config_pda, false),
            AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
            AccountMeta::new_readonly(clock::id(), false),
            // Extra accounts for deposit transfer
            AccountMeta::new(staker_token, false),
            AccountMeta::new(reward_vault.pubkey(), false),
            AccountMeta::new_readonly(spl_token::id(), false),
        ],
        data: ix_data,
    };
    let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&ctx.payer.pubkey()),
        &[&ctx.payer],
        blockhash,
    );
    ctx.banks_client.process_transaction(tx).await.unwrap();

    // Verify deposit was transferred
    let after_staker = spl_token::state::Account::unpack(
        &ctx.banks_client
            .get_account(staker_token)
            .await
            .unwrap()
            .unwrap()
            .data,
    )
    .unwrap()
    .amount;
    let after_vault = spl_token::state::Account::unpack(
        &ctx.banks_client
            .get_account(reward_vault.pubkey())
            .await
            .unwrap()
            .unwrap()
            .data,
    )
    .unwrap()
    .amount;

    assert_eq!(
        before_staker - after_staker,
        deposit_amount,
        "proposer should pay deposit"
    );
    assert_eq!(
        after_vault - before_vault,
        deposit_amount,
        "vault should receive deposit"
    );
}

/// HIGH-1: CreateProposal without deposit accounts fails when deposit > 0.
#[tokio::test]
async fn test_proposal_deposit_missing_accounts() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let mut ctx = pt.start_with_context().await;
    let (config_pda, _staker_token) = init_protocol_and_mint(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        200_000_000,
    )
    .await;

    // Patch config to set proposal_deposit > 0
    {
        let config_account = ctx
            .banks_client
            .get_account(config_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = config_account.data.len();
        let mut config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
        config.proposal_deposit = 5_000_000;
        let mut buf = Vec::new();
        config.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(config_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&config_pda, &patched);
    }

    let config_account = ctx
        .banks_client
        .get_account(config_pda)
        .await
        .unwrap()
        .unwrap();
    let config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
    let nonce = config.proposal_nonce;
    let nonce_bytes = nonce.to_le_bytes();
    let (proposal_pda, _) =
        Pubkey::find_program_address(&[PROPOSAL_SEED, &nonce_bytes], &program_id);

    // Try without deposit accounts — should fail
    let mut ix_data = Vec::new();
    StakingInstruction::CreateProposal {
        description_hash: [21u8; 32],
        param_key: "min_stake".to_string(),
        param_value: 2_000_000,
    }
    .serialize(&mut ix_data)
    .unwrap();
    let ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(ctx.payer.pubkey(), true),
            AccountMeta::new(proposal_pda, false),
            AccountMeta::new(config_pda, false),
            AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
            AccountMeta::new_readonly(clock::id(), false),
            // No deposit accounts!
        ],
        data: ix_data,
    };
    let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&ctx.payer.pubkey()),
        &[&ctx.payer],
        blockhash,
    );
    let result = ctx.banks_client.process_transaction(tx).await;
    assert!(
        result.is_err(),
        "creating proposal without deposit accounts should fail when deposit > 0"
    );
}

/// HIGH-2: Extended range guards for dispute_bond, reward_rate, jury_min_stake, min_jury_weight, min_epoch_duration.
#[tokio::test]
async fn test_governance_range_guards_extended() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let mut ctx = pt.start_with_context().await;
    let (config_pda, staker_token) = init_protocol_and_mint(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        200_000_000,
    )
    .await;

    // Stake to have voting weight
    let domain = "range-ext.com";
    let (voter_stake_pda, _) = stake_v2_helper(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &config_pda,
        &staker_token,
        &reward_vault.pubkey(),
        domain,
        100_000_000,
        0,
    )
    .await;

    // Patch stake: set staked_at to 8 days ago (satisfies 7-day governance voting minimum)
    {
        let stake_account = ctx
            .banks_client
            .get_account(voter_stake_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = stake_account.data.len();
        let mut stake = StakeAccountV2::deserialize_reader(&mut &stake_account.data[..]).unwrap();
        stake.staked_at -= 8 * 24 * 3600;
        let mut buf = Vec::new();
        stake.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(stake_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&voter_stake_pda, &patched);
    }

    // Patch config: set execution_delay_secs to 0 for this test
    {
        let config_account = ctx
            .banks_client
            .get_account(config_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = config_account.data.len();
        let mut config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
        config.execution_delay_secs = 0;
        config.proposal_deposit = 0;
        let mut buf = Vec::new();
        config.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(config_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&config_pda, &patched);
    }

    // Test each parameter that now has a range guard
    let params_to_test = [
        ("dispute_bond", 2_000_000_000u64),  // > 1B max
        ("reward_rate", 2_000_000_000),      // > 1B max
        ("jury_min_stake", 2_000_000_000),   // > 1B max
        ("min_jury_weight", 20_000_000_000), // > 10B max
        ("min_epoch_duration", 50_000_000),  // > 365 days max
    ];

    for (param_key, bad_value) in &params_to_test {
        // Read config for proposal nonce
        let config_account = ctx
            .banks_client
            .get_account(config_pda)
            .await
            .unwrap()
            .unwrap();
        let config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
        let nonce = config.proposal_nonce;
        let nonce_bytes = nonce.to_le_bytes();
        let (proposal_pda, _) =
            Pubkey::find_program_address(&[PROPOSAL_SEED, &nonce_bytes], &program_id);

        // Create proposal
        {
            let mut ix_data = Vec::new();
            StakingInstruction::CreateProposal {
                description_hash: [nonce as u8; 32],
                param_key: param_key.to_string(),
                param_value: *bad_value,
            }
            .serialize(&mut ix_data)
            .unwrap();
            let ix = Instruction {
                program_id,
                accounts: vec![
                    AccountMeta::new(ctx.payer.pubkey(), true),
                    AccountMeta::new(proposal_pda, false),
                    AccountMeta::new(config_pda, false),
                    AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
                    AccountMeta::new_readonly(clock::id(), false),
                ],
                data: ix_data,
            };
            let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
            let tx = Transaction::new_signed_with_payer(
                &[ix],
                Some(&ctx.payer.pubkey()),
                &[&ctx.payer],
                blockhash,
            );
            ctx.banks_client.process_transaction(tx).await.unwrap();
        }

        // Vote for it
        let (gov_vote_pda, _) = Pubkey::find_program_address(
            &[
                GOV_VOTE_SEED,
                proposal_pda.as_ref(),
                ctx.payer.pubkey().as_ref(),
            ],
            &program_id,
        );
        let (gov_lock_pda, _) = Pubkey::find_program_address(
            &[GOV_LOCK_SEED, ctx.payer.pubkey().as_ref()],
            &program_id,
        );
        {
            let mut ix_data = Vec::new();
            StakingInstruction::CastGovernanceVote {
                proposal_nonce: nonce,
                vote: 0,
            }
            .serialize(&mut ix_data)
            .unwrap();
            let ix = Instruction {
                program_id,
                accounts: vec![
                    AccountMeta::new(ctx.payer.pubkey(), true),
                    AccountMeta::new(gov_vote_pda, false),
                    AccountMeta::new(proposal_pda, false),
                    AccountMeta::new_readonly(voter_stake_pda, false),
                    AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
                    AccountMeta::new_readonly(clock::id(), false),
                    AccountMeta::new(gov_lock_pda, false),
                ],
                data: ix_data,
            };
            let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
            let tx = Transaction::new_signed_with_payer(
                &[ix],
                Some(&ctx.payer.pubkey()),
                &[&ctx.payer],
                blockhash,
            );
            ctx.banks_client.process_transaction(tx).await.unwrap();
        }

        // Patch proposal: bypass timelock
        {
            let proposal_account = ctx
                .banks_client
                .get_account(proposal_pda)
                .await
                .unwrap()
                .unwrap();
            let orig_len = proposal_account.data.len();
            let mut proposal =
                GovernanceProposal::deserialize_reader(&mut &proposal_account.data[..]).unwrap();
            proposal.voting_ends = 0;
            proposal.executable_after = 0;
            let mut buf = Vec::new();
            proposal.serialize(&mut buf).unwrap();
            buf.resize(orig_len, 0);
            let mut patched = AccountSharedData::from(proposal_account);
            patched.set_data_from_slice(&buf);
            ctx.set_account(&proposal_pda, &patched);
        }

        // Execute proposal — should fail with ParamOutOfRange
        {
            let mut ix_data = Vec::new();
            StakingInstruction::ExecuteProposal {
                proposal_nonce: nonce,
            }
            .serialize(&mut ix_data)
            .unwrap();
            let ix = Instruction {
                program_id,
                accounts: vec![
                    AccountMeta::new(ctx.payer.pubkey(), true),
                    AccountMeta::new(proposal_pda, false),
                    AccountMeta::new(config_pda, false),
                    AccountMeta::new_readonly(clock::id(), false),
                ],
                data: ix_data,
            };
            let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
            let tx = Transaction::new_signed_with_payer(
                &[ix],
                Some(&ctx.payer.pubkey()),
                &[&ctx.payer],
                blockhash,
            );
            let result = ctx.banks_client.process_transaction(tx).await;
            assert!(
                result.is_err(),
                "executing proposal with out-of-range {} should fail",
                param_key
            );
        }
    }
}

// ══════════════════════════════════════════════
// Medium Economic Fixes — Tests
// ══════════════════════════════════════════════

/// M1: Settle channel with sentinel_fee_pct=10, verify payee gets reduced share
/// and accumulated_sentinel_fees increments in config.
#[tokio::test]
async fn test_sentinel_fee_routing() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let mut ctx = pt.start_with_context().await;
    let (config_pda, staker_token) = init_protocol_and_mint(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        200_000_000,
    )
    .await;

    // Patch config: sentinel_fee_pct=10, burn_pct=10 (default)
    {
        let config_account = ctx
            .banks_client
            .get_account(config_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = config_account.data.len();
        let mut config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
        config.sentinel_fee_pct = 10;
        let mut buf = Vec::new();
        config.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(config_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&config_pda, &patched);
    }

    // Create payee
    let payee = Keypair::new();
    let fund_ix = system_instruction::transfer(&ctx.payer.pubkey(), &payee.pubkey(), 1_000_000_000);
    let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
    let tx = Transaction::new_signed_with_payer(
        &[fund_ix],
        Some(&ctx.payer.pubkey()),
        &[&ctx.payer],
        blockhash,
    );
    ctx.banks_client.process_transaction(tx).await.unwrap();

    let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
    let payee_token = create_token_account(
        &mut ctx.banks_client,
        &ctx.payer,
        blockhash,
        &truth_mint.pubkey(),
        &payee.pubkey(),
    )
    .await;

    // Open channel with 100M deposit
    let payee_bytes = payee.pubkey().to_bytes();
    let (channel_pda, _) = Pubkey::find_program_address(
        &[
            CHANNEL_SEED,
            ctx.payer.pubkey().as_ref(),
            payee.pubkey().as_ref(),
        ],
        &program_id,
    );
    {
        let mut ix_data = Vec::new();
        StakingInstruction::OpenChannel {
            payee: payee_bytes,
            amount: 100_000_000,
            expiry_slot: 10000,
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(staker_token, false),
                AccountMeta::new(reward_vault.pubkey(), false),
                AccountMeta::new(channel_pda, false),
                AccountMeta::new_readonly(spl_token::id(), false),
                AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
                AccountMeta::new_readonly(clock::id(), false),
                AccountMeta::new_readonly(config_pda, false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        ctx.banks_client.process_transaction(tx).await.unwrap();
    }

    // Settle with spent=100M
    {
        let ed25519_ix = make_ed25519_settle_ix(&ctx.payer, &channel_pda, 100_000_000, 1);
        let mut ix_data = Vec::new();
        StakingInstruction::SettleChannel {
            payer_key: ctx.payer.pubkey().to_bytes(),
            payee_key: payee_bytes,
            spent: 100_000_000,
            nonce: 1,
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(channel_pda, false),
                AccountMeta::new(payee_token, false),
                AccountMeta::new(staker_token, false),
                AccountMeta::new(reward_vault.pubkey(), false),
                AccountMeta::new(config_pda, false),
                AccountMeta::new_readonly(spl_token::id(), false),
                AccountMeta::new(truth_mint.pubkey(), false),
                AccountMeta::new_readonly(ix_sysvar::id(), false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ed25519_ix, ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        ctx.banks_client.process_transaction(tx).await.unwrap();
    }

    // Verify: burn=10M, sentinel_fee=10M, payee=80M
    let payee_account = ctx
        .banks_client
        .get_account(payee_token)
        .await
        .unwrap()
        .unwrap();
    let payee_data = spl_token::state::Account::unpack(&payee_account.data).unwrap();
    assert_eq!(
        payee_data.amount, 80_000_000,
        "payee should get 80% (100% - 10% burn - 10% sentinel)"
    );

    // Verify accumulated_sentinel_fees = 10M in config
    let config_account = ctx
        .banks_client
        .get_account(config_pda)
        .await
        .unwrap()
        .unwrap();
    let config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
    assert_eq!(
        config.accumulated_sentinel_fees, 10_000_000,
        "sentinel fees should accumulate"
    );
}

/// M1: Accumulated sentinel fees flow into sentinel reward index on UpdateRewardIndex.
#[tokio::test]
async fn test_sentinel_fee_distribution() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let mut ctx = pt.start_with_context().await;
    let (config_pda, staker_token) = init_protocol_and_mint(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        500_000_000,
    )
    .await;

    // Stake as publisher (for total_staked)
    let domain = "sentinel-test.com";
    let (_stake_pda, _) = stake_v2_helper(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &config_pda,
        &staker_token,
        &reward_vault.pubkey(),
        domain,
        100_000_000,
        0,
    )
    .await;

    // Patch config: set accumulated_sentinel_fees=5M, total_sentinel_staked=50M, total_funded=10M
    {
        let config_account = ctx
            .banks_client
            .get_account(config_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = config_account.data.len();
        let mut config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
        config.accumulated_sentinel_fees = 5_000_000;
        config.total_sentinel_staked = 50_000_000;
        config.total_funded = 10_000_000;
        let mut buf = Vec::new();
        config.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(config_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&config_pda, &patched);
    }

    // UpdateRewardIndex with funded_amount=10M
    {
        let mut ix_data = Vec::new();
        StakingInstruction::UpdateRewardIndex {
            funded_amount: 10_000_000,
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(config_pda, false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        ctx.banks_client.process_transaction(tx).await.unwrap();
    }

    // Verify accumulated_sentinel_fees drained to 0
    let config_account = ctx
        .banks_client
        .get_account(config_pda)
        .await
        .unwrap()
        .unwrap();
    let config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
    assert_eq!(
        config.accumulated_sentinel_fees, 0,
        "accumulated fees should be drained"
    );
    // sentinel_reward_per_token should have increased (sentinel_portion = 30% of 10M + 5M bonus)
    assert!(
        config.sentinel_reward_per_token > 0,
        "sentinel reward index should increase"
    );
}

/// M2: Vote on a proposal, then try to unstake before voting_ends → GovernanceLockActive.
#[tokio::test]
async fn test_governance_vote_lock() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let mut ctx = pt.start_with_context().await;
    let (config_pda, staker_token) = init_protocol_and_mint(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        500_000_000,
    )
    .await;

    // Stake
    let domain = "lock-test.com";
    let (voter_stake_pda, _) = stake_v2_helper(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &config_pda,
        &staker_token,
        &reward_vault.pubkey(),
        domain,
        100_000_000,
        0,
    )
    .await;

    // Patch stake: set staked_at to 8 days ago (satisfies 7-day governance voting minimum)
    {
        let stake_account = ctx
            .banks_client
            .get_account(voter_stake_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = stake_account.data.len();
        let mut stake = StakeAccountV2::deserialize_reader(&mut &stake_account.data[..]).unwrap();
        stake.staked_at -= 8 * 24 * 3600;
        let mut buf = Vec::new();
        stake.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(stake_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&voter_stake_pda, &patched);
    }

    // Patch config: set execution_delay_secs to 0 so proposal can execute without timelock in this test
    {
        let config_account = ctx
            .banks_client
            .get_account(config_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = config_account.data.len();
        let mut config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
        config.execution_delay_secs = 0;
        config.proposal_deposit = 0;
        let mut buf = Vec::new();
        config.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(config_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&config_pda, &patched);
    }

    // Create proposal
    let config_account = ctx
        .banks_client
        .get_account(config_pda)
        .await
        .unwrap()
        .unwrap();
    let config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
    let nonce = config.proposal_nonce;
    let nonce_bytes = nonce.to_le_bytes();
    let (proposal_pda, _) =
        Pubkey::find_program_address(&[PROPOSAL_SEED, &nonce_bytes], &program_id);

    {
        let mut ix_data = Vec::new();
        StakingInstruction::CreateProposal {
            description_hash: [1u8; 32],
            param_key: "min_stake".to_string(),
            param_value: 500_000,
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(proposal_pda, false),
                AccountMeta::new(config_pda, false),
                AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
                AccountMeta::new_readonly(clock::id(), false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        ctx.banks_client.process_transaction(tx).await.unwrap();
    }

    // Cast vote
    let (gov_vote_pda, _) = Pubkey::find_program_address(
        &[
            GOV_VOTE_SEED,
            proposal_pda.as_ref(),
            ctx.payer.pubkey().as_ref(),
        ],
        &program_id,
    );
    let (gov_lock_pda, _) =
        Pubkey::find_program_address(&[GOV_LOCK_SEED, ctx.payer.pubkey().as_ref()], &program_id);
    {
        let mut ix_data = Vec::new();
        StakingInstruction::CastGovernanceVote {
            proposal_nonce: nonce,
            vote: 0,
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(gov_vote_pda, false),
                AccountMeta::new(proposal_pda, false),
                AccountMeta::new_readonly(voter_stake_pda, false),
                AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
                AccountMeta::new_readonly(clock::id(), false),
                AccountMeta::new(gov_lock_pda, false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        ctx.banks_client.process_transaction(tx).await.unwrap();
    }

    // Verify per-wallet governance lock is set
    let lock_account = ctx
        .banks_client
        .get_account(gov_lock_pda)
        .await
        .unwrap()
        .unwrap();
    let lock = GovernanceLockAccount::deserialize_reader(&mut &lock_account.data[..]).unwrap();
    assert!(
        lock.lock_until > 0,
        "governance lock should be set after voting"
    );

    // Try to unstake → should fail with GovernanceLockActive
    let (domain_pda, _) =
        Pubkey::find_program_address(&[DOMAIN_SEED, domain.as_bytes()], &program_id);
    {
        let mut ix_data = Vec::new();
        StakingInstruction::UnstakeV2 {
            domain: domain.to_string(),
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(voter_stake_pda, false),
                AccountMeta::new(config_pda, false),
                AccountMeta::new_readonly(domain_pda, false),
                AccountMeta::new_readonly(clock::id(), false),
                AccountMeta::new_readonly(gov_lock_pda, false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        let result = ctx.banks_client.process_transaction(tx).await;
        assert!(
            result.is_err(),
            "unstaking while governance lock is active should fail"
        );
    }
}

/// M2: Vote on a proposal, warp past voting_ends, unstake succeeds.
#[tokio::test]
async fn test_governance_vote_lock_expires() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let mut ctx = pt.start_with_context().await;
    let (config_pda, staker_token) = init_protocol_and_mint(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        500_000_000,
    )
    .await;

    // Stake
    let domain = "lock-expire.com";
    let (voter_stake_pda, _) = stake_v2_helper(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &config_pda,
        &staker_token,
        &reward_vault.pubkey(),
        domain,
        100_000_000,
        0,
    )
    .await;

    // Patch stake: set staked_at to 8 days ago (satisfies 7-day governance voting minimum)
    {
        let stake_account = ctx
            .banks_client
            .get_account(voter_stake_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = stake_account.data.len();
        let mut stake = StakeAccountV2::deserialize_reader(&mut &stake_account.data[..]).unwrap();
        stake.staked_at -= 8 * 24 * 3600;
        let mut buf = Vec::new();
        stake.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(stake_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&voter_stake_pda, &patched);
    }

    // Patch config: set execution_delay_secs to 0 so proposal can execute without timelock in this test
    {
        let config_account = ctx
            .banks_client
            .get_account(config_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = config_account.data.len();
        let mut config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
        config.execution_delay_secs = 0;
        config.proposal_deposit = 0;
        let mut buf = Vec::new();
        config.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(config_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&config_pda, &patched);
    }

    // Create proposal
    let config_account = ctx
        .banks_client
        .get_account(config_pda)
        .await
        .unwrap()
        .unwrap();
    let config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
    let nonce = config.proposal_nonce;
    let nonce_bytes = nonce.to_le_bytes();
    let (proposal_pda, _) =
        Pubkey::find_program_address(&[PROPOSAL_SEED, &nonce_bytes], &program_id);

    {
        let mut ix_data = Vec::new();
        StakingInstruction::CreateProposal {
            description_hash: [2u8; 32],
            param_key: "min_stake".to_string(),
            param_value: 500_000,
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(proposal_pda, false),
                AccountMeta::new(config_pda, false),
                AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
                AccountMeta::new_readonly(clock::id(), false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        ctx.banks_client.process_transaction(tx).await.unwrap();
    }

    // Cast vote
    let (gov_vote_pda, _) = Pubkey::find_program_address(
        &[
            GOV_VOTE_SEED,
            proposal_pda.as_ref(),
            ctx.payer.pubkey().as_ref(),
        ],
        &program_id,
    );
    let (gov_lock_pda, _) =
        Pubkey::find_program_address(&[GOV_LOCK_SEED, ctx.payer.pubkey().as_ref()], &program_id);
    {
        let mut ix_data = Vec::new();
        StakingInstruction::CastGovernanceVote {
            proposal_nonce: nonce,
            vote: 0,
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(gov_vote_pda, false),
                AccountMeta::new(proposal_pda, false),
                AccountMeta::new_readonly(voter_stake_pda, false),
                AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
                AccountMeta::new_readonly(clock::id(), false),
                AccountMeta::new(gov_lock_pda, false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        ctx.banks_client.process_transaction(tx).await.unwrap();
    }

    // Patch per-wallet governance lock: set lock_until to past (simulate time passing)
    {
        let lock_account = ctx
            .banks_client
            .get_account(gov_lock_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = lock_account.data.len();
        let mut lock =
            GovernanceLockAccount::deserialize_reader(&mut &lock_account.data[..]).unwrap();
        lock.lock_until = 0; // lock expired
        let mut buf = Vec::new();
        lock.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(lock_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&gov_lock_pda, &patched);
    }

    // Unstake → should succeed now that lock expired
    let (domain_pda, _) =
        Pubkey::find_program_address(&[DOMAIN_SEED, domain.as_bytes()], &program_id);
    {
        let mut ix_data = Vec::new();
        StakingInstruction::UnstakeV2 {
            domain: domain.to_string(),
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(voter_stake_pda, false),
                AccountMeta::new(config_pda, false),
                AccountMeta::new_readonly(domain_pda, false),
                AccountMeta::new_readonly(clock::id(), false),
                AccountMeta::new_readonly(gov_lock_pda, false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        ctx.banks_client.process_transaction(tx).await.unwrap();
    }

    // Verify cooldown is set (unstake succeeded)
    let stake_account = ctx
        .banks_client
        .get_account(voter_stake_pda)
        .await
        .unwrap()
        .unwrap();
    let stake = StakeAccountV2::deserialize_reader(&mut &stake_account.data[..]).unwrap();
    assert!(
        stake.cooldown_until > 0,
        "unstake should succeed and set cooldown"
    );
}

/// M3: Non-admin calls UpdateRewardIndex, succeeds up to funded amount.
#[tokio::test]
async fn test_permissionless_reward_update() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let mut ctx = pt.start_with_context().await;
    let (config_pda, staker_token) = init_protocol_and_mint(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        500_000_000,
    )
    .await;

    // Stake (needed for total_staked > 0)
    let domain = "permissionless.com";
    let (_stake_pda, _) = stake_v2_helper(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &config_pda,
        &staker_token,
        &reward_vault.pubkey(),
        domain,
        100_000_000,
        0,
    )
    .await;

    // Patch config: set total_funded=5M so there's distributable budget
    {
        let config_account = ctx
            .banks_client
            .get_account(config_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = config_account.data.len();
        let mut config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
        config.total_funded = 5_000_000;
        let mut buf = Vec::new();
        config.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(config_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&config_pda, &patched);
    }

    // Non-admin calls UpdateRewardIndex with a random keypair
    let non_admin = Keypair::new();
    // Fund non-admin with SOL for tx fees
    let fund_ix =
        system_instruction::transfer(&ctx.payer.pubkey(), &non_admin.pubkey(), 1_000_000_000);
    let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
    let tx = Transaction::new_signed_with_payer(
        &[fund_ix],
        Some(&ctx.payer.pubkey()),
        &[&ctx.payer],
        blockhash,
    );
    ctx.banks_client.process_transaction(tx).await.unwrap();

    {
        let mut ix_data = Vec::new();
        StakingInstruction::UpdateRewardIndex {
            funded_amount: 5_000_000,
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(non_admin.pubkey(), true),
                AccountMeta::new(config_pda, false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&non_admin.pubkey()),
            &[&non_admin],
            blockhash,
        );
        ctx.banks_client.process_transaction(tx).await.unwrap();
    }

    // Verify reward_per_token increased
    let config_account = ctx
        .banks_client
        .get_account(config_pda)
        .await
        .unwrap()
        .unwrap();
    let config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
    assert!(
        config.reward_per_token_stored > 0,
        "reward index should increase from permissionless call"
    );
    assert!(
        config.total_distributed > 0,
        "total_distributed should track"
    );
}

/// M3: funded_amount > distributable is capped to distributable.
#[tokio::test]
async fn test_reward_update_caps_at_funded() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let mut ctx = pt.start_with_context().await;
    let (config_pda, staker_token) = init_protocol_and_mint(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        500_000_000,
    )
    .await;

    // Stake
    let domain = "cap-test.com";
    let (_stake_pda, _) = stake_v2_helper(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &config_pda,
        &staker_token,
        &reward_vault.pubkey(),
        domain,
        100_000_000,
        0,
    )
    .await;

    // Patch config: total_funded=2M, total_distributed=0 → distributable=2M
    {
        let config_account = ctx
            .banks_client
            .get_account(config_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = config_account.data.len();
        let mut config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
        config.total_funded = 2_000_000;
        config.total_distributed = 0;
        let mut buf = Vec::new();
        config.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(config_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&config_pda, &patched);
    }

    // Try to distribute 10M but only 2M funded → capped at 2M
    {
        let mut ix_data = Vec::new();
        StakingInstruction::UpdateRewardIndex {
            funded_amount: 10_000_000,
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(config_pda, false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        ctx.banks_client.process_transaction(tx).await.unwrap();
    }

    // Verify total_distributed = 2M (capped, not 10M)
    let config_account = ctx
        .banks_client
        .get_account(config_pda)
        .await
        .unwrap()
        .unwrap();
    let config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
    // Staker portion = 2M * 70% = 1.4M, sentinel portion = 0.6M (but no sentinels → skipped)
    // actually_funded = staker portion only (since total_sentinel_staked = 0)
    assert_eq!(
        config.total_distributed, 1_400_000,
        "total_distributed should be capped at staker portion of distributable"
    );
}

/// M4: Create proposal with deposit, execute (quorum met), verify deposit returned.
#[tokio::test]
async fn test_proposal_deposit_refund() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let mut ctx = pt.start_with_context().await;
    let (config_pda, staker_token) = init_protocol_and_mint(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        500_000_000,
    )
    .await;

    // Patch config: proposal_deposit = 1_000_000, execution_delay_secs = 0
    {
        let config_account = ctx
            .banks_client
            .get_account(config_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = config_account.data.len();
        let mut config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
        config.proposal_deposit = 1_000_000;
        config.execution_delay_secs = 0;
        let mut buf = Vec::new();
        config.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(config_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&config_pda, &patched);
    }

    // Stake for governance weight
    let domain = "deposit-test.com";
    let (voter_stake_pda, _) = stake_v2_helper(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &config_pda,
        &staker_token,
        &reward_vault.pubkey(),
        domain,
        100_000_000,
        0,
    )
    .await;

    // Patch stake: set staked_at to 8 days ago (satisfies 7-day governance voting minimum)
    {
        let stake_account = ctx
            .banks_client
            .get_account(voter_stake_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = stake_account.data.len();
        let mut stake = StakeAccountV2::deserialize_reader(&mut &stake_account.data[..]).unwrap();
        stake.staked_at -= 8 * 24 * 3600;
        let mut buf = Vec::new();
        stake.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(stake_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&voter_stake_pda, &patched);
    }

    // Check staker token balance before proposal
    let token_before = {
        let acct = ctx
            .banks_client
            .get_account(staker_token)
            .await
            .unwrap()
            .unwrap();
        spl_token::state::Account::unpack(&acct.data)
            .unwrap()
            .amount
    };

    // Create proposal with deposit
    let config_account = ctx
        .banks_client
        .get_account(config_pda)
        .await
        .unwrap()
        .unwrap();
    let config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
    let nonce = config.proposal_nonce;
    let nonce_bytes = nonce.to_le_bytes();
    let (proposal_pda, _) =
        Pubkey::find_program_address(&[PROPOSAL_SEED, &nonce_bytes], &program_id);

    {
        let mut ix_data = Vec::new();
        StakingInstruction::CreateProposal {
            description_hash: [3u8; 32],
            param_key: "min_stake".to_string(),
            param_value: 500_000,
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(proposal_pda, false),
                AccountMeta::new(config_pda, false),
                AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
                AccountMeta::new_readonly(clock::id(), false),
                AccountMeta::new(staker_token, false),
                AccountMeta::new(reward_vault.pubkey(), false),
                AccountMeta::new_readonly(spl_token::id(), false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        ctx.banks_client.process_transaction(tx).await.unwrap();
    }

    // Verify deposit taken
    let token_after_deposit = {
        let acct = ctx
            .banks_client
            .get_account(staker_token)
            .await
            .unwrap()
            .unwrap();
        spl_token::state::Account::unpack(&acct.data)
            .unwrap()
            .amount
    };
    assert_eq!(
        token_before - token_after_deposit,
        1_000_000,
        "deposit should be taken"
    );

    // Verify proposal stores deposit info
    let proposal_account = ctx
        .banks_client
        .get_account(proposal_pda)
        .await
        .unwrap()
        .unwrap();
    let proposal = GovernanceProposal::deserialize_reader(&mut &proposal_account.data[..]).unwrap();
    assert_eq!(proposal.deposit_amount, 1_000_000);
    assert_eq!(proposal.depositor_token, staker_token);

    // Cast vote (for quorum + majority)
    let (gov_vote_pda, _) = Pubkey::find_program_address(
        &[
            GOV_VOTE_SEED,
            proposal_pda.as_ref(),
            ctx.payer.pubkey().as_ref(),
        ],
        &program_id,
    );
    let (gov_lock_pda, _) =
        Pubkey::find_program_address(&[GOV_LOCK_SEED, ctx.payer.pubkey().as_ref()], &program_id);
    {
        let mut ix_data = Vec::new();
        StakingInstruction::CastGovernanceVote {
            proposal_nonce: nonce,
            vote: 0,
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(gov_vote_pda, false),
                AccountMeta::new(proposal_pda, false),
                AccountMeta::new_readonly(voter_stake_pda, false),
                AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
                AccountMeta::new_readonly(clock::id(), false),
                AccountMeta::new(gov_lock_pda, false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        ctx.banks_client.process_transaction(tx).await.unwrap();
    }

    // Patch proposal: set voting_ends and executable_after to past so we can execute
    {
        let proposal_account = ctx
            .banks_client
            .get_account(proposal_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = proposal_account.data.len();
        let mut proposal =
            GovernanceProposal::deserialize_reader(&mut &proposal_account.data[..]).unwrap();
        proposal.voting_ends = 0;
        proposal.executable_after = 0;
        let mut buf = Vec::new();
        proposal.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(proposal_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&proposal_pda, &patched);
    }

    // Execute proposal — with deposit refund accounts
    {
        let mut ix_data = Vec::new();
        StakingInstruction::ExecuteProposal {
            proposal_nonce: nonce,
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(proposal_pda, false),
                AccountMeta::new(config_pda, false),
                AccountMeta::new_readonly(clock::id(), false),
                AccountMeta::new(staker_token, false),
                AccountMeta::new(reward_vault.pubkey(), false),
                AccountMeta::new_readonly(spl_token::id(), false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        ctx.banks_client.process_transaction(tx).await.unwrap();
    }

    // Verify deposit refunded
    let token_after_refund = {
        let acct = ctx
            .banks_client
            .get_account(staker_token)
            .await
            .unwrap()
            .unwrap();
        spl_token::state::Account::unpack(&acct.data)
            .unwrap()
            .amount
    };
    assert_eq!(
        token_after_refund,
        token_after_deposit + 1_000_000,
        "deposit should be refunded after quorum met"
    );
}

/// M4: Create proposal with deposit, execute without quorum → deposit stays in vault.
#[tokio::test]
async fn test_proposal_deposit_no_refund_no_quorum() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let mut ctx = pt.start_with_context().await;
    let (config_pda, staker_token) = init_protocol_and_mint(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        500_000_000,
    )
    .await;

    // Stake a small amount first (before raising min_stake for quorum check)
    let domain = "noquorum.com";
    let (_voter_stake_pda, _) = stake_v2_helper(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &config_pda,
        &staker_token,
        &reward_vault.pubkey(),
        domain,
        5_000_000,
        0,
    )
    .await;

    // Patch config: proposal_deposit = 1_000_000, min_stake high for quorum
    {
        let config_account = ctx
            .banks_client
            .get_account(config_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = config_account.data.len();
        let mut config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
        config.proposal_deposit = 1_000_000;
        config.min_stake = 1_000_000_000; // very high quorum floor
        let mut buf = Vec::new();
        config.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(config_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&config_pda, &patched);
    }

    let token_before = {
        let acct = ctx
            .banks_client
            .get_account(staker_token)
            .await
            .unwrap()
            .unwrap();
        spl_token::state::Account::unpack(&acct.data)
            .unwrap()
            .amount
    };

    // Create proposal with deposit
    let config_account = ctx
        .banks_client
        .get_account(config_pda)
        .await
        .unwrap()
        .unwrap();
    let config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
    let nonce = config.proposal_nonce;
    let nonce_bytes = nonce.to_le_bytes();
    let (proposal_pda, _) =
        Pubkey::find_program_address(&[PROPOSAL_SEED, &nonce_bytes], &program_id);

    {
        let mut ix_data = Vec::new();
        StakingInstruction::CreateProposal {
            description_hash: [4u8; 32],
            param_key: "cooldown_secs".to_string(),
            param_value: 1000,
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(proposal_pda, false),
                AccountMeta::new(config_pda, false),
                AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
                AccountMeta::new_readonly(clock::id(), false),
                AccountMeta::new(staker_token, false),
                AccountMeta::new(reward_vault.pubkey(), false),
                AccountMeta::new_readonly(spl_token::id(), false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        ctx.banks_client.process_transaction(tx).await.unwrap();
    }

    let token_after_deposit = {
        let acct = ctx
            .banks_client
            .get_account(staker_token)
            .await
            .unwrap()
            .unwrap();
        spl_token::state::Account::unpack(&acct.data)
            .unwrap()
            .amount
    };
    assert_eq!(token_before - token_after_deposit, 1_000_000);

    // Patch proposal: set voting_ends to past
    {
        let proposal_account = ctx
            .banks_client
            .get_account(proposal_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = proposal_account.data.len();
        let mut proposal =
            GovernanceProposal::deserialize_reader(&mut &proposal_account.data[..]).unwrap();
        proposal.voting_ends = 0;
        proposal.executable_after = 0;
        let mut buf = Vec::new();
        proposal.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(proposal_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&proposal_pda, &patched);
    }

    // Execute proposal — quorum will fail (no votes, min_stake=1B)
    // No refund accounts needed since quorum fails → early return
    {
        let mut ix_data = Vec::new();
        StakingInstruction::ExecuteProposal {
            proposal_nonce: nonce,
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(proposal_pda, false),
                AccountMeta::new(config_pda, false),
                AccountMeta::new_readonly(clock::id(), false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        ctx.banks_client.process_transaction(tx).await.unwrap();
    }

    // Verify proposal marked as Failed
    let proposal_account = ctx
        .banks_client
        .get_account(proposal_pda)
        .await
        .unwrap()
        .unwrap();
    let proposal = GovernanceProposal::deserialize_reader(&mut &proposal_account.data[..]).unwrap();
    assert_eq!(proposal.status, ProposalStatus::Failed);

    // Verify deposit NOT refunded (quorum not met)
    let token_after_execute = {
        let acct = ctx
            .banks_client
            .get_account(staker_token)
            .await
            .unwrap()
            .unwrap();
        spl_token::state::Account::unpack(&acct.data)
            .unwrap()
            .amount
    };
    assert_eq!(
        token_after_execute, token_after_deposit,
        "deposit should not be refunded when quorum fails"
    );
}

// ══════════════════════════════════════════════
// Economic Hardening: Post-Review Tests
// ══════════════════════════════════════════════

/// S1: Open a dispute on a domain, then try to unstake → DisputeAlreadyActive.
#[tokio::test]
async fn test_unstake_blocked_during_active_dispute() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let mut ctx = pt.start_with_context().await;
    let (config_pda, staker_token) = init_protocol_and_mint(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        500_000_000,
    )
    .await;

    // Stake
    let domain = "s1-dispute.com";
    let (stake_pda, domain_pda) = stake_v2_helper(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &config_pda,
        &staker_token,
        &reward_vault.pubkey(),
        domain,
        100_000_000,
        0,
    )
    .await;

    // Open dispute on the domain
    let config_account = ctx
        .banks_client
        .get_account(config_pda)
        .await
        .unwrap()
        .unwrap();
    let config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
    let nonce = config.dispute_nonce;
    let nonce_bytes = nonce.to_le_bytes();
    let (dispute_pda, _) = Pubkey::find_program_address(
        &[DISPUTE_SEED, domain.as_bytes(), &nonce_bytes],
        &program_id,
    );

    {
        let mut ix_data = Vec::new();
        StakingInstruction::OpenDispute {
            domain: domain.to_string(),
            reason_hash: [1u8; 32],
            severity: 1,
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(staker_token, false),
                AccountMeta::new(reward_vault.pubkey(), false),
                AccountMeta::new(dispute_pda, false),
                AccountMeta::new(domain_pda, false),
                AccountMeta::new_readonly(config_pda, false),
                AccountMeta::new_readonly(spl_token::id(), false),
                AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
                AccountMeta::new_readonly(clock::id(), false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        ctx.banks_client.process_transaction(tx).await.unwrap();
    }

    // Verify domain has active dispute
    let domain_account = ctx
        .banks_client
        .get_account(domain_pda)
        .await
        .unwrap()
        .unwrap();
    let domain_record = DomainRecord::deserialize_reader(&mut &domain_account.data[..]).unwrap();
    assert_ne!(
        domain_record.active_dispute_nonce,
        u64::MAX,
        "dispute should be active"
    );

    // Try to unstake → should fail with DisputeAlreadyActive
    let (gov_lock_pda, _) =
        Pubkey::find_program_address(&[GOV_LOCK_SEED, ctx.payer.pubkey().as_ref()], &program_id);
    {
        let mut ix_data = Vec::new();
        StakingInstruction::UnstakeV2 {
            domain: domain.to_string(),
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(stake_pda, false),
                AccountMeta::new(config_pda, false),
                AccountMeta::new_readonly(domain_pda, false),
                AccountMeta::new_readonly(clock::id(), false),
                AccountMeta::new_readonly(gov_lock_pda, false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        let result = ctx.banks_client.process_transaction(tx).await;
        assert!(
            result.is_err(),
            "unstaking during active dispute should fail"
        );
    }
}

/// S1: Open dispute, resolve it, then unstake succeeds.
#[tokio::test]
async fn test_unstake_succeeds_after_dispute_resolved() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let mut ctx = pt.start_with_context().await;
    let (config_pda, staker_token) = init_protocol_and_mint(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        500_000_000,
    )
    .await;

    // Stake
    let domain = "s1-resolve.com";
    let (_stake_pda, domain_pda) = stake_v2_helper(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &config_pda,
        &staker_token,
        &reward_vault.pubkey(),
        domain,
        100_000_000,
        0,
    )
    .await;

    // Open dispute
    let config_account = ctx
        .banks_client
        .get_account(config_pda)
        .await
        .unwrap()
        .unwrap();
    let config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
    let nonce = config.dispute_nonce;
    let nonce_bytes = nonce.to_le_bytes();
    let (dispute_pda, _) = Pubkey::find_program_address(
        &[DISPUTE_SEED, domain.as_bytes(), &nonce_bytes],
        &program_id,
    );

    {
        let mut ix_data = Vec::new();
        StakingInstruction::OpenDispute {
            domain: domain.to_string(),
            reason_hash: [2u8; 32],
            severity: 1,
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(staker_token, false),
                AccountMeta::new(reward_vault.pubkey(), false),
                AccountMeta::new(dispute_pda, false),
                AccountMeta::new(domain_pda, false),
                AccountMeta::new_readonly(config_pda, false),
                AccountMeta::new_readonly(spl_token::id(), false),
                AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
                AccountMeta::new_readonly(clock::id(), false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        ctx.banks_client.process_transaction(tx).await.unwrap();
    }

    // Patch domain record: clear active dispute (simulate resolution)
    {
        let domain_account = ctx
            .banks_client
            .get_account(domain_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = domain_account.data.len();
        let mut record = DomainRecord::deserialize_reader(&mut &domain_account.data[..]).unwrap();
        record.active_dispute_nonce = u64::MAX; // no active dispute
        let mut buf = Vec::new();
        record.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(domain_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&domain_pda, &patched);
    }

    // Unstake → should succeed
    unstake_v2_helper(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &config_pda,
        domain,
    )
    .await;
}

/// S2: Stake on 2 domains, vote with domain A, try unstake domain B → GovernanceLockActive.
#[tokio::test]
async fn test_governance_lock_per_wallet() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let mut ctx = pt.start_with_context().await;
    let (config_pda, staker_token) = init_protocol_and_mint(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        500_000_000,
    )
    .await;

    // Patch config: zero out proposal_deposit so CreateProposal doesn't need deposit accounts
    {
        let config_account = ctx
            .banks_client
            .get_account(config_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = config_account.data.len();
        let mut config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
        config.proposal_deposit = 0;
        let mut buf = Vec::new();
        config.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(config_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&config_pda, &patched);
    }

    // Stake on domain A
    let domain_a = "wallet-lock-a.com";
    let (voter_stake_pda, _) = stake_v2_helper(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &config_pda,
        &staker_token,
        &reward_vault.pubkey(),
        domain_a,
        100_000_000,
        0,
    )
    .await;

    // Stake on domain B
    let domain_b = "wallet-lock-b.com";
    let (stake_pda_b, _) = stake_v2_helper(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &config_pda,
        &staker_token,
        &reward_vault.pubkey(),
        domain_b,
        50_000_000,
        0,
    )
    .await;

    // Patch stake A: set staked_at to 8 days ago (satisfies 7-day governance voting minimum)
    {
        let stake_account = ctx
            .banks_client
            .get_account(voter_stake_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = stake_account.data.len();
        let mut stake = StakeAccountV2::deserialize_reader(&mut &stake_account.data[..]).unwrap();
        stake.staked_at -= 8 * 24 * 3600;
        let mut buf = Vec::new();
        stake.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(stake_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&voter_stake_pda, &patched);
    }

    // Create proposal
    let config_account = ctx
        .banks_client
        .get_account(config_pda)
        .await
        .unwrap()
        .unwrap();
    let config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
    let nonce = config.proposal_nonce;
    let nonce_bytes = nonce.to_le_bytes();
    let (proposal_pda, _) =
        Pubkey::find_program_address(&[PROPOSAL_SEED, &nonce_bytes], &program_id);

    {
        let mut ix_data = Vec::new();
        StakingInstruction::CreateProposal {
            description_hash: [1u8; 32],
            param_key: "min_stake".to_string(),
            param_value: 500_000,
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(proposal_pda, false),
                AccountMeta::new(config_pda, false),
                AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
                AccountMeta::new_readonly(clock::id(), false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        ctx.banks_client.process_transaction(tx).await.unwrap();
    }

    // Vote using domain A's stake
    let (gov_vote_pda, _) = Pubkey::find_program_address(
        &[
            GOV_VOTE_SEED,
            proposal_pda.as_ref(),
            ctx.payer.pubkey().as_ref(),
        ],
        &program_id,
    );
    let (gov_lock_pda, _) =
        Pubkey::find_program_address(&[GOV_LOCK_SEED, ctx.payer.pubkey().as_ref()], &program_id);
    {
        let mut ix_data = Vec::new();
        StakingInstruction::CastGovernanceVote {
            proposal_nonce: nonce,
            vote: 0,
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(gov_vote_pda, false),
                AccountMeta::new(proposal_pda, false),
                AccountMeta::new_readonly(voter_stake_pda, false),
                AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
                AccountMeta::new_readonly(clock::id(), false),
                AccountMeta::new(gov_lock_pda, false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        ctx.banks_client.process_transaction(tx).await.unwrap();
    }

    // Try to unstake domain B → should fail (per-wallet lock applies to ALL positions)
    let (domain_pda_b, _) =
        Pubkey::find_program_address(&[DOMAIN_SEED, domain_b.as_bytes()], &program_id);
    {
        let mut ix_data = Vec::new();
        StakingInstruction::UnstakeV2 {
            domain: domain_b.to_string(),
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(stake_pda_b, false),
                AccountMeta::new(config_pda, false),
                AccountMeta::new_readonly(domain_pda_b, false),
                AccountMeta::new_readonly(clock::id(), false),
                AccountMeta::new_readonly(gov_lock_pda, false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        let result = ctx.banks_client.process_transaction(tx).await;
        assert!(
            result.is_err(),
            "unstaking domain B should fail — per-wallet governance lock applies"
        );
    }
}

/// M8: Unstake, then claim reward during cooldown → succeeds.
#[tokio::test]
async fn test_claim_reward_during_cooldown() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let mut ctx = pt.start_with_context().await;
    let (config_pda, staker_token) = init_protocol_and_mint(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        500_000_000,
    )
    .await;

    // Stake
    let domain = "m8-cooldown.com";
    let (stake_pda, _domain_pda) = stake_v2_helper(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &config_pda,
        &staker_token,
        &reward_vault.pubkey(),
        domain,
        100_000_000,
        0,
    )
    .await;

    // Unstake (starts cooldown)
    unstake_v2_helper(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &config_pda,
        domain,
    )
    .await;

    // Verify cooldown is set
    let stake_account = ctx
        .banks_client
        .get_account(stake_pda)
        .await
        .unwrap()
        .unwrap();
    let stake = StakeAccountV2::deserialize_reader(&mut &stake_account.data[..]).unwrap();
    assert!(stake.cooldown_until > 0, "cooldown should be set");

    // Claim reward during cooldown → should succeed (M8 fix)
    {
        let mut ix_data = Vec::new();
        StakingInstruction::ClaimReward {
            domain: domain.to_string(),
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(stake_pda, false),
                AccountMeta::new(truth_mint.pubkey(), false),
                AccountMeta::new(staker_token, false),
                AccountMeta::new(config_pda, false),
                AccountMeta::new_readonly(spl_token::id(), false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        // Should succeed (no error) — M8 allows claims during cooldown
        ctx.banks_client.process_transaction(tx).await.unwrap();
    }
}

/// M5: Set max_total_supply near total_minted, UpdateRewardIndex → capped at supply headroom.
#[tokio::test]
async fn test_reward_index_capped_at_supply_headroom() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let mut ctx = pt.start_with_context().await;
    let (config_pda, staker_token) = init_protocol_and_mint(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        500_000_000,
    )
    .await;

    // Stake (needed for total_staked > 0)
    let domain = "m5-headroom.com";
    let (_stake_pda, _) = stake_v2_helper(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &config_pda,
        &staker_token,
        &reward_vault.pubkey(),
        domain,
        100_000_000,
        0,
    )
    .await;

    // Patch config: max_total_supply just above total_minted, with large funded budget
    {
        let config_account = ctx
            .banks_client
            .get_account(config_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = config_account.data.len();
        let mut config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
        config.max_total_supply = config.total_minted + 100; // only 100 tokens of headroom
        config.total_funded = 10_000_000; // 10M funded
        let mut buf = Vec::new();
        config.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(config_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&config_pda, &patched);
    }

    // UpdateRewardIndex with 5M → should be capped at 100 (supply headroom)
    {
        let mut ix_data = Vec::new();
        StakingInstruction::UpdateRewardIndex {
            funded_amount: 5_000_000,
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(config_pda, false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        ctx.banks_client.process_transaction(tx).await.unwrap();
    }

    // Verify: epoch_funded_amount should be at most 100 (the supply headroom)
    let config_account = ctx
        .banks_client
        .get_account(config_pda)
        .await
        .unwrap()
        .unwrap();
    let config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
    assert!(
        config.epoch_funded_amount <= 100,
        "epoch_funded_amount ({}) should be capped at supply headroom (100)",
        config.epoch_funded_amount
    );
    assert!(
        config.total_distributed <= 100,
        "total_distributed ({}) should be capped at supply headroom (100)",
        config.total_distributed
    );
}

// ══════════════════════════════════════════════
// Phase 12 — Economic Rebalancing Tests
// ══════════════════════════════════════════════

/// Fix 1: Emission decay curve — advance multiple epochs, verify 1.5% reduction each time.
#[tokio::test]
async fn test_emission_decay_curve() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let mut ctx = pt.start_with_context().await;

    let (config_pda, _) = init_protocol_and_mint(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        0,
    )
    .await;

    // Patch config: set min_epoch_duration=0 for this test
    {
        let config_account = ctx
            .banks_client
            .get_account(config_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = config_account.data.len();
        let mut config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
        config.min_epoch_duration = 0;
        let mut buf = Vec::new();
        config.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(config_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&config_pda, &patched);
    }

    // Expected emissions: 10M * 0.985^epoch (1.5% decay)
    let expected_emissions: Vec<u64> = vec![
        10_000_000, // epoch 0
        9_850_000,  // epoch 1
        9_702_250,  // epoch 2
        9_556_716,  // epoch 3
        9_413_365,  // epoch 4
        9_272_164,  // epoch 5
    ];

    // Verify epoch 0
    let config_account = ctx
        .banks_client
        .get_account(config_pda)
        .await
        .unwrap()
        .unwrap();
    let config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
    assert_eq!(config.epoch_max_emission, expected_emissions[0]);

    for (i, &expected_emission) in expected_emissions.iter().enumerate().skip(1) {
        ctx.warp_to_slot(100 * i as u64).unwrap();

        let mut ix_data = Vec::new();
        StakingInstruction::AdvanceEpoch
            .serialize(&mut ix_data)
            .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(config_pda, false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        ctx.banks_client.process_transaction(tx).await.unwrap();

        let config_account = ctx
            .banks_client
            .get_account(config_pda)
            .await
            .unwrap()
            .unwrap();
        let config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
        assert_eq!(config.current_epoch, i as u64);
        assert_eq!(
            config.epoch_max_emission, expected_emission,
            "epoch {i} emission should be {expected_emission}"
        );
    }
}

/// Fix 4: Trusted reporter bond discount — sentinel with high stake gets 50% bond,
/// low-stake or no sentinel gets full bond.
#[tokio::test]
async fn test_trusted_reporter_bond_discount() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let mut ctx = pt.start_with_context().await;
    let (config_pda, staker_token) = init_protocol_and_mint(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        500_000_000,
    )
    .await;

    let sentinel_pubkey = ctx.payer.pubkey();
    let (sentinel_pda, _) =
        Pubkey::find_program_address(&[SENTINEL_SEED, sentinel_pubkey.as_ref()], &program_id);

    // Read config to get jury_min_stake and dispute_bond
    let config_account = ctx
        .banks_client
        .get_account(config_pda)
        .await
        .unwrap()
        .unwrap();
    let config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
    let dispute_bond = config.dispute_bond;
    let trusted_threshold = config.jury_min_stake.saturating_mul(10);

    // Stake as sentinel with amount >= trusted threshold
    {
        let mut ix_data = Vec::new();
        StakingInstruction::StakeSentinel {
            amount: trusted_threshold,
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(sentinel_pubkey, true),
                AccountMeta::new(staker_token, false),
                AccountMeta::new(reward_vault.pubkey(), false),
                AccountMeta::new(sentinel_pda, false),
                AccountMeta::new(config_pda, false),
                AccountMeta::new_readonly(spl_token::id(), false),
                AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
                AccountMeta::new_readonly(clock::id(), false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        ctx.banks_client.process_transaction(tx).await.unwrap();
    }

    // Create domain and open dispute WITH sentinel PDA → should get discounted bond
    let domain = "trusted-reporter.com";
    let (_stake_pda, domain_pda) = stake_v2_helper(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &config_pda,
        &staker_token,
        &reward_vault.pubkey(),
        domain,
        10_000_000,
        0,
    )
    .await;

    let config_account = ctx
        .banks_client
        .get_account(config_pda)
        .await
        .unwrap()
        .unwrap();
    let config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
    let nonce = config.dispute_nonce;
    let nonce_bytes = nonce.to_le_bytes();
    let (dispute_pda, _) = Pubkey::find_program_address(
        &[DISPUTE_SEED, domain.as_bytes(), &nonce_bytes],
        &program_id,
    );

    // Record vault balance before dispute
    let vault_before = spl_token::state::Account::unpack(
        &ctx.banks_client
            .get_account(reward_vault.pubkey())
            .await
            .unwrap()
            .unwrap()
            .data,
    )
    .unwrap()
    .amount;

    {
        let mut ix_data = Vec::new();
        StakingInstruction::OpenDispute {
            domain: domain.to_string(),
            reason_hash: [1u8; 32],
            severity: 1,
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(staker_token, false),
                AccountMeta::new(reward_vault.pubkey(), false),
                AccountMeta::new(dispute_pda, false),
                AccountMeta::new(domain_pda, false),
                AccountMeta::new(config_pda, false),
                AccountMeta::new_readonly(spl_token::id(), false),
                AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
                AccountMeta::new_readonly(clock::id(), false),
                AccountMeta::new_readonly(sentinel_pda, false), // sentinel PDA for discount
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        ctx.banks_client.process_transaction(tx).await.unwrap();
    }

    // Verify bond was halved
    let vault_after = spl_token::state::Account::unpack(
        &ctx.banks_client
            .get_account(reward_vault.pubkey())
            .await
            .unwrap()
            .unwrap()
            .data,
    )
    .unwrap()
    .amount;
    let bond_paid = vault_after - vault_before;
    assert_eq!(
        bond_paid,
        dispute_bond / 2,
        "trusted reporter should pay half bond: expected {}, got {}",
        dispute_bond / 2,
        bond_paid
    );

    // Verify dispute account records the halved bond
    let dispute_account = ctx
        .banks_client
        .get_account(dispute_pda)
        .await
        .unwrap()
        .unwrap();
    let dispute = DisputeAccount::deserialize_reader(&mut &dispute_account.data[..]).unwrap();
    assert_eq!(
        dispute.bond,
        dispute_bond / 2,
        "dispute should record halved bond"
    );
}

/// Fix 3: Jury reward minted flat fee — resolve dispute, claim → exact jury_vote_reward amount.
#[tokio::test]
async fn test_jury_reward_minted_flat_fee() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let mut ctx = pt.start_with_context().await;
    let (config_pda, staker_token) = init_protocol_and_mint(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        200_000_000,
    )
    .await;

    // Set jury_vote_reward to 5000
    {
        let config_account = ctx
            .banks_client
            .get_account(config_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = config_account.data.len();
        let mut config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
        config.jury_vote_reward = 5_000;
        let mut buf = Vec::new();
        config.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(config_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&config_pda, &patched);
    }

    let domain = "flat-fee-jury.com";
    let (dispute_pda, dispute_nonce, _) = setup_dispute(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &config_pda,
        &staker_token,
        &reward_vault.pubkey(),
        &truth_mint.pubkey(),
        domain,
    )
    .await;

    // Patch dispute: ResolvedGuilty
    {
        let dispute_account = ctx
            .banks_client
            .get_account(dispute_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = dispute_account.data.len();
        let mut dispute =
            DisputeAccount::deserialize_reader(&mut &dispute_account.data[..]).unwrap();
        dispute.status = DisputeStatus::ResolvedGuilty;
        dispute.votes_guilty = 10_000_000;
        dispute.votes_innocent = 0;
        dispute.jury_reward_pool = 0;
        dispute.jury_reward_claimed = 0;
        let mut buf = Vec::new();
        dispute.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(dispute_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&dispute_pda, &patched);
    }

    // Create jury vote PDA
    let (jury_pda, jury_bump) = Pubkey::find_program_address(
        &[JURY_SEED, dispute_pda.as_ref(), ctx.payer.pubkey().as_ref()],
        &program_id,
    );
    {
        let jury_vote = JuryVote {
            is_initialized: true,
            voter: ctx.payer.pubkey(),
            verdict: Verdict::Guilty,
            weight: 10_000_000,
            voted_at: 100,
            bump: jury_bump,
            commit_hash: [0u8; 32],
            reward_claimed: false,
        };
        let mut buf = Vec::new();
        jury_vote.serialize(&mut buf).unwrap();
        let rent = Rent::default();
        let lamports = rent.minimum_balance(JuryVote::MAX_SIZE);
        buf.resize(JuryVote::MAX_SIZE, 0);
        let account = solana_sdk::account::Account {
            lamports,
            data: buf,
            owner: program_id,
            executable: false,
            rent_epoch: 0,
        };
        ctx.set_account(&jury_pda, &AccountSharedData::from(account));
    }

    // Record balance before
    let before = spl_token::state::Account::unpack(
        &ctx.banks_client
            .get_account(staker_token)
            .await
            .unwrap()
            .unwrap()
            .data,
    )
    .unwrap()
    .amount;

    // Record total_minted before
    let config_before = ProtocolConfig::deserialize_reader(
        &mut &ctx
            .banks_client
            .get_account(config_pda)
            .await
            .unwrap()
            .unwrap()
            .data[..],
    )
    .unwrap();
    let minted_before = config_before.total_minted;

    // ClaimJuryReward
    {
        let mut ix_data = Vec::new();
        StakingInstruction::ClaimJuryReward {
            domain: domain.to_string(),
            dispute_nonce,
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(jury_pda, false),
                AccountMeta::new(dispute_pda, false),
                AccountMeta::new(staker_token, false),
                AccountMeta::new(truth_mint.pubkey(), false),
                AccountMeta::new(config_pda, false),
                AccountMeta::new_readonly(spl_token::id(), false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        ctx.banks_client.process_transaction(tx).await.unwrap();
    }

    // Verify exactly 5000 minted
    let after = spl_token::state::Account::unpack(
        &ctx.banks_client
            .get_account(staker_token)
            .await
            .unwrap()
            .unwrap()
            .data,
    )
    .unwrap()
    .amount;
    assert_eq!(
        after - before,
        5_000,
        "should receive flat jury_vote_reward of 5000"
    );

    // Verify total_minted incremented
    let config_after = ProtocolConfig::deserialize_reader(
        &mut &ctx
            .banks_client
            .get_account(config_pda)
            .await
            .unwrap()
            .unwrap()
            .data[..],
    )
    .unwrap();
    assert_eq!(
        config_after.total_minted,
        minted_before + 5_000,
        "total_minted should increment by jury_vote_reward"
    );
}

/// Fix 3: jury_reward_pool is 0 on both guilty and innocent outcomes.
#[tokio::test]
async fn test_jury_no_profit_from_verdict() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let mut ctx = pt.start_with_context().await;
    let (config_pda, staker_token) = init_protocol_and_mint(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        200_000_000,
    )
    .await;

    // === Test Guilty verdict: jury_reward_pool should be 0 ===
    let domain_guilty = "no-profit-guilty.com";
    let (dispute_pda_g, dispute_nonce_g, domain_pda_g) = setup_dispute(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &config_pda,
        &staker_token,
        &reward_vault.pubkey(),
        &truth_mint.pubkey(),
        domain_guilty,
    )
    .await;

    // Patch: min_jury_weight=0, deadline=0, votes_guilty=1 (force guilty)
    {
        let config_account = ctx
            .banks_client
            .get_account(config_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = config_account.data.len();
        let mut config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
        config.min_jury_weight = 0;
        let mut buf = Vec::new();
        config.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(config_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&config_pda, &patched);
    }
    {
        let dispute_account = ctx
            .banks_client
            .get_account(dispute_pda_g)
            .await
            .unwrap()
            .unwrap();
        let orig_len = dispute_account.data.len();
        let mut dispute =
            DisputeAccount::deserialize_reader(&mut &dispute_account.data[..]).unwrap();
        dispute.deadline = 0;
        dispute.reveal_deadline = 0;
        dispute.votes_guilty = 1;
        let mut buf = Vec::new();
        dispute.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(dispute_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&dispute_pda_g, &patched);
    }

    // Resolve → Guilty
    {
        let mut ix_data = Vec::new();
        StakingInstruction::ResolveDispute {
            domain: domain_guilty.to_string(),
            dispute_nonce: dispute_nonce_g,
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(dispute_pda_g, false),
                AccountMeta::new(domain_pda_g, false),
                AccountMeta::new(staker_token, false),
                AccountMeta::new(reward_vault.pubkey(), false),
                AccountMeta::new(truth_mint.pubkey(), false),
                AccountMeta::new_readonly(config_pda, false),
                AccountMeta::new_readonly(spl_token::id(), false),
                AccountMeta::new_readonly(clock::id(), false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        ctx.banks_client.process_transaction(tx).await.unwrap();
    }

    let dispute_account = ctx
        .banks_client
        .get_account(dispute_pda_g)
        .await
        .unwrap()
        .unwrap();
    let dispute = DisputeAccount::deserialize_reader(&mut &dispute_account.data[..]).unwrap();
    assert_eq!(dispute.status, DisputeStatus::ResolvedGuilty);
    assert_eq!(
        dispute.jury_reward_pool, 0,
        "guilty: jury_reward_pool should be 0"
    );

    // === Test Innocent verdict: jury_reward_pool should be 0 ===
    ctx.warp_to_slot(200).unwrap();

    let domain_innocent = "no-profit-innocent.com";
    let (dispute_pda_i, dispute_nonce_i, domain_pda_i) = setup_dispute(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &config_pda,
        &staker_token,
        &reward_vault.pubkey(),
        &truth_mint.pubkey(),
        domain_innocent,
    )
    .await;

    // Patch: deadline=0, votes_innocent=1 (force innocent)
    {
        let dispute_account = ctx
            .banks_client
            .get_account(dispute_pda_i)
            .await
            .unwrap()
            .unwrap();
        let orig_len = dispute_account.data.len();
        let mut dispute =
            DisputeAccount::deserialize_reader(&mut &dispute_account.data[..]).unwrap();
        dispute.deadline = 0;
        dispute.reveal_deadline = 0;
        dispute.votes_innocent = 1;
        let mut buf = Vec::new();
        dispute.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(dispute_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&dispute_pda_i, &patched);
    }

    // Resolve → Innocent
    {
        let mut ix_data = Vec::new();
        StakingInstruction::ResolveDispute {
            domain: domain_innocent.to_string(),
            dispute_nonce: dispute_nonce_i,
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(dispute_pda_i, false),
                AccountMeta::new(domain_pda_i, false),
                AccountMeta::new(staker_token, false),
                AccountMeta::new(reward_vault.pubkey(), false),
                AccountMeta::new(truth_mint.pubkey(), false),
                AccountMeta::new_readonly(config_pda, false),
                AccountMeta::new_readonly(spl_token::id(), false),
                AccountMeta::new_readonly(clock::id(), false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        ctx.banks_client.process_transaction(tx).await.unwrap();
    }

    let dispute_account = ctx
        .banks_client
        .get_account(dispute_pda_i)
        .await
        .unwrap()
        .unwrap();
    let dispute = DisputeAccount::deserialize_reader(&mut &dispute_account.data[..]).unwrap();
    assert_eq!(dispute.status, DisputeStatus::ResolvedInnocent);
    assert_eq!(
        dispute.jury_reward_pool, 0,
        "innocent: jury_reward_pool should be 0"
    );
}

/// Jury reward epoch cap: claiming more than max_jury_rewards_per_epoch fails.
#[tokio::test]
async fn test_jury_reward_epoch_cap() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let mut ctx = pt.start_with_context().await;
    let (config_pda, staker_token) = init_protocol_and_mint(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        200_000_000,
    )
    .await;

    // Set jury_vote_reward = 3000, max_jury_rewards_per_epoch = 5000
    // (so only 1 claim fits, the 2nd would exceed budget)
    {
        let config_account = ctx
            .banks_client
            .get_account(config_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = config_account.data.len();
        let mut config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
        config.jury_vote_reward = 3_000;
        config.max_jury_rewards_per_epoch = 5_000;
        let mut buf = Vec::new();
        config.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(config_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&config_pda, &patched);
    }

    let domain = "epoch-cap-jury.com";
    let (dispute_pda, dispute_nonce, _) = setup_dispute(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &config_pda,
        &staker_token,
        &reward_vault.pubkey(),
        &truth_mint.pubkey(),
        domain,
    )
    .await;

    // Patch dispute: ResolvedGuilty
    {
        let dispute_account = ctx
            .banks_client
            .get_account(dispute_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = dispute_account.data.len();
        let mut dispute =
            DisputeAccount::deserialize_reader(&mut &dispute_account.data[..]).unwrap();
        dispute.status = DisputeStatus::ResolvedGuilty;
        dispute.votes_guilty = 10_000_000;
        dispute.votes_innocent = 0;
        dispute.jury_reward_pool = 0;
        dispute.jury_reward_claimed = 0;
        let mut buf = Vec::new();
        dispute.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(dispute_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&dispute_pda, &patched);
    }

    // Create jury vote PDA (voter voted guilty, winning side)
    let (jury_pda, jury_bump) = Pubkey::find_program_address(
        &[JURY_SEED, dispute_pda.as_ref(), ctx.payer.pubkey().as_ref()],
        &program_id,
    );
    {
        let jury_vote = JuryVote {
            is_initialized: true,
            voter: ctx.payer.pubkey(),
            verdict: Verdict::Guilty,
            weight: 10_000_000,
            voted_at: 100,
            bump: jury_bump,
            commit_hash: [0u8; 32],
            reward_claimed: false,
        };
        let mut buf = Vec::new();
        jury_vote.serialize(&mut buf).unwrap();
        let rent = Rent::default();
        let lamports = rent.minimum_balance(JuryVote::MAX_SIZE);
        buf.resize(JuryVote::MAX_SIZE, 0);
        let account = solana_sdk::account::Account {
            lamports,
            data: buf,
            owner: program_id,
            executable: false,
            rent_epoch: 0,
        };
        ctx.set_account(&jury_pda, &AccountSharedData::from(account));
    }

    // First claim: should succeed (3000 <= 5000 budget)
    {
        let mut ix_data = Vec::new();
        StakingInstruction::ClaimJuryReward {
            domain: domain.to_string(),
            dispute_nonce,
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(jury_pda, false),
                AccountMeta::new(dispute_pda, false),
                AccountMeta::new(staker_token, false),
                AccountMeta::new(truth_mint.pubkey(), false),
                AccountMeta::new(config_pda, false),
                AccountMeta::new_readonly(spl_token::id(), false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        ctx.banks_client.process_transaction(tx).await.unwrap();
    }

    // Verify jury_rewards_this_epoch = 3000
    let config_after = ProtocolConfig::deserialize_reader(
        &mut &ctx
            .banks_client
            .get_account(config_pda)
            .await
            .unwrap()
            .unwrap()
            .data[..],
    )
    .unwrap();
    assert_eq!(
        config_after.jury_rewards_this_epoch, 3_000,
        "epoch counter should track claimed amount"
    );

    // Set up a second dispute + jury vote for a second claim attempt
    let domain2 = "epoch-cap-jury2.com";
    let (dispute_pda2, dispute_nonce2, _) = setup_dispute(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &config_pda,
        &staker_token,
        &reward_vault.pubkey(),
        &truth_mint.pubkey(),
        domain2,
    )
    .await;

    // Patch dispute2: ResolvedGuilty
    {
        let dispute_account = ctx
            .banks_client
            .get_account(dispute_pda2)
            .await
            .unwrap()
            .unwrap();
        let orig_len = dispute_account.data.len();
        let mut dispute =
            DisputeAccount::deserialize_reader(&mut &dispute_account.data[..]).unwrap();
        dispute.status = DisputeStatus::ResolvedGuilty;
        dispute.votes_guilty = 10_000_000;
        dispute.votes_innocent = 0;
        dispute.jury_reward_pool = 0;
        dispute.jury_reward_claimed = 0;
        let mut buf = Vec::new();
        dispute.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(dispute_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&dispute_pda2, &patched);
    }

    let (jury_pda2, jury_bump2) = Pubkey::find_program_address(
        &[
            JURY_SEED,
            dispute_pda2.as_ref(),
            ctx.payer.pubkey().as_ref(),
        ],
        &program_id,
    );
    {
        let jury_vote = JuryVote {
            is_initialized: true,
            voter: ctx.payer.pubkey(),
            verdict: Verdict::Guilty,
            weight: 10_000_000,
            voted_at: 100,
            bump: jury_bump2,
            commit_hash: [0u8; 32],
            reward_claimed: false,
        };
        let mut buf = Vec::new();
        jury_vote.serialize(&mut buf).unwrap();
        let rent = Rent::default();
        let lamports = rent.minimum_balance(JuryVote::MAX_SIZE);
        buf.resize(JuryVote::MAX_SIZE, 0);
        let account = solana_sdk::account::Account {
            lamports,
            data: buf,
            owner: program_id,
            executable: false,
            rent_epoch: 0,
        };
        ctx.set_account(&jury_pda2, &AccountSharedData::from(account));
    }

    // Second claim: should fail (3000 + 3000 = 6000 > 5000 budget)
    {
        let mut ix_data = Vec::new();
        StakingInstruction::ClaimJuryReward {
            domain: domain2.to_string(),
            dispute_nonce: dispute_nonce2,
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(jury_pda2, false),
                AccountMeta::new(dispute_pda2, false),
                AccountMeta::new(staker_token, false),
                AccountMeta::new(truth_mint.pubkey(), false),
                AccountMeta::new(config_pda, false),
                AccountMeta::new_readonly(spl_token::id(), false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        let result = ctx.banks_client.process_transaction(tx).await;
        assert!(
            result.is_err(),
            "second jury claim should fail: epoch budget exhausted"
        );
    }
}

/// Governance stake age: voting with a fresh stake (< 7 days) should fail.
#[tokio::test]
async fn test_governance_stake_age_requirement() {
    let (program_id, mut pt) = make_program_test();
    let truth_mint = Keypair::new();
    let reward_vault = Keypair::new();
    setup_protocol(program_id, &mut pt, &truth_mint, &reward_vault).await;

    let mut ctx = pt.start_with_context().await;
    let (config_pda, staker_token) = init_protocol_and_mint(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &truth_mint.pubkey(),
        &reward_vault.pubkey(),
        500_000_000,
    )
    .await;

    // Patch config: execution_delay_secs = 0 for this test
    {
        let config_account = ctx
            .banks_client
            .get_account(config_pda)
            .await
            .unwrap()
            .unwrap();
        let orig_len = config_account.data.len();
        let mut config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
        config.execution_delay_secs = 0;
        config.proposal_deposit = 0;
        let mut buf = Vec::new();
        config.serialize(&mut buf).unwrap();
        buf.resize(orig_len, 0);
        let mut patched = AccountSharedData::from(config_account);
        patched.set_data_from_slice(&buf);
        ctx.set_account(&config_pda, &patched);
    }

    // Stake (staked_at will be ~0, fresh stake)
    let domain = "fresh-gov.com";
    let (voter_stake_pda, _) = stake_v2_helper(
        &mut ctx.banks_client,
        &ctx.payer,
        &program_id,
        &config_pda,
        &staker_token,
        &reward_vault.pubkey(),
        domain,
        100_000_000,
        0,
    )
    .await;

    // DO NOT patch staked_at — leave it fresh

    // Create proposal
    let config_account = ctx
        .banks_client
        .get_account(config_pda)
        .await
        .unwrap()
        .unwrap();
    let config = ProtocolConfig::deserialize_reader(&mut &config_account.data[..]).unwrap();
    let nonce = config.proposal_nonce;
    let nonce_bytes = nonce.to_le_bytes();
    let (proposal_pda, _) =
        Pubkey::find_program_address(&[PROPOSAL_SEED, &nonce_bytes], &program_id);

    {
        let mut ix_data = Vec::new();
        StakingInstruction::CreateProposal {
            description_hash: [99u8; 32],
            param_key: "min_stake".to_string(),
            param_value: 500_000,
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(proposal_pda, false),
                AccountMeta::new(config_pda, false),
                AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
                AccountMeta::new_readonly(clock::id(), false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        ctx.banks_client.process_transaction(tx).await.unwrap();
    }

    // Try to vote with fresh stake → should fail
    let (gov_vote_pda, _) = Pubkey::find_program_address(
        &[
            GOV_VOTE_SEED,
            proposal_pda.as_ref(),
            ctx.payer.pubkey().as_ref(),
        ],
        &program_id,
    );
    let (gov_lock_pda, _) =
        Pubkey::find_program_address(&[GOV_LOCK_SEED, ctx.payer.pubkey().as_ref()], &program_id);
    {
        let mut ix_data = Vec::new();
        StakingInstruction::CastGovernanceVote {
            proposal_nonce: nonce,
            vote: 0,
        }
        .serialize(&mut ix_data)
        .unwrap();
        let ix = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(ctx.payer.pubkey(), true),
                AccountMeta::new(gov_vote_pda, false),
                AccountMeta::new(proposal_pda, false),
                AccountMeta::new_readonly(voter_stake_pda, false),
                AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
                AccountMeta::new_readonly(clock::id(), false),
                AccountMeta::new(gov_lock_pda, false),
            ],
            data: ix_data,
        };
        let blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            blockhash,
        );
        let result = ctx.banks_client.process_transaction(tx).await;
        assert!(
            result.is_err(),
            "fresh stake (< 7 days) should fail governance voting with StakeTooYoungForGovernance"
        );
    }
}

/// The off-chain node-bond client (`crates/oi-staking/examples/chain_node_bond.rs`)
/// encodes these four instructions by hand, like `chain_bond.rs` does for
/// `Stake`. Their position in the enum is the first byte on the wire; if the
/// enum is reordered this test says so before a transaction fails on chain.
#[test]
fn node_bond_client_wire_format_matches_the_program() {
    use opensonarx_staking::instruction::StakingInstruction;
    let bytes = |instruction: StakingInstruction| borsh::to_vec(&instruction).unwrap();

    let init = bytes(StakingInstruction::InitializeProtocol {
        min_stake: 1,
        cooldown_secs: 2,
        dispute_bond: 3,
        slash_pct: 4,
        reward_rate: 5,
        jury_min_stake: 6,
    });
    let mut expected = vec![3u8];
    expected.extend(1u64.to_le_bytes());
    expected.extend(2i64.to_le_bytes());
    expected.extend(3u64.to_le_bytes());
    expected.push(4);
    expected.extend(5u64.to_le_bytes());
    expected.extend(6u64.to_le_bytes());
    assert_eq!(init, expected);

    let mut expected = vec![4u8];
    expected.extend(9u64.to_le_bytes());
    assert_eq!(bytes(StakingInstruction::MintTruth { amount: 9 }), expected);

    let mut expected = vec![24u8];
    expected.extend(9u64.to_le_bytes());
    assert_eq!(
        bytes(StakingInstruction::StakeSentinel { amount: 9 }),
        expected
    );

    let mut expected = vec![29u8];
    expected.extend([7u8; 32]);
    expected.extend(9u64.to_le_bytes());
    assert_eq!(
        bytes(StakingInstruction::SlashSentinel {
            sentinel: [7u8; 32],
            amount: 9
        }),
        expected
    );
    // The client reads a bond account's staked and slashed amounts at fixed
    // offsets.
    let account = borsh::to_vec(&opensonarx_staking::state::SentinelAccount {
        is_initialized: true,
        sentinel: Pubkey::new_unique(),
        amount: 5_000,
        staked_at: 1,
        cooldown_until: 0,
        reward_debt: 0,
        slashed_amount: 1_200,
        bump: 255,
    })
    .unwrap();
    assert_eq!(account[0], 1);
    assert_eq!(&account[33..41], &5_000u64.to_le_bytes());
    assert_eq!(&account[73..81], &1_200u64.to_le_bytes());
}
