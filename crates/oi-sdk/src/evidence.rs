//! Latency-bounded Stage-2 evidence policy for search results.
//!
//! The current slice uses only evidence that already exists on a live result:
//! retained/distilled content, extracted facts when present, publication time,
//! result shape, and conservative source-dependence priors. With no typed
//! source history or eligible claim-specific position, OpenUncertainty must
//! return `EVIDENTIAL_ONLY`; this module never invents a probability.

use crate::query_intent::{
    QueryIntent, authoritative_source_for_query, classify_query,
    current_query_allows_undated_display, exact_entity_host_match, intent_adjustment,
    is_current_preview, is_high_stakes_guidance_query, is_hub_url, relevance_terms,
};
use oi_index::uncertainty::{
    CashCalibration, ClaimAssessment, DecisionCandidate, DecisionPolicy, EvidenceState,
    OpenUncertainty, OpenUncertaintyConfig, SourceHistory, UncertaintyMode,
};
use oi_types::{EvidenceLineageMetadata, SearchDecisionMetadata, SearchResultItem};
use std::collections::{HashMap, HashSet};

const POLICY_ID: &str = "search-evidential-v3-critical-requirements";
// Trust may order candidates that are comparably relevant, but it must not
// rescue an off-topic page merely because that page is retained, older, or
// easier to audit. Search relevance scores are not calibrated probabilities;
// this band is therefore a conservative ordering guard rather than a claim
// about semantic equivalence.
const SEARCH_RELIANCE_RELEVANCE_BAND: f64 = 0.12;

/// Apply Stage 2 in place while preserving the original retrieval rank in
/// `SearchDecisionMetadata`.
pub fn apply_decision_policy(query: &str, results: &mut Vec<SearchResultItem>) {
    apply_decision_policy_with_registry(query, results, &[]);
}

/// Re-run Stage 2 with structured poll lineage after the official registry
/// join. Original Stage-1 ranks survive this second pass; only the reliance
/// decision is recomputed. The registry validates metadata, never the reported
/// percentages.
pub(crate) fn apply_decision_policy_with_registry(
    query: &str,
    results: &mut Vec<SearchResultItem>,
    poll_registry: &[crate::pesqele::PesqEleRecord],
) {
    if results.is_empty() {
        return;
    }

    let classified_intent = classify_query(query);
    let intent = if classified_intent == QueryIntent::Navigational
        && !results
            .iter()
            .any(|result| verified_navigation_destination(query, result))
    {
        // A short query is only provisionally navigational. If discovery
        // cannot verify a canonical entity host or public-authority path, the
        // result slate is informational/ambiguous and still needs reliance
        // assessment. This matches the answer layer, which already refuses to
        // call such a result an official destination.
        QueryIntent::Exploratory
    } else {
        classified_intent
    };
    if !is_claim_bearing(query, intent) {
        for (index, result) in results.iter_mut().enumerate() {
            let rank = (index + 1) as u32;
            let mut decision = not_applicable(rank, result.final_score, intent);
            for signal in intent_adjustment(query, result).signals {
                if !decision
                    .evidence_signals
                    .iter()
                    .any(|existing| existing == signal)
                {
                    decision.evidence_signals.push(signal.to_string());
                }
            }
            result.rank = rank;
            result.decision = Some(decision);
        }
        return;
    }

    let mut domain_counts: HashMap<String, usize> = HashMap::new();
    for result in results.iter() {
        *domain_counts.entry(domain(&result.url)).or_default() += 1;
    }
    let lineage_context = LineageContext::new(results, poll_registry);

    let config = OpenUncertaintyConfig::default();
    let estimator = OpenUncertainty::new(config.clone(), CashCalibration::fit(&[], &config));
    let policy = policy_for(query, intent);
    let candidates = std::mem::take(results)
        .into_iter()
        .enumerate()
        .map(|(index, result)| {
            let retrieval_rank = result
                .decision
                .as_ref()
                .map(|decision| decision.retrieval_rank as usize)
                .unwrap_or(index + 1);
            let base_relevance = result.final_score;
            let (evidence, signals, lineage) =
                evidence_for(query, intent, &result, &domain_counts, &lineage_context);
            let assessment = ClaimAssessment {
                claim_ref: format!("search:{}:{}", stable_query_ref(query), result.result_id),
                task: format!("search/{intent}"),
                horizon: horizon_for(query, intent).to_string(),
                policy_id: POLICY_ID.to_string(),
                source_history: SourceHistory::default(),
                evidence,
                conviction: None,
                deterministic_outcome: None,
            };
            let estimate = estimator.estimate(&assessment);
            DecisionCandidate {
                item: (result, signals, lineage),
                retrieval_rank,
                base_relevance,
                licensed_cost: 0.0,
                estimate,
            }
        })
        .collect();

    let mut decided = policy.rerank(candidates);
    // Some evidence is not merely "less trusted"; it establishes that the
    // page cannot answer the reliance question at all (for example, a
    // different entity, a future-results preview, or the subject's own
    // legitimacy claim). Such a page must not define the relevance band that
    // blocks an answer-bearing alternative. If every candidate has this
    // problem, keep the ordinary relevance order and expose the uncertainty
    // rather than manufacturing an eligible result.
    let has_structurally_eligible_candidate = decided
        .iter()
        .any(|decision| structurally_eligible_for_reliance_top(&decision.item.1));
    let strongest_relevance = decided
        .iter()
        .filter(|decision| {
            !has_structurally_eligible_candidate
                || structurally_eligible_for_reliance_top(&decision.item.1)
        })
        .map(|decision| decision.base_relevance)
        .max_by(f64::total_cmp)
        .unwrap_or_default();
    for decision in &mut decided {
        let structurally_eligible = structurally_eligible_for_reliance_top(&decision.item.1);
        if has_structurally_eligible_candidate && !structurally_eligible {
            decision
                .item
                .1
                .push("structurally_ineligible_for_reliance_top".to_string());
        } else if decision.base_relevance + SEARCH_RELIANCE_RELEVANCE_BAND < strongest_relevance {
            decision
                .item
                .1
                .push("outside_top_relevance_band".to_string());
        }
    }
    decided.sort_by(|left, right| {
        let left_structurally_eligible = !has_structurally_eligible_candidate
            || structurally_eligible_for_reliance_top(&left.item.1);
        let right_structurally_eligible = !has_structurally_eligible_candidate
            || structurally_eligible_for_reliance_top(&right.item.1);
        let left_in_band =
            left.base_relevance + SEARCH_RELIANCE_RELEVANCE_BAND >= strongest_relevance;
        let right_in_band =
            right.base_relevance + SEARCH_RELIANCE_RELEVANCE_BAND >= strongest_relevance;
        right_structurally_eligible
            .cmp(&left_structurally_eligible)
            .then_with(|| right_in_band.cmp(&left_in_band))
            // Cold web candidates frequently have no retained body, history,
            // lineage, calibrated outcomes, or eligible cash. In that state
            // their numeric evidential estimates are identical. Preserve the
            // distinction between relevance and trust by using only a
            // query-relative source role here: primary/official, edited
            // reference, unclassified, or unverified UGC. This is a reliance
            // ordering prior, never a truth probability, and it cannot rescue
            // a candidate outside the relevance band.
            .then_with(|| source_role_tier(&right.item.1).cmp(&source_role_tier(&left.item.1)))
            .then_with(|| right.decision_score.total_cmp(&left.decision_score))
            .then_with(|| left.retrieval_rank.cmp(&right.retrieval_rank))
    });
    for (index, decision) in decided.iter_mut().enumerate() {
        decision.decision_rank = index + 1;
    }
    *results = decided
        .into_iter()
        .map(|decision| {
            let (mut result, signals, lineage) = decision.item;
            result.rank = decision.decision_rank as u32;
            result.decision = Some(SearchDecisionMetadata {
                retrieval_rank: decision.retrieval_rank as u32,
                decision_rank: decision.decision_rank as u32,
                decision_score: decision.decision_score,
                policy_id: decision.estimate.policy_id.clone(),
                uncertainty_mode: mode_name(decision.estimate.mode).to_string(),
                p_reliable: decision.estimate.p_reliable,
                interval_90: decision.estimate.interval_90,
                support: decision.estimate.support,
                conflict: decision.estimate.conflict,
                ignorance: decision.estimate.ignorance,
                critical_missing: decision.estimate.critical_missing,
                independence: decision.estimate.independence,
                dependency_uncertainty: decision.estimate.uncertainty.dependency_uncertainty,
                staleness_uncertainty: decision.estimate.uncertainty.staleness_uncertainty,
                distribution_shift: decision.estimate.uncertainty.distribution_shift,
                cash_eligible: decision.estimate.cash.eligible,
                cash_calibrated: decision.estimate.cash.calibrated,
                cash_log_bayes_factor: decision.estimate.cash.log_bayes_factor,
                evidence_signals: signals,
                evidence_lineage: lineage,
            });
            result
        })
        .collect();
}

