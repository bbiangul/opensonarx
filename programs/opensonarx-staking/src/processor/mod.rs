pub mod admin;
pub mod channel;
pub mod commit_jury_vote;
pub mod dispute;
pub mod governance;
pub mod jury_reward;
pub mod reputation;
pub mod reveal_jury_vote;
pub mod reward;
pub mod sentinel;
pub mod stake;

use borsh::BorshDeserialize;
use solana_program::program_error::ProgramError;

use crate::error::StakingError;
use crate::state::{ProtocolConfig, StakeAccountV2, GovernanceProposal, GovernanceLockAccount};

/// Check that minting `amount` tokens won't exceed the supply cap, and increment total_minted.
pub(crate) fn check_and_increment_supply(config: &mut ProtocolConfig, amount: u64) -> Result<(), ProgramError> {
    let new_total = config.total_minted.checked_add(amount)
        .ok_or(StakingError::Overflow)?;
    if new_total > config.max_total_supply {
        return Err(StakingError::SupplyCapExceeded.into());
    }
    config.total_minted = new_total;
    Ok(())
}

/// Deserialize from account data, tolerating trailing zeros (MAX_SIZE padding).
/// Unlike `try_from_slice`, this does NOT require all bytes to be consumed.
pub(crate) fn deser<T: BorshDeserialize>(data: &[u8]) -> Result<T, ProgramError> {
    T::deserialize_reader(&mut &data[..]).map_err(|_| ProgramError::InvalidAccountData)
}

/// Deserialize a DomainRecord, handling accounts created before newer fields
/// were added. Falls back through layout versions:
/// V4 (319 bytes): is_ugc_platform + effective_slash_pct + active_dispute_nonce (current)
/// V3 (311 bytes): is_ugc_platform + effective_slash_pct
/// V2 (310 bytes): is_ugc_platform only
/// V1 (309 bytes): neither
pub(crate) fn deser_domain_record(data: &[u8]) -> Result<crate::state::DomainRecord, ProgramError> {
    // Try full deserialization first (current layout with active_dispute_nonce)
    if let Ok(record) = deser::<crate::state::DomainRecord>(data) {
        return Ok(record);
    }
    // Fall back to V3 layout (311 bytes): has effective_slash_pct but not active_dispute_nonce
    #[derive(BorshDeserialize)]
    struct DomainRecordV3 {
        is_initialized: bool,
        domain: String,
        total_staked: u64,
        publisher_count: u32,
        curator_count: u32,
        blacklisted: bool,
        dns_verified: bool,
        dns_wallet: solana_program::pubkey::Pubkey,
        bump: u8,
        is_ugc_platform: bool,
        effective_slash_pct: u8,
    }
    if let Ok(v3) = DomainRecordV3::deserialize_reader(&mut &data[..]) {
        return Ok(crate::state::DomainRecord {
            is_initialized: v3.is_initialized,
            domain: v3.domain,
            total_staked: v3.total_staked,
            publisher_count: v3.publisher_count,
            curator_count: v3.curator_count,
            blacklisted: v3.blacklisted,
            dns_verified: v3.dns_verified,
            dns_wallet: v3.dns_wallet,
            bump: v3.bump,
            is_ugc_platform: v3.is_ugc_platform,
            effective_slash_pct: v3.effective_slash_pct,
            active_dispute_nonce: u64::MAX,
        });
    }
    // Fall back to V2 layout (310 bytes): has is_ugc_platform but not effective_slash_pct
    #[derive(BorshDeserialize)]
    struct DomainRecordV2 {
        is_initialized: bool,
        domain: String,
        total_staked: u64,
        publisher_count: u32,
        curator_count: u32,
        blacklisted: bool,
        dns_verified: bool,
        dns_wallet: solana_program::pubkey::Pubkey,
        bump: u8,
        is_ugc_platform: bool,
    }
    if let Ok(v2) = DomainRecordV2::deserialize_reader(&mut &data[..]) {
        return Ok(crate::state::DomainRecord {
            is_initialized: v2.is_initialized,
            domain: v2.domain,
            total_staked: v2.total_staked,
            publisher_count: v2.publisher_count,
            curator_count: v2.curator_count,
            blacklisted: v2.blacklisted,
            dns_verified: v2.dns_verified,
            dns_wallet: v2.dns_wallet,
            bump: v2.bump,
            is_ugc_platform: v2.is_ugc_platform,
            effective_slash_pct: 0,
            active_dispute_nonce: u64::MAX,
        });
    }
    // Fall back to V1 layout (309 bytes): no is_ugc_platform, no effective_slash_pct
    #[derive(BorshDeserialize)]
    struct DomainRecordV1 {
        is_initialized: bool,
        domain: String,
        total_staked: u64,
        publisher_count: u32,
        curator_count: u32,
        blacklisted: bool,
        dns_verified: bool,
        dns_wallet: solana_program::pubkey::Pubkey,
        bump: u8,
    }
    let v1 = DomainRecordV1::deserialize_reader(&mut &data[..])
        .map_err(|_| ProgramError::InvalidAccountData)?;
    Ok(crate::state::DomainRecord {
        is_initialized: v1.is_initialized,
        domain: v1.domain,
        total_staked: v1.total_staked,
        publisher_count: v1.publisher_count,
        curator_count: v1.curator_count,
        blacklisted: v1.blacklisted,
        dns_verified: v1.dns_verified,
        dns_wallet: v1.dns_wallet,
        bump: v1.bump,
        is_ugc_platform: false,
        effective_slash_pct: 0,
        active_dispute_nonce: u64::MAX,
    })
}

