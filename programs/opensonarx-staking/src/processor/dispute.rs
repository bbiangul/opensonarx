use super::{deser, deser_config, deser_stake_v2};
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

/// OpenDispute: reporter bonds $TRUTH, creates DisputeAccount.
/// `severity`: 0=low, 1=medium (default), 2=high — affects slash percentage on guilty verdict.
pub fn process_open_dispute(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    domain: &str,
    reason_hash: [u8; 32],
    severity: u8,
) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();
    let reporter = next_account_info(accounts_iter)?;
    let reporter_token = next_account_info(accounts_iter)?;
    let vault_token = next_account_info(accounts_iter)?;
    let dispute_pda = next_account_info(accounts_iter)?;
    let domain_pda = next_account_info(accounts_iter)?;
    let config_pda = next_account_info(accounts_iter)?;
    let token_program = next_account_info(accounts_iter)?;
    let system_program = next_account_info(accounts_iter)?;
    let clock_sysvar = next_account_info(accounts_iter)?;

    if !reporter.is_signer {
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

    // Validate vault and token program
    if *vault_token.key != config.reward_vault {
        return Err(ProgramError::InvalidSeeds);
    }
    if *token_program.key != spl_token::id() {
        return Err(ProgramError::IncorrectProgramId);
    }

    // Verify domain exists
    let (expected_domain, _) =
        Pubkey::find_program_address(&[DOMAIN_SEED, domain.as_bytes()], program_id);
    if *domain_pda.key != expected_domain {
        return Err(ProgramError::InvalidSeeds);
    }
    let mut domain_record = super::deser_domain_record(&domain_pda.data.borrow())?;
    if !domain_record.is_initialized {
        return Err(ProgramError::UninitializedAccount);
    }

    // Fix 2: Only one active dispute per domain at a time
    if domain_record.active_dispute_nonce != u64::MAX {
        return Err(StakingError::DisputeAlreadyActive.into());
    }

    let nonce = config.dispute_nonce;
    let nonce_bytes = nonce.to_le_bytes();

    // Derive dispute PDA
    let (expected_dispute, dispute_bump) =
        Pubkey::find_program_address(&[DISPUTE_SEED, domain.as_bytes(), &nonce_bytes], program_id);
    if *dispute_pda.key != expected_dispute {
        return Err(ProgramError::InvalidSeeds);
    }

    // Optional account 9: sentinel PDA for trusted reporter discount
    let sentinel_pda = next_account_info(accounts_iter).ok();

    let clock = Clock::from_account_info(clock_sysvar)?;
    let rent = Rent::get()?;

    // Trusted reporter discount: 50% bond if sentinel stake >= jury_min_stake * 10
    let mut bond = config.dispute_bond;
    let trusted_threshold = config.jury_min_stake.saturating_mul(10);
    if let Some(sentinel_pda) = sentinel_pda {
        let (expected_sentinel, _) =
            Pubkey::find_program_address(&[SENTINEL_SEED, reporter.key.as_ref()], program_id);
        if *sentinel_pda.key == expected_sentinel
            && sentinel_pda.data_len() > 0
            && sentinel_pda.lamports() > 0
        {
            if let Ok(sentinel) = super::deser::<SentinelAccount>(&sentinel_pda.data.borrow()) {
                if sentinel.is_initialized
                    && sentinel.sentinel == *reporter.key
                    && sentinel.amount >= trusted_threshold
                    && sentinel.cooldown_until == 0
                {
                    bond /= 2;
                    msg!("Trusted reporter discount: bond halved to {}", bond);
                }
            }
        }
    }
    let transfer_ix = spl_token::instruction::transfer(
        token_program.key,
        reporter_token.key,
        vault_token.key,
        reporter.key,
        &[],
        bond,
    )?;
    invoke(
        &transfer_ix,
        &[
            reporter_token.clone(),
            vault_token.clone(),
            reporter.clone(),
            token_program.clone(),
        ],
    )?;

    // Create dispute PDA
    let space = DisputeAccount::MAX_SIZE;
    let lamports = rent.minimum_balance(space);
    invoke_signed(
        &system_instruction::create_account(
            reporter.key,
            dispute_pda.key,
            lamports,
            space as u64,
            program_id,
        ),
        &[
            reporter.clone(),
            dispute_pda.clone(),
            system_program.clone(),
        ],
        &[&[
            DISPUTE_SEED,
            domain.as_bytes(),
            &nonce_bytes,
            &[dispute_bump],
        ]],
    )?;

    let three_days: i64 = 3 * 24 * 3600;
    let seven_days: i64 = 7 * 24 * 3600;
    let dispute = DisputeAccount {
        is_initialized: true,
        reporter: *reporter.key,
        domain: domain.to_string(),
        bond,
        reason_hash,
        status: DisputeStatus::CommitPhase,
        votes_guilty: 0,
        votes_innocent: 0,
        jury_count: 0,
        created_at: clock.unix_timestamp,
        deadline: clock.unix_timestamp + seven_days,
        nonce,
        bump: dispute_bump,
        commit_deadline: clock.unix_timestamp + three_days,
        reveal_deadline: clock.unix_timestamp + seven_days,
        severity: severity.min(2), // clamp to 0-2
        jury_reward_pool: 0,
        jury_reward_claimed: 0,
    };
    borsh::to_writer(&mut &mut dispute_pda.data.borrow_mut()[..], &dispute)?;

    // Mark domain as having an active dispute
    domain_record.active_dispute_nonce = nonce;

    // Realloc domain PDA if pre-upgrade (smaller than current MAX_SIZE)
    let domain_max = DomainRecord::MAX_SIZE;
    if domain_pda.data_len() < domain_max {
        let rent = Rent::get()?;
        let new_lamports = rent.minimum_balance(domain_max);
        let current_lamports = domain_pda.lamports();
        if new_lamports > current_lamports {
            let diff = new_lamports - current_lamports;
            // Fund from reporter
            **reporter.lamports.borrow_mut() = reporter
                .lamports()
                .checked_sub(diff)
                .ok_or(StakingError::Overflow)?;
            **domain_pda.lamports.borrow_mut() = current_lamports
                .checked_add(diff)
                .ok_or(StakingError::Overflow)?;
        }
        domain_pda.realloc(domain_max, false)?;
    }
    borsh::to_writer(&mut &mut domain_pda.data.borrow_mut()[..], &domain_record)?;

    // Increment nonce in config (checked to prevent overflow/collision)
    config.dispute_nonce = nonce.checked_add(1).ok_or(StakingError::Overflow)?;
    borsh::to_writer(&mut &mut config_pda.data.borrow_mut()[..], &config)?;

    msg!("Dispute opened for {} (nonce={})", domain, nonce);
    Ok(())
}

