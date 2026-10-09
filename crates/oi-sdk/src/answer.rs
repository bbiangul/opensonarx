//! Answer assembly over retained evidence.
//!
//! A model reads the request and the retained passages and proposes claims
//! with quotes; a second model call judges each passage's stance toward each
//! claim. Nothing here interprets language. This module only does what can be
//! reproduced from retained bytes: it locates every proposed quote verbatim,
//! discards what it cannot locate, counts distinct reporting families, and
//! applies the numeric answer policy.

use crate::checklist::{EvidenceChecklist, EvidenceRequirement, RequirementStatus};
use crate::source::source_family;
use oi_index::uncertainty::{
    CashCalibration, ClaimAssessment, EvidenceState, OpenUncertainty, OpenUncertaintyConfig,
    SourceHistory,
};
use oi_types::{
    ClaimExtraction, EvidencePassage, RequestKind, SearchResultItem, StanceJudgment, StanceLabel,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Numeric policy for answer assembly. Versioned by `policy_id`, which is
/// recorded on every checklist so an answer names the policy that shaped it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnswerPolicy {
    pub policy_id: String,
    /// Distinct reporting families required to present a claim as an answer.
    pub min_source_families: usize,
    /// A shorter quote locates too little text to count as a citation.
    pub min_quote_chars: usize,
    pub max_passages: usize,
    pub max_passage_chars: usize,
    pub max_claims: usize,
    /// Result pages fetched in the foreground when they have no retained text.
    pub max_hydrations: usize,
}

