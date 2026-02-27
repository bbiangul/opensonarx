use super::deser_config;
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    msg,
    program::invoke,
    program_error::ProgramError,
    pubkey::Pubkey,
    sysvar::Sysvar,
};

use crate::error::StakingError;
use crate::state::*;

/// FundRewards: transfer $TRUTH into the reward vault.
///
/// NOTE: Funded tokens remain permanently in the vault. Staker/sentinel rewards
/// are minted (not transferred), so FundRewards acts as a rate-limiter for
/// emissions, not a reward pool. Callers should understand this is a one-way
/// transfer with no withdrawal mechanism.
pub fn process_fund_rewards(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    amount: u64,
) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();
    let funder = next_account_info(accounts_iter)?;
    let funder_token = next_account_info(accounts_iter)?;
    let vault_token = next_account_info(accounts_iter)?;
    let token_program = next_account_info(accounts_iter)?;
    let config_pda = next_account_info(accounts_iter)?;

    if !funder.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }

    // Validate config, vault, and token program
    let (expected_config, _) = Pubkey::find_program_address(&[CONFIG_SEED], program_id);
    if *config_pda.key != expected_config {
        return Err(ProgramError::InvalidSeeds);
    }
    let mut config = deser_config(&config_pda.data.borrow())?;
    if *vault_token.key != config.reward_vault {
        return Err(ProgramError::InvalidSeeds);
    }
    if *token_program.key != spl_token::id() {
        return Err(ProgramError::IncorrectProgramId);
    }

    let transfer_ix = spl_token::instruction::transfer(
        token_program.key,
        funder_token.key,
        vault_token.key,
        funder.key,
        &[],
        amount,
    )?;
    invoke(
        &transfer_ix,
        &[funder_token.clone(), vault_token.clone(), funder.clone(), token_program.clone()],
    )?;

    // M3 fix: track cumulative funded amount for permissionless distribution cap
    config.total_funded = config.total_funded
        .checked_add(amount)
        .ok_or(StakingError::Overflow)?;
    borsh::to_writer(&mut &mut config_pda.data.borrow_mut()[..], &config)?;

    msg!("Funded {} $TRUTH to reward vault", amount);
    Ok(())
}

/// UpdateRewardIndex: permissionless update of the cumulative reward_per_token.
///
/// M3 fix: anyone can call this (no admin check). Distribution is capped at
/// the amount actually funded via FundRewards (total_funded - total_distributed).
///
/// Also enforces per-epoch emission cap and drains accumulated sentinel fees (M1).
///
/// 70% of emissions go to staker (publisher/curator) rewards, 30% to sentinel rewards.
pub fn process_update_reward_index(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    funded_amount: u64,
) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();
    let _caller = next_account_info(accounts_iter)?; // kept for account layout compat
    let config_pda = next_account_info(accounts_iter)?;

    let (expected_config, _) = Pubkey::find_program_address(&[CONFIG_SEED], program_id);
    if *config_pda.key != expected_config {
        return Err(ProgramError::InvalidSeeds);
    }

    let mut config = deser_config(&config_pda.data.borrow())?;
    if !config.is_initialized {
        return Err(ProgramError::UninitializedAccount);
    }

    // M3 fix: cap at distributable amount (total_funded - total_distributed)
    let distributable = config.total_funded.saturating_sub(config.total_distributed);
    let funded_amount = funded_amount.min(distributable);

    // Use stored epoch emission cap (set by AdvanceEpoch, initialized to 10M)
    let epoch_cap = config.epoch_max_emission;

    // Cap funded_amount at remaining budget for this epoch
    let remaining_budget = epoch_cap.saturating_sub(config.epoch_funded_amount);
    let capped_amount = funded_amount.min(remaining_budget);

    // M5 fix: cap at supply headroom to prevent phantom rewards
    let supply_headroom = config.max_total_supply.saturating_sub(config.total_minted);
    let capped_amount = capped_amount.min(supply_headroom);

    if capped_amount == 0 {
        msg!("Epoch {} emission cap reached ({}), no more funding allowed", config.current_epoch, epoch_cap);
        borsh::to_writer(&mut &mut config_pda.data.borrow_mut()[..], &config)?;
        return Ok(());
    }

    // Governable staker/sentinel split (default 70/30)
    let staker_portion = capped_amount * config.staker_emission_pct as u64 / 100;
    let sentinel_emission = capped_amount.saturating_sub(staker_portion);

    // M1 fix: drain accumulated sentinel fees into sentinel reward index.
    // Only drain when sentinels exist — otherwise fees stay in accumulator
    // until sentinels join (prevents permanent fee loss).
    let sentinel_bonus = if config.total_sentinel_staked > 0 {
        let bonus = config.accumulated_sentinel_fees;
        config.accumulated_sentinel_fees = 0;
        bonus
    } else {
        0
    };
    let sentinel_portion = sentinel_emission.saturating_add(sentinel_bonus);

    // Update staker reward index (skip if no stakers — emissions would be lost)
    if config.total_staked > 0 && staker_portion > 0 {
        let increment = (staker_portion as u128)
            .checked_mul(1_000_000_000_000_000_000u128) // 1e18
            .ok_or(StakingError::Overflow)?
            .checked_div(config.total_staked as u128)
            .ok_or(StakingError::Overflow)?;

        config.reward_per_token_stored = config
            .reward_per_token_stored
            .checked_add(increment)
            .ok_or(StakingError::Overflow)?;
    } else if staker_portion > 0 {
        msg!("Warning: {} staker emissions skipped (no stakers), not counted against cap", staker_portion);
    }

    // Update sentinel reward index (skip if no sentinels — emissions would be lost)
    if config.total_sentinel_staked > 0 && sentinel_portion > 0 {
        let sentinel_increment = (sentinel_portion as u128)
            .checked_mul(1_000_000_000_000_000_000u128)
            .ok_or(StakingError::Overflow)?
            .checked_div(config.total_sentinel_staked as u128)
            .ok_or(StakingError::Overflow)?;

        config.sentinel_reward_per_token = config
            .sentinel_reward_per_token
            .checked_add(sentinel_increment)
            .ok_or(StakingError::Overflow)?;
    } else if sentinel_portion > 0 {
        msg!("Warning: {} sentinel emissions skipped (no sentinels), not counted against cap", sentinel_portion);
    }

    // Only count successfully distributed EMISSIONS against the cap.
    // Sentinel fee bonus is excluded — those tokens came from channel settlements
    // (already in vault), not from FundRewards, so they shouldn't consume the
    // epoch emission budget or the total_funded distribution budget.
    let mut actually_funded = 0u64;
    if config.total_staked > 0 {
        actually_funded = actually_funded.saturating_add(staker_portion);
    }
    if config.total_sentinel_staked > 0 {
        actually_funded = actually_funded.saturating_add(sentinel_emission);
    }

    config.epoch_funded_amount = config.epoch_funded_amount
        .checked_add(actually_funded)
        .ok_or(StakingError::Overflow)?;

    // M3 fix: track cumulative distributed amount
    config.total_distributed = config.total_distributed
        .checked_add(actually_funded)
        .ok_or(StakingError::Overflow)?;

    borsh::to_writer(&mut &mut config_pda.data.borrow_mut()[..], &config)?;
    msg!(
        "Reward index updated: {} staker / {} sentinel (epoch {} cap={})",
        staker_portion, sentinel_portion, config.current_epoch, epoch_cap
    );

    Ok(())
}

