#![allow(unexpected_cfgs, deprecated)]

pub mod error;
pub mod instruction;
pub mod processor;
pub mod state;

use borsh::BorshDeserialize;
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    clock::Clock,
    entrypoint,
    entrypoint::ProgramResult,
    msg,
    program::invoke_signed,
    program_error::ProgramError,
    pubkey::Pubkey,
    rent::Rent,
    system_instruction,
    sysvar::Sysvar,
};

use instruction::StakingInstruction;
use state::{StakeAccount, STAKE_SEED};

solana_program::declare_id!("Fg6PaFpoGXkYsidMpWTK6W2BeZ7FEfcYkg476zPFsLnS");

entrypoint!(process_instruction);

pub fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    let instruction = StakingInstruction::try_from_slice(instruction_data)
        .map_err(|_| ProgramError::InvalidInstructionData)?;

    match instruction {
        // Legacy V1
        StakingInstruction::Stake { domain, amount } => {
            process_stake(program_id, accounts, &domain, amount)
        }
        StakingInstruction::Unstake { domain } => process_unstake(program_id, accounts, &domain),

        // Legacy V1 Withdraw
        StakingInstruction::WithdrawV1 { domain } => {
            process_withdraw_v1(program_id, accounts, &domain)
        }

        // Phase 0: Foundation
        StakingInstruction::InitializeProtocol {
            min_stake,
            cooldown_secs,
            dispute_bond,
            slash_pct,
            reward_rate,
            jury_min_stake,
        } => processor::admin::process_initialize_protocol(
            program_id,
            accounts,
            min_stake,
            cooldown_secs,
            dispute_bond,
            slash_pct,
            reward_rate,
            jury_min_stake,
        ),
        StakingInstruction::MintTruth { amount } => {
            processor::admin::process_mint_truth(program_id, accounts, amount)
        }

        // Phase 1: SPL Token Staking
        StakingInstruction::StakeV2 {
            domain,
            amount,
            stake_type,
        } => processor::stake::process_stake_v2(program_id, accounts, &domain, amount, stake_type),
        StakingInstruction::UnstakeV2 { domain } => {
            processor::stake::process_unstake_v2(program_id, accounts, &domain)
        }
        StakingInstruction::Withdraw { domain } => {
            processor::stake::process_withdraw(program_id, accounts, &domain)
        }
        StakingInstruction::ClaimReward { domain } => {
            processor::stake::process_claim_reward(program_id, accounts, &domain)
        }

        // Phase 2: DNS & Domain Management
        StakingInstruction::VerifyDns { domain, dns_wallet } => {
            processor::admin::process_verify_dns(program_id, accounts, &domain, dns_wallet)
        }
        StakingInstruction::BlacklistDomain { domain } => {
            processor::admin::process_blacklist_domain(program_id, accounts, &domain)
        }
        StakingInstruction::SetUgcPlatform { domain, is_ugc } => {
            processor::admin::process_set_ugc_platform(program_id, accounts, &domain, is_ugc)
        }

        // Phase 3: Disputes & Slashing
        StakingInstruction::OpenDispute {
            domain,
            reason_hash,
            severity,
        } => processor::dispute::process_open_dispute(
            program_id,
            accounts,
            &domain,
            reason_hash,
            severity,
        ),
        StakingInstruction::CastJuryVote {
            domain,
            dispute_nonce,
            verdict,
        } => processor::dispute::process_cast_jury_vote(
            program_id,
            accounts,
            &domain,
            dispute_nonce,
            verdict,
        ),
        StakingInstruction::CommitJuryVote {
            domain,
            dispute_nonce,
            commit_hash,
        } => processor::commit_jury_vote::process_commit_jury_vote(
            program_id,
            accounts,
            &domain,
            dispute_nonce,
            commit_hash,
        ),
        StakingInstruction::RevealJuryVote {
            domain,
            dispute_nonce,
            verdict,
            salt,
        } => processor::reveal_jury_vote::process_reveal_jury_vote(
            program_id,
            accounts,
            &domain,
            dispute_nonce,
            verdict,
            salt,
        ),
        StakingInstruction::ResolveDispute {
            domain,
            dispute_nonce,
        } => processor::dispute::process_resolve_dispute(
            program_id,
            accounts,
            &domain,
            dispute_nonce,
        ),

        // Phase 4: Rewards
        StakingInstruction::FundRewards { amount } => {
            processor::reward::process_fund_rewards(program_id, accounts, amount)
        }
        StakingInstruction::UpdateRewardIndex { funded_amount } => {
            processor::reward::process_update_reward_index(program_id, accounts, funded_amount)
        }

        // Phase 5: DAO Governance
        StakingInstruction::CreateProposal {
            description_hash,
            param_key,
            param_value,
        } => processor::governance::process_create_proposal(
            program_id,
            accounts,
            description_hash,
            &param_key,
            param_value,
        ),
        StakingInstruction::CastGovernanceVote {
            proposal_nonce,
            vote,
        } => processor::governance::process_cast_governance_vote(
            program_id,
            accounts,
            proposal_nonce,
            vote,
        ),
        StakingInstruction::ExecuteProposal { proposal_nonce } => {
            processor::governance::process_execute_proposal(program_id, accounts, proposal_nonce)
        }

        // Phase 6: State Channels
        StakingInstruction::OpenChannel {
            payee,
            amount,
            expiry_slot,
        } => processor::channel::process_open_channel(
            program_id,
            accounts,
            payee,
            amount,
            expiry_slot,
        ),
        StakingInstruction::SettleChannel {
            payer_key,
            payee_key,
            spent,
            nonce,
        } => processor::channel::process_settle_channel(
            program_id, accounts, payer_key, payee_key, spent, nonce,
        ),

        // Phase 7: Sentinel Nodes
        StakingInstruction::StakeSentinel { amount } => {
            processor::sentinel::process_stake_sentinel(program_id, accounts, amount)
        }
        StakingInstruction::UnstakeSentinel => {
            processor::sentinel::process_unstake_sentinel(program_id, accounts)
        }
        StakingInstruction::ClaimSentinelReward => {
            processor::sentinel::process_claim_sentinel_reward(program_id, accounts)
        }
        StakingInstruction::WithdrawSentinel => {
            processor::sentinel::process_withdraw_sentinel(program_id, accounts)
        }
        StakingInstruction::AdvanceEpoch => {
            processor::reward::process_advance_epoch(program_id, accounts)
        }
        StakingInstruction::SlashSentinel { sentinel, amount } => {
            processor::sentinel::process_slash_sentinel(program_id, accounts, sentinel, amount)
        }

        // Phase 9: Jury Incentives
        StakingInstruction::ClaimJuryReward {
            domain,
            dispute_nonce,
        } => processor::jury_reward::process_claim_jury_reward(
            program_id,
            accounts,
            &domain,
            dispute_nonce,
        ),

        // Reputation
        StakingInstruction::CommitReputationRoot { merkle_root, epoch } => {
            processor::reputation::process_commit_reputation_root(
                program_id,
                accounts,
                merkle_root,
                epoch,
            )
        }
    }
}

