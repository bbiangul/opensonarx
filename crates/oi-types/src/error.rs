use thiserror::Error;

/// CLI exit codes from Tech Addendum Section 1
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum ExitCode {
    Success = 0,
    UserError = 1,
    NetworkError = 2,
    InsufficientFunds = 3,
    Unauthorized = 4,
    DisputeConflict = 5,
}

/// Error codes from Tech Addendum Section 5.1
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorCode {
    // E1000-E1004: Query
    QueryNoResults,
    QueryTimeout,
    QueryShardUnavailable,
    QueryPartialResults,
    QueryInvalidFilter,

    // E2000-E2003: Payment
    InsufficientBalance,
    TicketRejected,
    TicketExpired,
    SettlementFailed,

    // E3000-E3004: Stake
    DomainNotVerified,
    DomainAlreadyStaked,
    UnstakeCooldownActive,
    AmountBelowMinimum,
    DomainBlacklisted,

    // E3005-E3009: Commit-Reveal Voting
    CommitPhaseExpired,
    RevealPhasePending,
    InvalidCommitHash,
    PrematureReveal,
    CommitAlreadySubmitted,

    // E4000-E4003: Dispute
    DisputeAlreadyActive,
    BondInsufficient,
    ReporterCooldown,
    JuryNotEligible,

    // E5000-E5002: Network
    NoPeers,
    DhtLookupFailed,
    NodeUnreachable,

    // E6000-E6002: Auth
    InvalidWallet,
    InvalidApiKey,
    SignatureInvalid,

    // E6003-E6007: Gateway
    QuotaExceeded,
    SessionExpired,
    MagicLinkExpired,
    EmailNotVerified,
    FreeTierExhausted,

    // E6008-E6012: Fiat Bridge
    StripePaymentFailed,
    FiatBondNotFound,
    FiatBondAlreadyResolved,
    DisputeNotResolvable,
    FiatAmountMismatch,
}

impl ErrorCode {
    pub fn as_u32(self) -> u32 {
        match self {
            Self::QueryNoResults => 1000,
            Self::QueryTimeout => 1001,
            Self::QueryShardUnavailable => 1002,
            Self::QueryPartialResults => 1003,
            Self::QueryInvalidFilter => 1004,
            Self::InsufficientBalance => 2000,
            Self::TicketRejected => 2001,
            Self::TicketExpired => 2002,
            Self::SettlementFailed => 2003,
            Self::DomainNotVerified => 3000,
            Self::DomainAlreadyStaked => 3001,
            Self::UnstakeCooldownActive => 3002,
            Self::AmountBelowMinimum => 3003,
            Self::DomainBlacklisted => 3004,
            Self::CommitPhaseExpired => 3005,
            Self::RevealPhasePending => 3006,
            Self::InvalidCommitHash => 3007,
            Self::PrematureReveal => 3008,
            Self::CommitAlreadySubmitted => 3009,
            Self::DisputeAlreadyActive => 4000,
            Self::BondInsufficient => 4001,
            Self::ReporterCooldown => 4002,
            Self::JuryNotEligible => 4003,
            Self::NoPeers => 5000,
            Self::DhtLookupFailed => 5001,
            Self::NodeUnreachable => 5002,
            Self::InvalidWallet => 6000,
            Self::InvalidApiKey => 6001,
            Self::SignatureInvalid => 6002,
            Self::QuotaExceeded => 6003,
            Self::SessionExpired => 6004,
            Self::MagicLinkExpired => 6005,
            Self::EmailNotVerified => 6006,
            Self::FreeTierExhausted => 6007,
            Self::StripePaymentFailed => 6008,
            Self::FiatBondNotFound => 6009,
            Self::FiatBondAlreadyResolved => 6010,
            Self::DisputeNotResolvable => 6011,
            Self::FiatAmountMismatch => 6012,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::QueryNoResults => "QUERY_NO_RESULTS",
            Self::QueryTimeout => "QUERY_TIMEOUT",
            Self::QueryShardUnavailable => "QUERY_SHARD_UNAVAILABLE",
            Self::QueryPartialResults => "QUERY_PARTIAL_RESULTS",
            Self::QueryInvalidFilter => "QUERY_INVALID_FILTER",
            Self::InsufficientBalance => "INSUFFICIENT_BALANCE",
            Self::TicketRejected => "TICKET_REJECTED",
            Self::TicketExpired => "TICKET_EXPIRED",
            Self::SettlementFailed => "SETTLEMENT_FAILED",
            Self::DomainNotVerified => "DOMAIN_NOT_VERIFIED",
            Self::DomainAlreadyStaked => "DOMAIN_ALREADY_STAKED",
            Self::UnstakeCooldownActive => "UNSTAKE_COOLDOWN_ACTIVE",
            Self::AmountBelowMinimum => "AMOUNT_BELOW_MINIMUM",
            Self::DomainBlacklisted => "DOMAIN_BLACKLISTED",
            Self::CommitPhaseExpired => "COMMIT_PHASE_EXPIRED",
            Self::RevealPhasePending => "REVEAL_PHASE_PENDING",
            Self::InvalidCommitHash => "INVALID_COMMIT_HASH",
            Self::PrematureReveal => "PREMATURE_REVEAL",
            Self::CommitAlreadySubmitted => "COMMIT_ALREADY_SUBMITTED",
            Self::DisputeAlreadyActive => "DISPUTE_ALREADY_ACTIVE",
            Self::BondInsufficient => "BOND_INSUFFICIENT",
            Self::ReporterCooldown => "REPORTER_COOLDOWN",
            Self::JuryNotEligible => "JURY_NOT_ELIGIBLE",
            Self::NoPeers => "NO_PEERS",
            Self::DhtLookupFailed => "DHT_LOOKUP_FAILED",
            Self::NodeUnreachable => "NODE_UNREACHABLE",
            Self::InvalidWallet => "INVALID_WALLET",
            Self::InvalidApiKey => "INVALID_API_KEY",
            Self::SignatureInvalid => "SIGNATURE_INVALID",
            Self::QuotaExceeded => "QUOTA_EXCEEDED",
            Self::SessionExpired => "SESSION_EXPIRED",
            Self::MagicLinkExpired => "MAGIC_LINK_EXPIRED",
            Self::EmailNotVerified => "EMAIL_NOT_VERIFIED",
            Self::FreeTierExhausted => "FREE_TIER_EXHAUSTED",
            Self::StripePaymentFailed => "STRIPE_PAYMENT_FAILED",
            Self::FiatBondNotFound => "FIAT_BOND_NOT_FOUND",
            Self::FiatBondAlreadyResolved => "FIAT_BOND_ALREADY_RESOLVED",
            Self::DisputeNotResolvable => "DISPUTE_NOT_RESOLVABLE",
            Self::FiatAmountMismatch => "FIAT_AMOUNT_MISMATCH",
        }
    }

