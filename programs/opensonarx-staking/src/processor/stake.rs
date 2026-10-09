use super::{deser_config, deser_gov_lock, deser_stake_v2};
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

/// Validate an entity ref string for on-chain storage.
/// Must be non-empty, ≤253 bytes, contain `.` before first `/`,
/// and only use allowed characters.
pub fn validate_entity_ref(domain: &str) -> ProgramResult {
    if domain.is_empty() {
        msg!("Entity ref cannot be empty");
        return Err(ProgramError::InvalidArgument);
    }
    if domain.len() > 253 {
        msg!("Entity ref exceeds 253 bytes");
        return Err(ProgramError::InvalidArgument);
    }
    // Must contain a dot (domain separator)
    let dot_pos = domain.find('.');
    let slash_pos = domain.find('/');
    match (dot_pos, slash_pos) {
        (None, _) => {
            msg!("Entity ref must contain '.'");
            return Err(ProgramError::InvalidArgument);
        }
        (Some(d), Some(s)) if d > s => {
            msg!("'.' must appear before first '/'");
            return Err(ProgramError::InvalidArgument);
        }
        _ => {}
    }
    // Check allowed characters.
    // Domain portion (before first '/') must be lowercase (a-z, 0-9, '.', '-').
    // Path portion (after first '/') allows uppercase (for channel IDs etc).
    let slash_pos = domain.find('/');
    let domain_part = match slash_pos {
        Some(pos) => &domain[..pos],
        None => domain,
    };

    // Reject leading/trailing dot or dash in domain
    if domain_part.starts_with('.')
        || domain_part.ends_with('.')
        || domain_part.starts_with('-')
        || domain_part.ends_with('-')
    {
        msg!("Domain cannot start or end with '.' or '-'");
        return Err(ProgramError::InvalidArgument);
    }
    let path_part = slash_pos.map(|pos| &domain[pos + 1..]);

    // Validate domain characters: lowercase only
    for c in domain_part.chars() {
        if !c.is_ascii_lowercase() && !c.is_ascii_digit() && !matches!(c, '.' | '-') {
            msg!("Invalid character in domain: '{}' (must be lowercase)", c);
            return Err(ProgramError::InvalidArgument);
        }
    }
    // Validate path characters (if any): allow uppercase for channel IDs
    if let Some(path) = path_part {
        for c in path.chars() {
            if !c.is_ascii_alphanumeric() && !matches!(c, '.' | '/' | '_' | '@' | '-') {
                msg!("Invalid character in entity path: '{}'", c);
                return Err(ProgramError::InvalidArgument);
            }
        }
    }
    Ok(())
}

