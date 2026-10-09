//! Typed evidence requirements attached to an answer. A checklist records
//! whether required evidence was located, not whether a claim is true.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequirementStatus {
    Satisfied,
    Missing,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceRequirement {
    pub id: String,
    pub claim_index: Option<usize>,
    pub critical: bool,
    pub status: RequirementStatus,
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EvidenceChecklist {
    pub policy_id: String,
    pub complete_answer_supported: bool,
    pub critical_missing: bool,
    pub requirements: Vec<EvidenceRequirement>,
}
