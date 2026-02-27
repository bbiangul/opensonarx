use solana_program::program_error::ProgramError;
use thiserror::Error;

#[derive(Error, Debug, Clone, Copy)]
pub enum StakingError {
    #[error("Domain not verified")]
    DomainNotVerified, // E3000

    #[error("Domain already staked by this wallet")]
    DomainAlreadyStaked, // E3001

    #[error("Cooldown still active")]
    CooldownActive, // E3002

    #[error("Below minimum stake")]
    BelowMinimum, // E3003

    #[error("Domain blacklisted")]
    DomainBlacklisted, // E3004

    #[error("Dispute already active")]
    DisputeAlreadyActive, // E4000

    #[error("Bond insufficient")]
    BondInsufficient, // E4001

    #[error("Reporter in cooldown")]
    ReporterCooldown, // E4002

    #[error("Jury not eligible")]
    JuryNotEligible, // E4003

    #[error("Unauthorized")]
    Unauthorized,

    #[error("Cooldown not expired")]
    CooldownNotExpired,

    #[error("Dispute not resolved")]
    DisputeNotResolved,

    #[error("Voting period active")]
    VotingPeriodActive,

    #[error("Quorum not met")]
    QuorumNotMet,

    #[error("Already voted")]
    AlreadyVoted,

    #[error("Conflict of interest")]
    ConflictOfInterest,

    #[error("Invalid nonce")]
    InvalidNonce,

    #[error("Voting period ended")]
    VotingPeriodEnded,

    #[error("Account already initialized")]
    AlreadyInitialized,

    #[error("Invalid stake type")]
    InvalidStakeType,

    #[error("Arithmetic overflow")]
    Overflow,

    #[error("Channel expired")]
    ChannelExpired,

    #[error("Dispute deadline not reached")]
    DisputeDeadlineNotReached,

    #[error("Commit phase expired")]
    CommitPhaseExpired,

    #[error("Reveal phase not started")]
    RevealPhasePending,

    #[error("Invalid commit hash")]
    InvalidCommitHash,

    #[error("Premature reveal")]
    PrematureReveal,

    #[error("Commit already submitted")]
    CommitAlreadySubmitted,

    #[error("Total supply cap exceeded")]
    SupplyCapExceeded,

    #[error("Minimum epoch duration not reached")]
    EpochTooEarly,

    #[error("Dispute jury quorum not met")]
    DisputeQuorumNotMet,

    #[error("Governance timelock not expired")]
    TimelockActive,

    #[error("Invalid Ed25519 signature")]
    InvalidSignature,

    #[error("Insufficient proposal deposit")]
    InsufficientProposalDeposit,

    #[error("Governance parameter out of range")]
    ParamOutOfRange,

    #[error("Jury reward already claimed")]
    JuryRewardAlreadyClaimed,

    #[error("No jury reward available")]
    NoJuryReward,

    #[error("Unauthorized reputation root submission")]
    UnauthorizedReputationRoot,

    #[error("Stale epoch — must be monotonically increasing")]
    StaleEpoch,

    #[error("Governance lock not yet expired")]
    GovernanceLockActive,

    #[error("Jury reward budget exhausted for this epoch")]
    JuryRewardBudgetExhausted,

    #[error("Stake too young for governance voting")]
    StakeTooYoungForGovernance,
}

impl From<StakingError> for ProgramError {
    fn from(e: StakingError) -> Self {
        ProgramError::Custom(e as u32)
    }
}
