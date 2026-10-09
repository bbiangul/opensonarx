//! Model-facing evidence types and the traits behind which language
//! understanding lives.
//!
//! Reading a query, finding the claim a passage makes, and deciding whether
//! one text supports or contradicts another are semantic judgements. They are
//! made by a versioned model behind these traits, never by term lists in the
//! caller. Model output is probabilistic evidence: the caller verifies every
//! proposed quote against retained text before it may be cited.

use crate::Result;
use serde::{Deserialize, Serialize};

/// One retained text the model may read and cite.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidencePassage {
    /// Stable within one request; the model cites passages by this id.
    pub id: String,
    pub url: String,
    pub title: String,
    pub text: String,
    /// `false` for a provider snippet that was never fetched and retained.
    /// Such a passage can inform routing but can never back a claim.
    pub retained: bool,
    pub published_at: Option<i64>,
    pub observed_at: Option<i64>,
}

/// What the request asks for, as read by the model.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestKind {
    /// Seeks a statement that evidence can support or contradict.
    Question,
    /// Seeks a site or an action on a site.
    Destination,
    /// Open-ended; a slate of sources is the useful output.
    Exploration,
    /// No model read the request.
    #[default]
    Unclassified,
}

impl RequestKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Question => "question",
            Self::Destination => "destination",
            Self::Exploration => "exploration",
            Self::Unclassified => "unclassified",
        }
    }
}

/// A span the model says appears verbatim in a passage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProposedQuote {
    pub passage_id: String,
    pub quote: String,
}

/// A claim answering the request, with the spans the model says state it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProposedClaim {
    pub text: String,
    pub quotes: Vec<ProposedQuote>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClaimExtraction {
    pub request_kind: RequestKind,
    pub claims: Vec<ProposedClaim>,
    /// For a destination request, the passage whose URL is the destination.
    pub destination_passage_id: Option<String>,
    /// What the passages lack for a complete answer, in the model's words.
    pub missing: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StanceLabel {
    Supports,
    Contradicts,
    Neutral,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StanceJudgment {
    pub passage_id: String,
    pub label: StanceLabel,
    /// The span the judge says grounds a non-neutral label. A support or
    /// contradiction whose quote is not found verbatim is discarded.
    pub quote: Option<String>,
}

/// Reads the request and the passages and proposes claims with quotes.
pub trait ClaimExtractor: Send + Sync {
    fn extract(&self, query: &str, passages: &[EvidencePassage]) -> Result<ClaimExtraction>;

    /// Recorded in the answer trace so every claim names the model behind it.
    fn model_version(&self) -> String;
}

/// Judges each passage's stance toward one claim.
pub trait StanceJudge: Send + Sync {
    /// One judgment per passage it could judge; a missing passage is neutral.
    fn judge(&self, claim: &str, passages: &[EvidencePassage]) -> Result<Vec<StanceJudgment>>;

    fn model_version(&self) -> String;
}

/// Judges whether each claim answers the request as it was asked.
///
/// A claim can be true, quoted verbatim and supported by every source and
/// still answer a different question: about a similarly named entity, another
/// period, or the opposite of what was asked. Faithfulness to the passages is
/// the stance judge's concern; this judgement is about the request.
pub trait ResponsivenessJudge: Send + Sync {
    /// One verdict per claim, in order: `true` when the claim answers the
    /// request as asked.
    fn answers_request(&self, query: &str, claims: &[&str]) -> Result<Vec<bool>>;

    fn model_version(&self) -> String;
}
