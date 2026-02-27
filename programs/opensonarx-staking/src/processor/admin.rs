use super::deser_config;
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    msg,
    program::invoke_signed,
    program_error::ProgramError,
    pubkey::Pubkey,
    rent::Rent,
    system_instruction,
    sysvar::Sysvar,
};

use crate::error::StakingError;
use crate::state::*;

/// InitializeProtocol: create config PDA, $TRUTH mint, and reward vault.
#[allow(clippy::too_many_arguments)]
pub fn process_initialize_protocol(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    min_stake: u64,
    cooldown_secs: i64,
    dispute_bond: u64,
    slash_pct: u8,
    reward_rate: u64,
    jury_min_stake: u64,
) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();
    let authority = next_account_info(accounts_iter)?;
    let config_pda = next_account_info(accounts_iter)?;
    let truth_mint = next_account_info(accounts_iter)?;
    let reward_vault = next_account_info(accounts_iter)?;
    let token_program = next_account_info(accounts_iter)?;
    let system_program = next_account_info(accounts_iter)?;
    let rent_sysvar = next_account_info(accounts_iter)?;

    if !authority.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }

    // Derive config PDA
    let (expected_config, config_bump) =
        Pubkey::find_program_address(&[CONFIG_SEED], program_id);
    if *config_pda.key != expected_config {
        msg!("Invalid config PDA");
        return Err(ProgramError::InvalidSeeds);
    }

    // Check not already initialized
    if config_pda.data_len() > 0 {
        let existing = deser_config(&config_pda.data.borrow());
        if let Ok(c) = existing {
            if c.is_initialized {
                return Err(StakingError::AlreadyInitialized.into());
            }
        }
    }

    // Validate token program
    if *token_program.key != spl_token::id() {
        return Err(ProgramError::IncorrectProgramId);
    }

    // Create config account
    let rent = Rent::from_account_info(rent_sysvar)?;
    let space = ProtocolConfig::MAX_SIZE;
    let lamports = rent.minimum_balance(space);

    invoke_signed(
        &system_instruction::create_account(
            authority.key,
            config_pda.key,
            lamports,
            space as u64,
            program_id,
        ),
        &[authority.clone(), config_pda.clone(), system_program.clone()],
        &[&[CONFIG_SEED, &[config_bump]]],
    )?;

    // Initialize $TRUTH mint via SPL Token CPI
    // The mint account must be pre-created (client creates it, we initialize it)
    // Mint authority = config PDA
    let init_mint_ix = spl_token::instruction::initialize_mint2(
        token_program.key,
        truth_mint.key,
        &expected_config, // mint authority = config PDA
        Some(&expected_config), // freeze authority
        9, // 9 decimals like SOL
    )?;
    solana_program::program::invoke(
        &init_mint_ix,
        &[truth_mint.clone(), token_program.clone()],
    )?;

    // Initialize reward vault token account
    // The vault account must be pre-created, we initialize it as token account
    let init_vault_ix = spl_token::instruction::initialize_account3(
        token_program.key,
        reward_vault.key,
        truth_mint.key,
        &expected_config, // owner = config PDA
    )?;
    solana_program::program::invoke(
        &init_vault_ix,
        &[reward_vault.clone(), truth_mint.clone(), token_program.clone()],
    )?;

    // Write config
    let config = ProtocolConfig {
        is_initialized: true,
        authority: *authority.key,
        truth_mint: *truth_mint.key,
        min_stake,
        cooldown_secs,
        dispute_bond,
        slash_pct,
        reward_rate,
        jury_min_stake,
        blacklist_authority: *authority.key,
        reward_vault: *reward_vault.key,
        total_staked: 0,
        reward_per_token_stored: 0,
        proposal_nonce: 0,
        dispute_nonce: 0,
        current_epoch: 0,
        epoch_funded_amount: 0,
        epoch_max_emission: 10_000_000,
        sentinel_reward_per_token: 0,
        total_sentinel_staked: 0,
        min_epoch_duration: 604_800, // M7 fix: 7 days in seconds (prevents rapid epoch exhaustion)
        epoch_started_at: 0,       // Will be set on first AdvanceEpoch
        max_total_supply: 1_000_000_000, // 1B $TRUTH hard cap
        total_minted: 0,
        min_jury_weight: 1_000,        // Minimum jury weight for dispute quorum
        burn_pct: 10,
        staker_emission_pct: 70,
        execution_delay_secs: 86_400, // 24-hour timelock after voting ends
        proposal_deposit: 100,           // 100 $TRUTH deposit to create proposals (anti-spam)
        curator_yield_cap_bps: 1500,
        sentinel_fee_pct: 5,              // 5% of channel spend to sentinel reward pool
        accumulated_sentinel_fees: 0,
        total_funded: 0,
        total_distributed: 0,
        jury_vote_reward: 1_000,
        jury_rewards_this_epoch: 0,
        max_jury_rewards_per_epoch: 50_000, // 50x per-vote reward cap per epoch
        bump: config_bump,
    };
    borsh::to_writer(&mut &mut config_pda.data.borrow_mut()[..], &config)?;

    msg!("Protocol initialized with mint {}", truth_mint.key);
    Ok(())
}

