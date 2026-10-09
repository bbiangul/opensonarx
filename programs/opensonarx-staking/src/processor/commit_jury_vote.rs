use super::{deser, deser_config, deser_stake_v2};
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    clock::Clock,
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

/// CommitJuryVote: juror commits a hidden vote hash during the commit phase.
pub fn process_commit_jury_vote(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    domain: &str,
    dispute_nonce: u64,
    commit_hash: [u8; 32],
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
    let (expected_config, _) = Pubkey::find_program_address(&[CONFIG_SEED], program_id);
    if *config_pda.key != expected_config {
        return Err(ProgramError::InvalidSeeds);
    }

    let config = deser_config(&config_pda.data.borrow())?;
    if !config.is_initialized {
        return Err(ProgramError::UninitializedAccount);
    }

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

    // Accept Active (legacy transition) or CommitPhase
    if dispute.status != DisputeStatus::CommitPhase && dispute.status != DisputeStatus::Active {
        return Err(StakingError::CommitPhaseExpired.into());
    }

    let clock = Clock::from_account_info(clock_sysvar)?;

    // Check commit deadline
    if clock.unix_timestamp >= dispute.commit_deadline {
        return Err(StakingError::CommitPhaseExpired.into());
    }

    // Check voter eligibility via stake
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

    // Check not already committed (account should not exist)
    if jury_pda.data_len() > 0 && jury_pda.lamports() > 0 {
        let existing = deser::<JuryVote>(&jury_pda.data.borrow());
        if let Ok(v) = existing {
            if v.is_initialized {
                return Err(StakingError::CommitAlreadySubmitted.into());
            }
        }
    }

    // Create jury vote PDA with commit hash stored
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
        verdict: Verdict::Guilty, // placeholder, updated on reveal
        weight,
        voted_at: 0, // set on reveal
        bump: jury_bump,
        commit_hash,
        reward_claimed: false,
    };
    borsh::to_writer(&mut &mut jury_pda.data.borrow_mut()[..], &jury_vote)?;

    // Increment jury count
    dispute.jury_count += 1;
    borsh::to_writer(&mut &mut dispute_pda.data.borrow_mut()[..], &dispute)?;

    msg!("Jury commit submitted for dispute nonce={}", dispute_nonce);
    Ok(())
}