fn structurally_eligible_for_reliance_top(signals: &[String]) -> bool {
    const DISQUALIFYING_SIGNALS: &[&str] = &[
        "editorial_derivation_duplicate",
        "event_or_registration_page_not_verification",
        "future_results_may_be_absent",
        "named_entity_mismatch_not_evidence",
        "press_release_not_independent_reporting",
        "self_declaration_not_independent_confirmation",
        "topic_hub_not_direct_article",
    ];
    !signals.iter().any(|signal| {
        DISQUALIFYING_SIGNALS
            .iter()
            .any(|disqualifier| signal == disqualifier)
    })
}

fn is_claim_bearing(query: &str, intent: QueryIntent) -> bool {
    let _ = query;
    matches!(
        intent,
        QueryIntent::DueDiligence
            | QueryIntent::CurrentEvents
            | QueryIntent::Factual
            | QueryIntent::Exploratory
    )
}

fn verified_navigation_destination(query: &str, result: &SearchResultItem) -> bool {
    let signals = intent_adjustment(query, result).signals;
    signals.contains(&"exact_entity_host")
        || signals.contains(&"fuzzy_entity_host")
        || signals.contains(&"public_authority_entity_path")
}

fn not_applicable(rank: u32, relevance: f64, intent: QueryIntent) -> SearchDecisionMetadata {
    SearchDecisionMetadata {
        retrieval_rank: rank,
        decision_rank: rank,
        decision_score: relevance,
        policy_id: POLICY_ID.to_string(),
        uncertainty_mode: "not_applicable".to_string(),
        p_reliable: None,
        interval_90: None,
        support: 0.0,
        conflict: 0.0,
        ignorance: 1.0,
        critical_missing: false,
        independence: 1.0,
        dependency_uncertainty: 0.0,
        staleness_uncertainty: 0.0,
        distribution_shift: 0.0,
        cash_eligible: false,
        cash_calibrated: false,
        cash_log_bayes_factor: 0.0,
        evidence_signals: vec![format!("intent_{intent}_does_not_require_claim_reliance")],
        evidence_lineage: None,
    }
}

#[derive(Default)]
struct LineageContext {
    clusters: HashMap<String, MeasurementCluster>,
    result_units: HashMap<String, Vec<String>>,
    inferred_result_units: HashSet<(String, String)>,
    verified_units: HashSet<String>,
}

struct MeasurementCluster {
    size: usize,
    representative_result_id: String,
}

impl LineageContext {
    fn new(results: &[SearchResultItem], poll_registry: &[crate::pesqele::PesqEleRecord]) -> Self {
        let verified_units = poll_registry
            .iter()
            .map(|record| record.registration_id.clone())
            .collect::<HashSet<_>>();
        let mut memberships: HashMap<String, Vec<(bool, u32, String)>> = HashMap::new();
        let mut result_units = HashMap::new();
        let mut unit_fingerprints: HashMap<String, Vec<crate::answer::PollMeasurementFingerprint>> =
            HashMap::new();
        for (index, result) in results.iter().enumerate() {
            let units = crate::pesqele::registration_ids_for_result(result);
            if units.is_empty() {
                continue;
            }
            let retrieval_rank = result
                .decision
                .as_ref()
                .map(|decision| decision.retrieval_rank)
                .unwrap_or((index + 1) as u32);
            for unit in &units {
                memberships.entry(unit.clone()).or_default().push((
                    false,
                    retrieval_rank,
                    result.result_id.clone(),
                ));
                unit_fingerprints
                    .entry(unit.clone())
                    .or_default()
                    .extend(crate::answer::poll_measurement_fingerprints(result, &[]));
            }
            result_units.insert(result.result_id.clone(), units);
        }

        // Some publishers report the exact registered measurement but omit
        // its ID. Attach such a page only when exactly one verified unit has
        // both (a) an identical candidate/percentage assignment already seen
        // on an ID-bearing page and (b) the registered institute named in the
        // result title/snippet. Requiring the institute in the exact numeric
        // sentence failed on list layouts whose heading supplies that context;
        // allowing the full body would instead accept unrelated footer links.
        // Ambiguous or merely similar pages remain unresolved.
        let mut inferred_result_units = HashSet::new();
        for (index, result) in results.iter().enumerate() {
            if result_units.contains_key(&result.result_id) {
                continue;
            }
            let mut candidates = Vec::new();
            for record in poll_registry {
                let Some(seed_fingerprints) = unit_fingerprints.get(&record.registration_id) else {
                    continue;
                };
                if seed_fingerprints.is_empty() {
                    continue;
                }
                let institute_terms = registry_institute_terms(record);
                if institute_terms.is_empty() {
                    continue;
                }
                if !result_surface_mentions_institute(result, &institute_terms) {
                    continue;
                }
                let result_fingerprints = crate::answer::poll_measurement_fingerprints(result, &[]);
                if result_fingerprints.iter().any(|result_fingerprint| {
                    seed_fingerprints
                        .iter()
                        .any(|seed| result_fingerprint.compatible_with(seed))
                }) {
                    candidates.push(record.registration_id.clone());
                }
            }
            candidates.sort();
            candidates.dedup();
            if candidates.len() != 1 {
                continue;
            }
            let unit = candidates.pop().unwrap_or_default();
            let retrieval_rank = result
                .decision
                .as_ref()
                .map(|decision| decision.retrieval_rank)
                .unwrap_or((index + 1) as u32);
            memberships.entry(unit.clone()).or_default().push((
                true,
                retrieval_rank,
                result.result_id.clone(),
            ));
            result_units.insert(result.result_id.clone(), vec![unit.clone()]);
            inferred_result_units.insert((result.result_id.clone(), unit));
        }

        let clusters = memberships
            .into_iter()
            .map(|(unit, mut members)| {
                members.sort();
                let representative_result_id = members
                    .first()
                    .map(|(_, _, result_id)| result_id.clone())
                    .unwrap_or_default();
                (
                    unit,
                    MeasurementCluster {
                        size: members.len(),
                        representative_result_id,
                    },
                )
            })
            .collect();
        Self {
            clusters,
            result_units,
            inferred_result_units,
            verified_units,
        }
    }

