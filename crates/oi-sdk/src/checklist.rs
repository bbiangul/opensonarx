//! Availability/scope requirements, not a certificate of world truth.
use crate::answer::SearchAnswer;
use crate::query_intent::{QueryIntent, classify_query, is_implicit_location_query, source_family};
use oi_types::SearchResultItem;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

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

fn normalized(text: &str) -> String {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn evaluate(
    query: &str,
    answer: &SearchAnswer,
    results: &[SearchResultItem],
) -> EvidenceChecklist {
    let mut checklist = EvidenceChecklist {
        policy_id: "answer-requirements-v1".into(),
        ..Default::default()
    };
    let mut add = |id: &str, claim_index, status, refs| {
        checklist.requirements.push(EvidenceRequirement {
            id: id.into(),
            claim_index,
            critical: true,
            status,
            evidence_refs: refs,
        })
    };
    if classify_query(query) == QueryIntent::DueDiligence {
        add(
            "independent_legal_operational_verification",
            None,
            RequirementStatus::Unknown,
            vec![],
        );
    }
    if is_implicit_location_query(query) {
        add(
            "explicit_location",
            None,
            RequirementStatus::Missing,
            vec![],
        );
    }
    if answer.status == "insufficient_evidence" {
        add(
            "answer_bearing_evidence",
            None,
            RequirementStatus::Missing,
            vec![],
        );
    }
    for (i, claim) in answer.claims.iter().enumerate() {
        let refs: Vec<String> = claim.citations.iter().map(|c| c.url.clone()).collect();
        add(
            "claim_citations",
            Some(i),
            if refs.is_empty() {
                RequirementStatus::Missing
            } else {
                RequirementStatus::Satisfied
            },
            refs,
        );
        let citations: Vec<_> = claim
            .citations
            .iter()
            .filter(|c| c.evidence_role == "claim_text")
            .collect();
        if !citations.is_empty() {
            let retained = citations.iter().all(|c| {
                c.evidence_excerpt.as_ref().is_some_and(|excerpt| {
                    let excerpt = normalized(excerpt);
                    !excerpt.is_empty()
                        && results.iter().any(|r| {
                            r.url == c.url
                                && !r.content_md.trim().is_empty()
                                && normalized(&r.content_md).contains(&excerpt)
                        })
                })
            });
            add(
                "retained_citation_excerpt",
                Some(i),
                if retained {
                    RequirementStatus::Satisfied
                } else {
                    RequirementStatus::Missing
                },
                citations.iter().map(|c| c.url.clone()).collect(),
            );
            let families: HashSet<_> = citations.iter().map(|c| source_family(&c.url)).collect();
            add(
                "distinct_reporting_families",
                Some(i),
                if families.len() >= 2 {
                    RequirementStatus::Satisfied
                } else {
                    RequirementStatus::Missing
                },
                citations.iter().map(|c| c.url.clone()).collect(),
            );
        } else {
            let quotes = claim
                .citations
                .iter()
                .all(|c| c.evidence_role == "structured_market_quote")
                && claim.citations.len() >= 2
                && claim
                    .evidence_lineage
                    .as_ref()
                    .is_some_and(|l| l.underlying_unit_kind == "market_quote_snapshot");
            add(
                "structured_claim_evidence",
                Some(i),
                if quotes {
                    RequirementStatus::Satisfied
                } else {
                    RequirementStatus::Missing
                },
                vec![],
            );
        }
        if let Some(l) = &claim.evidence_lineage {
            if l.registry_metadata_records > 0
                && l.underlying_unit_ids.iter().any(|id| id.contains("BR-"))
            {
                add(
                    "exact_poll_registry_identity",
                    Some(i),
                    if l.identity_status == "official_registry_joined"
                        && l.registry_metadata_records > 0
                    {
                        RequirementStatus::Satisfied
                    } else {
                        RequirementStatus::Missing
                    },
                    l.underlying_unit_ids.clone(),
                );
            }
        }
    }
    checklist.critical_missing = checklist.requirements.iter().any(|r| {
        r.critical
            && matches!(
                r.status,
                RequirementStatus::Missing | RequirementStatus::Unknown
            )
    });
    checklist.complete_answer_supported =
        answer.status == "extractive" && !answer.claims.is_empty() && !checklist.critical_missing;
    checklist
}

pub(crate) fn enforce(
    query: &str,
    answer: &mut SearchAnswer,
    results: &[SearchResultItem],
) -> EvidenceChecklist {
    let checklist = evaluate(query, answer, results);
    if answer.status == "extractive" && checklist.critical_missing {
        answer.status = "insufficient_evidence".into();
        answer.summary = "Uma exigência crítica de evidência não foi atendida. A conclusão foi suspensa; consulte o checklist do rastro.".into();
        answer.claims.clear();
        answer.uncertainty_mode = "evidential_only".into();
        answer.warning =
            Some("O volume de resultados não substitui os requisitos ausentes.".into());
    }
    checklist
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::answer::{AnswerCitation, AnswerClaim};
    fn answer() -> SearchAnswer {
        SearchAnswer {
            status: "extractive".into(),
            intent: "factual".into(),
            summary: "Unsupported claim".into(),
            claims: vec![AnswerClaim {
                text: "Unsupported claim".into(),
                citations: vec![AnswerCitation {
                    evidence_role: "claim_text".into(),
                    decision_rank: 1,
                    title: "Page".into(),
                    url: "https://example.org/a".into(),
                    source: "test".into(),
                    evidence_excerpt: Some("Unsupported claim".into()),
                }],
                evidence_lineage: None,
            }],
            uncertainty_mode: "evidential_only".into(),
            average_ignorance: None,
            warning: None,
        }
    }
    #[test]
    fn unavailable_source_blocks_even_many_citations() {
        let mut a = answer();
        a.claims[0].citations = vec![a.claims[0].citations[0].clone(); 1000];
        let c = enforce("who founded example", &mut a, &[]);
        assert!(c.critical_missing);
        assert_eq!(a.status, "insufficient_evidence");
        assert!(a.claims.is_empty());
    }
    #[test]
    fn diligence_does_not_turn_mentions_into_verification() {
        let c = evaluate("gama fund é legítimo", &answer(), &[]);
        assert!(
            c.requirements
                .iter()
                .any(|r| r.id == "independent_legal_operational_verification"
                    && r.status == RequirementStatus::Unknown)
        );
        assert!(!c.complete_answer_supported);
    }
    #[test]
    fn no_answer_is_not_complete_coverage() {
        let mut a = answer();
        a.claims.clear();
        a.status = "results_only".into();
        assert!(!evaluate("history of rivers", &a, &[]).complete_answer_supported);
    }

    #[test]
    fn polling_registration_does_not_certify_reported_percentages() {
        let mut a = answer();
        a.claims[0].evidence_lineage = Some(crate::answer::ClaimEvidenceLineage {
            underlying_unit_kind: "poll_measurement".into(),
            underlying_unit_ids: vec!["BR-12345/2026".into()],
            identity_status: "declared_registration_unresolved".into(),
            underlying_unit_count: None,
            reporting_source_families: 1,
            claim_text_citations: 1,
            independence_status: "unknown".into(),
            known_independent_observations: None,
            unresolved_dependencies: vec![],
            registry_metadata_records: 0,
            registry_verifies: vec![],
            registry_does_not_verify: vec!["reported_percentages".into()],
        });
        let c = evaluate("how to interpret a poll", &a, &[]);
        assert!(
            !c.requirements
                .iter()
                .any(|r| r.id == "exact_poll_registry_identity")
        );
        assert!(c.critical_missing); // retained excerpt is still required
    }
}
