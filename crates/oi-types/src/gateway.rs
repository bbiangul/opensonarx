use serde::{Deserialize, Serialize};

/// User tier for gateway access.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    #[default]
    Free,
    Pro,
    Enterprise,
}

impl Tier {
    pub fn as_str(&self) -> &'static str {
        match self {
            Tier::Free => "free",
            Tier::Pro => "pro",
            Tier::Enterprise => "enterprise",
        }
    }

    pub fn from_db_str(s: &str) -> Self {
        match s {
            "pro" => Tier::Pro,
            "enterprise" => Tier::Enterprise,
            _ => Tier::Free,
        }
    }

    pub fn default_query_limit(&self) -> u32 {
        match self {
            Tier::Free => 100,
            Tier::Pro => 10_000,
            Tier::Enterprise => 100_000,
        }
    }

    pub fn rate_limit_per_minute(&self) -> u32 {
        match self {
            Tier::Free => 20,
            Tier::Pro => 200,
            Tier::Enterprise => 2_000,
        }
    }
}

/// A web2 user account managed by the gateway.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserAccount {
    pub user_id: String,
    pub email: String,
    pub tier: Tier,
    #[serde(default)]
    pub stripe_customer_id: Option<String>,
    pub created_at: i64,
    pub last_login: i64,
}

/// Information about an API key issued by the gateway.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyInfo {
    pub key_id: String,
    pub key_prefix: String,
    pub user_id: String,
    pub created_at: i64,
    pub active: bool,
}

/// Tracks daily query usage against the free-tier quota.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryQuota {
    pub user_id: String,
    pub tier: Tier,
    pub queries_today: u32,
    pub max_queries_per_day: u32,
    pub resets_at: i64,
}

impl QueryQuota {
    /// Number of queries remaining for today.
    pub fn remaining(&self) -> u32 {
        self.max_queries_per_day.saturating_sub(self.queries_today)
    }

    /// Whether the daily quota has been exceeded.
    pub fn is_exceeded(&self) -> bool {
        self.queries_today >= self.max_queries_per_day
    }
}

/// Cumulative credit balance for accounting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreditBalance {
    pub user_id: String,
    pub total_queries: u64,
    pub total_cost_micro_usd: u64,
}

/// Request body for the magic link login endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MagicLinkRequest {
    pub email: String,
}

/// Response from the magic link request endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MagicLinkResponse {
    pub status: String,
    pub message: String,
}

/// Request body for verifying a magic link token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyTokenRequest {
    pub token: String,
}

/// Response after successful magic link verification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthResponse {
    pub api_key: String,
    pub session_token: String,
    pub user: UserAccount,
    pub quota: QueryQuota,
}

/// Search request sent to the gateway.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewaySearchRequest {
    pub query: String,
    #[serde(default = "default_top_k")]
    pub top_k: usize,
    #[serde(default)]
    pub min_stake: Option<u32>,
    #[serde(default)]
    pub freshness: Option<String>,
    #[serde(default)]
    pub types: Vec<String>,
    /// Locale hint for external engines (e.g., "en", "pt-BR")
    #[serde(default)]
    pub locale: Option<String>,
    /// Country/region code for geo-scoped search (e.g., "us", "br")
    #[serde(default)]
    pub gl: Option<String>,
    /// Domain scope filter (e.g., "nytimes.com")
    #[serde(default)]
    pub site: Option<String>,
    /// Minimum trust verdict to include (e.g. "trusted", "highly_trusted").
    /// Stake/reputation inform this label but never the ranking score.
    #[serde(default)]
    pub min_trust: Option<String>,
    /// Optional field selection. Only these fields included in results.
    /// Always-included: result_id, rank, url. Default (None): all fields.
    #[serde(default)]
    pub fields: Option<Vec<String>>,
    /// When true and local results are empty, synchronously crawl external URLs
    /// before returning. Adds latency but guarantees content_relevant is populated.
    #[serde(default)]
    pub wait_for_content: Option<bool>,
}

fn default_top_k() -> usize {
    5
}

/// Search response from the gateway.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewaySearchResponse {
    pub results: Vec<crate::SearchResultItem>,
    pub meta: GatewayResponseMeta,
}

/// Metadata attached to every gateway response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayResponseMeta {
    pub query_ms: f64,
    pub quota_remaining: u32,
    pub mode: String,
    /// Which search sources contributed results (e.g., ["local", "duckduckgo", "serper"])
    #[serde(default)]
    pub sources_used: Vec<String>,
}