// ── Legacy V1 processors (unchanged) ──

fn process_stake(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    domain: &str,
    amount: u64,
) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();
    let staker = next_account_info(accounts_iter)?;
    let stake_pda = next_account_info(accounts_iter)?;
    let system_program = next_account_info(accounts_iter)?;
    let clock_sysvar = next_account_info(accounts_iter)?;

    if !staker.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }

    let (expected_pda, bump) =
        Pubkey::find_program_address(&[STAKE_SEED, domain.as_bytes()], program_id);
    if *stake_pda.key != expected_pda {
        msg!("Invalid PDA");
        return Err(ProgramError::InvalidSeeds);
    }

    let clock = Clock::from_account_info(clock_sysvar)?;
    let rent = Rent::get()?;
    let space = StakeAccount::MAX_SIZE;
    let lamports = rent.minimum_balance(space);

    invoke_signed(
        &system_instruction::create_account(
            staker.key,
            stake_pda.key,
            lamports + amount,
            space as u64,
            program_id,
        ),
        &[staker.clone(), stake_pda.clone(), system_program.clone()],
        &[&[STAKE_SEED, domain.as_bytes(), &[bump]]],
    )?;

    let account = StakeAccount {
        is_initialized: true,
        staker: *staker.key,
        domain: domain.to_string(),
        amount_lamports: amount,
        staked_at: clock.unix_timestamp,
        cooldown_until: 0,
    };
    borsh::to_writer(&mut &mut stake_pda.data.borrow_mut()[..], &account)?;

    msg!("Staked {} lamports for {}", amount, domain);
    Ok(())
}

