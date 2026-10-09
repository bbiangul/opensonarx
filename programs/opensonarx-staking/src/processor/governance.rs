use super::{deser, deser_config, deser_gov_lock, deser_proposal, deser_stake_v2};
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    clock::Clock,
    entrypoint::ProgramResult,
    msg,
    program::{invoke, invoke_signed},
    program_error::ProgramError,
    pubkey::Pubkey,
    rent::Rent,
    system_instruction,
    sysvar::Sysvar,
};

use crate::error::StakingError;
use crate::state::*;

/// CreateProposal: staker proposes a protocol parameter change.
pub fn process_create_proposal(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    description_hash: [u8; 32],
    param_key: &str,
    param_value: u64,
) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();
    let proposer = next_account_info(accounts_iter)?;
    let proposal_pda = next_account_info(accounts_iter)?;
    let config_pda = next_account_info(accounts_iter)?;
    let system_program = next_account_info(accounts_iter)?;
    let clock_sysvar = next_account_info(accounts_iter)?;

    if !proposer.is_signer {
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

    let nonce = config.proposal_nonce;
    let nonce_bytes = nonce.to_le_bytes();

    // Derive proposal PDA
    let (expected_proposal, proposal_bump) =
        Pubkey::find_program_address(&[PROPOSAL_SEED, &nonce_bytes], program_id);
    if *proposal_pda.key != expected_proposal {
        return Err(ProgramError::InvalidSeeds);
    }

    let clock = Clock::from_account_info(clock_sysvar)?;
    let rent = Rent::get()?;

    // Create proposal account
    let space = GovernanceProposal::MAX_SIZE;
    let lamports = rent.minimum_balance(space);
    invoke_signed(
        &system_instruction::create_account(
            proposer.key,
            proposal_pda.key,
            lamports,
            space as u64,
            program_id,
        ),
        &[
            proposer.clone(),
            proposal_pda.clone(),
            system_program.clone(),
        ],
        &[&[PROPOSAL_SEED, &nonce_bytes, &[proposal_bump]]],
    )?;

    let one_day: i64 = 24 * 60 * 60;
    let voting_ends = clock.unix_timestamp + one_day;

    // M4 fix: track deposit amount and depositor token account for refund
    let (deposit_amount, depositor_token_key) = if config.proposal_deposit > 0 {
        let proposer_token = next_account_info(accounts_iter)?;
        let vault_token = next_account_info(accounts_iter)?;
        let token_program = next_account_info(accounts_iter)?;

        if *vault_token.key != config.reward_vault {
            return Err(ProgramError::InvalidSeeds);
        }
        if *token_program.key != spl_token::id() {
            return Err(ProgramError::IncorrectProgramId);
        }

        let transfer_ix = spl_token::instruction::transfer(
            token_program.key,
            proposer_token.key,
            vault_token.key,
            proposer.key,
            &[],
            config.proposal_deposit,
        )?;
        invoke(
            &transfer_ix,
            &[
                proposer_token.clone(),
                vault_token.clone(),
                proposer.clone(),
                token_program.clone(),
            ],
        )?;
        msg!(
            "Proposal deposit collected: {} $TRUTH",
            config.proposal_deposit
        );
        (config.proposal_deposit, *proposer_token.key)
    } else {
        (0u64, Pubkey::default())
    };

    let proposal = GovernanceProposal {
        is_initialized: true,
        proposer: *proposer.key,
        description_hash,
        param_key: param_key.to_string(),
        param_value,
        votes_for: 0,
        votes_against: 0,
        status: ProposalStatus::Active,
        created_at: clock.unix_timestamp,
        voting_ends,
        nonce,
        bump: proposal_bump,
        executable_after: voting_ends + config.execution_delay_secs,
        deposit_amount,
        depositor_token: depositor_token_key,
    };
    borsh::to_writer(&mut &mut proposal_pda.data.borrow_mut()[..], &proposal)?;

    // Increment proposal nonce (checked to prevent overflow/collision)
    config.proposal_nonce = nonce.checked_add(1).ok_or(StakingError::Overflow)?;
    borsh::to_writer(&mut &mut config_pda.data.borrow_mut()[..], &config)?;

    msg!(
        "Proposal created: {} = {} (nonce={})",
        param_key,
        param_value,
        nonce
    );
    Ok(())
}

