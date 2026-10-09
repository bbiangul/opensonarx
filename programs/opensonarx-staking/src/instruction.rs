use borsh::{BorshDeserialize, BorshSerialize};

#[derive(BorshSerialize, BorshDeserialize, Debug)]
pub enum StakingInstruction {
    // ── Legacy (kept for backwards compat) ──
    /// Stake SOL for a domain (legacy V1).
    ///
    /// Accounts:
    ///   0. `[signer, writable]` Staker
    ///   1. `[writable]` Stake PDA
    ///   2. `[]` System Program
    ///   3. `[]` Clock sysvar
    Stake { domain: String, amount: u64 },

    /// Begin unstaking — sets a 7-day cooldown (legacy V1).
    ///
    /// Accounts:
    ///   0. `[signer]` Staker (must match PDA staker)
    ///   1. `[writable]` Stake PDA
    ///   2. `[]` Clock sysvar
    Unstake { domain: String },

    /// Withdraw after cooldown — return lamports and close V1 PDA.
    ///
    /// Accounts:
    ///   0. `[signer, writable]` Staker
    ///   1. `[writable]` Stake PDA (V1: seeds ["stake", domain])
    ///   2. `[]` Clock sysvar
    WithdrawV1 { domain: String },

    // ── Phase 0: Foundation ──
    /// Initialize protocol config and $TRUTH mint.
    /// M7: min_epoch_duration defaults to 604800 (7 days).
    ///
    /// Accounts:
    ///   0. `[signer, writable]` Authority (pays for account creation)
    ///   1. `[writable]` Config PDA
    ///   2. `[writable]` $TRUTH Mint (PDA-derived or passed)
    ///   3. `[writable]` Reward Vault token account
    ///   4. `[]` Token Program
    ///   5. `[]` System Program
    ///   6. `[]` Rent sysvar
    InitializeProtocol {
        min_stake: u64,
        cooldown_secs: i64,
        dispute_bond: u64,
        slash_pct: u8,
        reward_rate: u64,
        jury_min_stake: u64,
    },

    /// Mint $TRUTH tokens (admin-only).
    ///
    /// Accounts:
    ///   0. `[signer]` Authority (must match config.authority)
    ///   1. `[writable]` Config PDA (total_minted updated for supply cap)
    ///   2. `[writable]` $TRUTH Mint
    ///   3. `[writable]` Destination token account
    ///   4. `[]` Token Program
    MintTruth { amount: u64 },

    // ── Phase 1: SPL Token Staking ──
    /// Stake $TRUTH for a domain.
    ///
    /// Accounts:
    ///   0. `[signer, writable]` Staker
    ///   1. `[writable]` Staker's $TRUTH token account
    ///   2. `[writable]` Program vault token account (reward vault)
    ///   3. `[writable]` StakeAccountV2 PDA
    ///   4. `[writable]` DomainRecord PDA
    ///   5. `[writable]` Config PDA (total_staked updated)
    ///   6. `[]` Token Program
    ///   7. `[]` System Program
    ///   8. `[]` Clock sysvar
    StakeV2 {
        domain: String,
        amount: u64,
        stake_type: u8, // 0=Publisher, 1=Curator
    },

    /// Begin unstaking V2 — sets cooldown and decrements total_staked.
    ///
    /// Accounts:
    ///   0. `[signer]` Staker
    ///   1. `[writable]` StakeAccountV2 PDA
    ///   2. `[writable]` Config PDA
    ///   3. `[]` DomainRecord PDA (S1: checked for active dispute)
    ///   4. `[]` Clock sysvar
    ///   5. `[]` GovernanceLockAccount PDA (S2: per-wallet governance lock)
    UnstakeV2 { domain: String },

    /// Withdraw after cooldown — transfer $TRUTH back and close PDA.
    /// Slashed tokens: 50% burned, 50% stays in vault as protocol revenue.
    ///
    /// Accounts:
    ///   0. `[signer, writable]` Staker
    ///   1. `[writable]` StakeAccountV2 PDA
    ///   2. `[writable]` Staker's $TRUTH token account
    ///   3. `[writable]` Program vault token account
    ///   4. `[writable]` DomainRecord PDA
    ///   5. `[writable]` Config PDA
    ///   6. `[]` Token Program
    ///   7. `[]` Clock sysvar
    ///   8. `[writable]` $TRUTH Mint (required — used for slash burn)
    Withdraw { domain: String },