impl Default for AnswerPolicy {
    fn default() -> Self {
        Self {
            policy_id: "answer-evidence-v2".to_string(),
            min_source_families: 2,
            min_quote_chars: 20,
            max_passages: 6,
            max_passage_chars: 3_000,
            max_claims: 3,
            max_hydrations: 4,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnswerCitation {
    /// `supports` or `contradicts`.
    pub evidence_role: String,
    /// One-based rank of the cited result in the returned slate.
    pub decision_rank: u32,
    pub title: String,
    pub url: String,
    pub source: String,
    /// The cited span, copied from retained text at the located offset.
    pub evidence_excerpt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnswerClaim {
    pub text: String,
    /// `corroborated`, `single_source`, `contested`, or `off_request` for a
    /// claim the passages support but that does not answer the request as
    /// asked. An off-request claim is related evidence, never the answer.
    pub status: String,
    pub citations: Vec<AnswerCitation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_lineage: Option<ClaimEvidenceLineage>,
    /// Descriptive evidence opinion from OpenUncertainty; not a probability.
    pub support: f64,
    pub conflict: f64,
    pub ignorance: f64,
}

/// What retained bytes establish about how a claim is reported. Source count
/// and independence are kept apart: several families can repeat one origin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimEvidenceLineage {
    pub reporting_source_families: usize,
    pub contradicting_source_families: usize,
    pub verbatim_citations: usize,
    /// Supporting families whose cited span is character-identical to another
    /// family's. Identical wording is evidence of a shared origin.
    pub shared_wording_families: usize,
    /// `shared_wording_detected` or `unresolved`. Never "independent": that
    /// would need a resolved derivation graph this module does not have.
    pub independence_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchAnswer {
    /// `destination`, `extractive`, `insufficient_evidence`, or `results_only`.
    pub status: String,
    /// The request kind as read by the model, or `unclassified`.
    pub intent: String,
    pub summary: String,
    pub claims: Vec<AnswerClaim>,
    pub uncertainty_mode: String,
    pub average_ignorance: Option<f64>,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnsweredSearch {
    pub query: String,
    pub answer: SearchAnswer,
    pub results: Vec<SearchResultItem>,
    pub sources_used: Vec<String>,
    pub hydrated_pages: usize,
    /// Official TSE/PesqEle registrations whose exact ID appears in the
    /// retained text of a cited page. A record is registration metadata; it
    /// does not validate any reported number.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub poll_registry: Vec<crate::pesqele::PesqEleRecord>,
    pub trace: AnswerTrace,
}

/// Observational record of one answered search. Never an input to ranking.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnswerTrace {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_checklist: Option<EvidenceChecklist>,
    /// Provider availability observed at completion; not an absence-of-facts
    /// signal.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unavailable_discovery_providers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extractor_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stance_model: Option<String>,
    /// A model call failed; the slate was returned without an answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_error: Option<String>,
    pub passages_read: usize,
    pub retained_passages: usize,
    pub proposed_claims: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub responsiveness_model: Option<String>,
    /// Supported claims set aside because they do not answer the request.
    #[serde(default)]
    pub off_request_claims: usize,
    /// Responsiveness rejections discarded because the judge did not show a
    /// literal mismatch between the request and the claim.
    #[serde(default)]
    pub unshown_rejections: usize,
    /// Quotes proposed by a model that were not found verbatim.
    pub rejected_quotes: usize,
    pub hydration_requested: usize,
    pub retrieval_ms: f64,
    pub hydration_ms: f64,
    pub extraction_ms: f64,
    pub stance_ms: f64,
    pub registry_lookup_ms: f64,
    #[serde(default)]
    pub registry_lookup_errors: usize,
    pub total_ms: f64,
    #[serde(default)]
    pub fresh_answer_cache_hit: bool,
}

/// Counters produced while assembling one answer.
#[derive(Debug, Clone, Copy, Default)]
pub struct AssemblyStats {
    pub rejected_quotes: usize,
}

/// Render retained Markdown as the plain text a model reads and quotes.
/// Verification runs against this same text, so a quote never has to
/// reproduce link or emphasis syntax.
pub fn plain_text(markdown: &str) -> String {
    let mut plain = String::with_capacity(markdown.len());
    let mut characters = markdown.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '*' | '`' => {}
            '!' if characters.peek() == Some(&'[') => {}
            '[' => {
                let mut label = String::new();
                let mut closed = false;
                for next in characters.by_ref() {
                    if next == ']' {
                        closed = true;
                        break;
                    }
                    label.push(next);
                }
                if closed && characters.peek() == Some(&'(') {
                    for next in characters.by_ref() {
                        if next == ')' {
                            break;
                        }
                    }
                    plain.push_str(&label);
                } else {
                    plain.push('[');
                    plain.push_str(&label);
                    if closed {
                        plain.push(']');
                    }
                }
            }
            '#' if plain.is_empty() || plain.ends_with('\n') => {
                while characters.peek() == Some(&'#') {
                    characters.next();
                }
                if characters.peek() == Some(&' ') {
                    characters.next();
                }
            }
            value => plain.push(value),
        }
    }
    plain
}

/// Build the passages a model may read from the decided slate, in rank order.
/// A result with retained content yields a citable passage; a result with
/// only a provider snippet yields a non-citable one.
pub fn build_passages(results: &[SearchResultItem], policy: &AnswerPolicy) -> Vec<EvidencePassage> {
    results
        .iter()
        .take(policy.max_passages)
        .enumerate()
        .filter_map(|(index, result)| {
            let retained = !result.content_md.trim().is_empty();
            let body = if retained {
                result
                    .content_relevant
                    .as_deref()
                    .filter(|relevant| !relevant.trim().is_empty())
                    .unwrap_or(&result.content_md)
            } else {
                &result.snippet
            };
            let text: String = plain_text(body)
                .chars()
                .take(policy.max_passage_chars)
                .collect();
            if text.trim().is_empty() && result.title.trim().is_empty() {
                return None;
            }
            Some(EvidencePassage {
                id: format!("p{}", index + 1),
                url: result.url.clone(),
                title: result.title.clone(),
                text,
                retained,
                published_at: result.published_at,
                observed_at: result.crawled_at.map(|at| at as i64),
            })
        })
        .collect()
}

/// Canonical characters for exact matching: whitespace runs collapse to one
/// space and typographic quotes and dashes map to their ASCII forms. Each
/// canonical character keeps the byte range it came from.
fn canonical(text: &str) -> Vec<(char, usize, usize)> {
    let mut out: Vec<(char, usize, usize)> = Vec::with_capacity(text.len());
    for (offset, character) in text.char_indices() {
        let end = offset + character.len_utf8();
        let mapped = match character {
            '\u{2018}' | '\u{2019}' | '\u{201B}' | '\u{2032}' => '\'',
            '\u{201C}' | '\u{201D}' | '\u{201F}' | '\u{2033}' | '\u{00AB}' | '\u{00BB}' => '"',
            '\u{2010}'..='\u{2015}' | '\u{2212}' => '-',
            value if value.is_whitespace() => ' ',
            value => value,
        };
        if mapped == ' ' {
            match out.last_mut() {
                Some(last) if last.0 == ' ' => last.2 = end,
                Some(_) => out.push((' ', offset, end)),
                None => {}
            }
        } else {
            out.push((mapped, offset, end));
        }
    }
    while out.last().is_some_and(|last| last.0 == ' ') {
        out.pop();
    }
    out
}

/// Locate `quote` verbatim in `passage` after canonical normalization and
/// return the span as it appears in the passage. `None` when the quote is
/// shorter than `min_chars` or does not occur.
pub fn locate_quote(passage: &str, quote: &str, min_chars: usize) -> Option<String> {
    let needle = canonical(quote);
    if needle.len() < min_chars.max(1) {
        return None;
    }
    let haystack = canonical(passage);
    if needle.len() > haystack.len() {
        return None;
    }
    let start = (0..=haystack.len() - needle.len()).find(|&start| {
        haystack[start..start + needle.len()]
            .iter()
            .zip(&needle)
            .all(|(left, right)| left.0 == right.0)
    })?;
    let from = haystack[start].1;
    let to = haystack[start + needle.len() - 1].2;
    Some(passage[from..to].to_string())
}

/// Merge proposed claims whose text is identical after folding, keeping the
/// first and pooling their quotes. Only character-identical repeats are
/// merged; whether two differently worded claims say the same thing is a
/// question for a model, not for this function.
pub fn merge_repeated_claims(extraction: &mut ClaimExtraction) {
    let mut merged: Vec<oi_types::ProposedClaim> = Vec::new();
    for claim in std::mem::take(&mut extraction.claims) {
        let key = crate::text::fold_text(&claim.text);
        match merged
            .iter_mut()
            .find(|kept| crate::text::fold_text(&kept.text) == key)
        {
            Some(kept) => {
                kept.quotes.extend(claim.quotes);
                kept.substitution = kept.substitution.take().or(claim.substitution);
            }
            None => merged.push(claim),
        }
    }
    extraction.claims = merged;
}

/// Whether a claimed mismatch is shown: `request_term` occurs literally in
/// the request, `claim_term` occurs literally in the claim (or, when given,
/// in one of its quotes), and the two differ. Terms match on word boundaries
/// after folding.
fn mismatch_is_shown(
    query: &str,
    claim: &oi_types::ProposedClaim,
    request_term: &str,
    claim_term: &str,
    search_quotes: bool,
) -> bool {
    let request_term = crate::text::fold_text(request_term);
    let claim_term = crate::text::fold_text(claim_term);
    if request_term.is_empty() || claim_term.is_empty() || request_term == claim_term {
        return false;
    }
    let contains = |text: &str, term: &str| {
        format!(" {} ", crate::text::fold_text(text)).contains(&format!(" {term} "))
    };
    contains(query, &request_term)
        && (contains(&claim.text, &claim_term)
            || (search_quotes
                && claim
                    .quotes
                    .iter()
                    .any(|quote| contains(&quote.quote, &claim_term))))
}

/// Decide, per claim, whether it stands as an answer to `query`:
/// `Some(false)` when a mismatch with the request is shown, `Some(true)` when
/// a judge examined it and none is shown, `None` when nobody examined it.
///
/// Two sources can show a mismatch. The extractor may declare that a claim
/// reads a term of the request as a different term (`substitution`), and a
/// responsiveness judge may reject a claim. Either one counts only if it
/// names a term that occurs literally in the request and a different term
/// that occurs literally in the claim; a rejection that cannot show that pair
/// is discarded and the claim stands. Returns the decisions and the number of
/// rejections discarded.
pub fn responsiveness_decisions(
    query: &str,
    claims: &[oi_types::ProposedClaim],
    verdicts: Option<&[oi_types::ResponsivenessVerdict]>,
) -> (Vec<Option<bool>>, usize) {
    let mut discarded = 0;
    let decisions = claims
        .iter()
        .enumerate()
        .map(|(index, claim)| {
            if let Some(substitution) = &claim.substitution {
                if mismatch_is_shown(
                    query,
                    claim,
                    &substitution.request_term,
                    &substitution.claim_term,
                    true,
                ) {
                    return Some(false);
                }
                discarded += 1;
            }
            let verdict = verdicts?.get(index)?;
            if verdict.answers {
                return Some(true);
            }
            let shown = match (&verdict.request_term, &verdict.claim_term) {
                (Some(request_term), Some(claim_term)) => {
                    mismatch_is_shown(query, claim, request_term, claim_term, false)
                }
                _ => false,
            };
            if !shown {
                discarded += 1;
            }
            Some(!shown)
        })
        .collect();
    (decisions, discarded)
}

struct Located<'a> {
    passage: &'a EvidencePassage,
    excerpt: String,
}

/// Model judgments about the proposed claims, indexed like
/// `extraction.claims`. A `None` field means that judge did not run: the
/// claim is then assembled without it and the checklist records the gap.
#[derive(Debug, Clone, Copy, Default)]
pub struct ClaimJudgments<'a> {
    /// Per claim, each retained passage's stance. Without it a claim rests on
    /// its extractor quotes alone and contradiction is unknown.
    pub stance: Option<&'a [Vec<StanceJudgment>]>,
    /// Per claim, whether it answers the request as asked, from
    /// `responsiveness_decisions`. `None` for a claim nobody examined.
    pub responsive: &'a [Option<bool>],
}