    fn lineage_for(&self, result: &SearchResultItem) -> Option<EvidenceLineageMetadata> {
        let units = self.result_units.get(&result.result_id)?;
        let unit_ref = units
            .iter()
            .filter_map(|unit| self.clusters.get(unit).map(|cluster| (unit, cluster)))
            .max_by(|(left_unit, left), (right_unit, right)| {
                left.size
                    .cmp(&right.size)
                    .then_with(|| right_unit.cmp(left_unit))
            })?;
        let (unit_ref, cluster) = unit_ref;
        let representative = cluster.representative_result_id == result.result_id;
        let inferred = self
            .inferred_result_units
            .contains(&(result.result_id.clone(), unit_ref.clone()));
        Some(EvidenceLineageMetadata {
            unit_kind: "poll_measurement".to_string(),
            unit_ref: unit_ref.clone(),
            relation: if representative {
                "measurement_representative".to_string()
            } else if inferred {
                "inferred_editorial_report_of".to_string()
            } else {
                "editorial_report_of".to_string()
            },
            cluster_size: cluster.size,
            representative,
            registry_verified: self.verified_units.contains(unit_ref),
        })
    }
}

fn registry_institute_terms(record: &crate::pesqele::PesqEleRecord) -> Vec<String> {
    const GENERIC: &[&str] = &[
        "cnpj",
        "empresa",
        "instituto",
        "institutos",
        "ltda",
        "pesquisa",
        "pesquisas",
    ];
    crate::query_intent::relevance_terms(&record.institute)
        .into_iter()
        .filter(|term| term.len() >= 5)
        .filter(|term| {
            term.chars()
                .any(|character| character.is_ascii_alphabetic())
        })
        .filter(|term| !GENERIC.contains(&term.as_str()))
        .collect()
}

fn result_surface_mentions_institute(
    result: &SearchResultItem,
    institute_terms: &[String],
) -> bool {
    let surface =
        crate::query_intent::relevance_terms(&format!("{} {}", result.title, result.snippet));
    institute_terms
        .iter()
        .any(|institute| surface.contains(institute))
}

fn policy_for(query: &str, intent: QueryIntent) -> DecisionPolicy {
    match intent {
        QueryIntent::CurrentEvents => DecisionPolicy {
            ignorance_penalty: 0.08,
            conflict_penalty: 0.35,
            dependency_penalty: 0.12,
            staleness_penalty: 0.25,
            distribution_shift_penalty: 0.18,
            licensed_cost_penalty: 0.0,
            exploration_floor: 0.15,
        },
        QueryIntent::DueDiligence => DecisionPolicy {
            ignorance_penalty: 0.18,
            conflict_penalty: 0.30,
            dependency_penalty: 0.16,
            staleness_penalty: 0.05,
            distribution_shift_penalty: 0.25,
            licensed_cost_penalty: 0.0,
            exploration_floor: 0.12,
        },
        QueryIntent::Factual => DecisionPolicy::default(),
        QueryIntent::Exploratory if is_high_stakes_guidance_query(query) => DecisionPolicy {
            ignorance_penalty: 0.14,
            conflict_penalty: 0.30,
            dependency_penalty: 0.14,
            staleness_penalty: 0.08,
            distribution_shift_penalty: 0.22,
            licensed_cost_penalty: 0.0,
            exploration_floor: 0.12,
        },
        QueryIntent::Navigational
        | QueryIntent::Transactional
        | QueryIntent::Tool
        | QueryIntent::Exploratory => DecisionPolicy::default(),
    }
}