    /// Claim accrued $TRUTH rewards.
    ///
    /// Accounts:
    ///   0. `[signer]` Staker
    ///   1. `[writable]` StakeAccountV2 PDA
    ///   2. `[writable]` $TRUTH Mint
    ///   3. `[writable]` Staker's $TRUTH token account
    ///   4. `[writable]` Config PDA (total_minted updated for supply cap)
    ///   5. `[]` Token Program
    ///   6. `[]` Clock sysvar (required for Curators, to enforce yield cap)
    ClaimReward { domain: String },

    // ── Phase 2: DNS & Domain Management ──
    /// Admin/oracle verifies DNS for a domain.
    ///
    /// Accounts:
    ///   0. `[signer]` Authority
    ///   1. `[writable]` DomainRecord PDA
    ///   2. `[]` Config PDA
    VerifyDns {
        domain: String,
        dns_wallet: [u8; 32],
    },

    /// Blacklist a domain.
    ///
    /// Accounts:
    ///   0. `[signer]` Blacklist authority
    ///   1. `[writable]` DomainRecord PDA
    ///   2. `[]` Config PDA
    BlacklistDomain { domain: String },

    /// Set or clear the UGC platform flag on a domain.
    /// When `is_ugc` is true, domain-level staking provides zero passthrough.
    ///
    /// Accounts:
    ///   0. `[signer]` Authority (must match config.authority)
    ///   1. `[writable]` DomainRecord PDA
    ///   2. `[]` Config PDA
    SetUgcPlatform { domain: String, is_ugc: bool },

    // ── Phase 3: Disputes & Slashing ──
    /// Open a dispute against a domain.
    ///
    /// Accounts:
    ///   0. `[signer, writable]` Reporter
    ///   1. `[writable]` Reporter's $TRUTH token account
    ///   2. `[writable]` Program vault token account
    ///   3. `[writable]` DisputeAccount PDA
    ///   4. `[writable]` DomainRecord PDA
    ///   5. `[]` Config PDA
    ///   6. `[]` Token Program
    ///   7. `[]` System Program
    ///   8. `[]` Clock sysvar
    ///   9. `[]` Reporter's SentinelAccount PDA (optional, for trusted reporter discount)
    OpenDispute {
        domain: String,
        reason_hash: [u8; 32],
        severity: u8, // 0=low, 1=medium, 2=high
    },

    /// Cast a jury vote on a dispute.
    ///
    /// Accounts:
    ///   0. `[signer, writable]` Voter
    ///   1. `[writable]` JuryVote PDA
    ///   2. `[]` DisputeAccount PDA
    ///   3. `[]` Voter's StakeAccountV2 PDA (for weight)
    ///   4. `[]` Config PDA
    ///   5. `[]` System Program
    ///   6. `[]` Clock sysvar
    CastJuryVote {
        domain: String,
        dispute_nonce: u64,
        verdict: u8, // 0=Guilty, 1=Innocent
    },

    /// Commit a hidden jury vote (commit-reveal scheme).
    /// commit_hash = SHA256(verdict || salt)
    ///
    /// Accounts:
    ///   0. `[signer, writable]` Voter
    ///   1. `[writable]` JuryVote PDA
    ///   2. `[]` DisputeAccount PDA
    ///   3. `[]` Voter's StakeAccountV2 PDA (for eligibility)
    ///   4. `[]` Config PDA
    ///   5. `[]` System Program
    ///   6. `[]` Clock sysvar
    CommitJuryVote {
        domain: String,
        dispute_nonce: u64,
        commit_hash: [u8; 32],
    },

    /// Reveal a previously committed jury vote.
    ///
    /// Accounts:
    ///   0. `[signer]` Voter
    ///   1. `[writable]` JuryVote PDA
    ///   2. `[writable]` DisputeAccount PDA
    ///   3. `[]` Config PDA
    ///   4. `[]` Clock sysvar
    RevealJuryVote {
        domain: String,
        dispute_nonce: u64,
        verdict: u8, // 0=Guilty, 1=Innocent
        salt: [u8; 32],
    },