/// Assemble the answer from model proposals and judgments.
pub fn assemble_answer(
    results: &[SearchResultItem],
    passages: &[EvidencePassage],
    extraction: &ClaimExtraction,
    judged: ClaimJudgments<'_>,
    policy: &AnswerPolicy,
) -> (SearchAnswer, EvidenceChecklist, AssemblyStats) {
    let judgments = judged.stance;
    let mut stats = AssemblyStats::default();
    let mut checklist = EvidenceChecklist {
        policy_id: policy.policy_id.clone(),
        ..Default::default()
    };
    let by_id: BTreeMap<&str, &EvidencePassage> = passages
        .iter()
        .map(|passage| (passage.id.as_str(), passage))
        .collect();
    let intent = extraction.request_kind.as_str().to_string();

    if extraction.request_kind == RequestKind::Destination
        && let Some(passage) = extraction
            .destination_passage_id
            .as_deref()
            .and_then(|id| by_id.get(id))
    {
        let label = if passage.title.trim().is_empty() {
            crate::source::host(&passage.url)
        } else {
            passage.title.trim().to_string()
        };
        return (
            SearchAnswer {
                status: "destination".to_string(),
                intent,
                summary: format!("{label} — {}", passage.url),
                claims: Vec::new(),
                uncertainty_mode: "not_applicable".to_string(),
                average_ignorance: None,
                warning: Some(
                    "Destino indicado pelo modelo a partir dos resultados; não é verificação de titularidade do site."
                        .to_string(),
                ),
            },
            checklist,
            stats,
        );
    }

    let estimator = {
        let config = OpenUncertaintyConfig::default();
        OpenUncertainty::new(config.clone(), CashCalibration::fit(&[], &config))
    };
    let mut locate = |passage_id: &str, quote: &str| -> Option<Located<'_>> {
        let located = by_id
            .get(passage_id)
            .filter(|passage| passage.retained)
            .and_then(|passage| {
                locate_quote(&passage.text, quote, policy.min_quote_chars)
                    .map(|excerpt| Located { passage, excerpt })
            });
        if located.is_none() {
            stats.rejected_quotes += 1;
        }
        located
    };

    let mut claims = Vec::new();
    for (index, proposed) in extraction.claims.iter().take(policy.max_claims).enumerate() {
        let mut extractor_quotes: BTreeMap<&str, Located<'_>> = BTreeMap::new();
        for quote in &proposed.quotes {
            if let Some(located) = locate(&quote.passage_id, &quote.quote) {
                extractor_quotes
                    .entry(located.passage.id.as_str())
                    .or_insert(located);
            }
        }

        let mut supporting: Vec<Located<'_>> = Vec::new();
        let mut contradicting: Vec<Located<'_>> = Vec::new();
        match judgments.and_then(|all| all.get(index)) {
            Some(judged) => {
                for judgment in judged {
                    if judgment.label == StanceLabel::Neutral {
                        continue;
                    }
                    let judge_quote = judgment
                        .quote
                        .as_deref()
                        .and_then(|quote| locate(&judgment.passage_id, quote));
                    match judgment.label {
                        StanceLabel::Supports => {
                            // The extractor's span for this passage is the
                            // citation when both models agree; otherwise the
                            // judge's own verified span.
                            if let Some(located) = extractor_quotes
                                .remove(judgment.passage_id.as_str())
                                .or(judge_quote)
                            {
                                supporting.push(located);
                            }
                        }
                        StanceLabel::Contradicts => contradicting.extend(judge_quote),
                        StanceLabel::Neutral => {}
                    }
                }
            }
            None => supporting.extend(extractor_quotes.into_values()),
        }
        if supporting.is_empty() && contradicting.is_empty() {
            continue;
        }

        let families = |located: &[Located<'_>]| -> BTreeSet<String> {
            located
                .iter()
                .map(|item| source_family(&item.passage.url))
                .collect()
        };
        let supporting_families = families(&supporting);
        let contradicting_families = families(&contradicting);
        let mut wording: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for item in &supporting {
            wording
                .entry(crate::text::fold_text(&item.excerpt))
                .or_default()
                .insert(source_family(&item.passage.url));
        }
        let shared_wording_families: usize = wording
            .values()
            .filter(|families| families.len() > 1)
            .map(BTreeSet::len)
            .sum();

        let responsive = judged.responsive.get(index).copied().flatten();
        let status = if responsive == Some(false) {
            "off_request"
        } else if !contradicting_families.is_empty() {
            "contested"
        } else if supporting_families.len() >= policy.min_source_families {
            "corroborated"
        } else {
            "single_source"
        };

        let refs: Vec<String> = supporting
            .iter()
            .map(|item| item.passage.url.clone())
            .collect();
        let mut requirements = vec![
            requirement(
                "verbatim_supporting_quote",
                index,
                !supporting.is_empty(),
                refs.clone(),
            ),
            requirement(
                "distinct_reporting_families",
                index,
                supporting_families.len() >= policy.min_source_families,
                refs,
            ),
            requirement(
                "no_retained_contradiction",
                index,
                contradicting_families.is_empty(),
                contradicting
                    .iter()
                    .map(|item| item.passage.url.clone())
                    .collect(),
            ),
        ];
        if judgments.is_none() {
            requirements.push(EvidenceRequirement {
                id: "stance_judged".to_string(),
                claim_index: Some(index),
                critical: true,
                status: RequirementStatus::Unknown,
                evidence_refs: Vec::new(),
            });
        }
        requirements.push(EvidenceRequirement {
            id: "answers_the_request".to_string(),
            claim_index: Some(index),
            critical: true,
            status: match responsive {
                Some(true) => RequirementStatus::Satisfied,
                Some(false) => RequirementStatus::Missing,
                None => RequirementStatus::Unknown,
            },
            evidence_refs: Vec::new(),
        });
        if responsive == Some(false) {
            // An off-request claim is not part of the answer, so what it
            // lacks cannot block the claims that are.
            for item in &mut requirements {
                item.critical = false;
            }
        }
        let unmet = requirements
            .iter()
            .filter(|item| item.status != RequirementStatus::Satisfied)
            .count();
        let estimate = estimator.estimate(&ClaimAssessment {
            claim_ref: format!("claim:{index}"),
            task: "search/answer".to_string(),
            horizon: "request".to_string(),
            policy_id: policy.policy_id.clone(),
            source_history: SourceHistory::default(),
            evidence: EvidenceState {
                support_mass: supporting_families.len() as f64,
                conflict_mass: contradicting_families.len() as f64,
                missing_mass: unmet as f64 / requirements.len() as f64,
                critical_missing: unmet > 0,
                ..EvidenceState::default()
            },
            conviction: None,
            deterministic_outcome: None,
        });
        checklist.requirements.extend(requirements);

        let cite = |role: &str, item: &Located<'_>| AnswerCitation {
            evidence_role: role.to_string(),
            decision_rank: results
                .iter()
                .position(|result| result.url == item.passage.url)
                .map(|position| position as u32 + 1)
                .unwrap_or(0),
            title: item.passage.title.clone(),
            url: item.passage.url.clone(),
            source: results
                .iter()
                .find(|result| result.url == item.passage.url)
                .map(|result| result.source.clone())
                .unwrap_or_default(),
            evidence_excerpt: Some(item.excerpt.clone()),
        };
        claims.push(AnswerClaim {
            text: proposed.text.trim().to_string(),
            status: status.to_string(),
            citations: supporting
                .iter()
                .map(|item| cite("supports", item))
                .chain(contradicting.iter().map(|item| cite("contradicts", item)))
                .collect(),
            evidence_lineage: Some(ClaimEvidenceLineage {
                reporting_source_families: supporting_families.len(),
                contradicting_source_families: contradicting_families.len(),
                verbatim_citations: supporting.len(),
                shared_wording_families,
                independence_status: if shared_wording_families > 0 {
                    "shared_wording_detected"
                } else {
                    "unresolved"
                }
                .to_string(),
            }),
            support: estimate.support,
            conflict: estimate.conflict,
            ignorance: estimate.ignorance,
        });
    }

    for missing in &extraction.missing {
        checklist.requirements.push(EvidenceRequirement {
            id: format!("model_reported_gap: {}", missing.trim()),
            claim_index: None,
            critical: false,
            status: RequirementStatus::Missing,
            evidence_refs: Vec::new(),
        });
    }
    let corroborated = claims
        .iter()
        .filter(|claim| claim.status == "corroborated")
        .count();
    let on_request = claims
        .iter()
        .filter(|claim| claim.status != "off_request")
        .count();
    let contested = claims.iter().any(|claim| claim.status == "contested");
    if on_request == 0 && extraction.request_kind == RequestKind::Question {
        checklist.requirements.push(EvidenceRequirement {
            id: "answer_bearing_evidence".to_string(),
            claim_index: None,
            critical: true,
            status: RequirementStatus::Missing,
            evidence_refs: Vec::new(),
        });
    }
    checklist.critical_missing = checklist
        .requirements
        .iter()
        .any(|item| item.critical && item.status != RequirementStatus::Satisfied);
    checklist.complete_answer_supported =
        corroborated > 0 && corroborated == on_request && !checklist.critical_missing;

    let average_ignorance = (!claims.is_empty())
        .then(|| claims.iter().map(|claim| claim.ignorance).sum::<f64>() / claims.len() as f64);
    let (status, summary, warning) = if claims.is_empty() {
        if extraction.request_kind == RequestKind::Question {
            (
                "insufficient_evidence",
                "Nenhuma afirmação pôde ser localizada literalmente no texto retido.".to_string(),
                None,
            )
        } else {
            (
                "results_only",
                "Resultados preservados sem sintetizar uma conclusão.".to_string(),
                None,
            )
        }
    } else if on_request == 0 {
        (
            "insufficient_evidence",
            "O texto retido não responde à pergunta como foi feita; as afirmações listadas são relacionadas, não a resposta."
                .to_string(),
            None,
        )
    } else if contested {
        (
            "insufficient_evidence",
            "As fontes retidas divergem; veja as citações de apoio e de contradição.".to_string(),
            Some("Há contradição no texto retido.".to_string()),
        )
    } else if corroborated > 0 {
        (
            "extractive",
            claims
                .iter()
                .filter(|claim| claim.status == "corroborated")
                .map(|claim| claim.text.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            (corroborated < claims.len()).then(|| {
                "Nem todas as afirmações listadas são a resposta corroborada; veja o status de cada uma."
                    .to_string()
            }),
        )
    } else {
        (
            "insufficient_evidence",
            format!(
                "Afirmação localizada em menos de {} famílias de fontes distintas.",
                policy.min_source_families
            ),
            None,
        )
    };

    (
        SearchAnswer {
            status: status.to_string(),
            intent,
            summary,
            claims,
            uncertainty_mode: "evidential_only".to_string(),
            average_ignorance,
            warning,
        },
        checklist,
        stats,
    )
}