/// CastGovernanceVote: staker votes on a proposal.
pub fn process_cast_governance_vote(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    proposal_nonce: u64,
    vote_raw: u8,
) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();
    let voter = next_account_info(accounts_iter)?;
    let gov_vote_pda = next_account_info(accounts_iter)?;
    let proposal_pda = next_account_info(accounts_iter)?;
    let voter_stake_pda = next_account_info(accounts_iter)?;
    let system_program = next_account_info(accounts_iter)?;
    let clock_sysvar = next_account_info(accounts_iter)?;
    let gov_lock_pda = next_account_info(accounts_iter)?;

    if !voter.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }

    let vote_choice = match vote_raw {
        0 => GovernanceVoteChoice::For,
        1 => GovernanceVoteChoice::Against,
        _ => return Err(ProgramError::InvalidInstructionData),
    };

    // Verify proposal PDA
    let nonce_bytes = proposal_nonce.to_le_bytes();
    let (expected_proposal, _) =
        Pubkey::find_program_address(&[PROPOSAL_SEED, &nonce_bytes], program_id);
    if *proposal_pda.key != expected_proposal {
        return Err(ProgramError::InvalidSeeds);
    }

    let mut proposal = deser_proposal(&proposal_pda.data.borrow())?;
    if !proposal.is_initialized || proposal.status != ProposalStatus::Active {
        return Err(StakingError::VotingPeriodEnded.into());
    }

    let clock = Clock::from_account_info(clock_sysvar)?;
    if clock.unix_timestamp > proposal.voting_ends {
        return Err(StakingError::VotingPeriodEnded.into());
    }

    // Get voter's stake weight (read-only — no longer written)
    let voter_stake = deser_stake_v2(&voter_stake_pda.data.borrow())?;
    if !voter_stake.is_initialized || voter_stake.staker != *voter.key {
        return Err(StakingError::Unauthorized.into());
    }
    let weight = voter_stake.amount;

    // Voter must not be in cooldown (unstaking)
    if voter_stake.cooldown_until != 0 {
        return Err(StakingError::JuryNotEligible.into());
    }

    // Minimum 7-day stake age for governance voting (prevents flash-loan governance attacks)
    if clock.unix_timestamp - voter_stake.staked_at < 7 * 24 * 3600 {
        return Err(StakingError::StakeTooYoungForGovernance.into());
    }
    // Validate stake PDA is owned by this program
    if voter_stake_pda.owner != program_id {
        return Err(ProgramError::IllegalOwner);
    }

    // Derive gov vote PDA: ["gov_vote", proposal_pda, voter]
    let (expected_vote, vote_bump) = Pubkey::find_program_address(
        &[GOV_VOTE_SEED, proposal_pda.key.as_ref(), voter.key.as_ref()],
        program_id,
    );
    if *gov_vote_pda.key != expected_vote {
        return Err(ProgramError::InvalidSeeds);
    }

    // Check not already voted
    if gov_vote_pda.data_len() > 0 && gov_vote_pda.lamports() > 0 {
        let existing = deser::<GovernanceVote>(&gov_vote_pda.data.borrow());
        if let Ok(v) = existing {
            if v.is_initialized {
                return Err(StakingError::AlreadyVoted.into());
            }
        }
    }

    // Create vote PDA
    let rent = Rent::get()?;
    let space = GovernanceVote::MAX_SIZE;
    let lamports = rent.minimum_balance(space);
    invoke_signed(
        &system_instruction::create_account(
            voter.key,
            gov_vote_pda.key,
            lamports,
            space as u64,
            program_id,
        ),
        &[voter.clone(), gov_vote_pda.clone(), system_program.clone()],
        &[&[
            GOV_VOTE_SEED,
            proposal_pda.key.as_ref(),
            voter.key.as_ref(),
            &[vote_bump],
        ]],
    )?;

    let gov_vote = GovernanceVote {
        is_initialized: true,
        voter: *voter.key,
        vote: vote_choice,
        weight,
        bump: vote_bump,
    };
    borsh::to_writer(&mut &mut gov_vote_pda.data.borrow_mut()[..], &gov_vote)?;

    // Update proposal tallies
    match vote_choice {
        GovernanceVoteChoice::For => proposal.votes_for = proposal.votes_for.saturating_add(weight),
        GovernanceVoteChoice::Against => {
            proposal.votes_against = proposal.votes_against.saturating_add(weight)
        }
    }
    borsh::to_writer(&mut &mut proposal_pda.data.borrow_mut()[..], &proposal)?;

    // S2 fix: per-wallet governance lock — lock the wallet (not just this stake) until voting_ends
    let (expected_gov_lock, gov_lock_bump) =
        Pubkey::find_program_address(&[GOV_LOCK_SEED, voter.key.as_ref()], program_id);
    if *gov_lock_pda.key != expected_gov_lock {
        return Err(ProgramError::InvalidSeeds);
    }

    if gov_lock_pda.data_len() == 0 || gov_lock_pda.lamports() == 0 {
        // Create GovernanceLockAccount
        let lock_space = GovernanceLockAccount::MAX_SIZE;
        let lock_lamports = rent.minimum_balance(lock_space);
        invoke_signed(
            &system_instruction::create_account(
                voter.key,
                gov_lock_pda.key,
                lock_lamports,
                lock_space as u64,
                program_id,
            ),
            &[voter.clone(), gov_lock_pda.clone(), system_program.clone()],
            &[&[GOV_LOCK_SEED, voter.key.as_ref(), &[gov_lock_bump]]],
        )?;
        let lock_account = GovernanceLockAccount {
            is_initialized: true,
            wallet: *voter.key,
            lock_until: proposal.voting_ends,
            bump: gov_lock_bump,
        };
        borsh::to_writer(&mut &mut gov_lock_pda.data.borrow_mut()[..], &lock_account)?;
    } else {
        // Update existing lock — extend to max(current, voting_ends)
        let mut lock_account = deser_gov_lock(&gov_lock_pda.data.borrow())?;
        lock_account.lock_until = lock_account.lock_until.max(proposal.voting_ends);
        borsh::to_writer(&mut &mut gov_lock_pda.data.borrow_mut()[..], &lock_account)?;
    }

    msg!(
        "Governance vote cast: {:?} with weight {}",
        vote_choice,
        weight
    );
    Ok(())
}

