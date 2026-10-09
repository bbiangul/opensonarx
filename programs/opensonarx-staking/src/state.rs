use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::pubkey::Pubkey;

// ──────────────────────────────────────────────
// PDA Seed Constants
// ──────────────────────────────────────────────
pub const STAKE_SEED: &[u8] = b"stake";
pub const CONFIG_SEED: &[u8] = b"config";
pub const DOMAIN_SEED: &[u8] = b"domain";
pub const DISPUTE_SEED: &[u8] = b"dispute";
pub const JURY_SEED: &[u8] = b"jury";
pub const PROPOSAL_SEED: &[u8] = b"proposal";
pub const GOV_VOTE_SEED: &[u8] = b"gov_vote";
pub const REWARD_VAULT_SEED: &[u8] = b"reward_vault";
pub const CHANNEL_SEED: &[u8] = b"channel";
pub const SENTINEL_SEED: &[u8] = b"sentinel";
pub const REPUTATION_ROOT_SEED: &[u8] = b"reputation_root";
pub const GOV_LOCK_SEED: &[u8] = b"gov_lock";

// ──────────────────────────────────────────────
// Enums
// ──────────────────────────────────────────────

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[borsh(use_discriminant = true)]
#[repr(u8)]
pub enum StakeType {
    Publisher = 0,
    Curator = 1,
    Sentinel = 2,
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[borsh(use_discriminant = true)]
#[repr(u8)]
pub enum DisputeStatus {
    Active = 0,
    ResolvedGuilty = 1,
    ResolvedInnocent = 2,
    CommitPhase = 3,
    RevealPhase = 4,
    QuorumFailed = 5,
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[borsh(use_discriminant = true)]
#[repr(u8)]
pub enum Verdict {
    Guilty = 0,
    Innocent = 1,
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[borsh(use_discriminant = true)]
#[repr(u8)]
pub enum ProposalStatus {
    Active = 0,
    Executed = 1,
    Failed = 2,
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[borsh(use_discriminant = true)]
#[repr(u8)]
pub enum GovernanceVoteChoice {
    For = 0,
    Against = 1,
}

// ──────────────────────────────────────────────
// Legacy StakeAccount (kept for backwards compat in tests)
// ──────────────────────────────────────────────

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct StakeAccount {
    pub is_initialized: bool,
    pub staker: Pubkey,
    pub domain: String,
    pub amount_lamports: u64,
    pub staked_at: i64,
    pub cooldown_until: i64,
}

impl StakeAccount {
    pub const MAX_SIZE: usize = 1 + 32 + (4 + 253) + 8 + 8 + 8;
}

// ──────────────────────────────────────────────
// 1. ProtocolConfig — seeds: ["config"]
// ──────────────────────────────────────────────

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct ProtocolConfig {
    pub is_initialized: bool,
    pub authority: Pubkey,
    pub truth_mint: Pubkey,
    pub min_stake: u64,
    pub cooldown_secs: i64,
    pub dispute_bond: u64,
    pub slash_pct: u8,    // 0-100
    pub reward_rate: u64, // DEPRECATED: vestigial field, kept for layout compat
    pub jury_min_stake: u64,
    pub blacklist_authority: Pubkey,
    pub reward_vault: Pubkey,
    pub total_staked: u64,
    pub reward_per_token_stored: u128,
    pub proposal_nonce: u64,
    pub dispute_nonce: u64,
    pub bump: u8,
    // ── Epoch tracking (added for Sentinel economics) ──
    pub current_epoch: u64,
    pub epoch_funded_amount: u64, // Tokens funded in current epoch
    pub epoch_max_emission: u64,  // Max emission for current epoch (1.5% decay per epoch)
    pub sentinel_reward_per_token: u128, // Separate reward index for Sentinels
    pub total_sentinel_staked: u64, // Total $TRUTH staked by Sentinels
    // ── Governable economic parameters (Phase: Economic Fixes) ──
    pub min_epoch_duration: i64, // Minimum seconds between epoch advances (0 = no constraint)
    pub epoch_started_at: i64,   // Timestamp when current epoch started
    pub max_total_supply: u64,   // Hard cap on total minted $TRUTH (u64::MAX = uncapped)
    pub total_minted: u64,       // Cumulative tokens minted across all operations
    pub min_jury_weight: u64,    // Minimum total jury weight for dispute quorum (0 = no quorum)
    pub burn_pct: u8,            // Burn percentage for channel settlement (0-100, default 10)
    pub staker_emission_pct: u8, // Staker share of emissions (0-100, default 70)
    pub execution_delay_secs: i64, // Governance timelock delay after voting ends
    pub proposal_deposit: u64,   // $TRUTH deposit required to create a proposal (0 = free)
    pub curator_yield_cap_bps: u64, // Curator yield cap in basis points (1500 = 15.00% APY)
    // ── Medium Economic Fixes ──
    pub sentinel_fee_pct: u8, // % of channel spend routed to sentinel reward pool (initializer default 5)
    pub accumulated_sentinel_fees: u64, // Sentinel fees awaiting distribution
    pub total_funded: u64,    // Cumulative tokens funded to vault via FundRewards
    pub total_distributed: u64, // Cumulative tokens distributed via UpdateRewardIndex
    pub jury_vote_reward: u64, // Flat $TRUTH minted per winning jury vote (default 1000)
    pub jury_rewards_this_epoch: u64, // Cumulative jury rewards minted in current epoch (reset on advance)
    pub max_jury_rewards_per_epoch: u64, // Cap on total jury rewards mintable per epoch (0 = uncapped)
}

impl ProtocolConfig {
    // 358 + 8 + 8 = 374
    pub const MAX_SIZE: usize = 374;
}

// ──────────────────────────────────────────────
// 2. StakeAccountV2 — seeds: ["stake", domain, staker]
// ──────────────────────────────────────────────

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct StakeAccountV2 {
    pub is_initialized: bool,
    pub staker: Pubkey,
    pub domain: String,
    pub amount: u64, // $TRUTH tokens (smallest unit)
    pub staked_at: i64,
    pub cooldown_until: i64, // 0 = active
    pub stake_type: StakeType,
    pub reward_debt: u128,
    pub dns_verified: bool,
    pub slashed_amount: u64,
    pub bump: u8,
    // ── Medium Economic Fixes ──
    pub governance_lock_until: i64, // Earliest unstake timestamp (0 = no lock)
}

impl StakeAccountV2 {
    // 1 + 32 + (4+253) + 8 + 8 + 8 + 1 + 16 + 1 + 8 + 1 + 8 = 349
    pub const MAX_SIZE: usize = 349;
}

// ──────────────────────────────────────────────
// 3. DomainRecord — seeds: ["domain", domain]
// ──────────────────────────────────────────────

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct DomainRecord {
    pub is_initialized: bool,
    pub domain: String,
    pub total_staked: u64,
    pub publisher_count: u32,
    pub curator_count: u32,
    pub blacklisted: bool,
    pub dns_verified: bool,
    pub dns_wallet: Pubkey,
    pub bump: u8,
    /// Whether this domain is a UGC platform (zero domain-level passthrough).
    /// Settable via governance (SetUgcPlatform instruction).
    pub is_ugc_platform: bool,
    /// Effective slash percentage for this domain (set by dispute severity).
    /// Used at withdraw time instead of config.slash_pct for graduated slashing.
    pub effective_slash_pct: u8,
    /// Nonce of the currently active dispute on this domain (u64::MAX = no active dispute).
    pub active_dispute_nonce: u64,
}

impl DomainRecord {
    // 1 + (4+253) + 8 + 4 + 4 + 1 + 1 + 32 + 1 + 1 + 1 + 8 = 319
    pub const MAX_SIZE: usize = 319;
}

// ──────────────────────────────────────────────
// 4. DisputeAccount — seeds: ["dispute", domain, nonce(le bytes)]
// ──────────────────────────────────────────────

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct DisputeAccount {
    pub is_initialized: bool,
    pub reporter: Pubkey,
    pub domain: String,
    pub bond: u64,
    pub reason_hash: [u8; 32],
    pub status: DisputeStatus,
    pub votes_guilty: u64,
    pub votes_innocent: u64,
    pub jury_count: u32,
    pub created_at: i64,
    pub deadline: i64,
    pub nonce: u64,
    pub bump: u8,
    pub commit_deadline: i64,
    pub reveal_deadline: i64,
    /// Dispute severity: 0=low, 1=medium (default), 2=high.
    /// Affects slash percentage applied to the domain.
    pub severity: u8,
    /// Pool of tokens allocated to winning-side jurors at resolution.
    pub jury_reward_pool: u64,
    /// Cumulative tokens claimed from the jury reward pool.
    pub jury_reward_claimed: u64,
}

impl DisputeAccount {
    // 1 + 32 + (4+253) + 8 + 32 + 1 + 8 + 8 + 4 + 8 + 8 + 8 + 1 + 8 + 8 + 1 + 8 + 8 = 409
    pub const MAX_SIZE: usize = 409;
}

// ──────────────────────────────────────────────
// 5. JuryVote — seeds: ["jury", dispute_pda, voter]
// ──────────────────────────────────────────────

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct JuryVote {
    pub is_initialized: bool,
    pub voter: Pubkey,
    pub verdict: Verdict,
    pub weight: u64,
    pub voted_at: i64,
    pub bump: u8,
    pub commit_hash: [u8; 32],
    /// Whether this juror has claimed their jury reward.
    pub reward_claimed: bool,
}

impl JuryVote {
    // 1 + 32 + 1 + 8 + 8 + 1 + 32 + 1 = 84
    pub const MAX_SIZE: usize = 84;
}

// ──────────────────────────────────────────────
// 6. GovernanceProposal — seeds: ["proposal", nonce(le bytes)]
// ──────────────────────────────────────────────

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct GovernanceProposal {
    pub is_initialized: bool,
    pub proposer: Pubkey,
    pub description_hash: [u8; 32],
    pub param_key: String, // e.g. "cooldown_secs", "slash_pct"
    pub param_value: u64,
    pub votes_for: u64,
    pub votes_against: u64,
    pub status: ProposalStatus,
    pub created_at: i64,
    pub voting_ends: i64,
    pub nonce: u64,
    pub bump: u8,
    /// Earliest timestamp at which this proposal can be executed (voting_ends + execution_delay).
    pub executable_after: i64,
    // ── Medium Economic Fixes ──
    pub deposit_amount: u64,     // $TRUTH deposited (for refund tracking)
    pub depositor_token: Pubkey, // Proposer's token account (refund destination)
}

impl GovernanceProposal {
    // 1 + 32 + 32 + (4+64) + 8 + 8 + 8 + 1 + 8 + 8 + 8 + 1 + 8 + 8 + 32 = 231
    pub const MAX_SIZE: usize = 231;
}

// ──────────────────────────────────────────────
// 7. GovernanceVote — seeds: ["gov_vote", proposal_pda, voter]
// ──────────────────────────────────────────────

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct GovernanceVote {
    pub is_initialized: bool,
    pub voter: Pubkey,
    pub vote: GovernanceVoteChoice,
    pub weight: u64,
    pub bump: u8,
}

impl GovernanceVote {
    // 1 + 32 + 1 + 8 + 1 = 43
    pub const MAX_SIZE: usize = 43;
}

// ──────────────────────────────────────────────
// 8. SentinelAccount — seeds: ["sentinel", sentinel_pubkey]
//    Global stake (not per-domain) for crawler nodes.
// ──────────────────────────────────────────────

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct SentinelAccount {
    pub is_initialized: bool,
    pub sentinel: Pubkey,
    pub amount: u64, // $TRUTH tokens staked
    pub staked_at: i64,
    pub cooldown_until: i64, // 0 = active
    pub reward_debt: u128,   // Sentinel-specific reward index snapshot
    pub slashed_amount: u64,
    pub bump: u8,
}

impl SentinelAccount {
    // 1 + 32 + 8 + 8 + 8 + 16 + 8 + 1 = 82
    pub const MAX_SIZE: usize = 82;
}

// ──────────────────────────────────────────────
// 9. StateChannelAccount — seeds: ["channel", payer, payee]
// ──────────────────────────────────────────────

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct StateChannelAccount {
    pub is_initialized: bool,
    pub payer: Pubkey,
    pub payee: Pubkey,
    pub deposited: u64,
    pub spent: u64,
    pub nonce: u64,
    pub expiry_slot: u64,
    pub bump: u8,
}

impl StateChannelAccount {
    // 1 + 32 + 32 + 8 + 8 + 8 + 8 + 1 = 98
    pub const MAX_SIZE: usize = 98;
}

// ──────────────────────────────────────────────
// 10. ReputationRootAccount — seeds: ["reputation_root"]
//     Singleton PDA storing the latest Merkle root of domain reputations.
// ──────────────────────────────────────────────

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct ReputationRootAccount {
    pub is_initialized: bool,  // 1
    pub merkle_root: [u8; 32], // 32
    pub epoch: u64,            // 8
    pub timestamp: i64,        // 8
    pub bump: u8,              // 1
}

impl ReputationRootAccount {
    // 1 + 32 + 8 + 8 + 1 = 50
    pub const MAX_SIZE: usize = 50;
}

// ──────────────────────────────────────────────
// 11. GovernanceLockAccount — seeds: ["gov_lock", wallet]
//     Per-wallet governance lock for vote-locking across all stake positions.
// ──────────────────────────────────────────────

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct GovernanceLockAccount {
    pub is_initialized: bool, // 1
    pub wallet: Pubkey,       // 32
    pub lock_until: i64,      // 8
    pub bump: u8,             // 1
}

impl GovernanceLockAccount {
    // 1 + 32 + 8 + 1 = 42
    pub const MAX_SIZE: usize = 42;
}