fn requirement(id: &str, claim: usize, satisfied: bool, refs: Vec<String>) -> EvidenceRequirement {
    EvidenceRequirement {
        id: id.to_string(),
        claim_index: Some(claim),
        critical: true,
        status: if satisfied {
            RequirementStatus::Satisfied
        } else {
            RequirementStatus::Missing
        },
        evidence_refs: refs,
    }
}

/// The answer returned when no model read the request: the slate is the
/// output and nothing is claimed.
pub fn results_only(policy: &AnswerPolicy) -> (SearchAnswer, EvidenceChecklist) {
    (
        SearchAnswer {
            status: "results_only".to_string(),
            intent: RequestKind::Unclassified.as_str().to_string(),
            summary: "Resultados preservados sem sintetizar uma conclusão.".to_string(),
            claims: Vec::new(),
            uncertainty_mode: "not_applicable".to_string(),
            average_ignorance: None,
            warning: None,
        },
        EvidenceChecklist {
            policy_id: policy.policy_id.clone(),
            ..Default::default()
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use oi_types::{ProposedClaim, ProposedQuote};

    fn result(rank: u32, url: &str, title: &str, content: &str) -> SearchResultItem {
        SearchResultItem {
            result_id: url.to_string(),
            rank,
            url: url.to_string(),
            title: title.to_string(),
            snippet: title.to_string(),
            content_md: content.to_string(),
            semantic_score: 0.8,
            bm25_score: 0.0,
            final_score: 0.8,
            stake_truth: 0,
            stake_usd: 0,
            stake_type: "none".to_string(),
            reputation: 0.0,
            content_hash: String::new(),
            extraction_quality: "full".to_string(),
            source: "local".to_string(),
            entity: None,
            content_kind: None,
            crawled_at: None,
            published_at: None,
            trust_verdict: "unverified".to_string(),
            reputation_votes: 0,
            disputed: false,
            facts: None,
            content_relevant: None,
            domain_first_seen_at: None,
            domain_url_count: None,
            domain_reputation_score: None,
            domain_total_votes: None,
            decision: None,
        }
    }

    fn claim(text: &str, quotes: &[(&str, &str)]) -> ProposedClaim {
        ProposedClaim {
            text: text.to_string(),
            quotes: quotes
                .iter()
                .map(|(passage_id, quote)| ProposedQuote {
                    passage_id: passage_id.to_string(),
                    quote: quote.to_string(),
                })
                .collect(),
            substitution: None,
        }
    }

    fn judged(passage_id: &str, label: StanceLabel, quote: &str) -> StanceJudgment {
        StanceJudgment {
            passage_id: passage_id.to_string(),
            label,
            quote: (!quote.is_empty()).then(|| quote.to_string()),
        }
    }

    fn question(claims: Vec<ProposedClaim>) -> ClaimExtraction {
        ClaimExtraction {
            request_kind: RequestKind::Question,
            claims,
            ..Default::default()
        }
    }

    fn two_sources() -> Vec<SearchResultItem> {
        vec![
            result(
                1,
                "https://news.test/a",
                "A",
                "Founded in 2010.\nThe **company** was started by [Ada Example](https://x.test/ada) in Recife.",
            ),
            result(
                2,
                "https://reference.test/b",
                "B",
                "According to filings, the company was started by Ada Example.",
            ),
        ]
    }

    #[test]
    fn plain_text_drops_link_emphasis_and_heading_syntax() {
        assert_eq!(
            plain_text(
                "# Title\nThe **company** was started by [Ada](https://x.test/a) in `Recife`."
            ),
            "Title\nThe company was started by Ada in Recife."
        );
        assert_eq!(plain_text("a [b] c ![d](e.png)"), "a [b] c d");
        assert_eq!(plain_text("x # y"), "x # y");
    }

    #[test]
    fn quote_is_located_across_whitespace_and_typographic_variants() {
        let passage = "He said: “It’s  done”\n— and left the building for good.";
        assert_eq!(
            locate_quote(passage, "\"It's done\" - and left the building", 10).as_deref(),
            Some("“It’s  done”\n— and left the building")
        );
    }

    #[test]
    fn quote_that_is_absent_reworded_or_too_short_is_rejected() {
        let passage = "The company was started by Ada Example in Recife.";
        assert!(locate_quote(passage, "The company was founded by Ada Example", 10).is_none());
        assert!(locate_quote(passage, "the company was started by Ada Example", 10).is_none());
        assert!(locate_quote(passage, "Ada", 10).is_none());
        assert!(locate_quote("short", "a much longer quote than the passage", 10).is_none());
        assert!(locate_quote(passage, "", 0).is_none());
    }

    #[test]
    fn snippet_only_results_become_non_citable_passages() {
        let mut snippet_only = result(1, "https://serp.test/x", "X", "");
        snippet_only.snippet = "The company was started by Ada Example.".to_string();
        let passages = build_passages(&[snippet_only], &AnswerPolicy::default());
        assert_eq!(passages.len(), 1);
        assert!(!passages[0].retained);

        let extraction = question(vec![claim(
            "Ada Example started the company.",
            &[("p1", "The company was started by Ada Example.")],
        )]);
        let (answer, checklist, stats) = assemble_answer(
            &[],
            &passages,
            &extraction,
            ClaimJudgments::default(),
            &AnswerPolicy::default(),
        );
        assert_eq!(answer.status, "insufficient_evidence");
        assert!(answer.claims.is_empty());
        assert_eq!(stats.rejected_quotes, 1);
        assert!(checklist.critical_missing);
    }

    #[test]
    fn claim_with_verified_quotes_in_two_families_is_extractive() {
        let results = two_sources();
        let passages = build_passages(&results, &AnswerPolicy::default());
        let extraction = question(vec![claim(
            "Ada Example started the company.",
            &[
                ("p1", "The company was started by Ada Example in Recife."),
                ("p2", "the company was started by Ada Example"),
            ],
        )]);
        let judgments = vec![vec![
            judged(
                "p1",
                StanceLabel::Supports,
                "The company was started by Ada Example",
            ),
            judged(
                "p2",
                StanceLabel::Supports,
                "the company was started by Ada Example",
            ),
        ]];
        let (answer, checklist, stats) = assemble_answer(
            &results,
            &passages,
            &extraction,
            ClaimJudgments {
                stance: Some(&judgments),
                responsive: &[Some(true)],
            },
            &AnswerPolicy::default(),
        );
        assert_eq!(answer.status, "extractive");
        assert_eq!(answer.summary, "Ada Example started the company.");
        assert_eq!(answer.claims[0].status, "corroborated");
        assert_eq!(answer.claims[0].citations.len(), 2);
        assert_eq!(answer.claims[0].citations[0].decision_rank, 1);
        assert_eq!(
            answer.claims[0].citations[0].evidence_excerpt.as_deref(),
            Some("The company was started by Ada Example in Recife.")
        );
        let lineage = answer.claims[0].evidence_lineage.as_ref().unwrap();
        assert_eq!(lineage.reporting_source_families, 2);
        assert_eq!(lineage.independence_status, "unresolved");
        assert!(checklist.complete_answer_supported);
        assert!(!checklist.critical_missing);
        assert_eq!(stats.rejected_quotes, 0);
        assert!(answer.claims[0].support > 0.0);
        assert!(answer.claims[0].ignorance > 0.0);
    }

    #[test]
    fn fabricated_quote_cannot_carry_a_claim() {
        let results = two_sources();
        let passages = build_passages(&results, &AnswerPolicy::default());
        let extraction = question(vec![claim(
            "Bob Example started the company.",
            &[
                ("p1", "The company was started by Bob Example in Recife."),
                ("p2", "the company was started by Bob Example"),
            ],
        )]);
        let (answer, _, stats) = assemble_answer(
            &results,
            &passages,
            &extraction,
            ClaimJudgments::default(),
            &AnswerPolicy::default(),
        );
        assert_eq!(answer.status, "insufficient_evidence");
        assert!(answer.claims.is_empty());
        assert_eq!(stats.rejected_quotes, 2);
    }

    #[test]
    fn two_pages_of_one_family_are_a_single_source() {
        let results = vec![
            result(
                1,
                "https://a.news.test/1",
                "A",
                "The company was started by Ada Example.",
            ),
            result(
                2,
                "https://b.news.test/2",
                "B",
                "The company was started by Ada Example.",
            ),
        ];
        let passages = build_passages(&results, &AnswerPolicy::default());
        let extraction = question(vec![claim(
            "Ada Example started the company.",
            &[
                ("p1", "The company was started by Ada Example."),
                ("p2", "The company was started by Ada Example."),
            ],
        )]);
        let (answer, checklist, _) = assemble_answer(
            &results,
            &passages,
            &extraction,
            ClaimJudgments::default(),
            &AnswerPolicy::default(),
        );
        assert_eq!(answer.status, "insufficient_evidence");
        assert_eq!(answer.claims[0].status, "single_source");
        assert_eq!(
            answer.claims[0]
                .evidence_lineage
                .as_ref()
                .unwrap()
                .reporting_source_families,
            1
        );
        assert!(checklist.critical_missing);
    }

    #[test]
    fn identical_wording_across_families_is_reported_as_shared() {
        let results = vec![
            result(
                1,
                "https://one.test/1",
                "A",
                "The company was started by Ada Example.",
            ),
            result(
                2,
                "https://two.test/2",
                "B",
                "The company was started by Ada Example.",
            ),
        ];
        let passages = build_passages(&results, &AnswerPolicy::default());
        let extraction = question(vec![claim(
            "Ada Example started the company.",
            &[
                ("p1", "The company was started by Ada Example."),
                ("p2", "The company was started by Ada Example."),
            ],
        )]);
        let (answer, _, _) = assemble_answer(
            &results,
            &passages,
            &extraction,
            ClaimJudgments::default(),
            &AnswerPolicy::default(),
        );
        let lineage = answer.claims[0].evidence_lineage.as_ref().unwrap();
        assert_eq!(lineage.shared_wording_families, 2);
        assert_eq!(lineage.independence_status, "shared_wording_detected");
    }

    #[test]
    fn judge_can_withdraw_an_extractor_quote_and_surface_a_contradiction() {
        let results = vec![
            result(
                1,
                "https://one.test/1",
                "A",
                "It is a myth that the company was started by Ada Example.",
            ),
            result(
                2,
                "https://two.test/2",
                "B",
                "Records show the company was started by Ada Example.",
            ),
        ];
        let passages = build_passages(&results, &AnswerPolicy::default());
        let extraction = question(vec![claim(
            "Ada Example started the company.",
            &[
                ("p1", "the company was started by Ada Example"),
                ("p2", "the company was started by Ada Example"),
            ],
        )]);
        let judgments = vec![vec![
            judged(
                "p1",
                StanceLabel::Contradicts,
                "It is a myth that the company was started by Ada Example.",
            ),
            judged(
                "p2",
                StanceLabel::Supports,
                "the company was started by Ada Example",
            ),
        ]];
        let (answer, checklist, _) = assemble_answer(
            &results,
            &passages,
            &extraction,
            ClaimJudgments {
                stance: Some(&judgments),
                responsive: &[],
            },
            &AnswerPolicy::default(),
        );
        assert_eq!(answer.status, "insufficient_evidence");
        assert_eq!(answer.claims[0].status, "contested");
        let roles: Vec<_> = answer.claims[0]
            .citations
            .iter()
            .map(|citation| citation.evidence_role.as_str())
            .collect();
        assert_eq!(roles, vec!["supports", "contradicts"]);
        assert!(answer.claims[0].conflict > 0.0);
        assert!(!checklist.complete_answer_supported);
    }

    fn verdict(answers: bool, request: &str, claim: &str) -> oi_types::ResponsivenessVerdict {
        oi_types::ResponsivenessVerdict {
            answers,
            request_term: (!request.is_empty()).then(|| request.to_string()),
            claim_term: (!claim.is_empty()).then(|| claim.to_string()),
        }
    }

    #[test]
    fn a_rejection_stands_only_when_it_shows_the_mismatched_terms() {
        let query = "Of what form are Mersenne tests?";
        let claims = [claim(
            "Mersenne primes are prime numbers of the form 2p − 1.",
            &[],
        )];
        let decide = |v: oi_types::ResponsivenessVerdict| {
            responsiveness_decisions(query, &claims, Some(&[v]))
        };
        assert_eq!(
            decide(verdict(false, "Mersenne tests", "Mersenne primes")),
            (vec![Some(false)], 0)
        );

        // No terms, a term absent from the request, a term absent from the
        // claim, or the same term twice: the rejection is discarded.
        for unsupported in [
            verdict(false, "", ""),
            verdict(false, "Fermat tests", "Mersenne primes"),
            verdict(false, "Mersenne tests", "Fermat primes"),
            verdict(false, "Mersenne", "mersenne"),
            verdict(false, "Mersenne tests", ""),
            // Terms match on word boundaries: "test" is not "tests".
            verdict(false, "test", "primes"),
        ] {
            assert_eq!(
                decide(unsupported.clone()),
                (vec![Some(true)], 1),
                "{unsupported:?}"
            );
        }
        assert_eq!(decide(verdict(true, "", "")), (vec![Some(true)], 0));
        // Nobody examined the claim: the decision is unknown, not "answers".
        assert_eq!(
            responsiveness_decisions(query, &claims, None),
            (vec![None], 0)
        );
        assert_eq!(
            responsiveness_decisions(query, &claims, Some(&[])),
            (vec![None], 0)
        );
    }

    #[test]
    fn a_substitution_declared_by_the_extractor_sets_the_claim_aside_without_a_judge() {
        let query = "What have Bloc's sidewalks done?";
        let substituted = |request: &str, term: &str| {
            let mut proposed = claim(
                "In Warsaw, sidewalks have improved markedly.",
                &[(
                    "p1",
                    "sidewalks and sanitation facilities in the capital improved",
                )],
            );
            proposed.substitution = Some(oi_types::TermSubstitution {
                request_term: request.to_string(),
                claim_term: term.to_string(),
            });
            vec![proposed]
        };
        assert_eq!(
            responsiveness_decisions(query, &substituted("Bloc's", "Warsaw"), None),
            (vec![Some(false)], 0)
        );
        // The substituted term may sit in a quote rather than in the claim.
        assert_eq!(
            responsiveness_decisions(query, &substituted("Bloc's", "the capital"), None),
            (vec![Some(false)], 0)
        );
        // A declared substitution that cannot be shown is discarded; with no
        // judge the claim is then unexamined, and a judge can still accept it.
        let unshown = substituted("Eastern Europe", "Warsaw");
        assert_eq!(
            responsiveness_decisions(query, &unshown, None),
            (vec![None], 1)
        );
        assert_eq!(
            responsiveness_decisions(query, &unshown, Some(&[verdict(true, "", "")])),
            (vec![Some(true)], 1)
        );
        // A shown substitution outranks a judge that accepted the claim.
        assert_eq!(
            responsiveness_decisions(
                query,
                &substituted("Bloc's", "Warsaw"),
                Some(&[verdict(true, "", "")])
            ),
            (vec![Some(false)], 0)
        );
    }

    #[test]
    fn supported_claim_that_does_not_answer_the_request_is_set_aside() {
        let results = two_sources();
        let passages = build_passages(&results, &AnswerPolicy::default());
        let quotes = [
            ("p1", "The company was started by Ada Example in Recife."),
            ("p2", "the company was started by Ada Example"),
        ];
        let extraction = question(vec![claim("Ada Example started the company.", &quotes)]);
        let (answer, checklist, _) = assemble_answer(
            &results,
            &passages,
            &extraction,
            ClaimJudgments {
                stance: None,
                responsive: &[Some(false)],
            },
            &AnswerPolicy::default(),
        );
        assert_eq!(answer.status, "insufficient_evidence");
        assert!(answer.summary.contains("não responde à pergunta"));
        assert_eq!(answer.claims.len(), 1, "the related claim stays visible");
        assert_eq!(answer.claims[0].status, "off_request");
        assert_eq!(answer.claims[0].citations.len(), 2);
        assert!(!checklist.complete_answer_supported);
        assert!(checklist.critical_missing, "a question was left unanswered");
        assert!(checklist.requirements.iter().any(|item| {
            item.id == "answers_the_request" && item.status == RequirementStatus::Missing
        }));
    }

    #[test]
    fn off_request_claim_does_not_block_the_claim_that_answers() {
        let results = two_sources();
        let passages = build_passages(&results, &AnswerPolicy::default());
        let quotes = [
            ("p1", "The company was started by Ada Example in Recife."),
            ("p2", "the company was started by Ada Example"),
        ];
        let extraction = question(vec![
            claim("Ada Example started the company.", &quotes),
            claim("The company is based in Recife.", &quotes),
        ]);
        let judgments = vec![
            vec![
                judged(
                    "p1",
                    StanceLabel::Supports,
                    "The company was started by Ada Example"
                ),
                judged(
                    "p2",
                    StanceLabel::Supports,
                    "the company was started by Ada Example"
                ),
            ];
            2
        ];
        let (answer, checklist, _) = assemble_answer(
            &results,
            &passages,
            &extraction,
            ClaimJudgments {
                stance: Some(&judgments),
                responsive: &[Some(true), Some(false)],
            },
            &AnswerPolicy::default(),
        );
        assert_eq!(answer.status, "extractive");
        assert_eq!(answer.summary, "Ada Example started the company.");
        let statuses: Vec<_> = answer.claims.iter().map(|c| c.status.as_str()).collect();
        assert_eq!(statuses, vec!["corroborated", "off_request"]);
        assert!(checklist.complete_answer_supported);
        assert!(answer.warning.is_some());
    }

    #[test]
    fn without_a_responsiveness_judge_the_answer_is_not_marked_complete() {
        let results = two_sources();
        let passages = build_passages(&results, &AnswerPolicy::default());
        let extraction = question(vec![claim(
            "Ada Example started the company.",
            &[
                ("p1", "The company was started by Ada Example in Recife."),
                ("p2", "the company was started by Ada Example"),
            ],
        )]);
        let judgments = vec![vec![
            judged(
                "p1",
                StanceLabel::Supports,
                "The company was started by Ada Example",
            ),
            judged(
                "p2",
                StanceLabel::Supports,
                "the company was started by Ada Example",
            ),
        ]];
        let (answer, checklist, _) = assemble_answer(
            &results,
            &passages,
            &extraction,
            ClaimJudgments {
                stance: Some(&judgments),
                responsive: &[],
            },
            &AnswerPolicy::default(),
        );
        assert_eq!(answer.claims[0].status, "corroborated");
        assert!(!checklist.complete_answer_supported);
        assert!(checklist.requirements.iter().any(|item| {
            item.id == "answers_the_request" && item.status == RequirementStatus::Unknown
        }));
    }

    #[test]
    fn judge_label_without_a_verifiable_quote_is_discarded() {
        let results = two_sources();
        let passages = build_passages(&results, &AnswerPolicy::default());
        let extraction = question(vec![claim("Ada Example started the company.", &[])]);
        let judgments = vec![vec![
            judged("p1", StanceLabel::Supports, ""),
            judged(
                "p2",
                StanceLabel::Contradicts,
                "words that are not in the passage",
            ),
        ]];
        let (answer, _, stats) = assemble_answer(
            &results,
            &passages,
            &extraction,
            ClaimJudgments {
                stance: Some(&judgments),
                responsive: &[],
            },
            &AnswerPolicy::default(),
        );
        assert!(answer.claims.is_empty());
        assert_eq!(stats.rejected_quotes, 1);
    }

    #[test]
    fn without_a_judge_stance_is_an_unmet_critical_requirement() {
        let results = two_sources();
        let passages = build_passages(&results, &AnswerPolicy::default());
        let extraction = question(vec![claim(
            "Ada Example started the company.",
            &[
                ("p1", "The company was started by Ada Example in Recife."),
                ("p2", "the company was started by Ada Example"),
            ],
        )]);
        let (answer, checklist, _) = assemble_answer(
            &results,
            &passages,
            &extraction,
            ClaimJudgments::default(),
            &AnswerPolicy::default(),
        );
        assert_eq!(answer.claims[0].status, "corroborated");
        assert!(checklist.critical_missing);
        assert!(!checklist.complete_answer_supported);
        assert!(checklist.requirements.iter().any(|item| {
            item.id == "stance_judged" && item.status == RequirementStatus::Unknown
        }));
    }

    #[test]
    fn destination_request_returns_the_chosen_result_without_claims() {
        let mut snippet_only = result(1, "https://example.test/", "Example", "");
        snippet_only.snippet = "Official site.".to_string();
        let passages = build_passages(
            std::slice::from_ref(&snippet_only),
            &AnswerPolicy::default(),
        );
        let extraction = ClaimExtraction {
            request_kind: RequestKind::Destination,
            destination_passage_id: Some("p1".to_string()),
            ..Default::default()
        };
        let (answer, _, _) = assemble_answer(
            &[snippet_only],
            &passages,
            &extraction,
            ClaimJudgments::default(),
            &AnswerPolicy::default(),
        );
        assert_eq!(answer.status, "destination");
        assert_eq!(answer.intent, "destination");
        assert_eq!(answer.summary, "Example — https://example.test/");
        assert!(answer.claims.is_empty());
    }

    #[test]
    fn exploration_without_claims_is_results_only() {
        let results = two_sources();
        let passages = build_passages(&results, &AnswerPolicy::default());
        let extraction = ClaimExtraction {
            request_kind: RequestKind::Exploration,
            missing: vec!["no single answer".to_string()],
            ..Default::default()
        };
        let (answer, checklist, _) = assemble_answer(
            &results,
            &passages,
            &extraction,
            ClaimJudgments::default(),
            &AnswerPolicy::default(),
        );
        assert_eq!(answer.status, "results_only");
        assert!(!checklist.critical_missing);
        assert_eq!(checklist.requirements.len(), 1);
    }

    #[test]
    fn repeated_claims_are_merged_and_their_quotes_pooled() {
        let mut extraction = question(vec![
            claim("Ada Example started the company.", &[("p1", "quote one")]),
            claim("ada example started the company", &[("p2", "quote two")]),
            claim(
                "Ada Example started the company in Recife.",
                &[("p1", "quote three")],
            ),
        ]);
        extraction.claims[1].substitution = Some(oi_types::TermSubstitution {
            request_term: "firm".to_string(),
            claim_term: "company".to_string(),
        });
        merge_repeated_claims(&mut extraction);
        assert_eq!(extraction.claims.len(), 2);
        assert!(
            extraction.claims[0].substitution.is_some(),
            "a merged repeat keeps it"
        );
        assert_eq!(
            extraction.claims[0].text,
            "Ada Example started the company."
        );
        assert_eq!(extraction.claims[0].quotes.len(), 2);
        assert_eq!(extraction.claims[1].quotes.len(), 1);
    }

    #[test]
    fn claims_beyond_the_policy_limit_are_not_assembled() {
        let results = two_sources();
        let passages = build_passages(&results, &AnswerPolicy::default());
        let numbered = |n: usize| {
            claim(
                &format!("Ada Example started the company ({n})."),
                &[("p2", "the company was started by Ada Example")],
            )
        };
        let extraction = question((1..=4).map(numbered).collect());
        let (answer, _, _) = assemble_answer(
            &results,
            &passages,
            &extraction,
            ClaimJudgments::default(),
            &AnswerPolicy::default(),
        );
        assert_eq!(answer.claims.len(), AnswerPolicy::default().max_claims);
    }
}