    /// CLI exit code for this error
    pub fn exit_code(self) -> ExitCode {
        match self {
            Self::QueryNoResults => ExitCode::Success,
            Self::QueryTimeout => ExitCode::NetworkError,
            Self::QueryShardUnavailable => ExitCode::NetworkError,
            Self::QueryPartialResults => ExitCode::Success,
            Self::QueryInvalidFilter => ExitCode::UserError,
            Self::InsufficientBalance => ExitCode::InsufficientFunds,
            Self::TicketRejected => ExitCode::NetworkError,
            Self::TicketExpired => ExitCode::NetworkError,
            Self::SettlementFailed => ExitCode::NetworkError,
            Self::DomainNotVerified => ExitCode::UserError,
            Self::DomainAlreadyStaked => ExitCode::UserError,
            Self::UnstakeCooldownActive => ExitCode::UserError,
            Self::AmountBelowMinimum => ExitCode::UserError,
            Self::DomainBlacklisted => ExitCode::UserError,
            Self::CommitPhaseExpired => ExitCode::UserError,
            Self::RevealPhasePending => ExitCode::UserError,
            Self::InvalidCommitHash => ExitCode::UserError,
            Self::PrematureReveal => ExitCode::UserError,
            Self::CommitAlreadySubmitted => ExitCode::UserError,
            Self::DisputeAlreadyActive => ExitCode::UserError,
            Self::BondInsufficient => ExitCode::InsufficientFunds,
            Self::ReporterCooldown => ExitCode::UserError,
            Self::JuryNotEligible => ExitCode::UserError,
            Self::NoPeers => ExitCode::NetworkError,
            Self::DhtLookupFailed => ExitCode::NetworkError,
            Self::NodeUnreachable => ExitCode::NetworkError,
            Self::InvalidWallet => ExitCode::Unauthorized,
            Self::InvalidApiKey => ExitCode::Unauthorized,
            Self::SignatureInvalid => ExitCode::Unauthorized,
            Self::QuotaExceeded => ExitCode::UserError,
            Self::SessionExpired => ExitCode::Unauthorized,
            Self::MagicLinkExpired => ExitCode::UserError,
            Self::EmailNotVerified => ExitCode::UserError,
            Self::FreeTierExhausted => ExitCode::NetworkError,
            Self::StripePaymentFailed => ExitCode::UserError,
            Self::FiatBondNotFound => ExitCode::UserError,
            Self::FiatBondAlreadyResolved => ExitCode::UserError,
            Self::DisputeNotResolvable => ExitCode::UserError,
            Self::FiatAmountMismatch => ExitCode::UserError,
        }
    }

    /// Whether the agent should retry this error
    pub fn retryable(self) -> bool {
        matches!(
            self,
            Self::QueryTimeout
                | Self::QueryShardUnavailable
                | Self::TicketRejected
                | Self::TicketExpired
                | Self::SettlementFailed
                | Self::NoPeers
                | Self::DhtLookupFailed
                | Self::NodeUnreachable
                | Self::FreeTierExhausted
        )
    }