/// WithdrawV1: after cooldown, return lamports and close V1 PDA.
fn process_withdraw_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    domain: &str,
) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();
    let staker = next_account_info(accounts_iter)?;
    let stake_pda = next_account_info(accounts_iter)?;
    let clock_sysvar = next_account_info(accounts_iter)?;

    if !staker.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }

    let (expected_pda, _) =
        Pubkey::find_program_address(&[STAKE_SEED, domain.as_bytes()], program_id);
    if *stake_pda.key != expected_pda {
        return Err(ProgramError::InvalidSeeds);
    }

    let account = StakeAccount::deserialize_reader(&mut &stake_pda.data.borrow()[..])
        .map_err(|_| ProgramError::InvalidAccountData)?;
    if !account.is_initialized {
        return Err(ProgramError::UninitializedAccount);
    }
    if account.staker != *staker.key {
        return Err(ProgramError::IllegalOwner);
    }
    if account.cooldown_until == 0 {
        msg!("Must call Unstake first");
        return Err(ProgramError::InvalidAccountData);
    }

    let clock = Clock::from_account_info(clock_sysvar)?;
    if clock.unix_timestamp < account.cooldown_until {
        msg!("Cooldown not expired");
        return Err(ProgramError::InvalidAccountData);
    }

    // Transfer all lamports (rent + staked amount) back to staker, closing the PDA
    let pda_lamports = stake_pda.lamports();
    **stake_pda.lamports.borrow_mut() = 0;
    **staker.lamports.borrow_mut() = staker
        .lamports()
        .checked_add(pda_lamports)
        .ok_or(ProgramError::ArithmeticOverflow)?;

    // Zero out account data
    let mut data = stake_pda.data.borrow_mut();
    for byte in data.iter_mut() {
        *byte = 0;
    }

    msg!("V1 Withdrew {} lamports for {}", pda_lamports, domain);
    Ok(())
}

fn process_unstake(program_id: &Pubkey, accounts: &[AccountInfo], domain: &str) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();
    let staker = next_account_info(accounts_iter)?;
    let stake_pda = next_account_info(accounts_iter)?;
    let clock_sysvar = next_account_info(accounts_iter)?;

    if !staker.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }

    let (expected_pda, _bump) =
        Pubkey::find_program_address(&[STAKE_SEED, domain.as_bytes()], program_id);
    if *stake_pda.key != expected_pda {
        return Err(ProgramError::InvalidSeeds);
    }

    let mut account = StakeAccount::deserialize_reader(&mut &stake_pda.data.borrow()[..])
        .map_err(|_| ProgramError::InvalidAccountData)?;
    if !account.is_initialized {
        msg!("Account not initialized");
        return Err(ProgramError::UninitializedAccount);
    }
    if account.staker != *staker.key {
        msg!("Staker mismatch");
        return Err(ProgramError::IllegalOwner);
    }

    let clock = Clock::from_account_info(clock_sysvar)?;
    let seven_days: i64 = 7 * 24 * 60 * 60;
    account.cooldown_until = clock.unix_timestamp + seven_days;
    borsh::to_writer(&mut &mut stake_pda.data.borrow_mut()[..], &account)?;

    msg!(
        "Unstake initiated for {}, cooldown until {}",
        domain,
        account.cooldown_until
    );
    Ok(())
}
