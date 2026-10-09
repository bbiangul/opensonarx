use super::{deser, deser_config};
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    clock::Clock,
    entrypoint::ProgramResult,
    msg,
    program::invoke,
    program::invoke_signed,
    program_error::ProgramError,
    pubkey::Pubkey,
    rent::Rent,
    system_instruction,
    sysvar::Sysvar,
};

use crate::error::StakingError;
use crate::state::*;

/// StakeSentinel: stake $TRUTH globally as a Sentinel (not per-domain).
///
/// Accounts:
///   0. `[signer, writable]` Sentinel
///   1. `[writable]` Sentinel's $TRUTH token account
///   2. `[writable]` Program vault token account
///   3. `[writable]` SentinelAccount PDA
///   4. `[writable]` Config PDA
///   5. `[]` Token Program
///   6. `[]` System Program
///   7. `[]` Clock sysvar
pub fn process_stake_sentinel(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    amount: u64,
) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();
    let sentinel = next_account_info(accounts_iter)?;
    let sentinel_token = next_account_info(accounts_iter)?;
    let vault_token = next_account_info(accounts_iter)?;
    let sentinel_pda = next_account_info(accounts_iter)?;
    let config_pda = next_account_info(accounts_iter)?;
    let token_program = next_account_info(accounts_iter)?;
    let system_program = next_account_info(accounts_iter)?;
    let clock_sysvar = next_account_info(accounts_iter)?;

    if !sentinel.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }

    // Validate config PDA
    let (expected_config, _) = Pubkey::find_program_address(&[CONFIG_SEED], program_id);
    if *config_pda.key != expected_config {
        return Err(ProgramError::InvalidSeeds);
    }

    let mut config = deser_config(&config_pda.data.borrow())?;
    if !config.is_initialized {
        return Err(ProgramError::UninitializedAccount);
    }
    if amount < config.min_stake {
        return Err(StakingError::BelowMinimum.into());
    }

    // Validate vault and token program
    if *vault_token.key != config.reward_vault {
        return Err(ProgramError::InvalidSeeds);
    }
    if *token_program.key != spl_token::id() {
        return Err(ProgramError::IncorrectProgramId);
    }

    // Derive sentinel PDA: ["sentinel", sentinel_pubkey]
    let (expected_sentinel, sentinel_bump) =
        Pubkey::find_program_address(&[SENTINEL_SEED, sentinel.key.as_ref()], program_id);
    if *sentinel_pda.key != expected_sentinel {
        return Err(ProgramError::InvalidSeeds);
    }

    let clock = Clock::from_account_info(clock_sysvar)?;
    let rent = Rent::get()?;

    // Transfer $TRUTH from sentinel to vault
    let transfer_ix = spl_token::instruction::transfer(
        token_program.key,
        sentinel_token.key,
        vault_token.key,
        sentinel.key,
        &[],
        amount,
    )?;
    invoke(
        &transfer_ix,
        &[
            sentinel_token.clone(),
            vault_token.clone(),
            sentinel.clone(),
            token_program.clone(),
        ],
    )?;

    // Create SentinelAccount PDA
    let space = SentinelAccount::MAX_SIZE;
    let lamports = rent.minimum_balance(space);
    invoke_signed(
        &system_instruction::create_account(
            sentinel.key,
            sentinel_pda.key,
            lamports,
            space as u64,
            program_id,
        ),
        &[
            sentinel.clone(),
            sentinel_pda.clone(),
            system_program.clone(),
        ],
        &[&[SENTINEL_SEED, sentinel.key.as_ref(), &[sentinel_bump]]],
    )?;

    let sentinel_account = SentinelAccount {
        is_initialized: true,
        sentinel: *sentinel.key,
        amount,
        staked_at: clock.unix_timestamp,
        cooldown_until: 0,
        reward_debt: config.sentinel_reward_per_token,
        slashed_amount: 0,
        bump: sentinel_bump,
    };
    borsh::to_writer(
        &mut &mut sentinel_pda.data.borrow_mut()[..],
        &sentinel_account,
    )?;

    // Update config
    config.total_sentinel_staked = config
        .total_sentinel_staked
        .checked_add(amount)
        .ok_or(StakingError::Overflow)?;
    borsh::to_writer(&mut &mut config_pda.data.borrow_mut()[..], &config)?;

    msg!("Sentinel staked {} $TRUTH by {}", amount, sentinel.key);
    Ok(())
}