/// StakeV2: transfer $TRUTH and create StakeAccountV2 + DomainRecord.
pub fn process_stake_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    domain: &str,
    amount: u64,
    stake_type_raw: u8,
) -> ProgramResult {
    // Validate entity ref format
    validate_entity_ref(domain)?;

    let accounts_iter = &mut accounts.iter();
    let staker = next_account_info(accounts_iter)?;
    let staker_token = next_account_info(accounts_iter)?;
    let vault_token = next_account_info(accounts_iter)?;
    let stake_pda = next_account_info(accounts_iter)?;
    let domain_pda = next_account_info(accounts_iter)?;
    let config_pda = next_account_info(accounts_iter)?;
    let token_program = next_account_info(accounts_iter)?;
    let system_program = next_account_info(accounts_iter)?;
    let clock_sysvar = next_account_info(accounts_iter)?;

    if !staker.is_signer {
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

    let stake_type = match stake_type_raw {
        0 => StakeType::Publisher,
        1 => StakeType::Curator,
        2 => return Err(StakingError::InvalidStakeType.into()), // Sentinels use StakeSentinel
        _ => return Err(StakingError::InvalidStakeType.into()),
    };

    // Derive stake PDA: ["stake", domain, staker]
    let (expected_stake, stake_bump) = Pubkey::find_program_address(
        &[STAKE_SEED, domain.as_bytes(), staker.key.as_ref()],
        program_id,
    );
    if *stake_pda.key != expected_stake {
        return Err(ProgramError::InvalidSeeds);
    }

    // Derive domain PDA
    let (expected_domain, domain_bump) =
        Pubkey::find_program_address(&[DOMAIN_SEED, domain.as_bytes()], program_id);
    if *domain_pda.key != expected_domain {
        return Err(ProgramError::InvalidSeeds);
    }

    let clock = Clock::from_account_info(clock_sysvar)?;
    let rent = Rent::get()?;

    // Transfer $TRUTH from staker to vault
    let transfer_ix = spl_token::instruction::transfer(
        token_program.key,
        staker_token.key,
        vault_token.key,
        staker.key,
        &[],
        amount,
    )?;
    invoke(
        &transfer_ix,
        &[
            staker_token.clone(),
            vault_token.clone(),
            staker.clone(),
            token_program.clone(),
        ],
    )?;

    // Create StakeAccountV2 PDA
    let stake_space = StakeAccountV2::MAX_SIZE;
    let stake_lamports = rent.minimum_balance(stake_space);
    invoke_signed(
        &system_instruction::create_account(
            staker.key,
            stake_pda.key,
            stake_lamports,
            stake_space as u64,
            program_id,
        ),
        &[staker.clone(), stake_pda.clone(), system_program.clone()],
        &[&[
            STAKE_SEED,
            domain.as_bytes(),
            staker.key.as_ref(),
            &[stake_bump],
        ]],
    )?;

    let stake_account = StakeAccountV2 {
        is_initialized: true,
        staker: *staker.key,
        domain: domain.to_string(),
        amount,
        staked_at: clock.unix_timestamp,
        cooldown_until: 0,
        stake_type,
        reward_debt: config.reward_per_token_stored,
        dns_verified: false,
        slashed_amount: 0,
        bump: stake_bump,
        governance_lock_until: 0,
    };
    borsh::to_writer(&mut &mut stake_pda.data.borrow_mut()[..], &stake_account)?;

    // Create or update DomainRecord
    if domain_pda.data_len() == 0 || domain_pda.lamports() == 0 {
        // Create new domain record
        let domain_space = DomainRecord::MAX_SIZE;
        let domain_lamports = rent.minimum_balance(domain_space);
        invoke_signed(
            &system_instruction::create_account(
                staker.key,
                domain_pda.key,
                domain_lamports,
                domain_space as u64,
                program_id,
            ),
            &[staker.clone(), domain_pda.clone(), system_program.clone()],
            &[&[DOMAIN_SEED, domain.as_bytes(), &[domain_bump]]],
        )?;

        let (pub_count, cur_count) = match stake_type {
            StakeType::Publisher => (1u32, 0u32),
            StakeType::Curator => (0u32, 1u32),
            StakeType::Sentinel => return Err(StakingError::InvalidStakeType.into()),
        };

        let record = DomainRecord {
            is_initialized: true,
            domain: domain.to_string(),
            total_staked: amount,
            publisher_count: pub_count,
            curator_count: cur_count,
            blacklisted: false,
            dns_verified: false,
            dns_wallet: Pubkey::default(),
            bump: domain_bump,
            is_ugc_platform: false,
            effective_slash_pct: 0,
            active_dispute_nonce: u64::MAX,
        };
        borsh::to_writer(&mut &mut domain_pda.data.borrow_mut()[..], &record)?;
    } else {
        // Update existing domain record
        let mut record = super::deser_domain_record(&domain_pda.data.borrow())?;
        if record.blacklisted {
            return Err(StakingError::DomainBlacklisted.into());
        }
        record.total_staked = record
            .total_staked
            .checked_add(amount)
            .ok_or(StakingError::Overflow)?;
        match stake_type {
            StakeType::Publisher => record.publisher_count += 1,
            StakeType::Curator => record.curator_count += 1,
            StakeType::Sentinel => return Err(StakingError::InvalidStakeType.into()),
        }
        borsh::to_writer(&mut &mut domain_pda.data.borrow_mut()[..], &record)?;
    }

    // Update total_staked in config
    config.total_staked = config
        .total_staked
        .checked_add(amount)
        .ok_or(StakingError::Overflow)?;
    borsh::to_writer(&mut &mut config_pda.data.borrow_mut()[..], &config)?;

    msg!(
        "StakeV2: {} $TRUTH for {} by {}",
        amount,
        domain,
        staker.key
    );
    Ok(())
}

/// UnstakeV2: begin cooldown period.
pub fn process_unstake_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    domain: &str,
) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();
    let staker = next_account_info(accounts_iter)?;
    let stake_pda = next_account_info(accounts_iter)?;
    let config_pda = next_account_info(accounts_iter)?;
    let domain_pda = next_account_info(accounts_iter)?;
    let clock_sysvar = next_account_info(accounts_iter)?;
    let gov_lock_pda = next_account_info(accounts_iter)?;

    if !staker.is_signer {
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

    let (expected_stake, _) = Pubkey::find_program_address(
        &[STAKE_SEED, domain.as_bytes(), staker.key.as_ref()],
        program_id,
    );
    if *stake_pda.key != expected_stake {
        return Err(ProgramError::InvalidSeeds);
    }

    let mut account = deser_stake_v2(&stake_pda.data.borrow())?;
    if !account.is_initialized {
        return Err(ProgramError::UninitializedAccount);
    }
    if account.staker != *staker.key {
        return Err(ProgramError::IllegalOwner);
    }
    if account.cooldown_until != 0 {
        return Err(StakingError::CooldownActive.into());
    }

    let clock = Clock::from_account_info(clock_sysvar)?;

    // S1 fix: block unstaking while domain has an active dispute
    let (expected_domain, _) =
        Pubkey::find_program_address(&[DOMAIN_SEED, domain.as_bytes()], program_id);
    if *domain_pda.key != expected_domain {
        return Err(ProgramError::InvalidSeeds);
    }
    if domain_pda.data_len() > 0 && domain_pda.lamports() > 0 {
        let domain_record = super::deser_domain_record(&domain_pda.data.borrow())?;
        if domain_record.is_initialized && domain_record.active_dispute_nonce != u64::MAX {
            return Err(StakingError::DisputeAlreadyActive.into());
        }
    }

    // S2 fix: per-wallet governance lock check — cannot unstake while any vote lock is active
    let (expected_gov_lock, _) =
        Pubkey::find_program_address(&[GOV_LOCK_SEED, staker.key.as_ref()], program_id);
    if *gov_lock_pda.key != expected_gov_lock {
        return Err(ProgramError::InvalidSeeds);
    }
    if gov_lock_pda.data_len() > 0 && gov_lock_pda.lamports() > 0 {
        let gov_lock = deser_gov_lock(&gov_lock_pda.data.borrow())?;
        if gov_lock.is_initialized && gov_lock.lock_until > clock.unix_timestamp {
            return Err(StakingError::GovernanceLockActive.into());
        }
    }

    account.cooldown_until = clock.unix_timestamp + config.cooldown_secs;
    borsh::to_writer(&mut &mut stake_pda.data.borrow_mut()[..], &account)?;

    // Fix 9: Decrement total_staked at unstake time (not withdraw) to prevent cooldown dilution
    config.total_staked = config.total_staked.saturating_sub(account.amount);
    borsh::to_writer(&mut &mut config_pda.data.borrow_mut()[..], &config)?;

    msg!(
        "UnstakeV2 initiated for {}, cooldown until {}",
        domain,
        account.cooldown_until
    );
    Ok(())
}