    /// Resolve a dispute after deadline.
    ///
    /// Accounts:
    ///   0. `[signer, writable]` Caller (anyone)
    ///   1. `[writable]` DisputeAccount PDA
    ///   2. `[writable]` DomainRecord PDA
    ///   3. `[writable]` Reporter's $TRUTH token account
    ///   4. `[writable]` Program vault token account
    ///   5. `[writable]` $TRUTH Mint
    ///   6. `[]` Config PDA
    ///   7. `[]` Token Program
    ///   8. `[]` Clock sysvar
    ResolveDispute { domain: String, dispute_nonce: u64 },

    // ── Phase 4: Rewards ──
    /// Fund the reward vault with $TRUTH.
    ///
    /// Accounts:
    ///   0. `[signer]` Funder
    ///   1. `[writable]` Funder's $TRUTH token account
    ///   2. `[writable]` Reward vault token account
    ///   3. `[]` Token Program
    ///   4. `[writable]` Config PDA (total_funded updated)
    FundRewards { amount: u64 },

    /// Update the global reward index (permissionless).
    ///
    /// Accounts:
    ///   0. `[]` Caller (anyone — no longer requires authority)
    ///   1. `[writable]` Config PDA
    UpdateRewardIndex { funded_amount: u64 },

    // ── Phase 5: DAO Governance ──
    /// Create a governance proposal. If `proposal_deposit > 0`, accounts 5-7 are required.
    ///
    /// Accounts:
    ///   0. `[signer, writable]` Proposer
    ///   1. `[writable]` GovernanceProposal PDA
    ///   2. `[writable]` Config PDA
    ///   3. `[]` System Program
    ///   4. `[]` Clock sysvar
    ///   5. `[writable]` Proposer's $TRUTH token account (for deposit)
    ///   6. `[writable]` Program vault token account (receives deposit)
    ///   7. `[]` Token Program
    CreateProposal {
        description_hash: [u8; 32],
        param_key: String,
        param_value: u64,
    },

    /// Vote on a governance proposal.
    ///
    /// Accounts:
    ///   0. `[signer, writable]` Voter
    ///   1. `[writable]` GovernanceVote PDA
    ///   2. `[writable]` GovernanceProposal PDA
    ///   3. `[]` Voter's StakeAccountV2 PDA (read-only, for weight)
    ///   4. `[]` System Program
    ///   5. `[]` Clock sysvar
    ///   6. `[writable]` GovernanceLockAccount PDA (S2: per-wallet lock)
    CastGovernanceVote {
        proposal_nonce: u64,
        vote: u8, // 0=For, 1=Against
    },

    /// Execute a passed proposal. If proposal has deposit and quorum met,
    /// additional accounts are needed for refund.
    ///
    /// Accounts:
    ///   0. `[signer]` Caller (anyone)
    ///   1. `[writable]` GovernanceProposal PDA
    ///   2. `[writable]` Config PDA
    ///   3. `[]` Clock sysvar
    ///   4. `[writable]` Depositor's $TRUTH token account (optional, for deposit refund)
    ///   5. `[writable]` Program vault token account (optional, for deposit refund)
    ///   6. `[]` Token Program (optional, for deposit refund)
    ExecuteProposal { proposal_nonce: u64 },

    // ── Phase 6: State Channels ──
    /// Open a state channel with $TRUTH deposit.
    ///
    /// Accounts:
    ///   0. `[signer, writable]` Payer
    ///   1. `[writable]` Payer's $TRUTH token account
    ///   2. `[writable]` Program vault token account
    ///   3. `[writable]` StateChannelAccount PDA
    ///   4. `[]` Token Program
    ///   5. `[]` System Program
    ///   6. `[]` Clock sysvar
    ///   7. `[]` Config PDA (for vault validation)
    OpenChannel {
        payee: [u8; 32],
        amount: u64,
        expiry_slot: u64,
    },