/// UnstakeSentinel: begin cooldown for a Sentinel.
///
/// Accounts:
///   0. `[signer]` Sentinel
///   1. `[writable]` SentinelAccount PDA
///   2. `[]` Config PDA
///   3. `[]` Clock sysvar
pub fn process_unstake_sentinel(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();
    let sentinel = next_account_info(accounts_iter)?;
    let sentinel_pda = next_account_info(accounts_iter)?;
    let config_pda = next_account_info(accounts_iter)?;
    let clock_sysvar = next_account_info(accounts_iter)?;

    if !sentinel.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }

    // Validate config PDA
    let (expected_unstake_config, _) = Pubkey::find_program_address(&[CONFIG_SEED], program_id);
    if *config_pda.key != expected_unstake_config {
        return Err(ProgramError::InvalidSeeds);
    }

    let mut config = deser_config(&config_pda.data.borrow())?;
    if !config.is_initialized {
        return Err(ProgramError::UninitializedAccount);
    }

    let (expected_sentinel, _) =
        Pubkey::find_program_address(&[SENTINEL_SEED, sentinel.key.as_ref()], program_id);
    if *sentinel_pda.key != expected_sentinel {
        return Err(ProgramError::InvalidSeeds);
    }

    let mut account = deser::<SentinelAccount>(&sentinel_pda.data.borrow())?;
    if !account.is_initialized {
        return Err(ProgramError::UninitializedAccount);
    }
    if account.sentinel != *sentinel.key {
        return Err(ProgramError::IllegalOwner);
    }
    if account.cooldown_until != 0 {
        return Err(StakingError::CooldownActive.into());
    }

    let clock = Clock::from_account_info(clock_sysvar)?;
    account.cooldown_until = clock.unix_timestamp + config.cooldown_secs;
    borsh::to_writer(&mut &mut sentinel_pda.data.borrow_mut()[..], &account)?;

    // Fix 9: Decrement total_sentinel_staked at unstake time (not withdraw)
    config.total_sentinel_staked = config.total_sentinel_staked.saturating_sub(account.amount);
    borsh::to_writer(&mut &mut config_pda.data.borrow_mut()[..], &config)?;

    msg!(
        "Sentinel unstake initiated, cooldown until {}",
        account.cooldown_until
    );
    Ok(())
}