fn evidence_for(
    query: &str,
    intent: QueryIntent,
    result: &SearchResultItem,
    domain_counts: &HashMap<String, usize>,
    lineage_context: &LineageContext,
) -> (EvidenceState, Vec<String>, Option<EvidenceLineageMetadata>) {
    let mut signals = Vec::new();
    if classify_query(query) == QueryIntent::Navigational && intent == QueryIntent::Exploratory {
        signals.push("ambiguous_navigation_requires_reliance_assessment".to_string());
    }
    let mut observed = 0.0;
    if !result.title.trim().is_empty() {
        observed += 0.20;
        signals.push("title_available".to_string());
    }
    if !result.snippet.trim().is_empty() {
        observed += 0.20;
        signals.push("snippet_available".to_string());
    }
    if !result.content_md.trim().is_empty() {
        observed += 0.35;
        signals.push("distilled_content_available".to_string());
    }
    if result
        .facts
        .as_ref()
        .is_some_and(|facts| !facts.facts.is_empty() || !facts.entities.is_empty())
    {
        observed += 0.15;
        signals.push("structured_extraction_available".to_string());
    }

    let intent_signals = intent_adjustment(query, result).signals;
    signals.push(
        query_relative_source_role(query, intent, result)
            .signal()
            .to_string(),
    );
    if authoritative_source_for_query(query, &result.url) {
        observed += 0.10;
        signals.push("query_relative_primary_authority".to_string());
    }
    let freshness_signal = intent_signals
        .iter()
        .find(|signal| signal.starts_with("published_within_"))
        .copied();
    if freshness_signal.is_some() {
        observed += 0.15;
        signals.push(freshness_signal.unwrap().to_string());
    }

    let lineage = (intent == QueryIntent::CurrentEvents)
        .then(|| lineage_context.lineage_for(result))
        .flatten();
    let count = domain_counts
        .get(&domain(&result.url))
        .copied()
        .unwrap_or(1);
    let (independence, dependency_uncertainty) = if let Some(lineage) = &lineage {
        signals.push(format!("underlying_measurement_ref:{}", lineage.unit_ref));
        signals.push(format!("measurement_cluster_size:{}", lineage.cluster_size));
        if lineage.registry_verified {
            signals.push("official_registry_metadata_joined".to_string());
        } else {
            signals.push("declared_registration_not_registry_joined".to_string());
        }
        if lineage.representative {
            signals.push("underlying_measurement_representative".to_string());
            (
                1.0,
                if lineage.registry_verified {
                    0.05
                } else {
                    0.25
                },
            )
        } else {
            if lineage.relation == "inferred_editorial_report_of" {
                signals
                    .push("measurement_link_inferred_from_registered_claim_signature".to_string());
            } else {
                signals.push("editorial_derivation_duplicate".to_string());
            }
            (
                0.0,
                if lineage.registry_verified {
                    0.05
                } else {
                    0.25
                },
            )
        }
    } else if count > 1 {
        signals.push("same_domain_cluster_repeated".to_string());
        (1.0, 0.65)
    } else if is_web_discovery(result) {
        signals.push("derivation_lineage_unknown".to_string());
        (1.0, 0.55)
    } else {
        (1.0, 0.35)
    };

    let mut conflict_mass = 0.0;
    let mut missing_mass = (1.0_f64 - observed).max(0.0);
    let mut distribution_shift = if is_web_discovery(result) { 0.65 } else { 0.25 };
    if lineage.is_some() {
        signals.push("claim_graph_partial_poll_measurement".to_string());
    } else if result.facts.is_none() {
        distribution_shift = (distribution_shift + 0.10_f64).min(1.0);
        signals.push("claim_graph_missing".to_string());
    }

    let staleness_uncertainty = match intent {
        QueryIntent::CurrentEvents => {
            if is_press_release_surface(result) {
                conflict_mass += 0.40;
                missing_mass += 0.25;
                distribution_shift = distribution_shift.max(0.90);
                signals.push("press_release_not_independent_reporting".to_string());
            }
            if is_hub_url(&result.url) {
                conflict_mass += 0.45;
                signals.push("topic_hub_not_direct_article".to_string());
                0.70
            } else if is_current_preview(result) {
                conflict_mass += 0.70;
                signals.push("future_results_may_be_absent".to_string());
                0.20
            } else if current_query_allows_undated_display(query) && result.crawled_at.is_some() {
                // Continuously updated state pages (office-holder records and
                // live quote surfaces) may not publish article timestamps. A
                // fresh crawl is an observation time, not proof of the value;
                // keep residual staleness and require claim-specific
                // corroboration before answer synthesis.
                signals.push("mutable_state_observed_at_crawl".to_string());
                0.15
            } else {
                match freshness_signal {
                    Some("published_within_1d") => 0.02,
                    Some("published_within_2d") => 0.08,
                    Some("published_within_7d") => 0.15,
                    Some("published_within_30d") => 0.30,
                    Some("published_within_180d") => 0.60,
                    _ => {
                        missing_mass += 0.20;
                        signals.push("publication_time_unresolved".to_string());
                        0.80
                    }
                }
            }
        }
        QueryIntent::DueDiligence => {
            if intent_signals.contains(&"diligence_entity_mismatch") {
                missing_mass += 1.0;
                conflict_mass += 0.50;
                distribution_shift = distribution_shift.max(0.95);
                signals.push("named_entity_mismatch_not_evidence".to_string());
            }
            if exact_entity_host_match(query, result) {
                missing_mass += 1.0;
                distribution_shift = distribution_shift.max(0.95);
                signals.push("self_declaration_not_independent_confirmation".to_string());
            }
            if is_non_verifying_action_page(result) {
                missing_mass += 0.35;
                distribution_shift = distribution_shift.max(0.75);
                signals.push("event_or_registration_page_not_verification".to_string());
            }
            0.20
        }
        QueryIntent::Factual => {
            if result.content_md.trim().is_empty() {
                0.35
            } else {
                0.15
            }
        }
        QueryIntent::Exploratory if is_high_stakes_guidance_query(query) => {
            if authoritative_source_for_query(query, &result.url) {
                signals.push("query_relative_authority".to_string());
                0.10
            } else if result.content_md.trim().is_empty() {
                missing_mass += 0.20;
                signals.push("high_stakes_source_not_retained".to_string());
                0.45
            } else {
                signals.push("high_stakes_authority_unresolved".to_string());
                0.25
            }
        }
        QueryIntent::Exploratory => {
            // A result-only informational slate still recommends which
            // sources a person should inspect. That is a reliance decision
            // even when synthesis is deliberately withheld. Run Stage 2,
            // without the stronger authority prior reserved for explicitly
            // high-impact guidance.
            if result.content_md.trim().is_empty() {
                missing_mass += 0.10;
                signals.push("exploratory_source_not_retained".to_string());
                0.20
            } else {
                signals.push("exploratory_source_retained".to_string());
                0.10
            }
        }
        QueryIntent::Navigational | QueryIntent::Transactional | QueryIntent::Tool => 0.0,
    };

    (
        EvidenceState {
            support_mass: 0.25 + observed,
            conflict_mass,
            missing_mass,
            critical_missing: intent == QueryIntent::DueDiligence
                || (matches!(intent, QueryIntent::Factual | QueryIntent::CurrentEvents)
                    && result.content_md.trim().is_empty()),
            independence,
            dependency_uncertainty,
            staleness_uncertainty,
            distribution_shift,
        },
        signals,
        lineage,
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum QueryRelativeSourceRole {
    UnverifiedUgc = 0,
    Unclassified = 1,
    EditedReference = 2,
    Primary = 3,
}

impl QueryRelativeSourceRole {
    fn signal(self) -> &'static str {
        match self {
            Self::UnverifiedUgc => "query_relative_source_role_unverified_ugc",
            Self::Unclassified => "query_relative_source_role_unclassified",
            Self::EditedReference => "query_relative_source_role_edited_reference",
            Self::Primary => "query_relative_source_role_primary",
        }
    }
}

fn source_role_tier(signals: &[String]) -> u8 {
    if signals
        .iter()
        .any(|signal| signal == QueryRelativeSourceRole::Primary.signal())
    {
        3
    } else if signals
        .iter()
        .any(|signal| signal == QueryRelativeSourceRole::EditedReference.signal())
    {
        2
    } else if signals
        .iter()
        .any(|signal| signal == QueryRelativeSourceRole::Unclassified.signal())
    {
        1
    } else {
        0
    }
}

fn query_relative_source_role(
    query: &str,
    intent: QueryIntent,
    result: &SearchResultItem,
) -> QueryRelativeSourceRole {
    let host = domain(&result.url);
    let topic_relevant = crate::engines::provider_surface_topic_relevant(query, result);

    if authoritative_source_for_query(query, &result.url)
        || (intent != QueryIntent::DueDiligence
            && is_public_authority_host(&host)
            && topic_relevant)
        || (intent != QueryIntent::DueDiligence
            && is_first_party_product_fact(query, &host)
            && topic_relevant)
    {
        return QueryRelativeSourceRole::Primary;
    }

    if (is_lexical_reference_query(query) && is_edited_lexical_reference(&host))
        || (is_high_stakes_guidance_query(query)
            && is_curated_health_reference(&host)
            && topic_relevant)
        || (is_scholarly_reference(&host) && topic_relevant)
        || (is_edited_general_reference(&host) && topic_relevant)
        || (is_hosted_documentation(query, result, &host) && topic_relevant)
    {
        return QueryRelativeSourceRole::EditedReference;
    }

    if is_unverified_ugc_host(&host) && !query_seeks_lived_experience(query) {
        QueryRelativeSourceRole::UnverifiedUgc
    } else {
        QueryRelativeSourceRole::Unclassified
    }
}

fn is_public_authority_host(host: &str) -> bool {
    host == "gov.br"
        || host.ends_with(".gov.br")
        || host.ends_with(".gov")
        || host.ends_with(".gob")
        || host.ends_with(".gouv.fr")
}

fn is_lexical_reference_query(query: &str) -> bool {
    let folded = query.to_ascii_lowercase();
    [
        "define ",
        "defined",
        "definition",
        "meaning",
        "means",
        "sinonimo",
        "synonym",
        "antonimo",
        "antonym",
    ]
    .iter()
    .any(|cue| folded.contains(cue))
}

fn is_edited_lexical_reference(host: &str) -> bool {
    host_matches_any(
        host,
        &[
            "britannica.com",
            "collinsdictionary.com",
            "dictionary.cambridge.org",
            "dictionary.com",
            "merriam-webster.com",
            "oxfordlearnersdictionaries.com",
        ],
    )
}

fn is_curated_health_reference(host: &str) -> bool {
    host_matches_any(
        host,
        &[
            "cancer.org",
            "cancerresearchuk.org",
            "clevelandclinic.org",
            "mayoclinic.org",
            "medlineplus.gov",
            "ncbi.nlm.nih.gov",
            "nih.gov",
            "paho.org",
            "pubmed.ncbi.nlm.nih.gov",
            "who.int",
        ],
    )
}

fn is_scholarly_reference(host: &str) -> bool {
    host.ends_with(".edu")
        || host_matches_any(
            host,
            &[
                "acm.org",
                "aip.org",
                "doi.org",
                "ieee.org",
                "nature.com",
                "ncbi.nlm.nih.gov",
                "sciencedirect.com",
                "springer.com",
                "wiley.com",
            ],
        )
}

fn is_edited_general_reference(host: &str) -> bool {
    host_matches_any(
        host,
        &[
            "britannica.com",
            "imdb.com",
            "investopedia.com",
            "wikipedia.org",
        ],
    )
}

fn is_unverified_ugc_host(host: &str) -> bool {
    host_matches_any(
        host,
        &[
            "facebook.com",
            "fandom.com",
            "instagram.com",
            "instructables.com",
            "medium.com",
            "quora.com",
            "reddit.com",
            "tiktok.com",
            "youtube.com",
        ],
    ) || host == "quizlet.com"
        || host.ends_with(".quizlet.com")
}

fn query_seeks_lived_experience(query: &str) -> bool {
    let folded = query.to_ascii_lowercase();
    [
        "experience",
        "experiencia",
        "forum",
        "people think",
        "opinioes",
        "opinions",
        "reddit",
        "review",
        "reviews",
    ]
    .iter()
    .any(|cue| folded.contains(cue))
}

fn is_first_party_product_fact(query: &str, host: &str) -> bool {
    const QUESTION_AND_PREDICATE: &[&str] = &[
        "annual",
        "are",
        "can",
        "coding",
        "cost",
        "does",
        "fee",
        "for",
        "how",
        "ingredients",
        "is",
        "language",
        "of",
        "price",
        "specification",
        "specifications",
        "the",
        "use",
        "uses",
        "version",
        "warranty",
        "what",
        "which",
    ];
    let product_fact = relevance_terms(query).iter().any(|term| {
        matches!(
            term.as_str(),
            "coding"
                | "cost"
                | "fee"
                | "ingredients"
                | "language"
                | "price"
                | "specification"
                | "specifications"
                | "version"
                | "warranty"
        )
    });
    if !product_fact {
        return false;
    }
    let label = registrable_host_label(host);
    relevance_terms(query).iter().any(|term| {
        term.len() >= 4 && !QUESTION_AND_PREDICATE.contains(&term.as_str()) && term == &label
    })
}

fn is_hosted_documentation(query: &str, result: &SearchResultItem, host: &str) -> bool {
    let hosted = host_matches_any(host, &["atlassian.net", "github.io", "readthedocs.io"]);
    if !hosted {
        return false;
    }
    let surface = relevance_terms(&format!("{} {}", result.title, result.url));
    let documentation = surface.iter().any(|term| {
        matches!(
            term.as_str(),
            "architecture" | "developer" | "developers" | "documentation" | "docs"
        )
    });
    let query_terms = relevance_terms(query);
    documentation
        && query_terms
            .iter()
            .filter(|term| term.len() >= 4)
            .any(|term| surface.contains(term))
}

fn registrable_host_label(host: &str) -> String {
    let labels = host.split('.').collect::<Vec<_>>();
    if labels.len() < 2 {
        return String::new();
    }
    let generic_second_level = ["ac", "co", "com", "edu", "gov", "net", "org"];
    let index = if labels.len() >= 3
        && labels.last().is_some_and(|label| label.len() == 2)
        && generic_second_level.contains(&labels[labels.len() - 2])
    {
        labels.len() - 3
    } else {
        labels.len() - 2
    };
    labels[index]
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .collect()
}

fn host_matches_any(host: &str, registered: &[&str]) -> bool {
    registered
        .iter()
        .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
}

fn is_press_release_surface(result: &SearchResultItem) -> bool {
    let surface = format!("{} {}", result.title, result.url).to_ascii_lowercase();
    [
        "advertorial",
        "businesswire.com",
        "globenewswire",
        "newsfilecorp.com",
        "newswire",
        "press-release",
        "press_release",
        "prnewswire.com",
        "sponsored",
    ]
    .iter()
    .any(|marker| surface.contains(marker))
}

fn is_non_verifying_action_page(result: &SearchResultItem) -> bool {
    let text = crate::query_intent::relevance_terms(&format!(
        "{} {} {}",
        result.title, result.snippet, result.url
    ))
    .join(" ");
    [
        "ama",
        "application",
        "aplicar",
        "evento",
        "event",
        "inscricao",
        "inscrever",
        "luma com",
        "sympla com",
    ]
    .iter()
    .any(|term| {
        text.split_whitespace().any(|word| word == *term)
            || (term.contains(' ') && text.contains(term))
    })
}

fn horizon_for(_query: &str, intent: QueryIntent) -> &'static str {
    match intent {
        QueryIntent::CurrentEvents => "live",
        QueryIntent::DueDiligence => "current-state",
        QueryIntent::Factual => "query-time",
        QueryIntent::Exploratory => "query-time",
        QueryIntent::Navigational | QueryIntent::Transactional | QueryIntent::Tool => {
            "not-applicable"
        }
    }
}