/// Withdraw: after cooldown, return $TRUTH and close PDA.
///
/// Accounts:
///   0. `[signer]` Staker
///   1. `[writable]` StakeAccountV2 PDA
///   2. `[writable]` Staker's $TRUTH token account
///   3. `[writable]` Program vault token account
///   4. `[]` DomainRecord PDA
///   5. `[writable]` Config PDA
///   6. `[]` Token Program
///   7. `[]` Clock sysvar
///   8. `[writable]` $TRUTH Mint (required — used for slash burn)
pub fn process_withdraw(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    domain: &str,
) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();
    let staker = next_account_info(accounts_iter)?;
    let stake_pda = next_account_info(accounts_iter)?;
    let staker_token = next_account_info(accounts_iter)?;
    let vault_token = next_account_info(accounts_iter)?;
    let domain_pda = next_account_info(accounts_iter)?;
    let config_pda = next_account_info(accounts_iter)?;
    let token_program = next_account_info(accounts_iter)?;
    let clock_sysvar = next_account_info(accounts_iter)?;
    let truth_mint = next_account_info(accounts_iter)?;

    if !staker.is_signer {
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

    let (expected_stake, _) = Pubkey::find_program_address(
        &[STAKE_SEED, domain.as_bytes(), staker.key.as_ref()],
        program_id,
    );
    if *stake_pda.key != expected_stake {
        return Err(ProgramError::InvalidSeeds);
    }

    let account = deser_stake_v2(&stake_pda.data.borrow())?;
    if !account.is_initialized {
        return Err(ProgramError::UninitializedAccount);
    }
    if account.staker != *staker.key {
        return Err(ProgramError::IllegalOwner);
    }
    if account.cooldown_until == 0 {
        return Err(StakingError::CooldownNotExpired.into());
    }

    let clock = Clock::from_account_info(clock_sysvar)?;
    if clock.unix_timestamp < account.cooldown_until {
        return Err(StakingError::CooldownNotExpired.into());
    }

    // Check domain for lazy slashing (blacklisted domains get slashed on withdraw).
    // Uses effective_slash_pct from DomainRecord (set by dispute severity) if available,
    // falling back to config.slash_pct for pre-upgrade domains.
    let (expected_domain, _) =
        Pubkey::find_program_address(&[DOMAIN_SEED, domain.as_bytes()], program_id);
    let mut slash_amount: u64 = 0;
    if *domain_pda.key == expected_domain {
        let record_opt = {
            let data = domain_pda.data.borrow();
            super::deser_domain_record(&data).ok()
        };
        if let Some(record) = &record_opt {
            if record.is_initialized && record.blacklisted {
                let pct = if record.effective_slash_pct > 0 {
                    record.effective_slash_pct
                } else {
                    config.slash_pct // fallback for pre-upgrade domains
                };
                slash_amount = (account.amount as u128 * pct as u128 / 100) as u64;
            }
        }
    }

    // Transfer $TRUTH from vault back to staker (minus slashed)
    let withdraw_amount = account
        .amount
        .saturating_sub(account.slashed_amount)
        .saturating_sub(slash_amount);
    if withdraw_amount > 0 {
        let transfer_ix = spl_token::instruction::transfer(
            token_program.key,
            vault_token.key,
            staker_token.key,
            config_pda.key,
            &[],
            withdraw_amount,
        )?;
        invoke_signed(
            &transfer_ix,
            &[
                vault_token.clone(),
                staker_token.clone(),
                config_pda.clone(),
                token_program.clone(),
            ],
            &[&[CONFIG_SEED, &[config_bump]]],
        )?;
    }

    // Slash burn: 50% of slashed tokens burned, 50% stays in vault as protocol revenue
    if slash_amount > 0 {
        if *truth_mint.key != config.truth_mint {
            return Err(ProgramError::InvalidSeeds);
        }
        let burn_half = slash_amount / 2;
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
                "Slash burn: {} $TRUTH burned, {} retained as protocol revenue",
                burn_half,
                slash_amount - burn_half
            );
        }
    }

    // Update domain record
    if *domain_pda.key == expected_domain {
        let record_opt = {
            let data = domain_pda.data.borrow();
            super::deser_domain_record(&data).ok()
        };
        if let Some(mut record) = record_opt {
            if record.is_initialized {
                record.total_staked = record.total_staked.saturating_sub(account.amount);
                match account.stake_type {
                    StakeType::Publisher => {
                        record.publisher_count = record.publisher_count.saturating_sub(1)
                    }
                    StakeType::Curator => {
                        record.curator_count = record.curator_count.saturating_sub(1)
                    }
                    StakeType::Sentinel => {} // Sentinels are global, not per-domain
                }
                borsh::to_writer(&mut &mut domain_pda.data.borrow_mut()[..], &record)?;
            }
        }
    }

    // Fix 9: total_staked already decremented at unstake time, no config write needed here.

    // Close stake PDA: transfer lamports back to staker
    let stake_lamports = stake_pda.lamports();
    **stake_pda.lamports.borrow_mut() = 0;
    **staker.lamports.borrow_mut() = staker
        .lamports()
        .checked_add(stake_lamports)
        .ok_or(StakingError::Overflow)?;

    // Zero out the data
    let mut data = stake_pda.data.borrow_mut();
    for byte in data.iter_mut() {
        *byte = 0;
    }

    msg!(
        "Withdrew {} $TRUTH for {} (slashed: {})",
        withdraw_amount,
        domain,
        slash_amount
    );
    Ok(())
}