/// WithdrawSentinel: after cooldown, return $TRUTH and close SentinelAccount PDA.
/// Slashed tokens: 50% burned, 50% stays in vault as protocol revenue.
///
/// Accounts:
///   0. `[signer, writable]` Sentinel
///   1. `[writable]` SentinelAccount PDA
///   2. `[writable]` Sentinel's $TRUTH token account
///   3. `[writable]` Program vault token account
///   4. `[writable]` Config PDA
///   5. `[]` Token Program
///   6. `[]` Clock sysvar
///   7. `[writable]` $TRUTH Mint (required — used for slash burn)
pub fn process_withdraw_sentinel(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();
    let sentinel = next_account_info(accounts_iter)?;
    let sentinel_pda = next_account_info(accounts_iter)?;
    let sentinel_token = next_account_info(accounts_iter)?;
    let vault_token = next_account_info(accounts_iter)?;
    let config_pda = next_account_info(accounts_iter)?;
    let token_program = next_account_info(accounts_iter)?;
    let clock_sysvar = next_account_info(accounts_iter)?;
    let truth_mint = next_account_info(accounts_iter)?;

    if !sentinel.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }

    // Validate config
    let config = deser_config(&config_pda.data.borrow())?;
    let (expected_config, config_bump) = Pubkey::find_program_address(&[CONFIG_SEED], program_id);
    if *config_pda.key != expected_config {
        return Err(ProgramError::InvalidSeeds);
    }

    // Validate vault and token program
    if *vault_token.key != config.reward_vault {
        return Err(ProgramError::InvalidSeeds);
    }
    if *token_program.key != spl_token::id() {
        return Err(ProgramError::IncorrectProgramId);
    }

    // Validate sentinel PDA
    let (expected_sentinel, _) =
        Pubkey::find_program_address(&[SENTINEL_SEED, sentinel.key.as_ref()], program_id);
    if *sentinel_pda.key != expected_sentinel {
        return Err(ProgramError::InvalidSeeds);
    }

    let account = deser::<SentinelAccount>(&sentinel_pda.data.borrow())?;
    if !account.is_initialized {
        return Err(ProgramError::UninitializedAccount);
    }
    if account.sentinel != *sentinel.key {
        return Err(ProgramError::IllegalOwner);
    }
    if account.cooldown_until == 0 {
        return Err(StakingError::CooldownNotExpired.into());
    }

    let clock = Clock::from_account_info(clock_sysvar)?;
    if clock.unix_timestamp < account.cooldown_until {
        return Err(StakingError::CooldownNotExpired.into());
    }

    // Transfer $TRUTH from vault back to sentinel (minus slashed)
    let withdraw_amount = account.amount.saturating_sub(account.slashed_amount);
    if withdraw_amount > 0 {
        let transfer_ix = spl_token::instruction::transfer(
            token_program.key,
            vault_token.key,
            sentinel_token.key,
            config_pda.key,
            &[],
            withdraw_amount,
        )?;
        invoke_signed(
            &transfer_ix,
            &[
                vault_token.clone(),
                sentinel_token.clone(),
                config_pda.clone(),
                token_program.clone(),
            ],
            &[&[CONFIG_SEED, &[config_bump]]],
        )?;
    }

    // Slash burn: 50% of slashed tokens burned, 50% stays in vault as protocol revenue
    if account.slashed_amount > 0 {
        if *truth_mint.key != config.truth_mint {
            return Err(ProgramError::InvalidSeeds);
        }
        let burn_half = account.slashed_amount / 2;
        if burn_half > 0 {
            let burn_ix = spl_token::instruction::burn(
                token_program.key,
                vault_token.key,
                truth_mint.key,
                config_pda.key,
                &[],
                burn_half,
            )?;
            invoke_signed(
                &burn_ix,
                &[
                    vault_token.clone(),
                    truth_mint.clone(),
                    config_pda.clone(),
                    token_program.clone(),
                ],
                &[&[CONFIG_SEED, &[config_bump]]],
            )?;
            msg!(
                "Sentinel slash burn: {} $TRUTH burned, {} retained as protocol revenue",
                burn_half,
                account.slashed_amount - burn_half
            );
        }
    }

    // Fix 9: total_sentinel_staked already decremented at unstake time, no config write needed here.

    // Close sentinel PDA: transfer lamports back to sentinel
    let pda_lamports = sentinel_pda.lamports();
    **sentinel_pda.lamports.borrow_mut() = 0;
    **sentinel.lamports.borrow_mut() = sentinel
        .lamports()
        .checked_add(pda_lamports)
        .ok_or(StakingError::Overflow)?;

    // Zero out the data
    let mut data = sentinel_pda.data.borrow_mut();
    for byte in data.iter_mut() {
        *byte = 0;
    }

    msg!(
        "Sentinel withdrew {} $TRUTH (slashed: {})",
        withdraw_amount,
        account.slashed_amount
    );
    Ok(())
}