    pub fn from_u32(code: u32) -> Option<Self> {
        match code {
            1000 => Some(Self::QueryNoResults),
            1001 => Some(Self::QueryTimeout),
            1002 => Some(Self::QueryShardUnavailable),
            1003 => Some(Self::QueryPartialResults),
            1004 => Some(Self::QueryInvalidFilter),
            2000 => Some(Self::InsufficientBalance),
            2001 => Some(Self::TicketRejected),
            2002 => Some(Self::TicketExpired),
            2003 => Some(Self::SettlementFailed),
            3000 => Some(Self::DomainNotVerified),
            3001 => Some(Self::DomainAlreadyStaked),
            3002 => Some(Self::UnstakeCooldownActive),
            3003 => Some(Self::AmountBelowMinimum),
            3004 => Some(Self::DomainBlacklisted),
            3005 => Some(Self::CommitPhaseExpired),
            3006 => Some(Self::RevealPhasePending),
            3007 => Some(Self::InvalidCommitHash),
            3008 => Some(Self::PrematureReveal),
            3009 => Some(Self::CommitAlreadySubmitted),
            4000 => Some(Self::DisputeAlreadyActive),
            4001 => Some(Self::BondInsufficient),
            4002 => Some(Self::ReporterCooldown),
            4003 => Some(Self::JuryNotEligible),
            5000 => Some(Self::NoPeers),
            5001 => Some(Self::DhtLookupFailed),
            5002 => Some(Self::NodeUnreachable),
            6000 => Some(Self::InvalidWallet),
            6001 => Some(Self::InvalidApiKey),
            6002 => Some(Self::SignatureInvalid),
            6003 => Some(Self::QuotaExceeded),
            6004 => Some(Self::SessionExpired),
            6005 => Some(Self::MagicLinkExpired),
            6006 => Some(Self::EmailNotVerified),
            6007 => Some(Self::FreeTierExhausted),
            6008 => Some(Self::StripePaymentFailed),
            6009 => Some(Self::FiatBondNotFound),
            6010 => Some(Self::FiatBondAlreadyResolved),
            6011 => Some(Self::DisputeNotResolvable),
            6012 => Some(Self::FiatAmountMismatch),
            _ => None,
        }
    }
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "E{}", self.as_u32())
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("{code}: {message}")]
    Protocol { code: ErrorCode, message: String },

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl Error {
    pub fn protocol(code: ErrorCode, message: impl Into<String>) -> Self {
        Self::Protocol {
            code,
            message: message.into(),
        }
    }

    pub fn code(&self) -> Option<ErrorCode> {
        match self {
            Self::Protocol { code, .. } => Some(*code),
            _ => None,
        }
    }

    pub fn exit_code(&self) -> ExitCode {
        match self {
            Self::Protocol { code, .. } => code.exit_code(),
            Self::Io(_) => ExitCode::NetworkError,
            Self::Serialization(_) => ExitCode::UserError,
            Self::Internal(_) => ExitCode::UserError,
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_code_roundtrip() {
        for code in [
            ErrorCode::QueryNoResults,
            ErrorCode::QueryTimeout,
            ErrorCode::InsufficientBalance,
            ErrorCode::DomainNotVerified,
            ErrorCode::DisputeAlreadyActive,
            ErrorCode::NoPeers,
            ErrorCode::InvalidWallet,
        ] {
            let n = code.as_u32();
            assert_eq!(ErrorCode::from_u32(n), Some(code));
        }
    }

    #[test]
    fn test_error_code_names() {
        assert_eq!(ErrorCode::QueryTimeout.name(), "QUERY_TIMEOUT");
        assert_eq!(ErrorCode::NoPeers.name(), "NO_PEERS");
        assert_eq!(format!("{}", ErrorCode::QueryTimeout), "E1001");
    }

    #[test]
    fn test_exit_codes() {
        assert_eq!(ErrorCode::QueryNoResults.exit_code(), ExitCode::Success);
        assert_eq!(ErrorCode::QueryTimeout.exit_code(), ExitCode::NetworkError);
        assert_eq!(ErrorCode::InsufficientBalance.exit_code(), ExitCode::InsufficientFunds);
        assert_eq!(ErrorCode::InvalidWallet.exit_code(), ExitCode::Unauthorized);
        assert_eq!(ErrorCode::QueryInvalidFilter.exit_code(), ExitCode::UserError);
    }

    #[test]
    fn test_retryable() {
        assert!(ErrorCode::QueryTimeout.retryable());
        assert!(ErrorCode::NoPeers.retryable());
        assert!(!ErrorCode::QueryNoResults.retryable());
        assert!(!ErrorCode::DomainNotVerified.retryable());
    }

    #[test]
    fn test_all_codes_covered() {
        // Verify all error code ranges exist
        assert!(ErrorCode::from_u32(1000).is_some());
        assert!(ErrorCode::from_u32(2000).is_some());
        assert!(ErrorCode::from_u32(3000).is_some());
        assert!(ErrorCode::from_u32(4000).is_some());
        assert!(ErrorCode::from_u32(5000).is_some());
        assert!(ErrorCode::from_u32(6000).is_some());
        assert!(ErrorCode::from_u32(6003).is_some());
        assert!(ErrorCode::from_u32(6007).is_some());
        assert!(ErrorCode::from_u32(6008).is_some());
        assert!(ErrorCode::from_u32(6012).is_some());
        assert!(ErrorCode::from_u32(9999).is_none());
    }
}