/// Deserialize a DisputeAccount, handling pre-upgrade accounts without jury reward fields.
pub(crate) fn deser_dispute_account(data: &[u8]) -> Result<crate::state::DisputeAccount, ProgramError> {
    if let Ok(dispute) = deser::<crate::state::DisputeAccount>(data) {
        return Ok(dispute);
    }
    // Pre-upgrade layout (393 bytes): no jury_reward_pool, no jury_reward_claimed
    #[derive(BorshDeserialize)]
    struct DisputeAccountV1 {
        is_initialized: bool,
        reporter: solana_program::pubkey::Pubkey,
        domain: String,
        bond: u64,
        reason_hash: [u8; 32],
        status: crate::state::DisputeStatus,
        votes_guilty: u64,
        votes_innocent: u64,
        jury_count: u32,
        created_at: i64,
        deadline: i64,
        nonce: u64,
        bump: u8,
        commit_deadline: i64,
        reveal_deadline: i64,
        severity: u8,
    }
    let v1 = DisputeAccountV1::deserialize_reader(&mut &data[..])
        .map_err(|_| ProgramError::InvalidAccountData)?;
    Ok(crate::state::DisputeAccount {
        is_initialized: v1.is_initialized,
        reporter: v1.reporter,
        domain: v1.domain,
        bond: v1.bond,
        reason_hash: v1.reason_hash,
        status: v1.status,
        votes_guilty: v1.votes_guilty,
        votes_innocent: v1.votes_innocent,
        jury_count: v1.jury_count,
        created_at: v1.created_at,
        deadline: v1.deadline,
        nonce: v1.nonce,
        bump: v1.bump,
        commit_deadline: v1.commit_deadline,
        reveal_deadline: v1.reveal_deadline,
        severity: v1.severity,
        jury_reward_pool: 0,
        jury_reward_claimed: 0,
    })
}