/// ClaimSentinelReward: claim accrued sentinel rewards via sentinel reward index.
///
/// Accounts:
///   0. `[signer]` Sentinel
///   1. `[writable]` SentinelAccount PDA
///   2. `[writable]` $TRUTH Mint
///   3. `[writable]` Sentinel's $TRUTH token account
///   4. `[]` Config PDA
///   5. `[]` Token Program
pub fn process_claim_sentinel_reward(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();
    let sentinel = next_account_info(accounts_iter)?;
    let sentinel_pda = next_account_info(accounts_iter)?;
    let truth_mint = next_account_info(accounts_iter)?;
    let sentinel_token = next_account_info(accounts_iter)?;
    let config_pda = next_account_info(accounts_iter)?;
    let token_program = next_account_info(accounts_iter)?;

    if !sentinel.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }

    let mut config = deser_config(&config_pda.data.borrow())?;
    let (expected_config, config_bump) = Pubkey::find_program_address(&[CONFIG_SEED], program_id);
    if *config_pda.key != expected_config {
        return Err(ProgramError::InvalidSeeds);
    }

    let (expected_sentinel, _) =
        Pubkey::find_program_address(&[SENTINEL_SEED, sentinel.key.as_ref()], program_id);
    if *sentinel_pda.key != expected_sentinel {
        return Err(ProgramError::InvalidSeeds);
    }

    let mut account = deser::<SentinelAccount>(&sentinel_pda.data.borrow())?;
    if !account.is_initialized {
        return Err(ProgramError::UninitializedAccount);
    }
    if account.sentinel != *sentinel.key {
        return Err(ProgramError::IllegalOwner);
    }

    // M8 fix: reward claims are allowed during cooldown so sentinels don't lose accrued rewards.

    // Validate mint and token program
    if *truth_mint.key != config.truth_mint {
        return Err(ProgramError::InvalidSeeds);
    }
    if *token_program.key != spl_token::id() {
        return Err(ProgramError::IncorrectProgramId);
    }

    // Sentinel-specific Synthetix reward math
    let pending = (account.amount as u128)
        .checked_mul(
            config
                .sentinel_reward_per_token
                .checked_sub(account.reward_debt)
                .ok_or(StakingError::Overflow)?,
        )
        .ok_or(StakingError::Overflow)?
        / 1_000_000_000_000_000_000u128; // 1e18

    if pending > 0 {
        let payout: u64 = pending.try_into().map_err(|_| StakingError::Overflow)?;

        // Enforce supply cap before minting
        super::check_and_increment_supply(&mut config, payout)?;

        let mint_ix = spl_token::instruction::mint_to(
            token_program.key,
            truth_mint.key,
            sentinel_token.key,
            config_pda.key,
            &[],
            payout,
        )?;
        invoke_signed(
            &mint_ix,
            &[
                truth_mint.clone(),
                sentinel_token.clone(),
                config_pda.clone(),
                token_program.clone(),
            ],
            &[&[CONFIG_SEED, &[config_bump]]],
        )?;
    }

    account.reward_debt = config.sentinel_reward_per_token;
    borsh::to_writer(&mut &mut sentinel_pda.data.borrow_mut()[..], &account)?;
    borsh::to_writer(&mut &mut config_pda.data.borrow_mut()[..], &config)?;

    msg!("Sentinel claimed {} $TRUTH rewards", pending);
    Ok(())
}

/// SlashSentinel: authority slashes a sentinel's staked amount.
///
/// Accounts:
///   0. `[signer]` Authority (must match config.authority)
///   1. `[writable]` SentinelAccount PDA
///   2. `[]` Config PDA
pub fn process_slash_sentinel(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    sentinel_bytes: [u8; 32],
    amount: u64,
) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();
    let authority = next_account_info(accounts_iter)?;
    let sentinel_pda = next_account_info(accounts_iter)?;
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
    if !config.is_initialized {
        return Err(ProgramError::UninitializedAccount);
    }
    if config.authority != *authority.key {
        return Err(StakingError::Unauthorized.into());
    }

    // Validate sentinel PDA
    let sentinel_key = Pubkey::new_from_array(sentinel_bytes);
    let (expected_sentinel, _) =
        Pubkey::find_program_address(&[SENTINEL_SEED, sentinel_key.as_ref()], program_id);
    if *sentinel_pda.key != expected_sentinel {
        return Err(ProgramError::InvalidSeeds);
    }

    let mut account = deser::<SentinelAccount>(&sentinel_pda.data.borrow())?;
    if !account.is_initialized {
        return Err(ProgramError::UninitializedAccount);
    }

    // Increment slashed_amount, capped at total staked amount
    let max_slash = account.amount.saturating_sub(account.slashed_amount);
    let actual_slash = amount.min(max_slash);
    account.slashed_amount = account.slashed_amount.saturating_add(actual_slash);
    borsh::to_writer(&mut &mut sentinel_pda.data.borrow_mut()[..], &account)?;

    msg!(
        "Sentinel {} slashed by {} (total slashed: {})",
        sentinel_key,
        actual_slash,
        account.slashed_amount
    );
    Ok(())
}