fn mode_name(mode: UncertaintyMode) -> &'static str {
    match mode {
        UncertaintyMode::Deterministic => "deterministic",
        UncertaintyMode::Calibrated => "calibrated",
        UncertaintyMode::EvidentialOnly => "evidential_only",
    }
}

fn is_web_discovery(result: &SearchResultItem) -> bool {
    result.source != "local" && result.source != "cached"
}

fn domain(url: &str) -> String {
    url.split("//")
        .nth(1)
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or("")
        .trim_start_matches("www.")
        .to_ascii_lowercase()
}

fn stable_query_ref(query: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(query.as_bytes());
    digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry_record() -> crate::pesqele::PesqEleRecord {
        crate::pesqele::PesqEleRecord {
            registration_id: "BR-04496/2026".to_string(),
            registered_at: "15/08/2026".to_string(),
            office: "Presidente".to_string(),
            disclosure_date: "21/08/2026".to_string(),
            institute: "Datafolha".to_string(),
            election: "Eleições Gerais 2026".to_string(),
            respondents: "2058".to_string(),
            fieldwork_start: "18/08/2026".to_string(),
            fieldwork_end: "20/08/2026".to_string(),
            statistician: "Renata Nunes Cesar".to_string(),
            statistician_registration: "7249A".to_string(),
            scope: "Brasil".to_string(),
            cost: "R$ 307.641,60".to_string(),
            self_funded: "Não".to_string(),
            contractors: "Folha e Globo".to_string(),
            methodology: "Entrevistas presenciais".to_string(),
            sampling_plan: "Margem de erro de dois pontos".to_string(),
            source_url: "https://pesqele-divulgacao.tse.jus.br/app/pesquisa/listar.xhtml"
                .to_string(),
            fetched_at: 0,
        }
    }

    fn result(url: &str, title: &str, score: f64, source: &str) -> SearchResultItem {
        SearchResultItem {
            result_id: url.to_string(),
            rank: 0,
            url: url.to_string(),
            title: title.to_string(),
            snippet: title.to_string(),
            content_md: if source == "local" {
                title.repeat(20)
            } else {
                String::new()
            },
            semantic_score: score,
            bm25_score: 0.0,
            final_score: score,
            stake_truth: 0,
            stake_usd: 0,
            stake_type: "none".to_string(),
            reputation: 0.0,
            content_hash: String::new(),
            extraction_quality: "full".to_string(),
            source: source.to_string(),
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

    #[test]
    fn navigational_query_keeps_retrieval_order() {
        let mut results = vec![
            result("https://claude.com/", "Claude", 0.9, "duckduckgo"),
            result(
                "https://en.wikipedia.org/wiki/Claude",
                "Claude",
                0.8,
                "duckduckgo",
            ),
        ];
        apply_decision_policy("claude", &mut results);
        assert_eq!(results[0].url, "https://claude.com/");
        assert_eq!(
            results[0].decision.as_ref().unwrap().uncertainty_mode,
            "not_applicable"
        );
    }

    #[test]
    fn high_impact_guidance_uses_evidential_uncertainty_without_inventing_probability() {
        let mut results = vec![result(
            "https://public-health.example/guidance",
            "Public health guidance about neurological symptoms",
            0.8,
            "local",
        )];

        apply_decision_policy("neurological syndrome symptoms", &mut results);

        let decision = results[0].decision.as_ref().unwrap();
        assert_eq!(decision.uncertainty_mode, "evidential_only");
        assert!(decision.p_reliable.is_none());
        assert!(!decision.cash_eligible);
        assert!(decision.evidence_signals.iter().any(
            |signal| signal.starts_with("high_stakes_") || signal == "query_relative_authority"
        ));
        assert_eq!(
            horizon_for("neurological syndrome symptoms", QueryIntent::Exploratory),
            "query-time"
        );
    }

    #[test]
    fn exploratory_evidential_mode_is_visible_in_the_answer_warning() {
        let query = "benefits of moringa and ginger";
        let mut results = vec![result(
            "https://health.example/moringa",
            "Moringa and ginger: evidence and safety",
            0.8,
            "serper",
        )];

        apply_decision_policy(query, &mut results);
        let answer = crate::answer::construct_answer(query, QueryIntent::Exploratory, &results);

        assert_eq!(answer.uncertainty_mode, "evidential_only");
        assert!(
            answer
                .warning
                .as_deref()
                .is_some_and(|warning| warning.contains("não há probabilidade calibrada"))
        );
    }

    #[test]
    fn ordinary_exploration_uses_evidential_reliance_policy() {
        let mut results = vec![result(
            "https://reference.example/vector-search",
            "Vector search explained",
            0.8,
            "local",
        )];

        apply_decision_policy("vector search explained", &mut results);

        assert_eq!(
            results[0].decision.as_ref().unwrap().uncertainty_mode,
            "evidential_only"
        );
        assert_eq!(
            horizon_for("vector search explained", QueryIntent::Exploratory),
            "query-time"
        );
    }

    #[test]
    fn query_relative_public_authority_beats_a_cold_blog_inside_relevance_band() {
        let blog = result(
            "https://tips.example/nebula-privacy-complaint",
            "How to file a Nebula privacy complaint",
            0.86,
            "serper",
        );
        let authority = result(
            "https://privacy.agency.gov/nebula/complaints",
            "Nebula privacy complaints — official filing guidance",
            0.80,
            "serper",
        );
        let mut results = vec![blog, authority];

        apply_decision_policy("how to file a Nebula privacy complaint", &mut results);

        assert!(results[0].url.contains("agency.gov"));
        assert!(
            results[0]
                .decision
                .as_ref()
                .unwrap()
                .evidence_signals
                .contains(&"query_relative_source_role_primary".to_string())
        );
        assert!(results[0].decision.as_ref().unwrap().p_reliable.is_none());
    }

    #[test]
    fn edited_lexical_reference_beats_unverified_ugc_inside_relevance_band() {
        let community = result(
            "https://www.quora.com/What-does-quasarite-mean",
            "What does quasarite mean?",
            0.88,
            "serper",
        );
        let dictionary = result(
            "https://dictionary.cambridge.org/dictionary/english/quasarite",
            "QUASARITE definition",
            0.82,
            "serper",
        );
        let mut results = vec![community, dictionary];

        apply_decision_policy("definition of quasarite", &mut results);

        assert!(results[0].url.contains("cambridge.org"));
        assert!(
            results[0]
                .decision
                .as_ref()
                .unwrap()
                .evidence_signals
                .contains(&"query_relative_source_role_edited_reference".to_string())
        );
    }

    #[test]
    fn first_party_product_fact_beats_a_cold_aggregator_inside_band() {
        let aggregator = result(
            "https://cards.example/orbitair-annual-fee",
            "OrbitAir card annual fee",
            0.88,
            "serper",
        );
        let first_party = result(
            "https://orbitair.com/cards/fees",
            "OrbitAir cards and annual fees",
            0.81,
            "serper",
        );
        let mut results = vec![aggregator, first_party];

        apply_decision_policy("what is the annual fee for OrbitAir", &mut results);

        assert!(results[0].url.contains("orbitair.com"));
        assert!(
            results[0]
                .decision
                .as_ref()
                .unwrap()
                .evidence_signals
                .contains(&"query_relative_source_role_primary".to_string())
        );
    }

    #[test]
    fn source_role_never_rescues_an_off_topic_authority_outside_band() {
        let relevant = result(
            "https://garden.example/quasar-orchids",
            "How to grow quasar orchids",
            0.90,
            "serper",
        );
        let authority = result(
            "https://plants.agency.gov/unrelated-ferns",
            "Official fern import rules",
            0.70,
            "serper",
        );
        let mut results = vec![relevant, authority];

        apply_decision_policy("how to grow quasar orchids", &mut results);

        assert!(results[0].url.contains("garden.example"));
        assert!(
            results[1]
                .decision
                .as_ref()
                .unwrap()
                .evidence_signals
                .contains(&"outside_top_relevance_band".to_string())
        );
    }

    #[test]
    fn experiential_query_does_not_blanket_demote_community_evidence() {
        let community = result(
            "https://reddit.com/r/gadgets/comments/orbitphone",
            "OrbitPhone reviews and owner experiences",
            0.88,
            "serper",
        );
        let article = result(
            "https://devices.example/orbitphone-review",
            "OrbitPhone review",
            0.82,
            "serper",
        );
        let mut results = vec![community, article];

        apply_decision_policy("OrbitPhone reviews and user experience", &mut results);

        assert!(results[0].url.contains("reddit.com"));
        assert!(
            results[0]
                .decision
                .as_ref()
                .unwrap()
                .evidence_signals
                .contains(&"query_relative_source_role_unclassified".to_string())
        );
    }

    #[test]
    fn trust_cannot_rescue_an_off_topic_retained_page() {
        let mut results = vec![
            result(
                "https://retained.example/unrelated",
                "Unrelated retained article",
                0.51,
                "local",
            ),
            result(
                "https://reference.example/correct-topic",
                "Correct topical reference",
                0.79,
                "serper",
            ),
        ];

        apply_decision_policy("how did the novalume language originate", &mut results);

        assert_eq!(results[0].url, "https://reference.example/correct-topic");
        assert!(
            results[1]
                .decision
                .as_ref()
                .unwrap()
                .evidence_signals
                .contains(&"outside_top_relevance_band".to_string())
        );
    }

    #[test]
    fn current_article_beats_topic_hub_and_preserves_dual_rank() {
        let mut results = vec![
            result(
                "https://example.com/tudo-sobre/pesquisas-eleitorais/",
                "Pesquisas eleitorais",
                0.90,
                "local",
            ),
            result(
                "https://news.example/2026/08/21/pesquisa-eleitoral.html",
                "Pesquisa eleitoral presidencial divulgada hoje",
                0.80,
                "duckduckgo",
            ),
        ];
        apply_decision_policy("pesquisa eleitoral presidente brasil hoje", &mut results);
        assert_eq!(
            results[0].url,
            "https://news.example/2026/08/21/pesquisa-eleitoral.html"
        );
        let decision = results[0].decision.as_ref().unwrap();
        assert_eq!(decision.retrieval_rank, 2);
        assert_eq!(decision.decision_rank, 1);
        assert_eq!(decision.uncertainty_mode, "evidential_only");
        assert!(decision.p_reliable.is_none());
        assert!(!decision.cash_eligible);
        assert_eq!(decision.cash_log_bayes_factor, 0.0);
    }

    #[test]
    fn official_poll_id_collapses_editorial_derivations_to_one_measurement() {
        let mut first = result(
            "https://news.example/poll",
            "Datafolha: Lula 39%, Flávio 33%; registro BR-04496/2026",
            0.90,
            "local",
        );
        first.content_md = first.title.repeat(8);
        let mut second = result(
            "https://paper.example/poll",
            "Pesquisa: Lula 39% e Flávio 33% (BR-04496/2026)",
            0.89,
            "local",
        );
        second.content_md = second.title.repeat(8);
        let mut inferred = result(
            "https://editorial.example/poll",
            "Datafolha: Flávio 33% e Lula 39% no primeiro turno",
            0.88,
            "local",
        );
        inferred.content_md = inferred.title.repeat(8);
        let unrelated = result(
            "https://other.example/poll",
            "Datafolha: Lula 40% e Flávio 31% no primeiro turno",
            0.87,
            "local",
        );
        let mut results = vec![first, second, inferred, unrelated];

        apply_decision_policy_with_registry(
            "pesquisa eleitoral presidente brasil hoje",
            &mut results,
            &[registry_record()],
        );

        let representative = results
            .iter()
            .find(|result| result.url.contains("news.example"))
            .unwrap()
            .decision
            .as_ref()
            .unwrap();
        let duplicate = results
            .iter()
            .find(|result| result.url.contains("paper.example"))
            .unwrap()
            .decision
            .as_ref()
            .unwrap();
        let inferred = results
            .iter()
            .find(|result| result.url.contains("editorial.example"))
            .unwrap()
            .decision
            .as_ref()
            .unwrap();
        let unrelated = results
            .iter()
            .find(|result| result.url.contains("other.example"))
            .unwrap()
            .decision
            .as_ref()
            .unwrap();
        assert_eq!(representative.independence, 1.0);
        assert_eq!(duplicate.independence, 0.0);
        assert_eq!(inferred.independence, 0.0);
        assert_eq!(unrelated.independence, 1.0);
        assert_eq!(representative.dependency_uncertainty, 0.05);
        assert!(
            representative
                .evidence_lineage
                .as_ref()
                .unwrap()
                .representative
        );
        assert!(
            representative
                .evidence_lineage
                .as_ref()
                .unwrap()
                .registry_verified
        );
        assert_eq!(duplicate.evidence_lineage.as_ref().unwrap().cluster_size, 3);
        assert_eq!(
            inferred.evidence_lineage.as_ref().unwrap().relation,
            "inferred_editorial_report_of"
        );
        assert!(unrelated.evidence_lineage.is_none());
        assert!(
            inferred
                .evidence_signals
                .contains(&"measurement_link_inferred_from_registered_claim_signature".to_string())
        );
        assert!(
            duplicate
                .evidence_signals
                .contains(&"editorial_derivation_duplicate".to_string())
        );
        assert!(representative.decision_score > duplicate.decision_score);
    }

    #[test]
    fn published_results_beat_an_article_that_only_announces_the_release() {
        let mut preview = result(
            "https://news.example/2026/08/21/que-horas-sai-a-pesquisa.ghtml",
            "Que horas sai a pesquisa Datafolha hoje? Veja previsão",
            0.95,
            "local",
        );
        preview.snippet =
            "A nova pesquisa deve sair às 18h30; veja como estava a rodada anterior.".to_string();
        let results = result(
            "https://paper.example/2026/08/21/datafolha-resultados.html",
            "Datafolha: Lula tem 39% e Flávio Bolsonaro 33% no primeiro turno",
            0.80,
            "local",
        );
        let mut candidates = vec![preview, results];

        apply_decision_policy("pesquisa eleitoral presidente brasil hoje", &mut candidates);

        assert_eq!(
            candidates[0].url,
            "https://paper.example/2026/08/21/datafolha-resultados.html"
        );
        assert!(
            candidates[1]
                .decision
                .as_ref()
                .unwrap()
                .evidence_signals
                .iter()
                .any(|signal| signal == "future_results_may_be_absent")
        );
    }

    #[test]
    fn editorial_current_news_beats_a_higher_scored_press_release() {
        let press_release = result(
            "https://example.com/2026/08/21/tmt-newswire/globenewswire/ai-finance",
            "Newswire: AI robots transform digital finance",
            0.90,
            "local",
        );
        let editorial = result(
            "https://news.example/2026/08/21/ai-policy.html",
            "OpenAI calls for stronger AI laws",
            0.80,
            "local",
        );
        let mut candidates = vec![press_release, editorial];

        apply_decision_policy("latest artificial intelligence news", &mut candidates);

        assert!(candidates[0].url.contains("news.example"));
        assert!(
            candidates[1]
                .decision
                .as_ref()
                .unwrap()
                .evidence_signals
                .iter()
                .any(|signal| signal == "press_release_not_independent_reporting")
        );
    }

    #[test]
    fn ambiguous_short_query_enters_reliance_when_no_destination_is_verified() {
        let mut results = vec![
            result(
                "https://www.reddit.com/r/reference/comments/orvexa",
                "Orvexa cuisine",
                0.84,
                "serper",
            ),
            result(
                "https://en.wikipedia.org/wiki/Orvexa_cuisine",
                "Orvexa cuisine",
                0.82,
                "serper",
            ),
        ];

        apply_decision_policy("orvexa cuisine", &mut results);

        assert!(results[0].url.contains("wikipedia.org"));
        let decision = results[0].decision.as_ref().expect("decision metadata");
        assert_eq!(decision.uncertainty_mode, "evidential_only");
        assert!(
            decision
                .evidence_signals
                .contains(&"ambiguous_navigation_requires_reliance_assessment".to_string())
        );
    }

    #[test]
    fn due_diligence_never_fabricates_probability_or_cash_effect() {
        let mut results = vec![result("https://gamafund.com/", "Gama Fund", 0.9, "local")];
        apply_decision_policy("gama fund é legítimo?", &mut results);
        let decision = results[0].decision.as_ref().unwrap();
        assert_eq!(decision.uncertainty_mode, "evidential_only");
        assert!(decision.p_reliable.is_none());
        assert!(!decision.cash_eligible);
        assert!(
            decision
                .evidence_signals
                .iter()
                .any(|signal| signal == "self_declaration_not_independent_confirmation")
        );
    }

    #[test]
    fn due_diligence_demotes_event_pages_below_independent_reporting() {
        let event = result(
            "https://luma.com/gama-fund-ama",
            "AMA - Gama Fund",
            0.90,
            "local",
        );
        let report = result(
            "https://news.example/gama-fund-google-monashees",
            "Gama Fund, de Google e Monashees, abre seleção",
            0.86,
            "local",
        );
        assert!(is_non_verifying_action_page(&event));
        let mut results = vec![event, report];
        apply_decision_policy("gama fund é legítimo?", &mut results);
        assert_eq!(
            results[0].url,
            "https://news.example/gama-fund-google-monashees",
            "scores: {:?}",
            results
                .iter()
                .map(|result| (
                    &result.url,
                    result.decision.as_ref().unwrap().decision_score,
                    &result.decision.as_ref().unwrap().evidence_signals,
                ))
                .collect::<Vec<_>>()
        );
        assert!(
            results[1]
                .decision
                .as_ref()
                .unwrap()
                .evidence_signals
                .contains(&"event_or_registration_page_not_verification".to_string())
        );
    }

    #[test]
    fn due_diligence_demotes_retained_homonym_below_named_external_result() {
        let homonym = result(
            "https://gama-am.ch/en/bond-funds/",
            "GAMA Asset Management bond funds",
            0.92,
            "local",
        );
        let named = result(
            "https://news.example/gama-fund-google-monashees",
            "Gama Fund, de Google e Monashees",
            0.76,
            "duckduckgo",
        );
        let mut results = vec![homonym, named];
        apply_decision_policy("gama fund é legítimo?", &mut results);
        assert_eq!(
            results[0].url,
            "https://news.example/gama-fund-google-monashees"
        );
        assert!(
            results[1]
                .decision
                .as_ref()
                .unwrap()
                .evidence_signals
                .contains(&"named_entity_mismatch_not_evidence".to_string())
        );
    }

    #[test]
    fn due_diligence_demotes_self_declaration_below_named_external_result() {
        let official = result("https://gamafund.com/", "Gama Fund", 0.90, "local");
        let named = result(
            "https://news.example/gama-fund-google-monashees",
            "Gama Fund, de Google e Monashees",
            0.76,
            "duckduckgo",
        );
        let mut results = vec![official, named];
        apply_decision_policy("gama fund é legítimo?", &mut results);
        assert_eq!(
            results[0].url,
            "https://news.example/gama-fund-google-monashees"
        );
    }
}