/// Deserialize a ProtocolConfig, handling pre-upgrade accounts without medium-fix fields.
pub(crate) fn deser_config(data: &[u8]) -> Result<ProtocolConfig, ProgramError> {
    if let Ok(config) = deser::<ProtocolConfig>(data) {
        return Ok(config);
    }
    // V3 layout (358 bytes): has jury_vote_reward but no jury_rewards_this_epoch/max_jury_rewards_per_epoch
    #[derive(BorshDeserialize)]
    struct ProtocolConfigV3 {
        is_initialized: bool,
        authority: solana_program::pubkey::Pubkey,
        truth_mint: solana_program::pubkey::Pubkey,
        min_stake: u64,
        cooldown_secs: i64,
        dispute_bond: u64,
        slash_pct: u8,
        reward_rate: u64,
        jury_min_stake: u64,
        blacklist_authority: solana_program::pubkey::Pubkey,
        reward_vault: solana_program::pubkey::Pubkey,
        total_staked: u64,
        reward_per_token_stored: u128,
        proposal_nonce: u64,
        dispute_nonce: u64,
        bump: u8,
        current_epoch: u64,
        epoch_funded_amount: u64,
        epoch_max_emission: u64,
        sentinel_reward_per_token: u128,
        total_sentinel_staked: u64,
        min_epoch_duration: i64,
        epoch_started_at: i64,
        max_total_supply: u64,
        total_minted: u64,
        min_jury_weight: u64,
        burn_pct: u8,
        staker_emission_pct: u8,
        execution_delay_secs: i64,
        proposal_deposit: u64,
        curator_yield_cap_bps: u64,
        sentinel_fee_pct: u8,
        accumulated_sentinel_fees: u64,
        total_funded: u64,
        total_distributed: u64,
        jury_vote_reward: u64,
    }
    if let Ok(v3) = ProtocolConfigV3::deserialize_reader(&mut &data[..]) {
        return Ok(ProtocolConfig {
            is_initialized: v3.is_initialized,
            authority: v3.authority,
            truth_mint: v3.truth_mint,
            min_stake: v3.min_stake,
            cooldown_secs: v3.cooldown_secs,
            dispute_bond: v3.dispute_bond,
            slash_pct: v3.slash_pct,
            reward_rate: v3.reward_rate,
            jury_min_stake: v3.jury_min_stake,
            blacklist_authority: v3.blacklist_authority,
            reward_vault: v3.reward_vault,
            total_staked: v3.total_staked,
            reward_per_token_stored: v3.reward_per_token_stored,
            proposal_nonce: v3.proposal_nonce,
            dispute_nonce: v3.dispute_nonce,
            bump: v3.bump,
            current_epoch: v3.current_epoch,
            epoch_funded_amount: v3.epoch_funded_amount,
            epoch_max_emission: v3.epoch_max_emission,
            sentinel_reward_per_token: v3.sentinel_reward_per_token,
            total_sentinel_staked: v3.total_sentinel_staked,
            min_epoch_duration: v3.min_epoch_duration,
            epoch_started_at: v3.epoch_started_at,
            max_total_supply: v3.max_total_supply,
            total_minted: v3.total_minted,
            min_jury_weight: v3.min_jury_weight,
            burn_pct: v3.burn_pct,
            staker_emission_pct: v3.staker_emission_pct,
            execution_delay_secs: v3.execution_delay_secs,
            proposal_deposit: v3.proposal_deposit,
            curator_yield_cap_bps: v3.curator_yield_cap_bps,
            sentinel_fee_pct: v3.sentinel_fee_pct,
            accumulated_sentinel_fees: v3.accumulated_sentinel_fees,
            total_funded: v3.total_funded,
            total_distributed: v3.total_distributed,
            jury_vote_reward: v3.jury_vote_reward,
            jury_rewards_this_epoch: 0,
            max_jury_rewards_per_epoch: 0, // 0 = uncapped for pre-upgrade accounts
        });
    }
    // V2 layout (350 bytes): no jury_vote_reward
    #[derive(BorshDeserialize)]
    struct ProtocolConfigV2 {
        is_initialized: bool,
        authority: solana_program::pubkey::Pubkey,
        truth_mint: solana_program::pubkey::Pubkey,
        min_stake: u64,
        cooldown_secs: i64,
        dispute_bond: u64,
        slash_pct: u8,
        reward_rate: u64,
        jury_min_stake: u64,
        blacklist_authority: solana_program::pubkey::Pubkey,
        reward_vault: solana_program::pubkey::Pubkey,
        total_staked: u64,
        reward_per_token_stored: u128,
        proposal_nonce: u64,
        dispute_nonce: u64,
        bump: u8,
        current_epoch: u64,
        epoch_funded_amount: u64,
        epoch_max_emission: u64,
        sentinel_reward_per_token: u128,
        total_sentinel_staked: u64,
        min_epoch_duration: i64,
        epoch_started_at: i64,
        max_total_supply: u64,
        total_minted: u64,
        min_jury_weight: u64,
        burn_pct: u8,
        staker_emission_pct: u8,
        execution_delay_secs: i64,
        proposal_deposit: u64,
        curator_yield_cap_bps: u64,
        sentinel_fee_pct: u8,
        accumulated_sentinel_fees: u64,
        total_funded: u64,
        total_distributed: u64,
    }
    if let Ok(v2) = ProtocolConfigV2::deserialize_reader(&mut &data[..]) {
        return Ok(ProtocolConfig {
            is_initialized: v2.is_initialized,
            authority: v2.authority,
            truth_mint: v2.truth_mint,
            min_stake: v2.min_stake,
            cooldown_secs: v2.cooldown_secs,
            dispute_bond: v2.dispute_bond,
            slash_pct: v2.slash_pct,
            reward_rate: v2.reward_rate,
            jury_min_stake: v2.jury_min_stake,
            blacklist_authority: v2.blacklist_authority,
            reward_vault: v2.reward_vault,
            total_staked: v2.total_staked,
            reward_per_token_stored: v2.reward_per_token_stored,
            proposal_nonce: v2.proposal_nonce,
            dispute_nonce: v2.dispute_nonce,
            bump: v2.bump,
            current_epoch: v2.current_epoch,
            epoch_funded_amount: v2.epoch_funded_amount,
            epoch_max_emission: v2.epoch_max_emission,
            sentinel_reward_per_token: v2.sentinel_reward_per_token,
            total_sentinel_staked: v2.total_sentinel_staked,
            min_epoch_duration: v2.min_epoch_duration,
            epoch_started_at: v2.epoch_started_at,
            max_total_supply: v2.max_total_supply,
            total_minted: v2.total_minted,
            min_jury_weight: v2.min_jury_weight,
            burn_pct: v2.burn_pct,
            staker_emission_pct: v2.staker_emission_pct,
            execution_delay_secs: v2.execution_delay_secs,
            proposal_deposit: v2.proposal_deposit,
            curator_yield_cap_bps: v2.curator_yield_cap_bps,
            sentinel_fee_pct: v2.sentinel_fee_pct,
            accumulated_sentinel_fees: v2.accumulated_sentinel_fees,
            total_funded: v2.total_funded,
            total_distributed: v2.total_distributed,
            jury_vote_reward: 0,
            jury_rewards_this_epoch: 0,
            max_jury_rewards_per_epoch: 0,
        });
    }
    // Pre-upgrade layout (325 bytes): no sentinel_fee_pct, accumulated_sentinel_fees, total_funded, total_distributed
    #[derive(BorshDeserialize)]
    struct ProtocolConfigV1 {
        is_initialized: bool,
        authority: solana_program::pubkey::Pubkey,
        truth_mint: solana_program::pubkey::Pubkey,
        min_stake: u64,
        cooldown_secs: i64,
        dispute_bond: u64,
        slash_pct: u8,
        reward_rate: u64,
        jury_min_stake: u64,
        blacklist_authority: solana_program::pubkey::Pubkey,
        reward_vault: solana_program::pubkey::Pubkey,
        total_staked: u64,
        reward_per_token_stored: u128,
        proposal_nonce: u64,
        dispute_nonce: u64,
        bump: u8,
        current_epoch: u64,
        epoch_funded_amount: u64,
        epoch_max_emission: u64,
        sentinel_reward_per_token: u128,
        total_sentinel_staked: u64,
        min_epoch_duration: i64,
        epoch_started_at: i64,
        max_total_supply: u64,
        total_minted: u64,
        min_jury_weight: u64,
        burn_pct: u8,
        staker_emission_pct: u8,
        execution_delay_secs: i64,
        proposal_deposit: u64,
        curator_yield_cap_bps: u64,
    }
    let v1 = ProtocolConfigV1::deserialize_reader(&mut &data[..])
        .map_err(|_| ProgramError::InvalidAccountData)?;
    Ok(ProtocolConfig {
        is_initialized: v1.is_initialized,
        authority: v1.authority,
        truth_mint: v1.truth_mint,
        min_stake: v1.min_stake,
        cooldown_secs: v1.cooldown_secs,
        dispute_bond: v1.dispute_bond,
        slash_pct: v1.slash_pct,
        reward_rate: v1.reward_rate,
        jury_min_stake: v1.jury_min_stake,
        blacklist_authority: v1.blacklist_authority,
        reward_vault: v1.reward_vault,
        total_staked: v1.total_staked,
        reward_per_token_stored: v1.reward_per_token_stored,
        proposal_nonce: v1.proposal_nonce,
        dispute_nonce: v1.dispute_nonce,
        bump: v1.bump,
        current_epoch: v1.current_epoch,
        epoch_funded_amount: v1.epoch_funded_amount,
        epoch_max_emission: v1.epoch_max_emission,
        sentinel_reward_per_token: v1.sentinel_reward_per_token,
        total_sentinel_staked: v1.total_sentinel_staked,
        min_epoch_duration: v1.min_epoch_duration,
        epoch_started_at: v1.epoch_started_at,
        max_total_supply: v1.max_total_supply,
        total_minted: v1.total_minted,
        min_jury_weight: v1.min_jury_weight,
        burn_pct: v1.burn_pct,
        staker_emission_pct: v1.staker_emission_pct,
        execution_delay_secs: v1.execution_delay_secs,
        proposal_deposit: v1.proposal_deposit,
        curator_yield_cap_bps: v1.curator_yield_cap_bps,
        sentinel_fee_pct: 0,
        accumulated_sentinel_fees: 0,
        total_funded: 0,
        total_distributed: 0,
        jury_vote_reward: 0,
        jury_rewards_this_epoch: 0,
        max_jury_rewards_per_epoch: 0,
    })
}