/// ExecuteProposal: after voting period, apply the result.
pub fn process_execute_proposal(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    proposal_nonce: u64,
) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();
    let _caller = next_account_info(accounts_iter)?;
    let proposal_pda = next_account_info(accounts_iter)?;
    let config_pda = next_account_info(accounts_iter)?;
    let clock_sysvar = next_account_info(accounts_iter)?;

    // Verify proposal PDA
    let nonce_bytes = proposal_nonce.to_le_bytes();
    let (expected_proposal, _) =
        Pubkey::find_program_address(&[PROPOSAL_SEED, &nonce_bytes], program_id);
    if *proposal_pda.key != expected_proposal {
        return Err(ProgramError::InvalidSeeds);
    }

    let mut proposal = deser_proposal(&proposal_pda.data.borrow())?;
    if !proposal.is_initialized || proposal.status != ProposalStatus::Active {
        return Err(ProgramError::InvalidAccountData);
    }

    let clock = Clock::from_account_info(clock_sysvar)?;
    if clock.unix_timestamp < proposal.voting_ends {
        return Err(StakingError::VotingPeriodActive.into());
    }

    // Enforce governance timelock — must wait until executable_after
    if clock.unix_timestamp < proposal.executable_after {
        return Err(StakingError::TimelockActive.into());
    }

    let (expected_config, _) = Pubkey::find_program_address(&[CONFIG_SEED], program_id);
    if *config_pda.key != expected_config {
        return Err(ProgramError::InvalidSeeds);
    }

    let mut config = deser_config(&config_pda.data.borrow())?;

    // Quorum check: votes_for + votes_against > 10% of total_staked
    let total_votes = proposal.votes_for.saturating_add(proposal.votes_against);
    let quorum_threshold = (config.total_staked / 10).max(config.min_stake); // 10% of total staked, with min_stake floor
    let quorum_met = total_votes >= quorum_threshold;

    if !quorum_met {
        proposal.status = ProposalStatus::Failed;
        borsh::to_writer(&mut &mut proposal_pda.data.borrow_mut()[..], &proposal)?;
        msg!(
            "Proposal failed: quorum not met ({} < {})",
            total_votes,
            quorum_threshold
        );
        return Ok(());
    }

    // Majority check
    if proposal.votes_for > proposal.votes_against {
        // Apply parameter change
        match proposal.param_key.as_str() {
            "min_stake" => {
                if proposal.param_value > 1_000_000_000 {
                    // max 1B tokens
                    return Err(StakingError::ParamOutOfRange.into());
                }
                config.min_stake = proposal.param_value;
            }
            "cooldown_secs" => {
                if proposal.param_value > 31_536_000 {
                    // max 365 days
                    return Err(StakingError::ParamOutOfRange.into());
                }
                config.cooldown_secs = proposal.param_value as i64;
            }
            "dispute_bond" => {
                if proposal.param_value > 1_000_000_000 {
                    // max 1B tokens
                    return Err(StakingError::ParamOutOfRange.into());
                }
                config.dispute_bond = proposal.param_value;
            }
            "slash_pct" => {
                if proposal.param_value < 5 || proposal.param_value > 100 {
                    return Err(StakingError::ParamOutOfRange.into());
                }
                config.slash_pct = proposal.param_value as u8;
            }
            "reward_rate" => {
                if proposal.param_value > 1_000_000_000 {
                    // max 1B tokens
                    return Err(StakingError::ParamOutOfRange.into());
                }
                config.reward_rate = proposal.param_value;
            }
            "jury_min_stake" => {
                if proposal.param_value > 1_000_000_000 {
                    // max 1B tokens
                    return Err(StakingError::ParamOutOfRange.into());
                }
                config.jury_min_stake = proposal.param_value;
            }
            "burn_pct" => {
                if proposal.param_value > 100
                    || (proposal.param_value as u8)
                        .checked_add(config.sentinel_fee_pct)
                        .is_none_or(|sum| sum > 100)
                {
                    return Err(StakingError::ParamOutOfRange.into());
                }
                config.burn_pct = proposal.param_value as u8;
            }
            "staker_emission_pct" => {
                if proposal.param_value > 100 {
                    return Err(StakingError::ParamOutOfRange.into());
                }
                config.staker_emission_pct = proposal.param_value as u8;
            }
            "min_epoch_duration" => {
                if proposal.param_value > 31_536_000 {
                    // max 365 days
                    return Err(StakingError::ParamOutOfRange.into());
                }
                config.min_epoch_duration = proposal.param_value as i64;
            }
            "max_total_supply" => {
                if proposal.param_value < config.total_minted {
                    proposal.status = ProposalStatus::Failed;
                    borsh::to_writer(&mut &mut proposal_pda.data.borrow_mut()[..], &proposal)?;
                    msg!(
                        "Proposal failed: max_total_supply {} < total_minted {}",
                        proposal.param_value,
                        config.total_minted
                    );
                    return Ok(());
                }
                config.max_total_supply = proposal.param_value;
            }
            "min_jury_weight" => {
                if proposal.param_value > 10_000_000_000 {
                    // max 10B
                    return Err(StakingError::ParamOutOfRange.into());
                }
                config.min_jury_weight = proposal.param_value;
            }
            "execution_delay_secs" => {
                if proposal.param_value < 3_600 || proposal.param_value > 2_592_000 {
                    // min 1 hour, max 30 days
                    return Err(StakingError::ParamOutOfRange.into());
                }
                config.execution_delay_secs = proposal.param_value as i64;
            }
            "proposal_deposit" => {
                if proposal.param_value > 1_000_000_000 {
                    // max 1B tokens
                    return Err(StakingError::ParamOutOfRange.into());
                }
                config.proposal_deposit = proposal.param_value;
            }
            "curator_yield_cap_bps" => {
                if proposal.param_value > 10_000 {
                    return Err(StakingError::ParamOutOfRange.into());
                }
                config.curator_yield_cap_bps = proposal.param_value;
            }
            "sentinel_fee_pct" => {
                if proposal.param_value > 50 {
                    return Err(StakingError::ParamOutOfRange.into());
                }
                if (proposal.param_value as u8)
                    .checked_add(config.burn_pct)
                    .is_none_or(|sum| sum > 100)
                {
                    return Err(StakingError::ParamOutOfRange.into());
                }
                config.sentinel_fee_pct = proposal.param_value as u8;
            }
            "jury_vote_reward" => {
                if proposal.param_value > 1_000_000_000 {
                    // max 1B
                    return Err(StakingError::ParamOutOfRange.into());
                }
                config.jury_vote_reward = proposal.param_value;
            }
            "max_jury_rewards_per_epoch" => {
                if proposal.param_value > 1_000_000_000 {
                    // max 1B
                    return Err(StakingError::ParamOutOfRange.into());
                }
                config.max_jury_rewards_per_epoch = proposal.param_value;
            }
            _ => {
                msg!("Unknown param_key: {}", proposal.param_key);
                proposal.status = ProposalStatus::Failed;
                borsh::to_writer(&mut &mut proposal_pda.data.borrow_mut()[..], &proposal)?;
                return Ok(());
            }
        }
        proposal.status = ProposalStatus::Executed;
        borsh::to_writer(&mut &mut config_pda.data.borrow_mut()[..], &config)?;
        msg!(
            "Proposal executed: {} = {}",
            proposal.param_key,
            proposal.param_value
        );
    } else {
        proposal.status = ProposalStatus::Failed;
        msg!("Proposal failed: insufficient votes for");
    }

    // M4 fix: refund deposit if quorum was met (regardless of pass/fail outcome)
    if proposal.deposit_amount > 0 {
        let (_, config_bump) = Pubkey::find_program_address(&[CONFIG_SEED], program_id);
        let depositor_token = next_account_info(accounts_iter)?;
        let vault_token = next_account_info(accounts_iter)?;
        let token_program = next_account_info(accounts_iter)?;

        // Validate refund accounts — reject mismatched accounts to prevent silent deposit loss
        if *depositor_token.key != proposal.depositor_token {
            return Err(ProgramError::InvalidSeeds);
        }
        if *vault_token.key != config.reward_vault {
            return Err(ProgramError::InvalidSeeds);
        }
        if *token_program.key != spl_token::id() {
            return Err(ProgramError::IncorrectProgramId);
        }

        let refund_ix = spl_token::instruction::transfer(
            token_program.key,
            vault_token.key,
            depositor_token.key,
            config_pda.key,
            &[],
            proposal.deposit_amount,
        )?;
        invoke_signed(
            &refund_ix,
            &[
                vault_token.clone(),
                depositor_token.clone(),
                config_pda.clone(),
                token_program.clone(),
            ],
            &[&[CONFIG_SEED, &[config_bump]]],
        )?;
        msg!(
            "Proposal deposit refunded: {} $TRUTH",
            proposal.deposit_amount
        );
    }

    borsh::to_writer(&mut &mut proposal_pda.data.borrow_mut()[..], &proposal)?;
    Ok(())
}
