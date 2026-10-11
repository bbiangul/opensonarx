pub mod claims;
pub mod earnings;
pub mod economics;
pub mod embed;
pub mod entity;
pub mod error;
pub mod evidence;
pub mod facts;
pub mod gateway;
pub mod proto;
pub mod reputation;
pub mod shard;
pub mod traits;
pub mod types;

pub use earnings::{EarningsSnapshot, EarningsTracker};
pub use error::{Error, ErrorCode, Result};
pub use evidence::{
    ClaimExtraction, ClaimExtractor, EvidencePassage, PassageClaim, PassageClaimExtractor,
    ProposedClaim, ProposedQuote, RequestKind, ResponsivenessJudge, ResponsivenessVerdict,
    StanceJudge, StanceJudgment, StanceLabel, TermSubstitution,
};
pub use gateway::*;
pub use reputation::*;
pub use shard::{BUILTIN_TOPICS, TopicDefinition, TopicRouter, compute_shard_id};
pub use traits::EmbeddingPipeline;
pub use traits::FactExtractionPipeline;
pub use traits::QueryLeaderboard;
pub use traits::ReputationProvider;
pub use traits::Reranker;
pub use traits::SearchEngine;
pub use traits::SearchParams;
pub use types::*;