/// Deserialize a StakeAccountV2, handling pre-upgrade accounts without governance_lock_until.
pub(crate) fn deser_stake_v2(data: &[u8]) -> Result<StakeAccountV2, ProgramError> {
    if let Ok(stake) = deser::<StakeAccountV2>(data) {
        return Ok(stake);
    }
    // Pre-upgrade layout (341 bytes): no governance_lock_until
    #[derive(BorshDeserialize)]
    struct StakeAccountV2Old {
        is_initialized: bool,
        staker: solana_program::pubkey::Pubkey,
        domain: String,
        amount: u64,
        staked_at: i64,
        cooldown_until: i64,
        stake_type: crate::state::StakeType,
        reward_debt: u128,
        dns_verified: bool,
        slashed_amount: u64,
        bump: u8,
    }
    let v1 = StakeAccountV2Old::deserialize_reader(&mut &data[..])
        .map_err(|_| ProgramError::InvalidAccountData)?;
    Ok(StakeAccountV2 {
        is_initialized: v1.is_initialized,
        staker: v1.staker,
        domain: v1.domain,
        amount: v1.amount,
        staked_at: v1.staked_at,
        cooldown_until: v1.cooldown_until,
        stake_type: v1.stake_type,
        reward_debt: v1.reward_debt,
        dns_verified: v1.dns_verified,
        slashed_amount: v1.slashed_amount,
        bump: v1.bump,
        governance_lock_until: 0,
    })
}