/// Result of a trust/stake verification for a URL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustCheckResult {
    pub url: String,
    pub domain: String,
    pub is_staked: bool,
    pub stake_info: Option<crate::StakeInfo>,
    pub trust_score: f64,
    pub verdict: String,
}

/// Bond status lifecycle: Held -> Released | Captured | Cancelled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FiatBondStatus {
    Held,
    Released,
    Captured,
    Cancelled,
}

impl FiatBondStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Held => "held",
            Self::Released => "released",
            Self::Captured => "captured",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn from_db_str(s: &str) -> Self {
        match s {
            "released" => Self::Released,
            "captured" => Self::Captured,
            "cancelled" => Self::Cancelled,
            _ => Self::Held,
        }
    }
}

/// A fiat bond record tracking a Stripe hold paired with an on-chain $TRUTH stake.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FiatBond {
    pub bond_id: String,
    pub user_id: String,
    pub domain: String,
    pub dispute_nonce: Option<u64>,
    pub amount_cents: u64,
    pub bond_truth_lamports: u64,
    pub stripe_payment_intent_id: String,
    pub status: FiatBondStatus,
    pub created_at: i64,
    pub resolved_at: Option<i64>,
}

/// Request to open a fiat-backed dispute.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FiatDisputeOpenRequest {
    pub domain: String,
    pub reason: String,
    pub payment_method_id: String,
}

/// Response after opening a fiat-backed dispute.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FiatDisputeOpenResponse {
    pub bond_id: String,
    pub domain: String,
    pub amount_cents: u64,
    pub bond_truth_lamports: u64,
    pub stripe_client_secret: Option<String>,
    pub status: String,
}

/// List of disputes returned by the gateway.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayDisputeListResponse {
    pub disputes: Vec<crate::DisputeInfo>,
}

/// Dispute info with optional fiat bond details.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayDisputeInfoResponse {
    pub dispute: crate::DisputeInfo,
    pub fiat_bond: Option<FiatBond>,
}

/// Outcome of a fiat bond resolution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "outcome")]
pub enum FiatBondOutcome {
    ReporterWon { reward_cents: u64 },
    ReporterLost,
}

/// Request to resolve a fiat bond after dispute settlement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FiatBondResolutionRequest {
    pub bond_id: String,
    pub outcome: FiatBondOutcome,
}

// ---------------------------------------------------------------------------
// Agent Feedback Data Moat types
// ---------------------------------------------------------------------------

/// A single feedback signal recording whether a search result solved a problem.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedbackSignal {
    pub signal_id: String,
    pub user_id: String,
    pub query_text: String,
    pub query_hash: String,
    pub result_id: String,
    pub result_url: String,
    pub result_rank: u32,
    pub useful: bool,
    pub created_at: i64,
    #[serde(default)]
    pub signature_verified: bool,
    #[serde(default)]
    pub sender_pubkey: String,
}

/// Aggregated feedback statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedbackStats {
    pub total_signals: u64,
    pub positive_signals: u64,
    pub negative_signals: u64,
    pub unique_queries: u64,
    pub top_helpful_domains: Vec<DomainFeedbackStat>,
}

/// Per-domain feedback aggregation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainFeedbackStat {
    pub domain: String,
    pub helpful_count: u64,
    pub unhelpful_count: u64,
    pub score: f64,
}