/// AdvanceEpoch: admin advances to the next emission epoch.
/// Resets epoch_funded_amount to 0, increments current_epoch.
/// Enforces min_epoch_duration — cannot advance too early.
///
/// Accounts:
///   0. `[signer]` Authority
///   1. `[writable]` Config PDA
///   2. `[]` Clock sysvar
pub fn process_advance_epoch(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();
    let authority = next_account_info(accounts_iter)?;
    let config_pda = next_account_info(accounts_iter)?;

    if !authority.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }

    let (expected_config, _) = Pubkey::find_program_address(&[CONFIG_SEED], program_id);
    if *config_pda.key != expected_config {
        return Err(ProgramError::InvalidSeeds);
    }

    let mut config = deser_config(&config_pda.data.borrow())?;
    if !config.is_initialized {
        return Err(ProgramError::UninitializedAccount);
    }
    if config.authority != *authority.key {
        return Err(StakingError::Unauthorized.into());
    }

    // Enforce minimum epoch duration (Clock sysvar is optional for backward compat —
    // if present and min_epoch_duration > 0, enforce the timing constraint)
    if config.min_epoch_duration > 0 {
        let clock_sysvar = next_account_info(accounts_iter)?;
        let clock = solana_program::clock::Clock::from_account_info(clock_sysvar)?;
        if clock.unix_timestamp < config.epoch_started_at + config.min_epoch_duration {
            return Err(StakingError::EpochTooEarly.into());
        }
        config.epoch_started_at = clock.unix_timestamp;
    }

    let prev_epoch = config.current_epoch;
    config.current_epoch = config.current_epoch.saturating_add(1);
    config.epoch_funded_amount = 0;
    config.jury_rewards_this_epoch = 0; // Reset per-epoch jury reward counter

    // Compute new epoch's max emission: 1.5% decay per epoch (~3 year schedule with 7-day epochs)
    let mut emission = 10_000_000u64;
    for _ in 0..config.current_epoch {
        emission = emission * 985 / 1000;
        if emission == 0 { break; }
    }
    config.epoch_max_emission = emission;

    borsh::to_writer(&mut &mut config_pda.data.borrow_mut()[..], &config)?;
    msg!(
        "Advanced from epoch {} to epoch {} (max emission: {})",
        prev_epoch, config.current_epoch, config.epoch_max_emission
    );

    Ok(())
}