/// Deserialize a GovernanceProposal, handling pre-upgrade accounts without deposit refund fields.
pub(crate) fn deser_proposal(data: &[u8]) -> Result<GovernanceProposal, ProgramError> {
    if let Ok(proposal) = deser::<GovernanceProposal>(data) {
        return Ok(proposal);
    }
    // Pre-upgrade layout (191 bytes): no deposit_amount, depositor_token
    #[derive(BorshDeserialize)]
    struct GovernanceProposalV1 {
        is_initialized: bool,
        proposer: solana_program::pubkey::Pubkey,
        description_hash: [u8; 32],
        param_key: String,
        param_value: u64,
        votes_for: u64,
        votes_against: u64,
        status: crate::state::ProposalStatus,
        created_at: i64,
        voting_ends: i64,
        nonce: u64,
        bump: u8,
        executable_after: i64,
    }
    let v1 = GovernanceProposalV1::deserialize_reader(&mut &data[..])
        .map_err(|_| ProgramError::InvalidAccountData)?;
    Ok(GovernanceProposal {
        is_initialized: v1.is_initialized,
        proposer: v1.proposer,
        description_hash: v1.description_hash,
        param_key: v1.param_key,
        param_value: v1.param_value,
        votes_for: v1.votes_for,
        votes_against: v1.votes_against,
        status: v1.status,
        created_at: v1.created_at,
        voting_ends: v1.voting_ends,
        nonce: v1.nonce,
        bump: v1.bump,
        executable_after: v1.executable_after,
        deposit_amount: 0,
        depositor_token: solana_program::pubkey::Pubkey::default(),
    })
}

/// Deserialize a JuryVote, handling pre-upgrade accounts without reward_claimed field.
pub(crate) fn deser_jury_vote(data: &[u8]) -> Result<crate::state::JuryVote, ProgramError> {
    if let Ok(vote) = deser::<crate::state::JuryVote>(data) {
        return Ok(vote);
    }
    // Pre-upgrade layout (83 bytes): no reward_claimed
    #[derive(BorshDeserialize)]
    struct JuryVoteV1 {
        is_initialized: bool,
        voter: solana_program::pubkey::Pubkey,
        verdict: crate::state::Verdict,
        weight: u64,
        voted_at: i64,
        bump: u8,
        commit_hash: [u8; 32],
    }
    let v1 = JuryVoteV1::deserialize_reader(&mut &data[..])
        .map_err(|_| ProgramError::InvalidAccountData)?;
    Ok(crate::state::JuryVote {
        is_initialized: v1.is_initialized,
        voter: v1.voter,
        verdict: v1.verdict,
        weight: v1.weight,
        voted_at: v1.voted_at,
        bump: v1.bump,
        commit_hash: v1.commit_hash,
        reward_claimed: false,
    })
}

/// Deserialize a GovernanceLockAccount, tolerating trailing zeros.
pub(crate) fn deser_gov_lock(data: &[u8]) -> Result<GovernanceLockAccount, ProgramError> {
    deser::<GovernanceLockAccount>(data)
}
