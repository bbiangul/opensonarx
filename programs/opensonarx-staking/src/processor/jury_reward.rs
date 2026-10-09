use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    msg,
    program::invoke_signed,
    program_error::ProgramError,
    pubkey::Pubkey,
};

use crate::error::StakingError;
use crate::state::*;

/// ClaimJuryReward: winning-side juror claims a flat jury_vote_reward minted from supply.
///
/// Accounts:
///   0. `[signer]` Voter
///   1. `[writable]` JuryVote PDA
///   2. `[writable]` DisputeAccount PDA
///   3. `[writable]` Voter's token account
///   4. `[writable]` $TRUTH Mint
///   5. `[writable]` Config PDA (total_minted updated)
///   6. `[]` Token Program
pub fn process_claim_jury_reward(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    domain: &str,
    dispute_nonce: u64,
) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();
    let voter = next_account_info(accounts_iter)?;
    let jury_pda = next_account_info(accounts_iter)?;
    let dispute_pda = next_account_info(accounts_iter)?;
    let voter_token = next_account_info(accounts_iter)?;
    let truth_mint = next_account_info(accounts_iter)?;
    let config_pda = next_account_info(accounts_iter)?;
    let token_program = next_account_info(accounts_iter)?;

    if !voter.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }

    // Validate config PDA
    let (expected_config, config_bump) = Pubkey::find_program_address(&[CONFIG_SEED], program_id);
    if *config_pda.key != expected_config {
        return Err(ProgramError::InvalidSeeds);
    }

    let mut config = super::deser_config(&config_pda.data.borrow())?;

    // Validate truth mint
    if *truth_mint.key != config.truth_mint {
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

    let dispute = super::deser_dispute_account(&dispute_pda.data.borrow())?;
    if !dispute.is_initialized {
        return Err(ProgramError::UninitializedAccount);
    }

    // Must be resolved (Guilty or Innocent)
    let winning_verdict = match dispute.status {
        DisputeStatus::ResolvedGuilty => Verdict::Guilty,
        DisputeStatus::ResolvedInnocent => Verdict::Innocent,
        _ => return Err(StakingError::DisputeNotResolved.into()),
    };

    // Verify jury vote PDA
    let (expected_jury, _) = Pubkey::find_program_address(
        &[JURY_SEED, dispute_pda.key.as_ref(), voter.key.as_ref()],
        program_id,
    );
    if *jury_pda.key != expected_jury {
        return Err(ProgramError::InvalidSeeds);
    }

    let mut jury_vote = super::deser_jury_vote(&jury_pda.data.borrow())?;
    if !jury_vote.is_initialized {
        return Err(ProgramError::UninitializedAccount);
    }
    if jury_vote.voter != *voter.key {
        return Err(ProgramError::IllegalOwner);
    }

    // Must have revealed (voted_at != 0)
    if jury_vote.voted_at == 0 {
        return Err(StakingError::NoJuryReward.into());
    }

    // Must not have already claimed
    if jury_vote.reward_claimed {
        return Err(StakingError::JuryRewardAlreadyClaimed.into());
    }

    // Must be on winning side
    if jury_vote.verdict != winning_verdict {
        return Err(StakingError::NoJuryReward.into());
    }

    // Flat fee per winning juror (minted from supply)
    let payout = config.jury_vote_reward;
    if payout == 0 {
        return Err(StakingError::NoJuryReward.into());
    }

    // Enforce per-epoch jury reward budget (0 = uncapped for backward compat)
    if config.max_jury_rewards_per_epoch > 0 {
        let new_epoch_total = config.jury_rewards_this_epoch.saturating_add(payout);
        if new_epoch_total > config.max_jury_rewards_per_epoch {
            return Err(StakingError::JuryRewardBudgetExhausted.into());
        }
        config.jury_rewards_this_epoch = new_epoch_total;
    }

    // Check and increment supply cap
    super::check_and_increment_supply(&mut config, payout)?;

    // Mint flat reward to voter's token account
    let mint_ix = spl_token::instruction::mint_to(
        token_program.key,
        truth_mint.key,
        voter_token.key,
        &expected_config,
        &[],
        payout,
    )?;
    invoke_signed(
        &mint_ix,
        &[
            truth_mint.clone(),
            voter_token.clone(),
            config_pda.clone(),
            token_program.clone(),
        ],
        &[&[CONFIG_SEED, &[config_bump]]],
    )?;

    // Write updated config (total_minted incremented)
    // Realloc config PDA if pre-upgrade
    let config_max = ProtocolConfig::MAX_SIZE;
    if config_pda.data_len() < config_max {
        config_pda.realloc(config_max, false)?;
    }
    borsh::to_writer(&mut &mut config_pda.data.borrow_mut()[..], &config)?;

    // Mark as claimed
    jury_vote.reward_claimed = true;
    // Realloc jury vote PDA if pre-upgrade
    let jury_max = JuryVote::MAX_SIZE;
    if jury_pda.data_len() < jury_max {
        jury_pda.realloc(jury_max, false)?;
    }
    borsh::to_writer(&mut &mut jury_pda.data.borrow_mut()[..], &jury_vote)?;

    msg!(
        "Jury reward claimed: {} $TRUTH minted for voter {}",
        payout,
        voter.key
    );
    Ok(())
}