/// ClaimReward: claim accrued $TRUTH via cumulative reward index.
pub fn process_claim_reward(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    domain: &str,
) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();
    let staker = next_account_info(accounts_iter)?;
    let stake_pda = next_account_info(accounts_iter)?;
    let truth_mint = next_account_info(accounts_iter)?;
    let staker_token = next_account_info(accounts_iter)?;
    let config_pda = next_account_info(accounts_iter)?;
    let token_program = next_account_info(accounts_iter)?;

    if !staker.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }

    let mut config = deser_config(&config_pda.data.borrow())?;
    let (expected_config, config_bump) = Pubkey::find_program_address(&[CONFIG_SEED], program_id);
    if *config_pda.key != expected_config {
        return Err(ProgramError::InvalidSeeds);
    }

    let (expected_stake, _) = Pubkey::find_program_address(
        &[STAKE_SEED, domain.as_bytes(), staker.key.as_ref()],
        program_id,
    );
    if *stake_pda.key != expected_stake {
        return Err(ProgramError::InvalidSeeds);
    }

    let mut account = deser_stake_v2(&stake_pda.data.borrow())?;
    if !account.is_initialized {
        return Err(ProgramError::UninitializedAccount);
    }
    if account.staker != *staker.key {
        return Err(ProgramError::IllegalOwner);
    }

    // M8 fix: reward claims are allowed during cooldown so stakers don't lose accrued rewards.

    // Validate truth mint and token program
    if *truth_mint.key != config.truth_mint {
        return Err(ProgramError::InvalidSeeds);
    }
    if *token_program.key != spl_token::id() {
        return Err(ProgramError::IncorrectProgramId);
    }

    // Synthetix-style reward math
    let pending = (account.amount as u128)
        .checked_mul(
            config
                .reward_per_token_stored
                .checked_sub(account.reward_debt)
                .ok_or(StakingError::Overflow)?,
        )
        .ok_or(StakingError::Overflow)?
        / 1_000_000_000_000_000_000u128; // 1e18

    // Enforce curator yield cap (governable via curator_yield_cap_bps) — requires Clock as 7th account
    let capped_pending = if account.stake_type == StakeType::Curator {
        let clock_info = next_account_info(accounts_iter)?;
        let clock = Clock::from_account_info(clock_info)?;
        let duration = (clock.unix_timestamp - account.staked_at).max(0) as u128;
        let seconds_per_year = 365u128 * 24 * 3600;
        let cap = (account.amount as u128) * config.curator_yield_cap_bps as u128 * duration
            / (10_000u128 * seconds_per_year);
        pending.min(cap)
    } else {
        pending
    };

    if capped_pending > 0 {
        let payout: u64 = capped_pending
            .try_into()
            .map_err(|_| StakingError::Overflow)?;

        // Enforce supply cap before minting
        super::check_and_increment_supply(&mut config, payout)?;

        // Mint rewards to staker
        let mint_ix = spl_token::instruction::mint_to(
            token_program.key,
            truth_mint.key,
            staker_token.key,
            config_pda.key,
            &[],
            payout,
        )?;
        invoke_signed(
            &mint_ix,
            &[
                truth_mint.clone(),
                staker_token.clone(),
                config_pda.clone(),
                token_program.clone(),
            ],
            &[&[CONFIG_SEED, &[config_bump]]],
        )?;
    }

    account.reward_debt = config.reward_per_token_stored;
    borsh::to_writer(&mut &mut stake_pda.data.borrow_mut()[..], &account)?;
    borsh::to_writer(&mut &mut config_pda.data.borrow_mut()[..], &config)?;

    msg!("Claimed {} $TRUTH rewards for {}", capped_pending, domain);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_valid_domain() {
        assert!(validate_entity_ref("example.com").is_ok());
    }

    #[test]
    fn test_validate_valid_entity_ref() {
        assert!(validate_entity_ref("youtube.com/c/3blue1brown").is_ok());
    }

    #[test]
    fn test_validate_valid_entity_with_uppercase_path() {
        // Path allows uppercase (for channel IDs like UC...)
        assert!(validate_entity_ref("youtube.com/channel/UCxyz123ABC").is_ok());
    }

    #[test]
    fn test_validate_valid_entity_with_at_sign() {
        assert!(validate_entity_ref("youtube.com/@handle").is_ok());
    }

    #[test]
    fn test_validate_reject_empty() {
        assert!(validate_entity_ref("").is_err());
    }

    #[test]
    fn test_validate_reject_too_long() {
        let long = format!("{}.com", "a".repeat(250));
        assert!(validate_entity_ref(&long).is_err());
    }

    #[test]
    fn test_validate_reject_no_dot() {
        assert!(validate_entity_ref("localhost").is_err());
    }

    #[test]
    fn test_validate_reject_dot_after_slash() {
        // Dot must appear before first slash
        assert!(validate_entity_ref("nohost/path.ext").is_err());
    }

    #[test]
    fn test_validate_reject_uppercase_in_domain() {
        assert!(validate_entity_ref("Example.com").is_err());
    }

    #[test]
    fn test_validate_reject_leading_dot() {
        assert!(validate_entity_ref(".example.com").is_err());
    }

    #[test]
    fn test_validate_reject_trailing_dot() {
        assert!(validate_entity_ref("example.com.").is_err());
    }

    #[test]
    fn test_validate_reject_leading_dash() {
        assert!(validate_entity_ref("-example.com").is_err());
    }

    #[test]
    fn test_validate_reject_trailing_dash() {
        assert!(validate_entity_ref("example.com-").is_err());
    }

    #[test]
    fn test_validate_reject_invalid_domain_chars() {
        assert!(validate_entity_ref("ex ample.com").is_err());
        assert!(validate_entity_ref("ex<script>.com").is_err());
    }

    #[test]
    fn test_validate_reject_invalid_path_chars() {
        assert!(validate_entity_ref("example.com/path with spaces").is_err());
        assert!(validate_entity_ref("example.com/path<script>").is_err());
    }

    #[test]
    fn test_validate_253_byte_boundary() {
        // Exactly 253 bytes should be ok
        let domain = format!("{}.com", "a".repeat(249));
        assert_eq!(domain.len(), 253);
        assert!(validate_entity_ref(&domain).is_ok());

        // 254 bytes should fail
        let too_long = format!("{}.com", "a".repeat(250));
        assert_eq!(too_long.len(), 254);
        assert!(validate_entity_ref(&too_long).is_err());
    }

    #[test]
    fn test_validate_subdomain() {
        assert!(validate_entity_ref("sub.example.com").is_ok());
    }

    #[test]
    fn test_validate_reddit_entity() {
        assert!(validate_entity_ref("reddit.com/r/rust").is_ok());
    }

    #[test]
    fn test_validate_x_entity() {
        assert!(validate_entity_ref("x.com/elonmusk").is_ok());
    }
}
