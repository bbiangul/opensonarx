use super::deser;
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    clock::Clock,
    entrypoint::ProgramResult,
    hash::hashv,
    msg,
    program_error::ProgramError,
    pubkey::Pubkey,
    sysvar::Sysvar,
};

use crate::error::StakingError;
use crate::state::*;

/// RevealJuryVote: juror reveals their previously committed vote.
pub fn process_reveal_jury_vote(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    domain: &str,
    dispute_nonce: u64,
    verdict_raw: u8,
    salt: [u8; 32],
) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();
    let voter = next_account_info(accounts_iter)?;
    let jury_pda = next_account_info(accounts_iter)?;
    let dispute_pda = next_account_info(accounts_iter)?;
    let _config_pda = next_account_info(accounts_iter)?;
    let clock_sysvar = next_account_info(accounts_iter)?;

    if !voter.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
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

    // Must be in RevealPhase
    if dispute.status != DisputeStatus::RevealPhase {
        // Auto-transition: if we're in CommitPhase but commit deadline has passed, move to RevealPhase
        let clock = Clock::from_account_info(clock_sysvar)?;
        if (dispute.status == DisputeStatus::CommitPhase || dispute.status == DisputeStatus::Active)
            && clock.unix_timestamp >= dispute.commit_deadline
        {
            dispute.status = DisputeStatus::RevealPhase;
        } else {
            return Err(StakingError::RevealPhasePending.into());
        }
    }

    let clock = Clock::from_account_info(clock_sysvar)?;

    // Check commit phase is over
    if clock.unix_timestamp < dispute.commit_deadline {
        return Err(StakingError::PrematureReveal.into());
    }

    // Check reveal deadline not passed
    if clock.unix_timestamp >= dispute.reveal_deadline {
        return Err(StakingError::VotingPeriodEnded.into());
    }

    // Verify jury vote PDA
    let (expected_jury, _) = Pubkey::find_program_address(
        &[JURY_SEED, dispute_pda.key.as_ref(), voter.key.as_ref()],
        program_id,
    );
    if *jury_pda.key != expected_jury {
        return Err(ProgramError::InvalidSeeds);
    }

    let mut jury_vote = deser::<JuryVote>(&jury_pda.data.borrow())?;
    if !jury_vote.is_initialized {
        return Err(ProgramError::UninitializedAccount);
    }
    if jury_vote.voter != *voter.key {
        return Err(ProgramError::IllegalOwner);
    }

    // Prevent double-reveal: if voted_at is already set, this vote was already revealed
    if jury_vote.voted_at != 0 {
        return Err(StakingError::AlreadyVoted.into());
    }

    // Compute expected hash: SHA256(verdict_raw || salt)
    let expected_hash = hashv(&[&[verdict_raw], &salt]);
    if expected_hash.to_bytes() != jury_vote.commit_hash {
        return Err(StakingError::InvalidCommitHash.into());
    }

    // Parse verdict
    let verdict = match verdict_raw {
        0 => Verdict::Guilty,
        1 => Verdict::Innocent,
        _ => return Err(ProgramError::InvalidInstructionData),
    };

    // Update jury vote with revealed verdict
    jury_vote.verdict = verdict;
    jury_vote.voted_at = clock.unix_timestamp;
    borsh::to_writer(&mut &mut jury_pda.data.borrow_mut()[..], &jury_vote)?;

    // Update dispute tallies based on weight
    let weight = jury_vote.weight;
    match verdict {
        Verdict::Guilty => dispute.votes_guilty = dispute.votes_guilty.saturating_add(weight),
        Verdict::Innocent => dispute.votes_innocent = dispute.votes_innocent.saturating_add(weight),
    }
    borsh::to_writer(&mut &mut dispute_pda.data.borrow_mut()[..], &dispute)?;

    msg!(
        "Jury vote revealed: {:?} with weight {} for dispute nonce={}",
        verdict,
        weight,
        dispute_nonce,
    );
    Ok(())
}