    /// Settle a state channel with highest-nonce ticket.
    ///
    /// Accounts:
    ///   0. `[signer]` Submitter (payer or payee)
    ///   1. `[writable]` StateChannelAccount PDA
    ///   2. `[writable]` Payee's $TRUTH token account
    ///   3. `[writable]` Payer's $TRUTH token account
    ///   4. `[writable]` Program vault token account
    ///   5. `[writable]` Config PDA (accumulated_sentinel_fees updated)
    ///   6. `[]` Token Program
    ///   7. `[writable]` $TRUTH Mint (required — used for burning spent fees)
    ///   8. `[]` Instructions sysvar (required, for Ed25519 signature verification)
    SettleChannel {
        payer_key: [u8; 32],
        payee_key: [u8; 32],
        spent: u64,
        nonce: u64,
    },

    // ── Phase 7: Sentinel Nodes ──
    /// Stake $TRUTH globally as a Sentinel (crawler node).
    ///
    /// Accounts:
    ///   0. `[signer, writable]` Sentinel
    ///   1. `[writable]` Sentinel's $TRUTH token account
    ///   2. `[writable]` Program vault token account
    ///   3. `[writable]` SentinelAccount PDA
    ///   4. `[writable]` Config PDA
    ///   5. `[]` Token Program
    ///   6. `[]` System Program
    ///   7. `[]` Clock sysvar
    StakeSentinel { amount: u64 },

    /// Begin unstaking as a Sentinel (7-day cooldown).
    /// Decrements total_sentinel_staked at unstake time.
    ///
    /// Accounts:
    ///   0. `[signer]` Sentinel
    ///   1. `[writable]` SentinelAccount PDA
    ///   2. `[writable]` Config PDA
    ///   3. `[]` Clock sysvar
    UnstakeSentinel,

    /// Claim accrued Sentinel rewards.
    ///
    /// Accounts:
    ///   0. `[signer]` Sentinel
    ///   1. `[writable]` SentinelAccount PDA
    ///   2. `[writable]` $TRUTH Mint
    ///   3. `[writable]` Sentinel's $TRUTH token account
    ///   4. `[writable]` Config PDA (total_minted updated for supply cap)
    ///   5. `[]` Token Program
    ClaimSentinelReward,

    /// Withdraw staked $TRUTH after Sentinel cooldown expires.
    /// Slashed tokens: 50% burned, 50% stays in vault as protocol revenue.
    ///
    /// Accounts:
    ///   0. `[signer, writable]` Sentinel
    ///   1. `[writable]` SentinelAccount PDA
    ///   2. `[writable]` Sentinel's $TRUTH token account
    ///   3. `[writable]` Program vault token account
    ///   4. `[writable]` Config PDA
    ///   5. `[]` Token Program
    ///   6. `[]` Clock sysvar
    ///   7. `[writable]` $TRUTH Mint (required — used for slash burn)
    WithdrawSentinel,

    /// Advance to the next emission epoch (admin).
    ///
    /// Accounts:
    ///   0. `[signer]` Authority
    ///   1. `[writable]` Config PDA
    ///   2. `[]` Clock sysvar (required when min_epoch_duration > 0)
    AdvanceEpoch,

    /// Slash a sentinel's staked amount (admin only).
    ///
    /// Accounts:
    ///   0. `[signer]` Authority (must match config.authority)
    ///   1. `[writable]` SentinelAccount PDA
    ///   2. `[]` Config PDA
    SlashSentinel { sentinel: [u8; 32], amount: u64 },

    /// Claim jury reward for a resolved dispute (winning side).
    /// Mints flat jury_vote_reward from supply (no proportional pool share).
    ///
    /// Accounts:
    ///   0. `[signer]` Voter
    ///   1. `[writable]` JuryVote PDA
    ///   2. `[writable]` DisputeAccount PDA
    ///   3. `[writable]` Voter's $TRUTH token account
    ///   4. `[writable]` $TRUTH Mint
    ///   5. `[writable]` Config PDA (total_minted updated)
    ///   6. `[]` Token Program
    ClaimJuryReward { domain: String, dispute_nonce: u64 },

    // ── Reputation ──
    /// Commit a Merkle root of domain reputation scores on-chain.
    /// Only the protocol authority can submit, and epoch must be monotonically increasing.
    ///
    /// Accounts:
    ///   0. `[signer]` Authority (must match config.authority)
    ///   1. `[writable]` ReputationRootAccount PDA
    ///   2. `[]` Config PDA
    ///   3. `[]` System Program
    CommitReputationRoot { merkle_root: [u8; 32], epoch: u64 },
}