/// Rebate issued for providing feedback.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedbackRebate {
    pub rebate_id: String,
    pub rebate_type: String,
    pub amount: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quota_remaining() {
        let quota = QueryQuota {
            user_id: "u1".into(),
            tier: Tier::Free,
            queries_today: 42,
            max_queries_per_day: 100,
            resets_at: 0,
        };
        assert_eq!(quota.remaining(), 58);
        assert!(!quota.is_exceeded());
    }

    #[test]
    fn test_quota_exceeded() {
        let quota = QueryQuota {
            user_id: "u1".into(),
            tier: Tier::Free,
            queries_today: 100,
            max_queries_per_day: 100,
            resets_at: 0,
        };
        assert_eq!(quota.remaining(), 0);
        assert!(quota.is_exceeded());
    }

    #[test]
    fn test_tier_default() {
        assert_eq!(Tier::default(), Tier::Free);
    }

    #[test]
    fn test_tier_methods() {
        assert_eq!(Tier::Free.as_str(), "free");
        assert_eq!(Tier::Pro.default_query_limit(), 10_000);
        assert_eq!(Tier::Enterprise.rate_limit_per_minute(), 2_000);
        assert_eq!(Tier::from_db_str("pro"), Tier::Pro);
        assert_eq!(Tier::from_db_str("unknown"), Tier::Free);
    }

    #[test]
    fn test_fiat_bond_status_methods() {
        assert_eq!(FiatBondStatus::Held.as_str(), "held");
        assert_eq!(FiatBondStatus::from_db_str("released"), FiatBondStatus::Released);
        assert_eq!(FiatBondStatus::from_db_str("unknown"), FiatBondStatus::Held);
    }

    #[test]
    fn test_gateway_types_roundtrip() {
        let req = GatewaySearchRequest {
            query: "rust async".into(),
            top_k: 10,
            min_stake: Some(100),
            freshness: None,
            types: vec!["text".into()],
            locale: None,
            gl: None,
            site: None,
            min_trust: Some("trusted".into()),
            fields: None,
            wait_for_content: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        let decoded: GatewaySearchRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.query, "rust async");
        assert_eq!(decoded.top_k, 10);
        assert_eq!(decoded.min_trust.as_deref(), Some("trusted"));
    }

    #[test]
    fn test_magic_link_roundtrip() {
        let req = MagicLinkRequest {
            email: "dev@example.com".into(),
        };
        let json = serde_json::to_string(&req).unwrap();
        let decoded: MagicLinkRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.email, "dev@example.com");
    }

    #[test]
    fn test_fiat_bond_roundtrip() {
        let bond = FiatBond {
            bond_id: "bond_123".into(),
            user_id: "u1".into(),
            domain: "spam.com".into(),
            dispute_nonce: Some(42),
            amount_cents: 1000,
            bond_truth_lamports: 10_000_000,
            stripe_payment_intent_id: "pi_test".into(),
            status: FiatBondStatus::Held,
            created_at: 1700000000,
            resolved_at: None,
        };
        let json = serde_json::to_string(&bond).unwrap();
        let decoded: FiatBond = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.bond_id, "bond_123");
        assert_eq!(decoded.status, FiatBondStatus::Held);
        assert_eq!(decoded.amount_cents, 1000);
    }

    #[test]
    fn test_fiat_bond_outcome_roundtrip() {
        let won = FiatBondOutcome::ReporterWon { reward_cents: 500 };
        let json = serde_json::to_string(&won).unwrap();
        let decoded: FiatBondOutcome = serde_json::from_str(&json).unwrap();
        match decoded {
            FiatBondOutcome::ReporterWon { reward_cents } => assert_eq!(reward_cents, 500),
            _ => panic!("Expected ReporterWon"),
        }

        let lost = FiatBondOutcome::ReporterLost;
        let json = serde_json::to_string(&lost).unwrap();
        let decoded: FiatBondOutcome = serde_json::from_str(&json).unwrap();
        assert!(matches!(decoded, FiatBondOutcome::ReporterLost));
    }

    #[test]
    fn test_trust_check_result_roundtrip() {
        let result = TrustCheckResult {
            url: "https://example.com/page".into(),
            domain: "example.com".into(),
            is_staked: true,
            stake_info: None,
            trust_score: 0.95,
            verdict: "trusted".into(),
        };
        let json = serde_json::to_string(&result).unwrap();
        let decoded: TrustCheckResult = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.verdict, "trusted");
        assert!(decoded.is_staked);
    }

    #[test]
    fn test_user_account_with_stripe() {
        let user = UserAccount {
            user_id: "u1".into(),
            email: "test@example.com".into(),
            tier: Tier::Pro,
            stripe_customer_id: Some("cus_123".into()),
            created_at: 1700000000,
            last_login: 1700000000,
        };
        let json = serde_json::to_string(&user).unwrap();
        let decoded: UserAccount = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.stripe_customer_id, Some("cus_123".into()));
    }

    #[test]
    fn test_feedback_signal_roundtrip() {
        let signal = FeedbackSignal {
            signal_id: "sig_1".into(),
            user_id: "u1".into(),
            query_text: "how to fix rust error".into(),
            query_hash: "abc123".into(),
            result_id: "r1".into(),
            result_url: "https://example.com/fix".into(),
            result_rank: 3,
            useful: true,
            created_at: 1700000000,
            signature_verified: false,
            sender_pubkey: String::new(),
        };
        let json = serde_json::to_string(&signal).unwrap();
        let decoded: FeedbackSignal = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.signal_id, "sig_1");
        assert!(decoded.useful);
    }
}