/// CastJuryVote: juror votes on a dispute.
pub fn process_cast_jury_vote(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    domain: &str,
    dispute_nonce: u64,
    verdict_raw: u8,
) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();
    let voter = next_account_info(accounts_iter)?;
    let jury_pda = next_account_info(accounts_iter)?;
    let dispute_pda = next_account_info(accounts_iter)?;
    let voter_stake_pda = next_account_info(accounts_iter)?;
    let config_pda = next_account_info(accounts_iter)?;
    let system_program = next_account_info(accounts_iter)?;
    let clock_sysvar = next_account_info(accounts_iter)?;

    if !voter.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }

    // Validate config PDA
    let (expected_config_key, _) = Pubkey::find_program_address(&[CONFIG_SEED], program_id);
    if *config_pda.key != expected_config_key {
        return Err(ProgramError::InvalidSeeds);
    }

    let config = deser_config(&config_pda.data.borrow())?;

    let verdict = match verdict_raw {
        0 => Verdict::Guilty,
        1 => Verdict::Innocent,
        _ => return Err(ProgramError::InvalidInstructionData),
    };

    // Verify dispute PDA
    let nonce_bytes = dispute_nonce.to_le_bytes();
    let (expected_dispute, _) =
        Pubkey::find_program_address(&[DISPUTE_SEED, domain.as_bytes(), &nonce_bytes], program_id);
    if *dispute_pda.key != expected_dispute {
        return Err(ProgramError::InvalidSeeds);
    }

    let mut dispute = deser::<DisputeAccount>(&dispute_pda.data.borrow())?;
    if !dispute.is_initialized {
        return Err(ProgramError::UninitializedAccount);
    }

    // Plaintext CastJuryVote is only allowed for legacy Active disputes.
    // Disputes in CommitPhase/RevealPhase MUST use the commit-reveal flow
    // (CommitJuryVote + RevealJuryVote) to preserve vote privacy.
    if dispute.status != DisputeStatus::Active {
        return Err(StakingError::CommitPhaseExpired.into());
    }

    let clock = Clock::from_account_info(clock_sysvar)?;
    if clock.unix_timestamp > dispute.deadline {
        return Err(StakingError::VotingPeriodEnded.into());
    }

    // Read voter's stake to check eligibility and get weight
    let voter_stake = deser_stake_v2(&voter_stake_pda.data.borrow())?;
    if !voter_stake.is_initialized || voter_stake.staker != *voter.key {
        return Err(StakingError::JuryNotEligible.into());
    }
    if voter_stake.amount < config.jury_min_stake {
        return Err(StakingError::JuryNotEligible.into());
    }
    // Fix 6: Minimum 7-day stake age for jury eligibility
    if clock.unix_timestamp - voter_stake.staked_at < 7 * 24 * 3600 {
        return Err(StakingError::JuryNotEligible.into());
    }
    // Conflict of interest: voter must not be staked on disputed domain
    if voter_stake.domain == domain {
        return Err(StakingError::ConflictOfInterest.into());
    }

    // Voter must not be in cooldown (unstaking)
    if voter_stake.cooldown_until != 0 {
        return Err(StakingError::JuryNotEligible.into());
    }

    // Validate stake PDA is owned by this program
    if voter_stake_pda.owner != program_id {
        return Err(ProgramError::IllegalOwner);
    }

    // Derive jury vote PDA: ["jury", dispute_pda, voter]
    let (expected_jury, jury_bump) = Pubkey::find_program_address(
        &[JURY_SEED, dispute_pda.key.as_ref(), voter.key.as_ref()],
        program_id,
    );
    if *jury_pda.key != expected_jury {
        return Err(ProgramError::InvalidSeeds);
    }

    // Check not already voted (account should not exist)
    if jury_pda.data_len() > 0 && jury_pda.lamports() > 0 {
        let existing = deser::<JuryVote>(&jury_pda.data.borrow());
        if let Ok(v) = existing {
            if v.is_initialized {
                return Err(StakingError::AlreadyVoted.into());
            }
        }
    }

    // Create jury vote PDA
    let rent = Rent::get()?;
    let space = JuryVote::MAX_SIZE;
    let lamports = rent.minimum_balance(space);
    invoke_signed(
        &system_instruction::create_account(
            voter.key,
            jury_pda.key,
            lamports,
            space as u64,
            program_id,
        ),
        &[voter.clone(), jury_pda.clone(), system_program.clone()],
        &[&[
            JURY_SEED,
            dispute_pda.key.as_ref(),
            voter.key.as_ref(),
            &[jury_bump],
        ]],
    )?;

    let weight = voter_stake.amount;
    let jury_vote = JuryVote {
        is_initialized: true,
        voter: *voter.key,
        verdict,
        weight,
        voted_at: clock.unix_timestamp,
        bump: jury_bump,
        commit_hash: [0u8; 32],
        reward_claimed: false,
    };
    borsh::to_writer(&mut &mut jury_pda.data.borrow_mut()[..], &jury_vote)?;

    // Update dispute tallies
    match verdict {
        Verdict::Guilty => dispute.votes_guilty = dispute.votes_guilty.saturating_add(weight),
        Verdict::Innocent => dispute.votes_innocent = dispute.votes_innocent.saturating_add(weight),
    }
    dispute.jury_count += 1;
    borsh::to_writer(&mut &mut dispute_pda.data.borrow_mut()[..], &dispute)?;

    msg!("Jury vote cast: {:?} with weight {}", verdict, weight);
    Ok(())
}