/// MintTruth: mint $TRUTH to a destination (admin only).
/// Config PDA must be writable (total_minted is updated for supply cap tracking).
pub fn process_mint_truth(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    amount: u64,
) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();
    let authority = next_account_info(accounts_iter)?;
    let config_pda = next_account_info(accounts_iter)?;
    let truth_mint = next_account_info(accounts_iter)?;
    let destination = next_account_info(accounts_iter)?;
    let token_program = next_account_info(accounts_iter)?;

    if !authority.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }

    let mut config = deser_config(&config_pda.data.borrow())?;
    if !config.is_initialized {
        return Err(ProgramError::UninitializedAccount);
    }
    if config.authority != *authority.key {
        return Err(StakingError::Unauthorized.into());
    }

    // Validate truth mint
    if *truth_mint.key != config.truth_mint {
        return Err(ProgramError::InvalidSeeds);
    }
    if *token_program.key != spl_token::id() {
        return Err(ProgramError::IncorrectProgramId);
    }

    // Derive config bump for signing
    let (expected_config, config_bump) =
        Pubkey::find_program_address(&[CONFIG_SEED], program_id);
    if *config_pda.key != expected_config {
        return Err(ProgramError::InvalidSeeds);
    }

    // Enforce supply cap
    super::check_and_increment_supply(&mut config, amount)?;
    borsh::to_writer(&mut &mut config_pda.data.borrow_mut()[..], &config)?;

    // CPI: mint_to with config PDA as authority
    let mint_ix = spl_token::instruction::mint_to(
        token_program.key,
        truth_mint.key,
        destination.key,
        &expected_config,
        &[],
        amount,
    )?;
    invoke_signed(
        &mint_ix,
        &[truth_mint.clone(), destination.clone(), config_pda.clone(), token_program.clone()],
        &[&[CONFIG_SEED, &[config_bump]]],
    )?;

    msg!("Minted {} $TRUTH to {}", amount, destination.key);
    Ok(())
}

/// VerifyDns: admin sets dns_verified on DomainRecord.
pub fn process_verify_dns(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    domain: &str,
    dns_wallet: [u8; 32],
) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();
    let authority = next_account_info(accounts_iter)?;
    let domain_pda = next_account_info(accounts_iter)?;
    let config_pda = next_account_info(accounts_iter)?;

    if !authority.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }

    // Validate config PDA
    let (expected_config, _) = Pubkey::find_program_address(&[CONFIG_SEED], program_id);
    if *config_pda.key != expected_config {
        return Err(ProgramError::InvalidSeeds);
    }

    let config = deser_config(&config_pda.data.borrow())?;
    if config.authority != *authority.key {
        return Err(StakingError::Unauthorized.into());
    }

    // Verify domain PDA
    let (expected_domain, _) =
        Pubkey::find_program_address(&[DOMAIN_SEED, domain.as_bytes()], program_id);
    if *domain_pda.key != expected_domain {
        return Err(ProgramError::InvalidSeeds);
    }

    let mut record = super::deser_domain_record(&domain_pda.data.borrow())?;
    if !record.is_initialized {
        return Err(ProgramError::UninitializedAccount);
    }

    record.dns_verified = true;
    record.dns_wallet = Pubkey::new_from_array(dns_wallet);
    borsh::to_writer(&mut &mut domain_pda.data.borrow_mut()[..], &record)?;

    msg!("DNS verified for {}", domain);
    Ok(())
}

/// BlacklistDomain: blacklist authority sets blacklisted flag.
pub fn process_blacklist_domain(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    domain: &str,
) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();
    let bl_authority = next_account_info(accounts_iter)?;
    let domain_pda = next_account_info(accounts_iter)?;
    let config_pda = next_account_info(accounts_iter)?;

    if !bl_authority.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }

    // Validate config PDA
    let (expected_bl_config, _) = Pubkey::find_program_address(&[CONFIG_SEED], program_id);
    if *config_pda.key != expected_bl_config {
        return Err(ProgramError::InvalidSeeds);
    }

    let config = deser_config(&config_pda.data.borrow())?;
    if config.blacklist_authority != *bl_authority.key {
        return Err(StakingError::Unauthorized.into());
    }

    let (expected_domain, _) =
        Pubkey::find_program_address(&[DOMAIN_SEED, domain.as_bytes()], program_id);
    if *domain_pda.key != expected_domain {
        return Err(ProgramError::InvalidSeeds);
    }

    let mut record = super::deser_domain_record(&domain_pda.data.borrow())?;
    if !record.is_initialized {
        return Err(ProgramError::UninitializedAccount);
    }

    record.blacklisted = true;
    borsh::to_writer(&mut &mut domain_pda.data.borrow_mut()[..], &record)?;

    msg!("Domain {} blacklisted", domain);
    Ok(())
}

/// SetUgcPlatform: admin sets or clears the UGC platform flag on a DomainRecord.
/// When `is_ugc` is true, domain-level staking provides zero passthrough to entity content.
pub fn process_set_ugc_platform(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    domain: &str,
    is_ugc: bool,
) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();
    let authority = next_account_info(accounts_iter)?;
    let domain_pda = next_account_info(accounts_iter)?;
    let config_pda = next_account_info(accounts_iter)?;

    if !authority.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }

    // Validate config PDA
    let (expected_config, _) = Pubkey::find_program_address(&[CONFIG_SEED], program_id);
    if *config_pda.key != expected_config {
        return Err(ProgramError::InvalidSeeds);
    }

    let config = deser_config(&config_pda.data.borrow())?;
    if config.authority != *authority.key {
        return Err(StakingError::Unauthorized.into());
    }

    // Verify domain PDA
    let (expected_domain, _) =
        Pubkey::find_program_address(&[DOMAIN_SEED, domain.as_bytes()], program_id);
    if *domain_pda.key != expected_domain {
        return Err(ProgramError::InvalidSeeds);
    }

    let mut record = super::deser_domain_record(&domain_pda.data.borrow())?;
    if !record.is_initialized {
        return Err(ProgramError::UninitializedAccount);
    }

    record.is_ugc_platform = is_ugc;
    borsh::to_writer(&mut &mut domain_pda.data.borrow_mut()[..], &record)?;

    msg!("Domain {} UGC platform flag set to {}", domain, is_ugc);
    Ok(())
}