/// ResolveDispute: after deadline, tally votes and slash or return bond.
pub fn process_resolve_dispute(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    domain: &str,
    dispute_nonce: u64,
) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();
    let _caller = next_account_info(accounts_iter)?;
    let dispute_pda = next_account_info(accounts_iter)?;
    let domain_pda = next_account_info(accounts_iter)?;
    let reporter_token = next_account_info(accounts_iter)?;
    let vault_token = next_account_info(accounts_iter)?;
    let _truth_mint = next_account_info(accounts_iter)?;
    let config_pda = next_account_info(accounts_iter)?;
    let token_program = next_account_info(accounts_iter)?;
    let clock_sysvar = next_account_info(accounts_iter)?;

    // Validate config PDA
    let (expected_resolve_config, config_bump) =
        Pubkey::find_program_address(&[CONFIG_SEED], program_id);
    if *config_pda.key != expected_resolve_config {
        return Err(ProgramError::InvalidSeeds);
    }

    let config = deser_config(&config_pda.data.borrow())?;

    // Validate vault and token program
    if *vault_token.key != config.reward_vault {
        return Err(ProgramError::InvalidSeeds);
    }
    if *token_program.key != spl_token::id() {
        return Err(ProgramError::IncorrectProgramId);
    }

    // Verify dispute PDA
    let nonce_bytes = dispute_nonce.to_le_bytes();
    let (expected_dispute, _) =
        Pubkey::find_program_address(&[DISPUTE_SEED, domain.as_bytes(), &nonce_bytes], program_id);
    if *dispute_pda.key != expected_dispute {
        return Err(ProgramError::InvalidSeeds);
    }

    let mut dispute = super::deser_dispute_account(&dispute_pda.data.borrow())?;
    if !dispute.is_initialized {
        return Err(ProgramError::UninitializedAccount);
    }
    // Only unresolved disputes can be resolved — guard against re-resolution
    if dispute.status != DisputeStatus::Active
        && dispute.status != DisputeStatus::CommitPhase
        && dispute.status != DisputeStatus::RevealPhase
    {
        return Err(StakingError::DisputeNotResolved.into());
    }

    let clock = Clock::from_account_info(clock_sysvar)?;
    if clock.unix_timestamp < dispute.deadline {
        return Err(StakingError::DisputeDeadlineNotReached.into());
    }

    // Helper: clear domain's active_dispute_nonce and write
    let clear_domain_dispute = |domain_pda: &AccountInfo| -> ProgramResult {
        let (expected_domain, _) =
            Pubkey::find_program_address(&[DOMAIN_SEED, domain.as_bytes()], program_id);
        if *domain_pda.key == expected_domain {
            let record_opt = {
                let data = domain_pda.data.borrow();
                super::deser_domain_record(&data).ok()
            };
            if let Some(mut record) = record_opt {
                if record.is_initialized {
                    record.active_dispute_nonce = u64::MAX;
                    // Realloc domain PDA if pre-upgrade
                    let domain_max = DomainRecord::MAX_SIZE;
                    if domain_pda.data_len() < domain_max {
                        domain_pda.realloc(domain_max, false)?;
                    }
                    borsh::to_writer(&mut &mut domain_pda.data.borrow_mut()[..], &record)?;
                }
            }
        }
        Ok(())
    };

    // Check dispute jury quorum (min_jury_weight)
    let total_jury_weight = dispute.votes_guilty.saturating_add(dispute.votes_innocent);
    if total_jury_weight < config.min_jury_weight {
        // Quorum not met — dispute fails, bond returned to reporter
        dispute.status = DisputeStatus::QuorumFailed;

        if dispute.bond > 0 {
            let transfer_ix = spl_token::instruction::transfer(
                token_program.key,
                vault_token.key,
                reporter_token.key,
                config_pda.key,
                &[],
                dispute.bond,
            )?;
            invoke_signed(
                &transfer_ix,
                &[
                    vault_token.clone(),
                    reporter_token.clone(),
                    config_pda.clone(),
                    token_program.clone(),
                ],
                &[&[CONFIG_SEED, &[config_bump]]],
            )?;
        }

        // Realloc dispute PDA if pre-upgrade
        let dispute_max = DisputeAccount::MAX_SIZE;
        if dispute_pda.data_len() < dispute_max {
            dispute_pda.realloc(dispute_max, false)?;
        }
        borsh::to_writer(&mut &mut dispute_pda.data.borrow_mut()[..], &dispute)?;
        clear_domain_dispute(domain_pda)?;
        msg!(
            "Dispute quorum not met ({} < {}), bond returned to reporter",
            total_jury_weight,
            config.min_jury_weight
        );
        return Ok(());
    }

    // Determine outcome
    // Design note (Issue #9): Tied votes favor innocence — presumption of innocence.
    let guilty = dispute.votes_guilty > dispute.votes_innocent;

    if guilty {
        dispute.status = DisputeStatus::ResolvedGuilty;

        // Reporter gets full bond back (no jury allocation from bond)
        dispute.jury_reward_pool = 0;
        dispute.jury_reward_claimed = 0;

        if dispute.bond > 0 {
            let transfer_ix = spl_token::instruction::transfer(
                token_program.key,
                vault_token.key,
                reporter_token.key,
                config_pda.key,
                &[],
                dispute.bond,
            )?;
            invoke_signed(
                &transfer_ix,
                &[
                    vault_token.clone(),
                    reporter_token.clone(),
                    config_pda.clone(),
                    token_program.clone(),
                ],
                &[&[CONFIG_SEED, &[config_bump]]],
            )?;
        }

        // Compute effective slash based on severity:
        //   0 (low):    slash_pct / 2
        //   1 (medium): slash_pct (base rate)
        //   2 (high):   min(slash_pct * 3 / 2, 100)
        let base = config.slash_pct as u16;
        let effective = match dispute.severity {
            0 => (base / 2) as u8,
            2 => (base * 3 / 2).min(100) as u8,
            _ => config.slash_pct, // severity 1 = base rate
        };

        // Blacklist domain — slashing applied lazily on withdraw using effective_slash_pct
        let (expected_domain, _) =
            Pubkey::find_program_address(&[DOMAIN_SEED, domain.as_bytes()], program_id);
        if *domain_pda.key == expected_domain {
            let record_opt = {
                let data = domain_pda.data.borrow();
                super::deser_domain_record(&data).ok()
            };
            if let Some(mut record) = record_opt {
                if record.is_initialized {
                    record.blacklisted = true;
                    record.effective_slash_pct = record.effective_slash_pct.max(effective);
                    record.active_dispute_nonce = u64::MAX;
                    // Realloc domain PDA if pre-upgrade
                    let domain_max = DomainRecord::MAX_SIZE;
                    if domain_pda.data_len() < domain_max {
                        domain_pda.realloc(domain_max, false)?;
                    }
                    borsh::to_writer(&mut &mut domain_pda.data.borrow_mut()[..], &record)?;
                }
            }
        }

        msg!("Dispute resolved: GUILTY for {}. Domain blacklisted, effective slash {}% (severity={}). Full bond returned to reporter.", domain, effective, dispute.severity);
    } else {
        dispute.status = DisputeStatus::ResolvedInnocent;
        // Bond stays in vault as protocol revenue (no jury allocation)
        dispute.jury_reward_pool = 0;
        dispute.jury_reward_claimed = 0;
        clear_domain_dispute(domain_pda)?;
        msg!(
            "Dispute resolved: INNOCENT for {}. Bond stays in vault as protocol revenue.",
            domain
        );
    }

    // Realloc dispute PDA if pre-upgrade
    let dispute_max = DisputeAccount::MAX_SIZE;
    if dispute_pda.data_len() < dispute_max {
        dispute_pda.realloc(dispute_max, false)?;
    }
    borsh::to_writer(&mut &mut dispute_pda.data.borrow_mut()[..], &dispute)?;

    Ok(())
}
