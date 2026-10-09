//! Conservative answer construction over the decided result slate.
//!
//! This is intentionally extractive. It never asks a language model to fill
//! gaps, never turns `EVIDENTIAL_ONLY` into a probability, and abstains when
//! two retained editorial families do not expose a compatible answer-bearing
//! statement.

use crate::query_intent::{
    QueryIntent, authoritative_source_for_query, intent_adjustment, is_current_office_holder_query,
    is_current_preview, is_hub_url, is_implicit_location_query, is_live_value_query, is_poll_query,
    is_recommendation_query, primary_source_url_matches_claim, relevance_terms, source_family,
};
use oi_types::SearchResultItem;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnswerCitation {
    /// `claim_text` for a ranked page that states the answer, or
    /// `registry_metadata` for out-of-band structured validation.
    pub evidence_role: String,
    /// One-based rank for ordinary results; zero for structured evidence that
    /// was joined after ranking and therefore never competed in retrieval.
    pub decision_rank: u32,
    pub title: String,
    pub url: String,
    pub source: String,
    pub evidence_excerpt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnswerClaim {
    pub text: String,
    pub citations: Vec<AnswerCitation>,
    /// Claim-level derivation structure. This keeps source-count and
    /// underlying-event-count separate so copied reporting cannot masquerade
    /// as multiple independent measurements.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_lineage: Option<ClaimEvidenceLineage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimEvidenceLineage {
    pub underlying_unit_kind: String,
    pub underlying_unit_ids: Vec<String>,
    /// `official_registry_joined`, `declared_registration_unresolved`,
    /// `normalized_claim_signature_only`, or `unresolved`.
    pub identity_status: String,
    /// Known only when the underlying real-world unit identity was resolved.
    /// A normalized factual predicate groups compatible wording, but does not
    /// prove that citations are independent observations; that remains `None`.
    pub underlying_unit_count: Option<usize>,
    pub reporting_source_families: usize,
    pub claim_text_citations: usize,
    /// What is known about dependence between the cited reporting families.
    /// This is deliberately separate from source count.
    #[serde(default)]
    pub independence_status: String,
    /// Count only when a structured join resolves independent underlying
    /// observations. `None` means unknown, not zero.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub known_independent_observations: Option<usize>,
    /// Plausible common-ancestry channels not resolved by the current graph.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unresolved_dependencies: Vec<String>,
    pub registry_metadata_records: usize,
    pub registry_verifies: Vec<String>,
    pub registry_does_not_verify: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchAnswer {
    /// `destination`, `extractive`, `insufficient_evidence`, or `results_only`.
    pub status: String,
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
    pub search_passes: u32,
    pub hydrated_pages: usize,
    /// Official poll registrations joined from TSE/PesqEle when an exact
    /// registration ID was present in retained editorial content.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub poll_registry: Vec<crate::pesqele::PesqEleRecord>,
    /// Query-time structured market observations. These are joined after
    /// retrieval and never compete for Stage-1 rank.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub market_quotes: Vec<crate::market_data::MarketQuoteRecord>,
    pub trace: AnswerTrace,
}

/// Wall-clock decomposition of the synchronous answered-search path. These
/// stages are observational only and never participate in ranking or trust.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactualChunkSelectionTrace {
    pub known_urls: usize,
    pub relation_bearing_urls: usize,
    pub reporting_families: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnswerTrace {
    /// Selection attempts in execution order, without protected excerpts.
    /// Counts are structural and do not establish source independence.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub factual_chunk_selections: Vec<FactualChunkSelectionTrace>,
    /// Provider availability observed at completion; not an absence-of-facts
    /// signal and not a substitute for per-query transport diagnostics.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unavailable_discovery_providers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_checklist: Option<crate::checklist::EvidenceChecklist>,
    pub initial_retrieval_ms: f64,
    /// Purpose-preserving discovery rewrites used only after the literal
    /// retrieval returned no recommendation results.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub retrieval_expansion_queries: Vec<String>,
    pub initial_hydration_ms: f64,
    pub corroboration_discovery_ms: f64,
    /// Provider discovery performed for the first claim-specific expansion.
    #[serde(default)]
    pub corroboration_provider_discovery_ms: f64,
    /// Exact claim-derived rewrites issued during the first corroboration
    /// pass. These are observational and make the production path auditable.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub corroboration_queries: Vec<String>,
    /// Raw unique provider candidates collected before original-query scope.
    #[serde(default)]
    pub corroboration_discovered_candidates: usize,
    /// URLs observed before scope filtering. Bounded by the discovery limit
    /// and exposed so false-negative retention can be audited in production.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub corroboration_discovered_urls: Vec<String>,
    /// Candidates that survived original-query intent and semantic scope.
    #[serde(default)]
    pub corroboration_retained_candidates: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub corroboration_retained_urls: Vec<String>,
    /// Exact URLs selected from the retained discovery set for foreground
    /// hydration. Provider snippets are not evidence; this exposes the
    /// bounded bridge from discovery to crawl for production audits.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub corroboration_hydration_urls: Vec<String>,
    /// Selected URLs for which at least one locally indexed content chunk was
    /// recovered after hydration. This distinguishes crawl/index recall from
    /// downstream entailment or compatibility failures.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub corroboration_exact_evidence_urls: Vec<String>,
    /// Official TSE list/detail discovery used to obtain poll registration IDs.
    #[serde(default)]
    pub poll_registration_discovery_ms: f64,
    /// Provider discovery performed by the bounded post-refresh scenario pass.
    #[serde(default)]
    pub poll_scenario_discovery_ms: f64,
    pub corroboration_hydration_ms: f64,
    pub retrieval_refresh_ms: f64,
    pub answer_construction_ms: f64,
    pub total_ms: f64,
    pub primary_source_expansion: bool,
    #[serde(default)]
    pub poll_scenario_expansion: bool,
    /// A broad poll contained multiple same-measurement first-round slates
    /// and the initially corroborated claim was explicitly marked as a
    /// secondary/alternate slate in one retained source. The pipeline issued
    /// a bounded exact-pair discovery pass for the primary slate rather than
    /// silently presenting the alternate one as the topline.
    #[serde(default)]
    pub poll_primary_scenario_audit: bool,
    /// A completed, still-fresh claim decision was reused from the
    /// process-local burst cache. Provenance remains attached to the answer;
    /// network and hydration stage timings are reset for this request.
    #[serde(default)]
    pub fresh_answer_cache_hit: bool,
    /// A best-effort factual completion crawl was omitted after the initial
    /// foreground hydration had already consumed the synchronous budget. The
    /// answer still passes through the normal fail-closed evidence gate.
    #[serde(default)]
    pub corroboration_hydration_budget_exhausted: bool,
    /// Number of factual-reference candidates returned by the bounded
    /// encyclopedia fallback before original-query scope filtering.
    #[serde(default)]
    pub factual_reference_results: usize,
    /// Relation-specific retained/discovery candidates available for display
    /// if the answer abstains. Observational only; these are never citations.
    #[serde(default)]
    pub factual_display_fallback_candidates: usize,
    /// Relation-specific results actually returned beside a factual
    /// abstention after final filtering and top-k truncation.
    #[serde(default)]
    pub factual_display_results: usize,
    pub fresh_local_evidence_hit: bool,
    pub initial_hydration_requested: usize,
    pub corroboration_hydration_requested: usize,
    pub poll_registry_lookup_ms: f64,
    pub poll_registry_ids: usize,
    pub poll_registry_records: usize,
    pub poll_registry_errors: usize,
    /// Bounded query-time structured market-data join.
    #[serde(default)]
    pub market_quote_lookup_ms: f64,
    #[serde(default)]
    pub market_quote_records: usize,
    #[serde(default)]
    pub market_quote_errors: usize,
}

pub fn construct_answer(
    query: &str,
    intent: QueryIntent,
    results: &[SearchResultItem],
) -> SearchAnswer {
    construct_answer_with_registry(query, intent, results, &[])
}

pub(crate) fn construct_answer_with_registry(
    query: &str,
    intent: QueryIntent,
    results: &[SearchResultItem],
    poll_registry: &[crate::pesqele::PesqEleRecord],
) -> SearchAnswer {
    construct_answer_with_structured_evidence(query, intent, results, poll_registry, &[])
}

pub(crate) fn construct_answer_with_structured_evidence(
    query: &str,
    intent: QueryIntent,
    results: &[SearchResultItem],
    poll_registry: &[crate::pesqele::PesqEleRecord],
    market_quotes: &[crate::market_data::MarketQuoteRecord],
) -> SearchAnswer {
    construct_answer_with_checklist(query, intent, results, poll_registry, market_quotes).0
}

/// Keep the checklist taken against the full retained answer evidence, before
/// display filtering. Rechecking an already-cleared answer loses the reason
/// for abstention, and display-only top-k is not the evidence set.
pub(crate) fn construct_answer_with_checklist(
    query: &str,
    intent: QueryIntent,
    results: &[SearchResultItem],
    poll_registry: &[crate::pesqele::PesqEleRecord],
    market_quotes: &[crate::market_data::MarketQuoteRecord],
) -> (SearchAnswer, crate::checklist::EvidenceChecklist) {
    let mut answer = construct_answer_with_evidence_unchecked(
        query,
        intent,
        results,
        poll_registry,
        market_quotes,
    );
    let checklist = crate::checklist::enforce(query, &mut answer, results);
    (answer, checklist)
}

fn construct_answer_with_evidence_unchecked(
    query: &str,
    intent: QueryIntent,
    results: &[SearchResultItem],
    poll_registry: &[crate::pesqele::PesqEleRecord],
    market_quotes: &[crate::market_data::MarketQuoteRecord],
) -> SearchAnswer {
    let (uncertainty_mode, average_ignorance) = uncertainty_summary(results);
    let evidence_warning = (uncertainty_mode == "evidential_only").then(|| {
        "Modo evidencial: não há probabilidade calibrada, e a dependência entre fontes ainda não foi resolvida."
            .to_string()
    });

    if intent == QueryIntent::CurrentEvents
        && is_live_value_query(query)
        && let Some(answer) = live_value_answer(query, market_quotes, average_ignorance)
    {
        return answer;
    }

    let Some(top) = results.first() else {
        let summary = if intent == QueryIntent::Factual {
            "Nenhuma evidência factual suficiente foi retida. Para não apresentar uma resposta ou lista parcial como completa, a resposta foi suspensa."
        } else if intent == QueryIntent::Exploratory && query_requests_implicit_location(query) {
            "A consulta pede resultados próximos, mas nenhuma cidade, bairro ou coordenada foi fornecida. Informe a localização para obter uma lista geograficamente válida."
        } else {
            "Nenhum resultado foi recuperado."
        };
        let warning = if intent == QueryIntent::Exploratory {
            exploratory_warning(query).or(evidence_warning)
        } else {
            evidence_warning
        };
        return SearchAnswer {
            status: "insufficient_evidence".to_string(),
            intent: intent.to_string(),
            summary: summary.to_string(),
            claims: Vec::new(),
            uncertainty_mode,
            average_ignorance,
            warning,
        };
    };

    match intent {
        QueryIntent::Navigational => {
            let signals = intent_adjustment(query, top).signals;
            let verified_destination = signals.contains(&"exact_entity_host")
                || signals.contains(&"fuzzy_entity_host")
                || signals.contains(&"public_authority_entity_path");
            if verified_destination {
                SearchAnswer {
                    status: "destination".to_string(),
                    intent: intent.to_string(),
                    summary: format!(
                        "Destino mais provável: {} — {}",
                        destination_label(top),
                        top.url
                    ),
                    claims: Vec::new(),
                    uncertainty_mode,
                    average_ignorance,
                    warning: None,
                }
            } else {
                SearchAnswer {
                    status: "results_only".to_string(),
                    intent: intent.to_string(),
                    summary: "Consulta curta ou ambígua: os resultados foram preservados, mas nenhum host canônico ou caminho de autoridade confirmou um destino único.".to_string(),
                    claims: Vec::new(),
                    uncertainty_mode,
                    average_ignorance,
                    warning: Some(
                        "Não interprete o primeiro resultado como site oficial sem uma correspondência verificável de entidade e origem."
                            .to_string(),
                    ),
                }
            }
        }
        QueryIntent::Transactional => SearchAnswer {
            status: "destination".to_string(),
            intent: intent.to_string(),
            summary: format!(
                "Melhor destino encontrado para a ação: {} — {}",
                destination_label(top),
                top.url
            ),
            claims: Vec::new(),
            uncertainty_mode,
            average_ignorance,
            warning: None,
        },
        QueryIntent::Tool => SearchAnswer {
            status: "tool_results".to_string(),
            intent: intent.to_string(),
            summary: format!(
                "Ferramentas funcionais encontradas; primeira opção: {} — {}",
                destination_label(top),
                top.url
            ),
            claims: Vec::new(),
            uncertainty_mode,
            average_ignorance,
            warning: None,
        },
        QueryIntent::DueDiligence => SearchAnswer {
            status: "insufficient_evidence".to_string(),
            intent: intent.to_string(),
            summary: "Os resultados recuperados não bastam para concluir legitimidade. Declarações da própria entidade, páginas de inscrição e cobertura sobre sua existência não substituem verificação jurídica, operacional e independente."
                .to_string(),
            claims: Vec::new(),
            uncertainty_mode,
            average_ignorance,
            warning: evidence_warning,
        },
        QueryIntent::Factual => factual_answer(query, intent, results, uncertainty_mode, average_ignorance, evidence_warning),
        QueryIntent::CurrentEvents => current_events_answer(
            query,
            intent,
            results,
            poll_registry,
            uncertainty_mode,
            average_ignorance,
            evidence_warning,
        ),
        QueryIntent::Exploratory => {
            let summary = if query_requests_implicit_location(query) {
                "Consulta local sem localização explícita: os resultados temáticos foram preservados, mas não estão validados como próximos de você. Informe cidade, bairro ou coordenada para ordenar por distância."
            } else {
                "Consulta exploratória: os resultados foram preservados sem sintetizar uma única conclusão."
            };
            SearchAnswer {
                status: "results_only".to_string(),
                intent: intent.to_string(),
                summary: summary.to_string(),
                claims: Vec::new(),
                uncertainty_mode,
                average_ignorance,
                warning: exploratory_warning(query).or(evidence_warning),
            }
        }
    }
}

fn query_requests_implicit_location(query: &str) -> bool {
    is_implicit_location_query(query)
}

fn exploratory_warning(query: &str) -> Option<String> {
    if query_requests_implicit_location(query) {
        return Some(
            "A localização do usuário não está disponível; estes resultados não são personalizados por distância."
                .to_string(),
        );
    }
    let terms = relevance_terms(query);
    if terms.iter().any(|term| {
        matches!(
            term.as_str(),
            "dengue"
                | "diagnostico"
                | "doenca"
                | "medica"
                | "medico"
                | "sintoma"
                | "sintomas"
                | "tratamento"
        )
    }) {
        return Some(
            "Consulta de saúde: fontes institucionais são priorizadas quando disponíveis, mas páginas não oficiais permanecem sem validação clínica e os resultados não substituem avaliação médica."
                .to_string(),
        );
    }
    if is_recommendation_query(query) {
        return Some(
            "Consulta de recomendação: relações comerciais, comissões de afiliados e independência editorial das fontes ainda não foram verificadas; compare critérios antes de comprar."
                .to_string(),
        );
    }
    None
}

fn destination_label(result: &SearchResultItem) -> String {
    if !result.title.trim().is_empty() {
        return result.title.trim().to_string();
    }
    result
        .url
        .split("//")
        .nth(1)
        .unwrap_or(&result.url)
        .split('/')
        .next()
        .unwrap_or(&result.url)
        .trim_start_matches("www.")
        .to_string()
}

/// Build a claim-specific follow-up query from the best retained poll result.
/// It carries the user's original scope, the leading percentage pair, and up
/// to subject tokens extracted from the retained claim; no candidate,
/// outlet, or institute is hardcoded.
pub(crate) fn poll_corroboration_queries(query: &str, results: &[SearchResultItem]) -> Vec<String> {
    if !is_poll_query(query) {
        return Vec::new();
    }
    let mut candidates = Vec::new();
    for result in results.iter().take(40) {
        if is_hub_url(&result.url) {
            continue;
        }
        // A demographic-breakdown page must not become final topline evidence:
        // its body contains many candidate/percentage pairs outside the broad
        // query's scope. Its retained headline can still be a precise recall
        // hint for another ordinary article. Discovery is allowed to use that
        // pair; final answer construction below continues to reject the page.
        let candidate_texts = if poll_breakdown_outside_query_scope(query, result) {
            vec![result.title.clone()]
        } else {
            candidate_sentences(result)
        };
        for raw_text in candidate_texts {
            let text = clean_poll_candidate(&raw_text);
            if has_repeated_leading_percent_pair(&text)
                || poll_pair_crosses_section_boundary(&text)
                || has_non_candidate_leading_percentage(&text)
            {
                continue;
            }
            let signature = percent_signature(&text);
            let keys = ordered_poll_candidate_keys(&text);
            if signature.len() < 2 || keys.len() < 2 {
                continue;
            }
            let folded = relevance_terms(&text).join(" ");
            let mut score = keys.len();
            if folded.contains("intencao voto") || folded.contains("intencoes voto") {
                score += 20;
            }
            if contains_first_round(&folded) {
                score += 10;
            }
            score += 10usize.saturating_sub(result.rank as usize);
            let context_key = poll_context_key(result, &keys);
            candidates.push((score, signature, keys, context_key));
        }
    }
    candidates.sort_by(|left, right| right.0.cmp(&left.0));
    let Some((_, signature, keys, context_key)) = candidates.first() else {
        return Vec::new();
    };
    // Keep enough proper-name tokens to cover both people when the first has
    // a multi-part name (taking only three often captured one candidate).
    let subjects = keys.iter().take(8).cloned().collect::<Vec<_>>().join(" ");
    let context = context_key
        .as_deref()
        .map(|key| format!(" {key}"))
        .unwrap_or_default();
    let primary = format!(
        "{query} {}% {}% {subjects}{context}",
        signature[0], signature[1]
    );
    let exact = format!(
        "\"{}%\" \"{}%\" {subjects}{context} pesquisa eleitoral",
        signature[0], signature[1]
    );
    let mut queries = vec![primary, exact];
    let mut registration_ids = results
        .iter()
        .flat_map(crate::pesqele::registration_ids_for_result)
        .collect::<Vec<_>>();
    registration_ids.sort();
    registration_ids.dedup();
    for registration_id in registration_ids.into_iter().take(2) {
        queries.push(format!(
            "\"{registration_id}\" pesquisa eleitoral resultado"
        ));
    }
    queries.dedup();
    queries
}

/// A broad poll query should make one bounded attempt to cover both the first
/// round and a runoff before presenting its scenario slate as complete. This
/// is a recall decision only: every added scenario must still pass the same
/// two-family claim and official-lineage gates.
pub(crate) fn poll_answer_needs_scenario_expansion(query: &str, answer: &SearchAnswer) -> bool {
    if !is_poll_query(query) {
        return false;
    }
    let query_folded = relevance_terms(query).join(" ");
    if contains_first_round(&query_folded) || contains_second_round(&query_folded) {
        return false;
    }
    let (has_first_round, has_second_round) = poll_claim_round_coverage(query, &answer.claims);
    if brazilian_presidential_round_model(query) {
        // A true cold query can have promising retained poll pages but no
        // two-family claim yet. Requiring an already-extractive answer here
        // creates a deadlock: the first/second-round discovery needed to
        // corroborate the claim is only attempted after the claim exists.
        // Expand both bounded round queries while evidence is insufficient;
        // the normal two-family and official-registry gates still decide
        // whether anything may be emitted.
        if answer.status == "insufficient_evidence" {
            return true;
        }
        if answer.status != "extractive" {
            return false;
        }
        !(has_first_round && has_second_round)
    } else {
        answer.status == "extractive" && (has_first_round ^ has_second_round)
    }
}

pub(crate) fn poll_scenario_completion_queries(
    query: &str,
    results: &[SearchResultItem],
    answer: &SearchAnswer,
) -> Vec<String> {
    if !poll_answer_needs_scenario_expansion(query, answer) {
        return Vec::new();
    }
    let (has_first_round, has_second_round) = poll_claim_round_coverage(query, &answer.claims);
    let context = results
        .iter()
        .take(10)
        .find_map(|result| poll_context_key(result, &[]));
    let mut queries = Vec::new();
    for (present, round) in [
        (has_first_round, "primeiro turno"),
        (has_second_round, "segundo turno"),
    ] {
        if present {
            continue;
        }
        queries.push(format!("{query} {round}"));
        if let Some(context) = &context {
            queries.push(format!("pesquisa eleitoral {round} {context}"));
        }
    }
    queries.sort();
    queries.dedup();
    queries
}

/// Detect a subtle but consequential poll failure mode: two first-round
/// slates from the same registered measurement can share the same leading
/// candidates but differ by a point because one adds/removes another name.
/// If the only initially corroborated slate is explicitly identified as
/// secondary inside a retained source, build a bounded exact-pair query for
/// the unqualified primary slate found alongside it.
///
/// This function does not promote the uncorroborated values into the answer.
/// It only asks discovery for another reporting family; the normal
/// two-family and official-lineage gates still decide whether the claim may be
/// emitted.
pub(crate) fn poll_primary_scenario_corroboration_queries(
    query: &str,
    results: &[SearchResultItem],
    answer: &SearchAnswer,
) -> Vec<String> {
    if !brazilian_presidential_round_model(query)
        || answer.status != "extractive"
        || contains_first_round(&relevance_terms(query).join(" "))
        || contains_second_round(&relevance_terms(query).join(" "))
    {
        return Vec::new();
    }
    let Some(first_round_claim) = answer
        .claims
        .iter()
        .find(|claim| poll_claim_round(query, &claim.text) == Some("first"))
    else {
        return Vec::new();
    };
    let emitted_signature =
        canonical_percent_signature(&percent_signature(&first_round_claim.text));
    let emitted_identities = poll_candidate_identity_sets(&first_round_claim.text);
    if emitted_signature.len() != 2 || emitted_identities.len() != 2 {
        return Vec::new();
    }

    #[derive(Default)]
    struct Variant {
        raw_signature: Vec<String>,
        keys: Vec<String>,
        has_primary_context: bool,
        has_secondary_context: bool,
        title_match: bool,
    }

    let latest_poll_publication = latest_poll_publication(results);
    for result in results.iter().take(40) {
        if result.content_md.trim().is_empty()
            || is_hub_url(&result.url)
            || is_current_preview(result)
            || poll_breakdown_outside_query_scope(query, result)
            || !poll_result_is_in_latest_window(result, latest_poll_publication)
        {
            continue;
        }
        let title_signature = canonical_percent_signature(&percent_signature(&result.title));
        let mut variants: HashMap<Vec<String>, Variant> = HashMap::new();
        for raw_text in candidate_sentences(result) {
            let text = clean_poll_candidate(&raw_text);
            if has_historical_poll_context(&text)
                || has_conflicting_poll_round_context(&text)
                || has_repeated_leading_percent_pair(&text)
                || poll_pair_crosses_section_boundary(&text)
                || has_non_candidate_leading_percentage(&text)
                || poll_claim_round(query, &text) != Some("first")
            {
                continue;
            }
            let raw_signature = percent_signature(&text);
            let identities = poll_candidate_identity_sets(&text);
            if raw_signature.len() != 2
                || identities.len() != 2
                || !poll_identity_sets_compatible(&emitted_identities, &identities)
            {
                continue;
            }
            let signature = canonical_percent_signature(&raw_signature);
            let scenario = poll_scenario_label(&text)
                .into_iter()
                .chain(poll_scenario_context(result, &text))
                .max_by_key(|label| poll_scenario_specificity(label));
            let secondary = scenario.as_deref().is_some_and(poll_scenario_is_secondary);
            let keys = ordered_poll_candidate_keys(&text);
            let entry = variants.entry(signature.clone()).or_default();
            if entry.raw_signature.is_empty() {
                entry.raw_signature = raw_signature;
            }
            if entry.keys.len() < keys.len() {
                entry.keys = keys;
            }
            entry.has_secondary_context |= secondary;
            entry.has_primary_context |= !secondary;
            entry.title_match |= signature == title_signature;
        }

        // Require the relationship to be explicit in one retained page: the
        // emitted pair is labelled as a secondary slate and a compatible
        // unqualified pair is present beside it. This avoids comparing
        // unrelated polls or dates that merely reuse the same candidates.
        let Some(emitted_variant) = variants.get(&emitted_signature) else {
            continue;
        };
        if !emitted_variant.has_secondary_context {
            continue;
        }
        let preferred = variants
            .iter()
            .filter(|(signature, variant)| {
                *signature != &emitted_signature
                    && variant.has_primary_context
                    && !variant.has_secondary_context
                    && variant.raw_signature.len() == 2
                    && variant.keys.len() >= 2
            })
            .max_by_key(|(_, variant)| {
                let leading_total = variant
                    .raw_signature
                    .iter()
                    .take(2)
                    .filter_map(|value| value.replace(',', ".").parse::<f64>().ok())
                    .sum::<f64>()
                    .round() as usize;
                usize::from(variant.title_match) * 100 + leading_total
            });
        let Some((_, preferred)) = preferred else {
            continue;
        };
        let subjects = preferred
            .keys
            .iter()
            .take(8)
            .cloned()
            .collect::<Vec<_>>()
            .join(" ");
        let context = poll_context_key(result, &preferred.keys)
            .map(|value| format!(" {value}"))
            .unwrap_or_default();
        let first = &preferred.raw_signature[0];
        let second = &preferred.raw_signature[1];
        let mut queries = vec![
            format!(
                "\"{first}%\" \"{second}%\" {subjects}{context} pesquisa eleitoral primeiro turno"
            ),
            format!("{query} {first}% {second}% {subjects}{context}"),
        ];
        for registration_id in crate::pesqele::registration_ids_for_result(result)
            .into_iter()
            .take(1)
        {
            queries.push(format!(
                "\"{registration_id}\" \"{first}%\" \"{second}%\" resultado"
            ));
        }
        queries.sort();
        queries.dedup();
        return queries;
    }
    Vec::new()
}

fn poll_scenario_is_secondary(label: &str) -> bool {
    let lower = label.to_lowercase();
    [
        "cenário com",
        "cenario com",
        "cenário sem",
        "cenario sem",
        "cenário alternativo",
        "cenario alternativo",
        "cenário espontâneo",
        "cenario espontaneo",
    ]
    .iter()
    .any(|cue| lower.contains(cue))
}

fn poll_claim_round_coverage(query: &str, claims: &[AnswerClaim]) -> (bool, bool) {
    let rounds = claims
        .iter()
        .filter_map(|claim| poll_claim_round(query, &claim.text))
        .collect::<HashSet<_>>();
    (rounds.contains("first"), rounds.contains("second"))
}

fn brazilian_presidential_round_model(query: &str) -> bool {
    let terms = relevance_terms(query);
    terms
        .iter()
        .any(|term| matches!(term.as_str(), "brasil" | "brazil" | "brasileira"))
        && terms.iter().any(|term| {
            matches!(
                term.as_str(),
                "presidente" | "presidencia" | "presidencial" | "presidential"
            )
        })
}

fn poll_claim_round(query: &str, claim: &str) -> Option<&'static str> {
    let folded = relevance_terms(claim).join(" ");
    if contains_second_round(&folded) {
        Some("second")
    } else if contains_first_round(&folded) || brazilian_presidential_round_model(query) {
        // For a broad Brazilian presidential query, an unlabeled stimulated
        // or named-slate scenario is a first-round result unless the retained
        // source explicitly says runoff. This jurisdictional rule prevents
        // two first-round slate variants from impersonating round coverage.
        Some("first")
    } else {
        None
    }
}

fn poll_context_key(result: &SearchResultItem, candidate_keys: &[String]) -> Option<String> {
    const GENERIC: &[&str] = &[
        "eleicao",
        "eleitoral",
        "intencao",
        "numero",
        "numeros",
        "pesquisa",
        "presidente",
        "resultado",
        "resultados",
        "turno",
        "voto",
    ];
    relevance_terms(&result.title).into_iter().find(|term| {
        term.len() >= 4 && !GENERIC.contains(&term.as_str()) && !candidate_keys.contains(term)
    })
}

pub(crate) fn poll_registry_context_terms(results: &[SearchResultItem]) -> Vec<String> {
    let mut counts = HashMap::new();
    for result in results.iter().take(20) {
        if let Some(term) = poll_context_key(result, &[]) {
            *counts.entry(term).or_insert(0usize) += 1;
        }
    }
    let mut terms = counts.into_iter().collect::<Vec<_>>();
    terms.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    let repeated = terms
        .iter()
        .filter(|(_, count)| *count >= 2)
        .take(3)
        .map(|(term, _)| term.clone())
        .collect::<Vec<_>>();
    if !repeated.is_empty() {
        repeated
    } else {
        // With no consensus, one best term is safer than feeding several
        // unrelated headline words into the official company selector.
        terms.into_iter().take(1).map(|(term, _)| term).collect()
    }
}

/// Build a claim-specific factual follow-up from the best answer-bearing
/// sentence already retained. The extracted proper-name keys narrow discovery
/// to pages that can confirm the proposed answer; they do not turn that answer
/// into fact until another source family exposes a compatible statement.
pub(crate) fn factual_corroboration_queries(
    query: &str,
    results: &[SearchResultItem],
) -> Vec<String> {
    let mut answer_candidates = results
        .iter()
        .take(40)
        .filter_map(|result| {
            let text = best_factual_sentence(query, result)?;
            let mut keys = ordered_answer_keys(query, &text);
            keys.truncate(4);
            (!keys.is_empty()).then_some(keys)
        })
        .collect::<Vec<_>>();
    // Retrieval rank is not a trustworthy proxy for the object of a
    // relation. A top page can contain a syntactically valid but generic
    // phrase (for example "linha do tempo") before another retained page
    // identifies the public institution that actually created the entity.
    // Prefer an authority-shaped answer when one is present anywhere in the
    // bounded evidence window; evidence construction still has to verify it.
    answer_candidates.sort_by_key(|keys| {
        std::cmp::Reverse(answer_likely_names_public_authority(&keys.join(" ")))
    });
    if let Some(keys) = answer_candidates.first() {
        let answer = keys.join(" ");
        let factual_terms = crate::query_intent::factual_query_terms(query);
        let relation = factual_terms
            .iter()
            .find(|term| matches!(term.as_str(), "created" | "founded" | "invented"))
            .cloned()
            .unwrap_or_default();
        let entity = factual_terms
            .iter()
            .filter(|term| *term != &relation)
            .cloned()
            .collect::<Vec<_>>()
            .join(" ");
        let mut queries = Vec::new();
        if !entity.is_empty() {
            // Normalize the follow-up around the extracted entity/answer
            // pair rather than the original surface form. This coalesces
            // paraphrases and explicitly gives a primary record a chance
            // to enter the retained evidence set. These are discovery
            // hints only: fetched content must still entail the claim and
            // the authority URL must match the institution in the claim.
            let pair = format!("\"{entity}\" \"{answer}\"");
            queries.push(pair.clone());
            if answer_likely_names_public_authority(&answer) {
                queries.push(format!("site:gov.br {pair}"));
            } else if relation == "founded" {
                // Founder discovery commonly starts from separate retained
                // biographies. A single-name query tends to return only that
                // person's profile and one encyclopedia family. Combine the
                // distinct answer keys already extracted from the retained
                // evidence into one relation-scoped discovery query. The
                // names are derived from fetched pages, never hard-coded, and
                // still cannot become an answer until complete corroboration.
                let entity_terms = crate::query_intent::relevance_terms(&entity);
                let mut founder_keys = Vec::new();
                for key in answer_candidates.iter().take(6).flatten() {
                    if !entity_terms.contains(key) && !founder_keys.contains(key) {
                        founder_keys.push(key.clone());
                    }
                    if founder_keys.len() == 6 {
                        break;
                    }
                }
                if founder_keys.len() >= 2 {
                    let named = founder_keys
                        .iter()
                        .map(|key| format!("\"{key}\""))
                        .collect::<Vec<_>>()
                        .join(" ");
                    queries.push(format!("\"{entity}\" {named} founders cofounders"));
                } else {
                    queries.push(format!("{pair} official source"));
                }
            } else {
                queries.push(format!("{pair} official source"));
            }
            if relation == "founded" || founder_enumeration_has_partial_cue(query, results) {
                queries.push(format!("\"{entity}\" founders cofounders complete list"));
            }
        } else {
            queries.push(format!("{query} {answer}"));
        }
        queries.dedup();
        return queries;
    }
    // A noisy local hit can survive initial retrieval while every retained
    // sentence fails the relation-direction gate. Do not let that unrelated
    // page suppress one bounded, relation-specific recovery pass.
    let factual_terms = crate::query_intent::factual_query_terms(query);
    let relation = factual_terms
        .iter()
        .find(|term| matches!(term.as_str(), "created" | "founded" | "invented"))
        .map(String::as_str);
    let entity = factual_terms
        .iter()
        .filter(|term| !matches!(term.as_str(), "created" | "founded" | "invented"))
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    if !entity.is_empty() {
        return match relation {
            Some("founded") => vec![
                format!("\"{entity}\" founders cofounders complete list"),
                format!("\"{entity}\" founded by official company history"),
            ],
            Some("created") => vec![
                format!("\"{entity}\" created by founded by origin"),
                format!("\"{entity}\" founders cofounders complete list"),
            ],
            Some("invented") => vec![format!("\"{entity}\" invented created by official history")],
            _ => Vec::new(),
        };
    }
    Vec::new()
}

fn answer_likely_names_public_authority(answer: &str) -> bool {
    let terms = relevance_terms(answer);
    let has = |term: &str| terms.iter().any(|candidate| candidate == term);
    (has("banco") && has("central"))
        || [
            "agencia",
            "agency",
            "court",
            "governo",
            "government",
            "ministerio",
            "ministry",
            "prefeitura",
            "tribunal",
        ]
        .iter()
        .any(|term| has(term))
}

pub(crate) fn factual_candidate_suggests_primary_authority(
    query: &str,
    results: &[SearchResultItem],
) -> bool {
    results.iter().take(40).any(|result| {
        best_factual_sentence(query, result).is_some_and(|text| {
            let answer = ordered_answer_keys(query, &text).join(" ");
            !answer.is_empty() && answer_likely_names_public_authority(&answer)
        })
    })
}

pub(crate) fn factual_answer_may_be_incomplete(
    query: &str,
    results: &[SearchResultItem],
    answer: &SearchAnswer,
) -> bool {
    if answer.status != "extractive"
        || !crate::query_intent::factual_query_terms(query)
            .iter()
            .any(|term| matches!(term.as_str(), "created" | "founded"))
    {
        return false;
    }
    founder_enumeration_has_partial_cue(query, results)
}

/// True only for a factual abstention that is explained by retained evidence
/// of an incomplete founder enumeration. This is materially different from
/// "the provider returned nothing": callers may reuse this decision for a
/// very short burst window without converting discovery failure into a
/// cached negative fact.
pub(crate) fn factual_answer_is_bounded_abstention(
    query: &str,
    results: &[SearchResultItem],
    answer: &SearchAnswer,
) -> bool {
    answer.status == "insufficient_evidence"
        && answer.claims.is_empty()
        && answer
            .summary
            .starts_with("As fontes retidas confirmam parte dos fundadores")
        && founder_enumeration_has_partial_cue(query, results)
}

fn founder_enumeration_has_partial_cue(query: &str, results: &[SearchResultItem]) -> bool {
    let query_terms = crate::query_intent::factual_query_terms(query);
    if !query_terms
        .iter()
        .any(|term| matches!(term.as_str(), "created" | "founded"))
    {
        return false;
    }
    let entity_terms: Vec<_> = query_terms
        .into_iter()
        .filter(|term| !matches!(term.as_str(), "created" | "founded" | "invented"))
        .collect();
    const PARTIAL_CUES: &[&str] = &[
        "grupo de colegas",
        "incluindo",
        "including",
        "entre eles",
        "among them",
        "e outros",
        "and others",
        "junto com outros",
        "along with other",
        "other cofounders",
        "other former",
        "other researchers",
        "outros cofundadores",
        "outros ex-membros",
        "outros pesquisadores",
    ];
    results.iter().take(40).any(|result| {
        let public_surface = format!("{} {}", result.title, result.url).to_lowercase();
        let public_surface_terms = relevance_terms(&public_surface);
        let count_cue = founder_count_cue(&public_surface);
        let relation_cue = public_surface_terms.iter().any(|term| term == "founded");
        let entity_covered = entity_terms
            .iter()
            .all(|term| public_surface_terms.contains(term));
        if count_cue && relation_cue && entity_covered {
            return true;
        }

        let sentences = factual_candidate_sentences(result);
        sentences.iter().enumerate().any(|(index, _)| {
            // Use a bounded three-sentence window. Enumeration markers often
            // follow the sentence that introduces the company and founder
            // relation, but a cue elsewhere on a long page must not taint an
            // unrelated factual query.
            let end = (index + 3).min(sentences.len());
            let context = format!(
                "{} {} {}",
                result.title,
                result.url,
                sentences[index..end].join(" ")
            )
            .to_lowercase();
            let context_terms = relevance_terms(&context);
            (PARTIAL_CUES.iter().any(|cue| context.contains(cue)) || founder_count_cue(&context))
                && context_terms.iter().any(|term| term == "founded")
                && entity_terms.iter().all(|term| context_terms.contains(term))
                && sentences[index..end].iter().any(|sentence| {
                    // Enumeration is an abstention/recall cue, not an
                    // asserted founding claim: infinitive "to found" may
                    // signal a wider group even for a creator question.
                    !sentence.contains('?')
                        && !has_negated_origin_relation(sentence)
                        && relevance_terms(sentence)
                            .iter()
                            .any(|term| term == "founded")
                        && ((factual_entity_terms_covered(query, sentence)
                            && factual_relation_direction_matches_entity(query, sentence))
                            || (factual_entity_terms_covered(query, &result.title)
                                && sentence_can_inherit_entity_context(query, sentence)))
                })
        })
    })
}

fn founder_count_cue(context: &str) -> bool {
    let tokenized: String = context
        .chars()
        .map(|character| {
            if character.is_alphanumeric() {
                character
            } else {
                ' '
            }
        })
        .collect();
    let tokens: Vec<_> = tokenized.split_whitespace().collect();
    tokens.iter().enumerate().any(|(index, term)| {
        let plausible_count = term
            .parse::<usize>()
            .is_ok_and(|count| (3..=50).contains(&count))
            || matches!(
                *term,
                "tres"
                    | "quatro"
                    | "cinco"
                    | "seis"
                    | "sete"
                    | "oito"
                    | "nove"
                    | "dez"
                    | "three"
                    | "four"
                    | "five"
                    | "six"
                    | "seven"
                    | "eight"
                    | "nine"
                    | "ten"
            );
        if !plausible_count {
            return false;
        }
        let lower = index.saturating_sub(6);
        let upper = (index + 7).min(tokens.len());
        tokens[lower..upper].iter().any(|nearby| {
            relevance_terms(nearby)
                .iter()
                .any(|normalized| normalized == "founded")
        })
    })
}

#[derive(Clone)]
struct ExtractiveCandidate<'a> {
    text: String,
    result: &'a SearchResultItem,
    answer_keys: HashSet<String>,
    poll_identities: Option<Vec<HashSet<String>>>,
}

fn factual_answer(
    query: &str,
    intent: QueryIntent,
    results: &[SearchResultItem],
    uncertainty_mode: String,
    average_ignorance: Option<f64>,
    warning: Option<String>,
) -> SearchAnswer {
    let mut candidates = Vec::new();
    for result in results.iter().take(40) {
        if result.content_md.trim().is_empty() {
            continue;
        }
        if let Some(text) = best_factual_sentence(query, result) {
            candidates.push(ExtractiveCandidate {
                answer_keys: answer_keys(query, &text),
                poll_identities: None,
                text,
                result,
            });
        }
    }

    let complete_enumeration_required = founder_enumeration_has_partial_cue(query, results);
    candidates.sort_by(|left, right| {
        if complete_enumeration_required {
            right
                .answer_keys
                .len()
                .cmp(&left.answer_keys.len())
                .then_with(|| {
                    primary_source_url_matches_claim(query, &right.text, &right.result.url).cmp(
                        &primary_source_url_matches_claim(query, &left.text, &left.result.url),
                    )
                })
                .then_with(|| left.text.chars().count().cmp(&right.text.chars().count()))
                .then_with(|| left.result.rank.cmp(&right.result.rank))
        } else {
            primary_source_url_matches_claim(query, &right.text, &right.result.url)
                .cmp(&primary_source_url_matches_claim(
                    query,
                    &left.text,
                    &left.result.url,
                ))
                .then_with(|| left.text.chars().count().cmp(&right.text.chars().count()))
                .then_with(|| left.result.rank.cmp(&right.result.rank))
        }
    });
    let primary_index = candidates.iter().position(|candidate| {
        candidates.iter().any(|other| {
            source_family(&candidate.result.url) != source_family(&other.result.url)
                && factual_candidates_compatible(
                    &candidate.answer_keys,
                    &other.answer_keys,
                    complete_enumeration_required,
                )
        })
    });

    if let Some(primary_index) = primary_index {
        let primary = &candidates[primary_index];
        let mut citations = vec![citation(primary.result, &primary.text)];
        for other in &candidates {
            if source_family(&primary.result.url) != source_family(&other.result.url)
                && factual_candidates_compatible(
                    &primary.answer_keys,
                    &other.answer_keys,
                    complete_enumeration_required,
                )
            {
                citations.push(citation(other.result, &other.text));
            }
        }
        dedup_citations(&mut citations);
        let evidence_lineage = factual_claim_lineage(query, &primary.answer_keys, &citations);
        return SearchAnswer {
            status: "extractive".to_string(),
            intent: intent.to_string(),
            summary: primary.text.clone(),
            claims: vec![AnswerClaim {
                text: primary.text.clone(),
                citations,
                evidence_lineage: Some(evidence_lineage),
            }],
            uncertainty_mode,
            average_ignorance,
            warning,
        };
    }

    let summary = if complete_enumeration_required {
        "As fontes retidas confirmam parte dos fundadores, mas também indicam outros cofundadores sem fornecer duas listas completas e compatíveis. Para não apresentar uma lista parcial como completa, a resposta foi suspensa."
            .to_string()
    } else {
        candidates.first().map_or_else(
        || "Não foi encontrada uma afirmação retida que responda diretamente à pergunta. Para não apresentar uma resposta ou lista parcial como completa, a resposta foi suspensa."
            .to_string(),
        |candidate| {
            format!(
                "Uma fonte contém uma resposta possível — “{}” —, mas não houve confirmação compatível em outro domínio retido. Para não apresentar uma resposta ou lista parcial como completa, ela não foi emitida como afirmação.",
                candidate.text
            )
        },
        )
    };
    SearchAnswer {
        status: "insufficient_evidence".to_string(),
        intent: intent.to_string(),
        summary,
        // `claims` is the asserted answer surface, not a debug channel. A
        // single possible sentence remains visible in the explanatory
        // summary above, but it must not become a machine-readable claim
        // until another source family corroborates it.
        claims: Vec::new(),
        uncertainty_mode,
        average_ignorance,
        warning,
    }
}

/// Group compatible factual wording under a stable predicate signature while
/// remaining explicit about what is *not* known. Two publisher families can
/// repeat the same relation without being two independent observations, so
/// this lineage records the reporting cardinality but leaves the underlying
/// real-world unit count unresolved.
fn factual_claim_lineage(
    query: &str,
    answer_keys: &HashSet<String>,
    citations: &[AnswerCitation],
) -> ClaimEvidenceLineage {
    use sha2::{Digest, Sha256};

    let mut relation_terms = crate::query_intent::factual_query_terms(query);
    relation_terms.sort();
    relation_terms.dedup();
    let mut normalized_answers = answer_keys.iter().cloned().collect::<Vec<_>>();
    normalized_answers.sort();
    let signature = format!(
        "{}\u{1f}{}",
        relation_terms.join("\u{1e}"),
        normalized_answers.join("\u{1e}")
    );
    let digest = Sha256::digest(signature.as_bytes());
    let unit_ref = digest[..12]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let reporting_source_families = citations
        .iter()
        .map(|citation| source_family(&citation.url))
        .collect::<HashSet<_>>()
        .len();
    ClaimEvidenceLineage {
        underlying_unit_kind: "factual_relation_claim".to_string(),
        underlying_unit_ids: vec![format!("claim:{unit_ref}")],
        identity_status: "normalized_claim_signature_only".to_string(),
        underlying_unit_count: None,
        reporting_source_families,
        claim_text_citations: citations.len(),
        independence_status: "unknown_editorial_derivation".to_string(),
        known_independent_observations: None,
        unresolved_dependencies: vec![
            "syndication".to_string(),
            "quotation".to_string(),
            "shared_upstream_source".to_string(),
        ],
        registry_metadata_records: 0,
        registry_verifies: Vec::new(),
        registry_does_not_verify: Vec::new(),
    }
}

fn factual_candidates_compatible(
    left: &HashSet<String>,
    right: &HashSet<String>,
    complete_enumeration_required: bool,
) -> bool {
    if !complete_enumeration_required {
        return !left.is_disjoint(right);
    }
    // A sentence that names only the two most visible founders must not
    // corroborate a purported complete list. Requiring a substantial,
    // near-identical name set makes the system abstain when sources expose a
    // partial list or disagree about membership.
    // Proper-name tokens are only an approximation of people. Six distinct
    // keys is conservative enough to stop "Dario + Daniela + Amodei +
    // OpenAI" from masquerading as four founders, while still allowing two
    // genuinely substantial and near-identical enumerations to corroborate.
    if left.len() < 6 || right.len() < 6 {
        return false;
    }
    let intersection = left.intersection(right).count();
    let union = left.union(right).count();
    intersection * 100 >= union * 90
}

fn live_value_answer(
    query: &str,
    market_quotes: &[crate::market_data::MarketQuoteRecord],
    average_ignorance: Option<f64>,
) -> Option<SearchAnswer> {
    let now_ms = crate::market_data::unix_time_ms();
    let quotes = crate::market_data::validated_quote_records(market_quotes, now_ms)?;
    let first = quotes.first()?;
    let min = quotes
        .iter()
        .map(|quote| quote.value)
        .fold(f64::INFINITY, f64::min);
    let max = quotes
        .iter()
        .map(|quote| quote.value)
        .fold(f64::NEG_INFINITY, f64::max);
    let midpoint = (min + max) / 2.0;
    let oldest_observation = quotes
        .iter()
        .map(|quote| quote.observed_at_unix_ms)
        .min()
        .unwrap_or(now_ms);
    let maximum_age_seconds = now_ms.saturating_sub(oldest_observation) / 1_000;
    let formatted_midpoint = format_market_value(midpoint, &first.quote_currency);
    let formatted_min = format_market_value(min, &first.quote_currency);
    let formatted_max = format_market_value(max, &first.quote_currency);
    let claim_text = format!(
        "Cotação indicativa de 1 {}: aproximadamente {} (faixa observada {}–{} entre {} fontes estruturadas; observação mais antiga há {} s).",
        first.asset_symbol,
        formatted_midpoint,
        formatted_min,
        formatted_max,
        quotes.len(),
        maximum_age_seconds,
    );
    let citations = quotes
        .iter()
        .map(|quote| AnswerCitation {
            evidence_role: "structured_market_quote".to_string(),
            decision_rank: 0,
            title: format!(
                "{} {} / {}",
                quote.provider, quote.asset_symbol, quote.quote_currency
            ),
            url: quote.source_url.clone(),
            source: format!("{}_market_data", quote.provider),
            evidence_excerpt: Some(format!(
                "{}; observed_at_unix_ms={}; kind={}",
                format_market_value(quote.value, &quote.quote_currency),
                quote.observed_at_unix_ms,
                quote.observation_kind,
            )),
        })
        .collect::<Vec<_>>();
    let unit_ids = quotes
        .iter()
        .map(|quote| {
            format!(
                "{}:{}:{}:{}",
                quote.provider,
                quote.asset_id,
                quote.quote_currency.to_ascii_lowercase(),
                quote.observed_at_unix_ms
            )
        })
        .collect::<Vec<_>>();
    Some(SearchAnswer {
        status: "extractive".to_string(),
        intent: QueryIntent::CurrentEvents.to_string(),
        summary: claim_text.clone(),
        claims: vec![AnswerClaim {
            text: claim_text,
            citations,
            evidence_lineage: Some(ClaimEvidenceLineage {
                underlying_unit_kind: "market_quote_snapshot".to_string(),
                underlying_unit_ids: unit_ids,
                identity_status: "asset_currency_timestamp_joined".to_string(),
                underlying_unit_count: Some(quotes.len()),
                reporting_source_families: quotes.len(),
                claim_text_citations: quotes.len(),
                independence_status: "cross_provider_dependency_unresolved".to_string(),
                known_independent_observations: None,
                unresolved_dependencies: vec![
                    "aggregator_may_include_exchange".to_string(),
                    "shared_market_microstructure".to_string(),
                ],
                registry_metadata_records: 0,
                registry_verifies: vec![
                    "asset_identity".to_string(),
                    "quote_currency".to_string(),
                    "provider_observation_timestamp".to_string(),
                    "bounded_cross_provider_spread".to_string(),
                ],
                registry_does_not_verify: vec![
                    "executable_fill_price".to_string(),
                    "future_value".to_string(),
                    "provider_independence".to_string(),
                    "investment_suitability".to_string(),
                ],
            }),
        }],
        uncertainty_mode: "evidential_only".to_string(),
        average_ignorance,
        warning: Some(format!(
            "Cotação indicativa, não recomendação financeira nem preço executável. As fontes podem compartilhar dados subjacentes; a dependência entre CoinGecko e Binance não foi tratada como observações independentes. Consulta original: {query}"
        )),
    })
}

fn format_market_value(value: f64, currency: &str) -> String {
    let prefix = match currency {
        "BRL" => "R$ ",
        "USD" => "US$ ",
        other => return format!("{value:.2} {other}"),
    };
    let raw = format!("{value:.2}");
    let (integer, decimals) = raw.split_once('.').unwrap_or((&raw, "00"));
    let mut grouped_reversed = String::new();
    for (index, character) in integer.chars().rev().enumerate() {
        if index > 0 && index % 3 == 0 {
            grouped_reversed.push('.');
        }
        grouped_reversed.push(character);
    }
    let grouped = grouped_reversed.chars().rev().collect::<String>();
    format!("{prefix}{grouped},{decimals}")
}

fn current_events_answer(
    query: &str,
    intent: QueryIntent,
    results: &[SearchResultItem],
    poll_registry: &[crate::pesqele::PesqEleRecord],
    uncertainty_mode: String,
    average_ignorance: Option<f64>,
    warning: Option<String>,
) -> SearchAnswer {
    if !is_poll_query(query) {
        if is_current_office_holder_query(query) {
            return current_office_holder_answer(
                query,
                intent,
                results,
                uncertainty_mode,
                average_ignorance,
                warning,
            );
        }
        let summary = if is_live_value_query(query) {
            "Foram encontrados resultados sobre o valor atual, mas o pipeline ainda não validou conjuntamente o valor, a moeda/unidade e o horário da cotação. Nenhum número foi sintetizado."
        } else {
            "Foram encontrados links de descoberta, mas o pipeline ainda não tem uma política validada para responder a este tipo de consulta atual. A data ou a atualidade desses links pode não estar confirmada. Nenhuma afirmação sobre o estado atual foi sintetizada."
        };
        return SearchAnswer {
            status: "insufficient_evidence".to_string(),
            intent: intent.to_string(),
            summary: summary.to_string(),
            claims: Vec::new(),
            uncertainty_mode,
            average_ignorance,
            warning,
        };
    }
    let latest_poll_publication = latest_poll_publication(results);
    let mut by_signature: HashMap<Vec<String>, Vec<ExtractiveCandidate<'_>>> = HashMap::new();
    for result in results.iter().take(40) {
        // Search-engine titles and snippets are discovery hints, not retained
        // evidence. A claim may only be synthesized from content that the
        // crawler actually fetched and indexed.
        if result.content_md.trim().is_empty()
            || is_hub_url(&result.url)
            || is_current_preview(result)
            || poll_breakdown_outside_query_scope(query, result)
        {
            continue;
        }
        if !poll_result_is_in_latest_window(result, latest_poll_publication) {
            continue;
        }
        for raw_text in candidate_sentences(result) {
            let text = clean_poll_candidate(&raw_text);
            if has_historical_poll_context(&text)
                || has_conflicting_poll_round_context(&text)
                || has_repeated_leading_percent_pair(&text)
                || poll_pair_crosses_section_boundary(&text)
                || has_non_candidate_leading_percentage(&text)
            {
                continue;
            }
            let signature = percent_signature(&text);
            let candidate_keys = poll_candidate_keys(&text);
            if signature.len() < 2 || candidate_keys.len() < 2 {
                continue;
            }
            by_signature
                .entry(canonical_percent_signature(&signature))
                .or_default()
                .push(ExtractiveCandidate {
                    poll_identities: Some(poll_candidate_identity_sets(&text)),
                    text,
                    result,
                    answer_keys: candidate_keys,
                });
        }
    }

    let mut corroborated = Vec::new();
    for signature_group in by_signature.into_values() {
        // The same small percentages recur for different candidates. Cluster
        // equal numeric signatures by compatible names before counting source
        // families; otherwise “2% / 1%” can falsely corroborate another pair.
        let mut name_clusters: Vec<Vec<ExtractiveCandidate<'_>>> = Vec::new();
        for candidate in signature_group {
            if let Some(cluster) = name_clusters.iter_mut().find(|cluster| {
                cluster
                    .iter()
                    .any(|other| poll_identities_compatible(&candidate, other))
            }) {
                cluster.push(candidate);
            } else {
                name_clusters.push(vec![candidate]);
            }
        }
        // One numeric signature may coincidentally appear for two unrelated
        // candidate pairs or in duplicated DOM blocks. Keep only the best
        // independently corroborated identity cluster for that signature.
        if let Some(best_cluster) = name_clusters
            .into_iter()
            .filter(|group| poll_source_family_count(group) >= 2)
            .max_by(|left, right| {
                poll_source_family_count(left)
                    .cmp(&poll_source_family_count(right))
                    .then_with(|| poll_group_score(left).cmp(&poll_group_score(right)))
            })
        {
            corroborated.push(best_cluster);
        }
    }
    corroborated.sort_by(|left, right| {
        poll_source_family_count(right)
            .cmp(&poll_source_family_count(left))
            .then_with(|| poll_group_score(right).cmp(&poll_group_score(left)))
            .then_with(|| {
                left.iter()
                    .map(|candidate| candidate.result.rank)
                    .min()
                    .cmp(&right.iter().map(|candidate| candidate.result.rank).min())
            })
    });

    let anchor_group = corroborated.first().cloned();
    let mut claims = Vec::new();
    let mut emitted_scenarios = HashSet::new();
    let mut emitted_rounds = HashSet::new();
    let enforce_round_slots = brazilian_presidential_round_model(query)
        && !contains_first_round(&relevance_terms(query).join(" "))
        && !contains_second_round(&relevance_terms(query).join(" "));
    let mut matched_registry_ids = HashSet::new();
    for mut group in corroborated {
        let same_protagonists = anchor_group.as_ref().is_some_and(|anchor| {
            group.iter().any(|candidate| {
                anchor
                    .iter()
                    .any(|other| poll_identities_compatible(candidate, other))
            })
        });
        if !claims.is_empty() && !same_protagonists {
            continue;
        }
        group.sort_by_key(|candidate| candidate.result.rank);
        let primary = group
            .iter()
            .filter(|candidate| candidate.text.len() >= 55)
            .min_by(|left, right| {
                poll_primary_score(right)
                    .cmp(&poll_primary_score(left))
                    .then_with(|| left.text.chars().count().cmp(&right.text.chars().count()))
                    .then_with(|| left.result.rank.cmp(&right.result.rank))
            })
            .unwrap_or(&group[0]);
        let mut citations: Vec<_> = group
            .iter()
            .map(|item| citation(item.result, &item.text))
            .collect();
        let group_registration_ids: HashSet<_> = group
            .iter()
            .flat_map(|candidate| poll_registration_ids_for_answer(candidate.result).into_iter())
            .collect();
        let mut matched_group_registry_ids = Vec::new();
        for record in poll_registry {
            if group_registration_ids.contains(&record.registration_id) {
                citations.push(registry_citation(record));
                matched_registry_ids.insert(record.registration_id.clone());
                matched_group_registry_ids.push(record.registration_id.clone());
            }
        }
        matched_group_registry_ids.sort();
        matched_group_registry_ids.dedup();
        let mut declared_registration_ids: Vec<_> = group_registration_ids.into_iter().collect();
        declared_registration_ids.sort();
        let unresolved_registration_ids = declared_registration_ids
            .iter()
            .filter(|registration_id| !matched_group_registry_ids.contains(registration_id))
            .cloned()
            .collect::<Vec<_>>();
        let identity_status = if !matched_group_registry_ids.is_empty() {
            "official_registry_joined"
        } else if !declared_registration_ids.is_empty() {
            "declared_registration_unresolved"
        } else {
            "unresolved"
        };
        let evidence_lineage = ClaimEvidenceLineage {
            underlying_unit_kind: "poll_measurement".to_string(),
            underlying_unit_ids: if matched_group_registry_ids.is_empty() {
                declared_registration_ids
            } else {
                matched_group_registry_ids.clone()
            },
            identity_status: identity_status.to_string(),
            underlying_unit_count: (!matched_group_registry_ids.is_empty())
                .then_some(matched_group_registry_ids.len()),
            reporting_source_families: poll_source_family_count(&group),
            claim_text_citations: poll_source_family_count(&group),
            independence_status: if !matched_group_registry_ids.is_empty() {
                "registered_measurement_with_editorial_reports".to_string()
            } else {
                "editorial_derivation_unresolved".to_string()
            },
            known_independent_observations: (!matched_group_registry_ids.is_empty())
                .then_some(matched_group_registry_ids.len()),
            unresolved_dependencies: unresolved_registration_ids
                .into_iter()
                .map(|registration_id| format!("unresolved_registration:{registration_id}"))
                .collect(),
            registry_metadata_records: matched_group_registry_ids.len(),
            registry_verifies: (!matched_group_registry_ids.is_empty())
                .then(|| {
                    vec![
                        "registration_exists".to_string(),
                        "institute".to_string(),
                        "scope".to_string(),
                        "declared_methodology".to_string(),
                    ]
                })
                .unwrap_or_default(),
            registry_does_not_verify: vec![
                "reported_percentages".to_string(),
                "sampling_accuracy".to_string(),
                "methodological_bias".to_string(),
            ],
        };
        dedup_citations(&mut citations);
        let claim = poll_claim_with_supported_pair(&primary.text);
        let claim = poll_claim_with_group_context(&group, &claim);
        let round = poll_claim_round(query, &claim);
        // A Brazilian presidential runoff is a head-to-head contest. A
        // fragment such as "Flávio 33%; Caiado 5%" belongs to a first-round
        // slate even when a nearby related-link heading mentions the second
        // round. Never let that low-total pair fill the runoff slot.
        if !poll_round_values_plausible(query, &claim, round) {
            continue;
        }
        if enforce_round_slots && round.is_some_and(|round| emitted_rounds.contains(round)) {
            continue;
        }
        let scenario_key = poll_emission_scenario_key(&claim);
        if !claims.is_empty()
            && !scenario_key.is_empty()
            && emitted_scenarios.contains(&scenario_key)
        {
            continue;
        }
        if !scenario_key.is_empty() {
            emitted_scenarios.insert(scenario_key);
        }
        if let Some(round) = round {
            emitted_rounds.insert(round);
        }
        let claim = if round == Some("first") {
            let folded = relevance_terms(&claim).join(" ");
            if !contains_first_round(&folded) && !contains_second_round(&folded) {
                // Brazilian named-slate scenarios are structurally
                // first-round results, but source headings often say only
                // "cenário com/sem X". Make the jurisdictional inference
                // explicit in presentation instead of hiding it in the slot
                // classifier.
                format!("primeiro turno — {claim}")
            } else {
                claim
            }
        } else {
            claim
        };
        claims.push(AnswerClaim {
            text: claim,
            citations,
            evidence_lineage: Some(evidence_lineage),
        });
        if claims.len() == 2 {
            break;
        }
    }

    if !claims.is_empty() {
        let summary = claims
            .iter()
            .map(|claim| claim.text.as_str())
            .collect::<Vec<_>>()
            .join(" • ");
        let warning = if matched_registry_ids.is_empty() {
            warning
        } else {
            let mut registrations: Vec<_> = matched_registry_ids.into_iter().collect();
            registrations.sort();
            Some(format!(
                "Modo evidencial: as matérias que repetem os números foram agrupadas como uma única medição ({registrations}). O registro oficial do TSE confirma existência, instituto, escopo e metodologia declarada; ele não valida os percentuais publicados, não torna as matérias medições independentes e não elimina erro amostral ou viés metodológico.",
                registrations = registrations.join(", ")
            ))
        };
        let query_folded = relevance_terms(query).join(" ");
        let (has_first_round, has_second_round) = poll_claim_round_coverage(query, &claims);
        let broad_round_query =
            !contains_first_round(&query_folded) && !contains_second_round(&query_folded);
        let warning = if broad_round_query && (has_first_round ^ has_second_round) {
            let partial = "Cobertura parcial de cenários: a evidência retida não sustentou simultaneamente resultados de primeiro e segundo turno.";
            Some(match warning {
                Some(warning) => format!("{warning} {partial}"),
                None => partial.to_string(),
            })
        } else {
            warning
        };
        return SearchAnswer {
            status: "extractive".to_string(),
            intent: intent.to_string(),
            summary,
            claims,
            uncertainty_mode,
            average_ignorance,
            warning,
        };
    }

    SearchAnswer {
        status: "insufficient_evidence".to_string(),
        intent: intent.to_string(),
        summary: "Há resultados recentes, mas não encontrei o mesmo dado material em duas famílias editoriais retidas. Para evitar transformar uma manchete isolada em consenso, a resposta foi suspensa."
            .to_string(),
        claims: Vec::new(),
        uncertainty_mode,
        average_ignorance,
        warning,
    }
}

fn latest_poll_publication(results: &[SearchResultItem]) -> Option<i64> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    results
        .iter()
        .take(40)
        .filter_map(|result| result.published_at)
        .filter(|published_at| *published_at <= now.saturating_add(3_600))
        .max()
}

fn poll_result_is_in_latest_window(
    result: &SearchResultItem,
    latest_publication: Option<i64>,
) -> bool {
    const SAME_MEASUREMENT_REPORTING_WINDOW_SECS: i64 = 3 * 86_400;
    let Some(latest) = latest_publication else {
        return true;
    };
    result.published_at.is_some_and(|published_at| {
        published_at <= latest.saturating_add(3_600)
            && latest.saturating_sub(published_at) <= SAME_MEASUREMENT_REPORTING_WINDOW_SECS
    })
}

#[derive(Clone)]
struct OfficeHolderCandidate<'a> {
    name: String,
    excerpt: String,
    result: &'a SearchResultItem,
    official: bool,
}

fn current_office_holder_answer(
    query: &str,
    intent: QueryIntent,
    results: &[SearchResultItem],
    uncertainty_mode: String,
    average_ignorance: Option<f64>,
    warning: Option<String>,
) -> SearchAnswer {
    let candidates = office_holder_candidates(query, results);
    let official = candidates
        .iter()
        .filter(|candidate| candidate.official)
        .max_by_key(|candidate| office_name_keys(&candidate.name).len());
    if let Some(official) = official
        && let Some(independent) = candidates.iter().find(|candidate| {
            !candidate.official
                && is_independent_office_editorial(&candidate.result.url)
                && is_recent_office_editorial(candidate.result)
                && source_family(&candidate.result.url) != source_family(&official.result.url)
                && office_names_compatible(&official.name, &candidate.name)
                && office_editorial_surface_names_holder(candidate.result, &candidate.name)
        })
    {
        // The primary record owns the canonical spelling. The editorial page
        // corroborates identity but must not append a party acronym, title,
        // or other surface-specific suffix to the displayed name.
        let display_name = official.name.clone();
        let claim = format!("O atual presidente do Brasil é {display_name}.");
        let official_excerpt = office_claim_excerpt(&official.excerpt, &official.name);
        let independent_excerpt = office_claim_excerpt(&independent.excerpt, &independent.name);
        let mut citations = vec![
            citation(official.result, &official_excerpt),
            citation(independent.result, &independent_excerpt),
        ];
        dedup_citations(&mut citations);
        let normalized_name = office_name_keys(&display_name)
            .into_iter()
            .collect::<Vec<_>>()
            .join("-");
        let lineage = ClaimEvidenceLineage {
            underlying_unit_kind: "current_office_holder_claim".to_string(),
            underlying_unit_ids: vec![format!("office-holder:brasil:presidente:{normalized_name}")],
            identity_status: "official_primary_record_and_editorial_corroboration".to_string(),
            underlying_unit_count: Some(1),
            reporting_source_families: citations.len(),
            claim_text_citations: citations.len(),
            independence_status: "official_record_plus_independent_reporting".to_string(),
            known_independent_observations: None,
            unresolved_dependencies: vec!["editorial_derivation_from_official_record".to_string()],
            registry_metadata_records: 0,
            registry_verifies: Vec::new(),
            registry_does_not_verify: Vec::new(),
        };
        return SearchAnswer {
            status: "extractive".to_string(),
            intent: intent.to_string(),
            summary: claim.clone(),
            claims: vec![AnswerClaim {
                text: claim,
                citations,
                evidence_lineage: Some(lineage),
            }],
            uncertainty_mode,
            average_ignorance,
            warning,
        };
    }

    SearchAnswer {
        status: "insufficient_evidence".to_string(),
        intent: intent.to_string(),
        summary: "Foram encontrados resultados sobre o ocupante atual do cargo, mas o pipeline ainda não corroborou a mesma identidade em um registro oficial vigente e uma família editorial independente. Nenhum nome foi sintetizado."
            .to_string(),
        claims: Vec::new(),
        uncertainty_mode,
        average_ignorance,
        warning,
    }
}

fn office_editorial_surface_names_holder(result: &SearchResultItem, name: &str) -> bool {
    let surface_terms = relevance_terms(&format!("{} {}", result.title, result.url));
    let names_holder = office_name_keys(name)
        .into_iter()
        .any(|key| key.len() >= 4 && surface_terms.iter().any(|surface| surface == &key));
    let names_office_and_jurisdiction = surface_terms
        .iter()
        .any(|term| matches!(term.as_str(), "presidente" | "presidencia"))
        && surface_terms
            .iter()
            .any(|term| matches!(term.as_str(), "brasil" | "brazil"));
    names_holder || names_office_and_jurisdiction
}

pub(crate) fn current_office_holder_corroboration_queries(
    query: &str,
    results: &[SearchResultItem],
) -> Vec<String> {
    if !is_current_office_holder_query(query) {
        return Vec::new();
    }
    let candidates = office_holder_candidates(query, results);
    let Some(official) = candidates
        .iter()
        .filter(|candidate| candidate.official)
        .max_by_key(|candidate| office_name_keys(&candidate.name).len())
    else {
        return Vec::new();
    };
    vec![
        format!("\"{}\" \"atual presidente\" Brasil", official.name),
        format!("{} presidente do Brasil atualmente", official.name),
    ]
}

pub(crate) fn current_office_holder_discovery_surface_matches(
    query: &str,
    retained_results: &[SearchResultItem],
    candidate: &SearchResultItem,
) -> bool {
    if !is_current_office_holder_query(query)
        || is_hub_url(&candidate.url)
        || !is_independent_office_editorial(&candidate.url)
    {
        return false;
    }
    let Some(official_name) = office_holder_candidates(query, retained_results)
        .into_iter()
        .filter(|candidate| candidate.official)
        .max_by_key(|candidate| office_name_keys(&candidate.name).len())
        .map(|candidate| candidate.name)
    else {
        return false;
    };
    let surface_terms = relevance_terms(&format!(
        "{} {} {}",
        candidate.title, candidate.snippet, candidate.url
    ));
    if surface_terms
        .iter()
        .any(|term| matches!(term.as_str(), "former" | "ex-presidente"))
    {
        return false;
    }
    office_name_keys(&official_name)
        .into_iter()
        .any(|key| key.len() >= 4 && surface_terms.iter().any(|surface| surface == &key))
}

fn is_independent_office_editorial(url: &str) -> bool {
    let family = source_family(url);
    !matches!(
        family.as_str(),
        "facebook.com"
            | "instagram.com"
            | "linkedin.com"
            | "lula.com.br"
            | "tiktok.com"
            | "wikipedia.org"
            | "x.com"
            | "youtube.com"
    )
}

fn is_recent_office_editorial(result: &SearchResultItem) -> bool {
    const MAX_EDITORIAL_AGE_SECS: i64 = 180 * 86_400;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    result.published_at.is_some_and(|published_at| {
        published_at <= now.saturating_add(3_600)
            && now.saturating_sub(published_at) <= MAX_EDITORIAL_AGE_SECS
    })
}

fn office_holder_candidates<'a>(
    query: &str,
    results: &'a [SearchResultItem],
) -> Vec<OfficeHolderCandidate<'a>> {
    let mut candidates = Vec::new();
    let mut seen = HashSet::new();
    for result in results.iter().take(40) {
        if result.content_md.trim().is_empty() {
            continue;
        }
        let official = authoritative_source_for_query(query, &result.url);
        for excerpt in office_holder_passages(result) {
            for name in extract_office_holder_names(&excerpt) {
                let key = format!("{}:{}", result.url, office_name_keys(&name).join("-"));
                if seen.insert(key) {
                    candidates.push(OfficeHolderCandidate {
                        name,
                        excerpt: excerpt.clone(),
                        result,
                        official,
                    });
                }
            }
        }
    }
    candidates
}

pub(crate) fn result_has_current_office_holder_candidate(
    query: &str,
    result: &SearchResultItem,
) -> bool {
    !office_holder_candidates(query, std::slice::from_ref(result)).is_empty()
}

fn office_holder_passages(result: &SearchResultItem) -> Vec<String> {
    let mut passages = Vec::new();
    let mut push = |text: &str| {
        let cleaned = text.trim().replace(char::is_control, " ");
        if (12..=1_200).contains(&cleaned.chars().count()) && !passages.contains(&cleaned) {
            passages.push(cleaned);
        }
    };
    push(&result.title);
    if let Some(relevant) = result
        .content_relevant
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        for (sentence, _, _) in oi_index::relevance::split_into_sentences(relevant) {
            push(&sentence);
        }
    }
    for (sentence, _, _) in oi_index::relevance::split_into_sentences(&result.content_md) {
        push(&sentence);
    }
    passages
}

fn office_claim_excerpt(excerpt: &str, name: &str) -> String {
    let words = excerpt.split_whitespace().collect::<Vec<_>>();
    let name_keys = office_name_keys(name);
    let folded_words = words
        .iter()
        .map(|word| office_token_key(word))
        .collect::<Vec<_>>();
    let start = folded_words
        .iter()
        .position(|word| name_keys.first().is_some_and(|key| key == word))
        .unwrap_or(0);
    let from = start.saturating_sub(12);
    let to = (start + name_keys.len() + 22).min(words.len());
    words[from..to].join(" ")
}

fn extract_office_holder_names(text: &str) -> Vec<String> {
    const MARKERS: &[&[&str]] = &[
        &["presidente", "da", "republica"],
        &["presidente", "do", "brasil"],
        &["brazil", "president"],
        &["brazilian", "president"],
        &["atual", "presidente"],
        &["atualmente", "o", "presidente"],
        &["current", "president"],
        &["incumbent", "president"],
        &["presidente"],
        &["president"],
    ];
    const CONNECTORS: &[&str] = &["da", "das", "de", "do", "dos", "e"];
    const NON_NAMES: &[&str] = &[
        "agenda",
        "brasil",
        "brasileiro",
        "cargo",
        "enciclopedia",
        "encyclopedia",
        "funcoes",
        "estado",
        "governo",
        "lista",
        "planalto",
        "presidencia",
        "republica",
        "wikipedia",
    ];
    let raw_tokens = text
        .split_whitespace()
        .map(|token| {
            token
                .trim_matches(|character: char| !character.is_alphanumeric() && character != '-')
                .to_string()
        })
        .collect::<Vec<_>>();
    let folded_tokens = raw_tokens
        .iter()
        .map(|token| office_token_key(token))
        .collect::<Vec<_>>();
    let mut names = Vec::new();
    for marker in MARKERS {
        if marker.len() > folded_tokens.len() {
            continue;
        }
        for start in 0..=folded_tokens.len() - marker.len() {
            if folded_tokens[start..start + marker.len()]
                .iter()
                .map(String::as_str)
                .ne(marker.iter().copied())
            {
                continue;
            }
            if start > 0 && matches!(folded_tokens[start - 1].as_str(), "ex" | "former") {
                continue;
            }
            let mut name_tokens = Vec::new();
            for raw in raw_tokens.iter().skip(start + marker.len()) {
                if raw.is_empty() {
                    continue;
                }
                let folded = office_token_key(raw);
                if CONNECTORS.contains(&folded.as_str()) && !name_tokens.is_empty() {
                    name_tokens.push(raw.clone());
                    continue;
                }
                let proper = raw.chars().next().is_some_and(char::is_uppercase)
                    && !NON_NAMES.contains(&folded.as_str());
                if !proper {
                    break;
                }
                name_tokens.push(raw.clone());
                if name_tokens.len() == 7 {
                    break;
                }
            }
            while name_tokens
                .last()
                .is_some_and(|token| CONNECTORS.contains(&office_token_key(token).as_str()))
            {
                name_tokens.pop();
            }
            while name_tokens.len() > 1
                && name_tokens.last().is_some_and(|token| {
                    let letters = token.chars().filter(|character| character.is_alphabetic());
                    let count = letters.clone().count();
                    (2..=4).contains(&count) && letters.into_iter().all(char::is_uppercase)
                })
            {
                name_tokens.pop();
            }
            if !name_tokens.is_empty() {
                names.push(name_tokens.join(" "));
            }
        }
    }
    names.sort_by_key(|name| std::cmp::Reverse(office_name_keys(name).len()));
    names.dedup_by(|left, right| office_name_keys(left) == office_name_keys(right));
    names
}

fn office_token_key(token: &str) -> String {
    let relevant = relevance_terms(token).join("");
    if relevant.is_empty() {
        token.to_lowercase()
    } else {
        relevant
    }
}

fn office_name_keys(name: &str) -> Vec<String> {
    relevance_terms(name)
        .into_iter()
        .filter(|term| !matches!(term.as_str(), "da" | "das" | "de" | "do" | "dos" | "e"))
        .collect()
}

fn office_names_compatible(left: &str, right: &str) -> bool {
    let left = office_name_keys(left).into_iter().collect::<HashSet<_>>();
    let right = office_name_keys(right).into_iter().collect::<HashSet<_>>();
    if left.is_empty() || right.is_empty() {
        return false;
    }
    left == right
        || (left.len() == 1 && left.is_subset(&right))
        || (right.len() == 1 && right.is_subset(&left))
        || (left.len() >= 2 && right.len() >= 2 && left.is_subset(&right))
        || (left.len() >= 2 && right.len() >= 2 && right.is_subset(&left))
}

fn has_historical_poll_context(text: &str) -> bool {
    let folded = relevance_terms(text).join(" ");
    [
        "como estava",
        "levantamento anterior",
        "na mesma rodada",
        "pesquisa anterior",
        "rodada anterior",
    ]
    .iter()
    .any(|cue| folded.contains(cue))
}

fn has_conflicting_poll_round_context(text: &str) -> bool {
    let folded = relevance_terms(text).join(" ");
    let first = contains_first_round(&folded);
    let second = contains_second_round(&folded);
    // A retained fragment that labels the same percentage window as both
    // rounds is usually a glued related-link/header boundary. Do not guess
    // which clause owns the numbers; other independent pages must provide a
    // clean extract.
    first && second
}

fn poll_breakdown_outside_query_scope(query: &str, result: &SearchResultItem) -> bool {
    const DIMENSIONS: &[&str] = &[
        "cor", "genero", "idade", "raca", "regiao", "religiao", "renda",
    ];
    let query_terms = relevance_terms(query);
    if DIMENSIONS
        .iter()
        .any(|term| query_terms.iter().any(|query| query == term))
    {
        return false;
    }
    let surface_terms = relevance_terms(&format!("{} {}", result.title, result.url));
    DIMENSIONS
        .iter()
        .any(|term| surface_terms.iter().any(|surface| surface == term))
}

fn contains_first_round(folded: &str) -> bool {
    ["primeiro turno", "1 turno", "1º turno", "1o turno"]
        .iter()
        .any(|cue| folded.contains(cue))
}

fn contains_second_round(folded: &str) -> bool {
    ["segundo turno", "2 turno", "2º turno", "2o turno"]
        .iter()
        .any(|cue| folded.contains(cue))
}

fn clean_poll_candidate(text: &str) -> String {
    // Distilled pages sometimes glue an advertisement, a section heading and
    // the result sentence together. Start at the most specific poll-context
    // cue when present; the excerpt remains verbatim from that point onward.
    // When multiple named slates are glued before the first percentage, the
    // nearest `com/sem <candidate>` cue owns the pair. Keeping an earlier cue
    // can produce a self-contradictory label such as "cenário com X: ...
    // cenário sem X — 39% / 33%".
    let lower = text.to_lowercase();
    let first_percentage = lower.find('%').unwrap_or(lower.len());
    if let Some(index) = ["cenário com", "cenario com", "cenário sem", "cenario sem"]
        .iter()
        .filter_map(|cue| lower[..first_percentage].rfind(cue))
        .max()
    {
        return text[index..].trim().to_string();
    }
    for cue in [
        "No cenário estimulado",
        "Em um cenário alternativo",
        "Na simulação de segundo turno",
        "Na hipótese de",
        "Cenário de primeiro turno",
    ] {
        if let Some(index) = text.find(cue) {
            return text[index..].trim().to_string();
        }
    }
    text.trim().to_string()
}

/// Reattach the closest scenario heading when percentage-window extraction
/// starts immediately after a Markdown heading or paragraph boundary. Two
/// pairs from the same poll can both be correct while describing different
/// candidate slates; returning bare percentages would make them look like a
/// contradiction. The prefix is retained source text, not an inferred label.
fn poll_claim_with_group_context(group: &[ExtractiveCandidate<'_>], claim: &str) -> String {
    let claim_terms = relevance_terms(claim);
    let institute = poll_institute_context(group).filter(|label| {
        relevance_terms(label)
            .first()
            .is_none_or(|term| !claim_terms.contains(term))
    });
    // Select a label supported across the corroborating source group. An
    // explicit label inside the final claim wins over equally generic nearby
    // context; only a more specific named slate corroborated by two sources
    // may replace it. This prevents glued related-link text from turning a
    // first-round pair into a second-round claim.
    let inline_scenario = poll_scenario_label(claim);
    let mut scenario_support: HashMap<String, (usize, usize, String)> = HashMap::new();
    let labels = group.iter().filter_map(|candidate| {
        poll_scenario_label(&candidate.text)
            .into_iter()
            .chain(poll_scenario_context(candidate.result, &candidate.text))
            .max_by_key(|value| poll_scenario_specificity(value))
    });
    for label in labels {
        let key = relevance_terms(&label).join(" ");
        let specificity = poll_scenario_specificity(&label);
        let entry = scenario_support
            .entry(key)
            .or_insert_with(|| (0, specificity, label));
        entry.0 += 1;
        entry.1 = entry.1.max(specificity);
    }
    let contextual_scenario = scenario_support.into_values().max_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| left.1.cmp(&right.1))
            .then_with(|| right.2.chars().count().cmp(&left.2.chars().count()))
    });
    let scenario = match (inline_scenario, contextual_scenario) {
        (Some(inline), Some((support, specificity, contextual)))
            if support >= 2 && specificity > poll_scenario_specificity(&inline) =>
        {
            Some(contextual)
        }
        (Some(inline), _) => Some(inline),
        (None, Some((_, _, contextual))) => Some(contextual),
        (None, None) => None,
    };
    match (institute, scenario) {
        (Some(institute), Some(scenario)) => format!(
            "{institute} — {}: {claim}",
            scenario.trim_end_matches(['.', ':', ';'])
        ),
        (None, Some(scenario)) => {
            format!("{}: {claim}", scenario.trim_end_matches(['.', ':', ';']))
        }
        (Some(institute), None) => format!("{institute} — {claim}"),
        (None, None) => claim.to_string(),
    }
}

fn poll_emission_scenario_key(claim: &str) -> String {
    // Institute prefixes are presentation context, not different scenarios.
    // Prefer the parsed scenario so `primeiro turno: ...` and
    // `Datafolha — primeiro turno: ...` occupy one slot, while explicit
    // `cenário com/sem <candidate>` labels remain distinct.
    poll_scenario_label(claim)
        .map(|label| relevance_terms(&label).join(" "))
        .filter(|key| !key.is_empty())
        .unwrap_or_else(|| {
            claim
                .split(':')
                .next()
                .map(relevance_terms)
                .map(|terms| terms.join(" "))
                .unwrap_or_default()
        })
}

fn poll_scenario_label(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    let dynamic = ["cenário com", "cenario com", "cenário sem", "cenario sem"]
        .iter()
        .filter_map(|cue| lower.rfind(cue).map(|index| (index, cue)))
        .max_by_key(|(index, _)| *index);
    if let Some((dynamic_index, _)) = dynamic {
        let first_percentage = lower.find('%').unwrap_or(lower.len());
        let explicit_runoff_after_slate = ["segundo turno", "2º turno", "2o turno", "2 turno"]
            .iter()
            .filter_map(|cue| lower.rfind(cue))
            .any(|round_index| round_index > dynamic_index && round_index < first_percentage);
        if explicit_runoff_after_slate {
            return Some("segundo turno".to_string());
        }
        if let Some(label) = poll_dynamic_scenario_label(&text[dynamic_index..]) {
            return Some(label);
        }
    }
    let folded = relevance_terms(text).join(" ");
    if folded.contains("cenario alternativo") {
        return Some("cenário alternativo".to_string());
    }
    if folded.contains("cenario espontaneo")
        || folded.contains("pesquisa espontanea")
        || folded.contains("voto espontaneo")
    {
        return Some("cenário espontâneo".to_string());
    }
    if folded.contains("pesquisa estimulada") || folded.contains("voto estimulado") {
        return Some("cenário estimulado".to_string());
    }
    let first = ["primeiro turno", "1 turno", "1º turno", "1o turno"]
        .iter()
        .filter_map(|cue| folded.rfind(cue).map(|index| (index, "primeiro turno")));
    let second = ["segundo turno", "2 turno", "2º turno", "2o turno"]
        .iter()
        .filter_map(|cue| folded.rfind(cue).map(|index| (index, "segundo turno")));
    if let Some((_, label)) = first.chain(second).max_by_key(|(index, _)| *index) {
        Some(label.to_string())
    } else if folded.contains("cenario estimulado") {
        Some("cenário estimulado".to_string())
    } else {
        None
    }
}

fn poll_scenario_context(result: &SearchResultItem, candidate: &str) -> Option<String> {
    let source = &result.content_md;
    let signature = percent_signature(candidate);
    if signature.len() < 2 {
        return None;
    }
    let first = format!("{}%", signature[0]);
    let second = format!("{}%", signature[1]);
    source
        .match_indices(&first)
        .filter_map(|(start, _)| {
            source[start..]
                .chars()
                .take(500)
                .collect::<String>()
                .contains(&second)
                .then(|| poll_scenario_before(source, start))
                .flatten()
        })
        .max_by(|left, right| {
            poll_scenario_specificity(left)
                .cmp(&poll_scenario_specificity(right))
                .then_with(|| right.chars().count().cmp(&left.chars().count()))
        })
}

fn poll_scenario_before(source: &str, candidate_start: usize) -> Option<String> {
    let context_start = source[..candidate_start]
        .char_indices()
        .rev()
        .nth(1_200)
        .map(|(index, _)| index)
        .unwrap_or(0);
    let context = &source[context_start..candidate_start];
    let folded = context.to_lowercase();
    let fixed_cues = [
        "no cenário estimulado",
        "cenário estimulado",
        "no cenario estimulado",
        "cenario estimulado",
        "no cenário espontâneo",
        "cenário espontâneo",
        "no cenario espontaneo",
        "cenario espontaneo",
        "pesquisa espontânea",
        "pesquisa espontanea",
        "voto espontâneo",
        "voto espontaneo",
        "cenário alternativo",
        "cenario alternativo",
        "cenário de primeiro turno",
        "cenario de primeiro turno",
        "simulação de primeiro turno",
        "simulacao de primeiro turno",
        "simulação de segundo turno",
        "simulacao de segundo turno",
        "primeiro turno",
        "segundo turno",
    ];
    let fixed = fixed_cues.iter().filter_map(|cue| {
        folded.rfind(cue).and_then(|index| {
            context
                .get(index..index + cue.len())
                .map(|label| (poll_scenario_specificity(label), index, label.to_string()))
        })
    });
    let dynamic = ["cenário com", "cenario com", "cenário sem", "cenario sem"]
        .iter()
        .filter_map(|cue| {
            folded.rfind(cue).and_then(|index| {
                poll_dynamic_scenario_label(&context[index..])
                    .map(|label| (poll_scenario_specificity(&label), index, label))
            })
        });
    fixed
        .chain(dynamic)
        // Specific named/mode context may intentionally outrank a generic
        // round cue. Among equally specific cues, however, the closest cue to
        // the percentage window owns the numbers. The previous tie-breaker
        // depended on label length and could select a distant "segundo turno"
        // over the immediately preceding "primeiro turno".
        .max_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)))
        .map(|(_, _, label)| label)
}

fn poll_round_values_plausible(query: &str, claim: &str, round: Option<&'static str>) -> bool {
    if round != Some("second") || !brazilian_presidential_round_model(query) {
        return true;
    }
    let values = percent_signature(claim)
        .into_iter()
        .take(2)
        .filter_map(|value| value.replace(',', ".").parse::<f64>().ok())
        .collect::<Vec<_>>();
    values.len() == 2 && values.iter().sum::<f64>() >= 60.0
}

fn poll_dynamic_scenario_label(text: &str) -> Option<String> {
    const STOP: &[&str] = &["a", "as", "o", "os", "no", "na", "um", "uma"];
    let mut words = text.split_whitespace();
    let first = words.next()?;
    let second = words.next()?;
    let mut label = vec![first, second];
    for word in words.take(4) {
        let clean = word.trim_matches(|character: char| !character.is_alphanumeric());
        if clean.is_empty()
            || STOP.contains(&clean.to_lowercase().as_str())
            || !clean.chars().next().is_some_and(char::is_uppercase)
        {
            break;
        }
        label.push(word);
    }
    Some(label.join(" "))
}

fn poll_institute_context(group: &[ExtractiveCandidate<'_>]) -> Option<String> {
    const GENERIC: &[&str] = &[
        "cenario",
        "divulga",
        "eleicao",
        "eleitoral",
        "estimulado",
        "intencao",
        "numero",
        "numeros",
        "pesquisa",
        "presidencia",
        "presidente",
        "resultado",
        "resultados",
        "turno",
        "veja",
        "voto",
    ];
    let candidate_keys: HashSet<_> = group
        .iter()
        .flat_map(|candidate| candidate.answer_keys.iter().cloned())
        .collect();
    let mut families_by_term: HashMap<String, HashSet<String>> = HashMap::new();
    for candidate in group {
        let family = source_family(&candidate.result.url);
        for term in relevance_terms(&candidate.result.title) {
            if term.len() >= 4
                && !GENERIC.contains(&term.as_str())
                && !candidate_keys.contains(&term)
            {
                families_by_term
                    .entry(term)
                    .or_default()
                    .insert(family.clone());
            }
        }
    }
    let term = families_by_term
        .into_iter()
        .filter(|(_, families)| families.len() >= 2)
        .max_by_key(|(_, families)| families.len())?
        .0;
    group.iter().find_map(|candidate| {
        candidate.result.title.split_whitespace().find_map(|token| {
            (relevance_terms(token).first() == Some(&term))
                .then(|| token.trim_matches(|character: char| !character.is_alphanumeric()))
                .filter(|label| !label.is_empty())
                .map(str::to_string)
        })
    })
}

fn poll_scenario_specificity(text: &str) -> usize {
    let folded = relevance_terms(text).join(" ");
    if folded.contains("cenario com") || folded.contains("cenario sem") {
        // An explicitly named candidate slate is more specific than the
        // generic "stimulated" mode that may appear earlier on the page.
        6
    } else if folded.contains("cenario alternativo") || folded.contains("cenario espontaneo") {
        5
    } else if contains_first_round(&folded) || contains_second_round(&folded) {
        // The election round materially changes the meaning of an otherwise
        // identical candidate pair and must outrank a generic mode label.
        4
    } else if folded.contains("simulacao segundo turno")
        || folded.contains("simulacao primeiro turno")
        || folded.contains("cenario estimulado")
    {
        2
    } else {
        0
    }
}

/// The corroboration key currently proves one candidate/value pair against a
/// second candidate/value pair. If the source sentence continues with a third
/// percentage, do not silently attach that uncorroborated value to the claim.
fn poll_claim_with_supported_pair(text: &str) -> String {
    let mut bounded = text.trim();
    let original_percent_positions: Vec<_> =
        bounded.match_indices('%').map(|(index, _)| index).collect();
    if let Some(after_second) = original_percent_positions.get(1).map(|index| index + 1)
        && let Some(pipe_offset) = bounded[after_second..].find(" | ")
    {
        // Distillers can glue a related headline after a complete poll pair
        // using the publisher's visual `|` separator. It is not part of the
        // supported claim and must not be emitted as prose.
        bounded = bounded[..after_second + pipe_offset].trim_end();
    }
    let percent_positions: Vec<_> = bounded.match_indices('%').map(|(index, _)| index).collect();
    if percent_positions.len() <= 2 {
        let mut claim = bounded
            .trim_end_matches(|character: char| {
                character.is_whitespace() || matches!(character, ',' | ';' | '–' | '—')
            })
            .to_string();
        if !claim.ends_with(['.', '!', '?']) {
            claim.push('.');
        }
        return claim;
    }
    let after_second = percent_positions[1] + 1;
    let before_third = percent_positions[2];
    let separator = bounded[after_second..before_third]
        .find([',', ';', '–', '—'])
        .map(|offset| after_second + offset)
        .unwrap_or(after_second);
    let mut claim = bounded[..separator]
        .trim_end_matches(|character: char| {
            character.is_whitespace() || matches!(character, ',' | ';' | '–' | '—')
        })
        .to_string();
    if !claim.ends_with(['.', '!', '?']) {
        claim.push('.');
    }
    claim
}

pub(crate) fn best_factual_sentence(query: &str, result: &SearchResultItem) -> Option<String> {
    factual_candidate_sentences(result)
        .into_iter()
        .filter(|sentence| !sentence.contains('?'))
        .filter(|sentence| has_required_relation_cue(query, sentence))
        .filter_map(|sentence| {
            // The page title may supply entity context only when the body uses
            // a bounded anaphor such as "the company was founded" or "the
            // system was created". Merely concatenating any entity title with
            // any relation sentence inverted the predicate for pages such as
            // "Anthropic identifies content created by Claude".
            let entity_in_sentence = factual_entity_terms_covered(query, &sentence);
            let entity_context_supported = (entity_in_sentence
                && factual_relation_direction_matches_entity(query, &sentence))
                || (factual_entity_terms_covered(query, &result.title)
                    && sentence_can_inherit_entity_context(query, &sentence));
            let keys = answer_keys(query, &sentence);
            (entity_context_supported && !keys.is_empty()).then_some(sentence)
        })
        .min_by_key(|sentence| sentence.chars().count())
}

/// Explicit diagnostic of caller-supplied retained text, not a truth label.
/// Not automatically attached to public answers: source access policies must
/// be applied by the caller before exposing protected excerpts.
#[derive(Debug, Serialize)]
pub struct FactualCandidateAudit {
    pub excerpt: String,
    pub required_relation: bool,
    pub question_fragment: bool,
    pub entity_in_sentence: bool,
    pub direction_matches: bool,
    pub matching_title_context: bool,
    pub inherits_context: bool,
    pub answer_keys: Vec<String>,
}

pub fn audit_factual_candidates(
    query: &str,
    result: &SearchResultItem,
) -> Vec<FactualCandidateAudit> {
    factual_candidate_sentences(result)
        .into_iter()
        .map(|sentence| FactualCandidateAudit {
            required_relation: has_required_relation_cue(query, &sentence),
            question_fragment: sentence.contains('?'),
            entity_in_sentence: factual_entity_terms_covered(query, &sentence),
            direction_matches: factual_relation_direction_matches_entity(query, &sentence),
            matching_title_context: factual_entity_terms_covered(query, &result.title),
            inherits_context: sentence_can_inherit_entity_context(query, &sentence),
            answer_keys: ordered_answer_keys(query, &sentence),
            excerpt: sentence,
        })
        .collect()
}

/// Require the requested entity to participate in the creation/founding
/// predicate, rather than merely occurring somewhere earlier in the sentence.
/// This rejects "Anthropic will identify content created by AI" while
/// retaining direct passive ("Anthropic was founded by ...") and
/// creator-first ("X founded Anthropic") constructions.
fn factual_relation_direction_matches_entity(query: &str, sentence: &str) -> bool {
    let query_terms = crate::query_intent::factual_query_terms(query);
    let entity_terms = query_terms
        .iter()
        .filter(|term| !matches!(term.as_str(), "created" | "founded" | "invented"))
        .cloned()
        .collect::<Vec<_>>();
    if entity_terms.is_empty() {
        return false;
    }
    let Some(relation) = query_terms
        .iter()
        .find(|term| matches!(term.as_str(), "created" | "founded" | "invented"))
    else {
        return true;
    };
    let accepted_relations: &[&str] = match relation.as_str() {
        "created" => &["created", "founded"],
        "founded" => &["founded"],
        "invented" => &["invented"],
        _ => return true,
    };
    // Pronunciation/alias asides can separate an explicit entity from its
    // verb. Ignore balanced parenthetical text only for the bounded syntax
    // check; the emitted sentence, denial guard and citation retain it.
    let mut syntax = String::new();
    let mut depth = 0usize;
    for ch in sentence.chars() {
        match ch {
            '(' | '[' => depth += 1,
            ')' | ']' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    syntax.push(' ');
                }
            }
            _ if depth == 0 => syntax.push(ch),
            _ => {}
        }
    }
    if depth != 0 {
        return false;
    }
    let sentence_terms = relevance_terms(&syntax);
    let sentence_lower = sentence.to_lowercase();
    let has_passive_copula = [" é ", " era ", " foi ", " is ", " was "]
        .iter()
        .any(|cue| sentence_lower.contains(cue));
    let entity_width = entity_terms.len();
    if sentence_terms.len() < entity_width {
        return false;
    }
    for entity_start in 0..=sentence_terms.len().saturating_sub(entity_width) {
        if &sentence_terms[entity_start..entity_start + entity_width] != entity_terms.as_slice() {
            continue;
        }
        let entity_end = entity_start + entity_width;
        for (relation_index, term) in sentence_terms.iter().enumerate() {
            if !accepted_relations.contains(&term.as_str()) {
                continue;
            }
            if relation_index < entity_start {
                // Creator-first: "Dario founded Anthropic" or "Banco Central
                // created Pix". The requested entity must be the immediate
                // object, not merely a later noun
                // in "OpenAI co-founder X joins Anthropic".
                let distance = entity_start.saturating_sub(relation_index);
                let direct_relation = distance <= 1
                    || (distance == 2
                        && sentence_terms
                            .get(relation_index + 1)
                            .is_some_and(|term| term == "of"));
                if direct_relation {
                    return true;
                }
                continue;
            }
            if relation_index >= entity_end {
                let distance = relation_index - entity_end;
                let passive = sentence_terms
                    .iter()
                    .skip(relation_index + 1)
                    .take(4)
                    .any(|marker| matches!(marker.as_str(), "by" | "pela" | "pelo" | "por"));
                // A normalized direct passive relation follows the entity
                // immediately or after one appositive token. More distant
                // wording usually belongs to another noun phrase.
                if distance <= 6 && passive && (distance <= 1 || has_passive_copula) {
                    return true;
                }
            }
        }
    }
    false
}

/// Whether an unfetched discovery surface is specifically about the factual
/// entity-relation requested by the user. This is a display-only gate: titles
/// and provider snippets that pass it may remain useful search leads, but can
/// never become answer evidence until their page is crawled and retained.
pub fn factual_discovery_surface_matches_relation(query: &str, result: &SearchResultItem) -> bool {
    best_factual_sentence(query, result).is_some()
        || factual_public_surface_matches_relation(query, result)
}

/// Whether the title or provider snippet itself makes the requested factual
/// relation legible. A retained page body can make a result auditable without
/// making a vague or misleading public title a good top result, so display
/// ordering uses this stricter surface-only predicate.
pub fn factual_public_surface_matches_relation(query: &str, result: &SearchResultItem) -> bool {
    [&result.title, &result.snippet].into_iter().any(|surface| {
        factual_entity_terms_covered(query, surface)
            && (factual_relation_direction_matches_entity(query, surface)
                || factual_relation_header_matches_entity(query, surface))
    })
}

/// A topical heading may put the entity before a compact descriptor such as
/// "Anthropic company history: founders" without asserting a passive clause.
/// Keep that page as a search lead, while rejecting arbitrary intervening
/// predicates such as "Anthropic identifies content created by AI".
fn factual_relation_header_matches_entity(query: &str, surface: &str) -> bool {
    const TOPICAL_BRIDGE_TERMS: &[&str] = &[
        "company",
        "empresa",
        "equipe",
        "history",
        "historia",
        "origem",
        "ownership",
        "team",
    ];
    let query_terms = crate::query_intent::factual_query_terms(query);
    let entity_terms = query_terms
        .iter()
        .filter(|term| !matches!(term.as_str(), "created" | "founded" | "invented"))
        .cloned()
        .collect::<Vec<_>>();
    let Some(relation) = query_terms
        .iter()
        .find(|term| matches!(term.as_str(), "created" | "founded" | "invented"))
    else {
        return false;
    };
    let accepted_relations: &[&str] = match relation.as_str() {
        "created" => &["created", "founded"],
        "founded" => &["founded"],
        "invented" => &["invented"],
        _ => return false,
    };
    if entity_terms.is_empty() {
        return false;
    }
    let terms = relevance_terms(surface);
    if terms.len() < entity_terms.len() {
        return false;
    }
    for entity_start in 0..=terms.len().saturating_sub(entity_terms.len()) {
        if &terms[entity_start..entity_start + entity_terms.len()] != entity_terms.as_slice() {
            continue;
        }
        let entity_end = entity_start + entity_terms.len();
        for relation_index in entity_end..terms.len() {
            if !accepted_relations.contains(&terms[relation_index].as_str()) {
                continue;
            }
            // `Founders Fund` is a proper organization name, not a heading
            // about the founders of the entity before it. Relation
            // normalization intentionally maps "founders" to `founded`, so
            // reject this bounded lexical collision before accepting a
            // compact topical header.
            if terms[relation_index] == "founded"
                && terms
                    .get(relation_index + 1)
                    .is_some_and(|term| term == "fund")
            {
                continue;
            }
            let bridge = &terms[entity_end..relation_index];
            if bridge.len() <= 4
                && bridge.iter().all(|term| {
                    TOPICAL_BRIDGE_TERMS.contains(&term.as_str())
                        || term.bytes().all(|byte| byte.is_ascii_digit())
                })
            {
                return true;
            }
        }
    }
    false
}

fn sentence_can_inherit_entity_context(query: &str, sentence: &str) -> bool {
    let relation = crate::query_intent::factual_query_terms(query)
        .into_iter()
        .find(|term| matches!(term.as_str(), "created" | "founded" | "invented"));
    let folded = relevance_terms(sentence).join(" ");
    let starts_with_any =
        |prefixes: &[&str]| prefixes.iter().any(|prefix| folded.starts_with(prefix));
    let contains_any = |phrases: &[&str]| phrases.iter().any(|phrase| folded.contains(phrase));

    match relation.as_deref() {
        Some("founded") => {
            (starts_with_any(&[
                "companhia",
                "empresa",
                "organizacao",
                "startup",
                "ela foi fundada",
                "foi fundada",
                "foi fundado",
                "founded por",
                "company",
                "organization",
                "startup",
                "it was founded",
            ]) || contains_any(&[
                "founded companhia",
                "founded company",
                "founded empresa",
                "founded organizacao",
                "founded organization",
                "founded startup",
            ])) && contains_any(&["founded"])
        }
        Some("created") | Some("invented") => {
            let generic_subject = starts_with_any(&[
                "companhia",
                "empresa",
                "ideia",
                "organizacao",
                "plataforma",
                "startup",
                "tecnologia",
                "produto",
                "projeto",
                "servico",
                "sistema",
                "created por",
                "desenvolvida por",
                "desenvolvido por",
                "founded por",
                "company",
                "organization",
                "platform",
                "product",
                "project",
                "service",
                "system",
                "it was created",
                "it was founded",
            ]) || contains_any(&[
                "founded companhia",
                "founded company",
                "founded empresa",
                "founded organizacao",
                "founded organization",
                "founded startup",
                "processo created plataforma",
                "processo created produto",
                "processo created sistema",
            ]);
            let generic_creation = contains_any(&[
                "ideia created",
                "created por",
                "desenvolvida por",
                "desenvolvido por",
                "foi created",
                "foi founded",
                "founded por",
                "nasceu dentro",
                "processo created",
                "was created",
                "was founded",
            ]);
            generic_subject && generic_creation
        }
        _ => false,
    }
}

fn factual_entity_terms_covered(query: &str, evidence: &str) -> bool {
    let terms = crate::query_intent::factual_query_terms(query);
    let entity_terms: Vec<_> = terms
        .iter()
        .filter(|term| !matches!(term.as_str(), "created" | "founded" | "invented"))
        .collect();
    if entity_terms.len() == terms.len() || entity_terms.is_empty() {
        return false;
    }
    let evidence_terms = relevance_terms(evidence);
    entity_terms
        .iter()
        .all(|term| evidence_terms.contains(term))
}

fn has_required_relation_cue(query: &str, sentence: &str) -> bool {
    if has_negated_origin_relation(sentence) {
        return false;
    }
    let query_terms = crate::query_intent::factual_query_terms(query);
    let folded_sentence = sentence.to_lowercase();
    if query_terms.iter().any(|term| term == "founded") {
        if folded_sentence.contains("quem fundou") || folded_sentence.contains("who founded") {
            return false;
        }
        if query_entity_is_active_relation_subject(query, sentence, "founded") {
            return false;
        }
        return [
            "cofundador",
            "cofundadora",
            "cofundou",
            "foi fundada",
            "foi fundado",
            "fundada por",
            "fundado por",
            "fundaram",
            "fundar",
            "fundou",
            "founded",
        ]
        .iter()
        .any(|cue| folded_sentence.contains(cue));
    }
    if query_terms.iter().any(|term| term == "created") {
        if folded_sentence.contains("quem criou") || folded_sentence.contains("who created") {
            return false;
        }
        if query_entity_is_active_relation_subject(query, sentence, "created") {
            return false;
        }
        if query_entity_is_active_relation_subject(query, sentence, "founded") {
            return false;
        }
        return [
            "a ideia de criar",
            "criador do",
            "criadora do",
            "criada por",
            "criada pela",
            "criado por",
            "criado pelo",
            "desenvolvida por",
            "desenvolvida pelo",
            "desenvolvido por",
            "desenvolvido pelo",
            "nasceu dentro da",
            "nasceu dentro do",
            "processo de criação",
            "cofundador",
            "cofundadora",
            "cofundou",
            "foi fundada",
            "foi fundado",
            "fundada por",
            "fundado por",
            "fundaram",
            "founded",
            "criaram",
            "criou",
            "created",
            "invented",
            "developed",
            "desenvolveu",
            "inventou",
            "inventado por",
            "inventada por",
        ]
        .iter()
        .any(|cue| folded_sentence.contains(cue));
    }
    if query_terms.iter().any(|term| term == "invented") {
        if folded_sentence.contains("quem inventou") || folded_sentence.contains("who invented") {
            return false;
        }
        return [
            "inventada por",
            "inventado por",
            "inventaram",
            "inventou",
            "invented",
        ]
        .iter()
        .any(|cue| folded_sentence.contains(cue));
    }
    true
}

/// A bounded denial guard, not a general entailment engine. Keep raw tokens
/// here: relevance normalization can discard the negation itself. Reject an
/// origin assertion whose local predicate is denied, including contractions.
fn has_negated_origin_relation(sentence: &str) -> bool {
    let folded = sentence.to_lowercase().replace('’', "'");
    let tokens = folded
        .split_whitespace()
        .map(|word| word.trim_matches(|c: char| !c.is_alphabetic() && c != '\''))
        .collect::<Vec<_>>();
    tokens.iter().enumerate().any(|(index, word)| {
        if !matches!(
            *word,
            "founded"
                | "created"
                | "invented"
                | "developed"
                | "fundada"
                | "fundado"
                | "fundou"
                | "fundaram"
                | "criada"
                | "criado"
                | "criou"
                | "inventada"
                | "inventado"
                | "inventou"
                | "desenvolvida"
                | "desenvolvido"
        ) {
            return false;
        }
        tokens[index.saturating_sub(6)..index]
            .iter()
            .enumerate()
            .any(|(offset, token)| {
                let next = tokens.get(index.saturating_sub(6) + offset + 1).copied();
                matches!(
                    *token,
                    "not"
                        | "never"
                        | "não"
                        | "nao"
                        | "nunca"
                        | "wasn't"
                        | "isn't"
                        | "didn't"
                        | "cannot"
                        | "can't"
                ) && !matches!(next, Some("only" | "just" | "apenas" | "só" | "so"))
            })
    })
}

fn query_entity_is_active_relation_subject(query: &str, sentence: &str, relation: &str) -> bool {
    let entity_terms: Vec<_> = crate::query_intent::factual_query_terms(query)
        .into_iter()
        .filter(|term| !matches!(term.as_str(), "created" | "founded" | "invented"))
        .collect();
    if entity_terms.is_empty() {
        return false;
    }
    let sentence_terms = relevance_terms(sentence);
    for (index, window) in sentence_terms.windows(entity_terms.len() + 1).enumerate() {
        if window[..entity_terms.len()] == entity_terms && window[entity_terms.len()] == relation {
            let passive_marker = sentence_terms.get(index + entity_terms.len() + 1);
            return passive_marker
                .is_none_or(|marker| !matches!(marker.as_str(), "by" | "pela" | "pelo" | "por"));
        }
    }
    false
}

fn factual_candidate_sentences(result: &SearchResultItem) -> Vec<String> {
    let mut candidates = Vec::new();
    // Titles identify/discover the entity; they are not standalone retained
    // claim evidence and must not override a denial in the fetched body.
    if let Some(relevant) = result
        .content_relevant
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        for (sentence, _, _) in oi_index::relevance::split_into_sentences(relevant) {
            push_candidate(&mut candidates, &sentence);
        }
    }
    for (sentence, _, _) in oi_index::relevance::split_into_sentences(&result.content_md) {
        push_candidate(&mut candidates, &sentence);
    }
    candidates
}

fn candidate_sentences(result: &SearchResultItem) -> Vec<String> {
    let mut candidates = Vec::new();
    push_candidate(&mut candidates, &result.title);
    let mut bodies = Vec::new();
    if let Some(relevant) = result
        .content_relevant
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        bodies.push(relevant);
    }
    if !result.content_md.trim().is_empty() && !bodies.iter().any(|body| *body == result.content_md)
    {
        bodies.push(&result.content_md);
    }
    for body in bodies {
        for (sentence, _, _) in oi_index::relevance::split_into_sentences(body) {
            push_candidate(&mut candidates, &sentence);
        }
        for window in percentage_windows(body) {
            push_candidate(&mut candidates, &window);
        }
    }
    candidates
}

/// Conservative measurement fingerprints used to attach an editorial report
/// that omits the registration ID to a *verified* registered poll. A match
/// requires the same two percentage/name assignments; callers may additionally
/// require an institute term in that exact candidate window. This is narrower
/// than general semantic similarity and is never used to create a registry
/// record or validate the reported percentages.
#[derive(Clone, Debug)]
pub(crate) struct PollMeasurementFingerprint {
    assignments: Vec<(String, HashSet<String>)>,
}

impl PollMeasurementFingerprint {
    pub(crate) fn compatible_with(&self, other: &Self) -> bool {
        if self.assignments.len() != 2 || other.assignments.len() != 2 {
            return false;
        }
        let assignment_matches =
            |left: &(String, HashSet<String>), right: &(String, HashSet<String>)| {
                left.0 == right.0 && !left.1.is_disjoint(&right.1)
            };
        (assignment_matches(&self.assignments[0], &other.assignments[0])
            && assignment_matches(&self.assignments[1], &other.assignments[1]))
            || (assignment_matches(&self.assignments[0], &other.assignments[1])
                && assignment_matches(&self.assignments[1], &other.assignments[0]))
    }
}

pub(crate) fn poll_measurement_fingerprints(
    result: &SearchResultItem,
    required_context_terms: &[String],
) -> Vec<PollMeasurementFingerprint> {
    if result.content_md.trim().is_empty() || is_hub_url(&result.url) || is_current_preview(result)
    {
        return Vec::new();
    }
    let mut fingerprints = Vec::new();
    for raw_text in candidate_sentences(result) {
        let text = clean_poll_candidate(&raw_text);
        if has_historical_poll_context(&text)
            || has_conflicting_poll_round_context(&text)
            || has_repeated_leading_percent_pair(&text)
            || poll_pair_crosses_section_boundary(&text)
            || has_non_candidate_leading_percentage(&text)
        {
            continue;
        }
        let context = relevance_terms(&text);
        if !required_context_terms.is_empty()
            && !required_context_terms
                .iter()
                .any(|required| context.contains(required))
        {
            continue;
        }
        let values = percent_signature(&text);
        let identities = poll_candidate_identity_sets(&text);
        if values.len() != 2 || identities.len() != 2 || identities.iter().any(HashSet::is_empty) {
            continue;
        }
        let assignments = values
            .into_iter()
            .zip(identities)
            .map(|(value, identity)| (normalize_percent_value(&value), identity))
            .collect::<Vec<_>>();
        let fingerprint = PollMeasurementFingerprint { assignments };
        if !fingerprints
            .iter()
            .any(|existing| fingerprint.compatible_with(existing))
        {
            fingerprints.push(fingerprint);
        }
    }
    fingerprints
}

fn normalize_percent_value(value: &str) -> String {
    value
        .replace(',', ".")
        .parse::<f64>()
        .map(|value| format!("{value:.3}"))
        .unwrap_or_else(|_| value.to_string())
}

fn percentage_windows(text: &str) -> Vec<String> {
    let characters: Vec<char> = text.chars().collect();
    let positions: Vec<usize> = characters
        .iter()
        .enumerate()
        .filter_map(|(index, character)| (*character == '%').then_some(index))
        .collect();
    let mut windows = Vec::new();
    for (pair_index, pair) in positions.windows(2).enumerate() {
        let first = pair[0];
        let second = pair[1];
        if second.saturating_sub(first) > 260 {
            continue;
        }
        // Do not let an earlier, unrelated percentage (for example an ad's
        // "71% OFF") become the first value when this window is later
        // normalized into a claim signature.
        let previous_percent_boundary = pair_index
            .checked_sub(1)
            .map(|index| positions[index] + 1)
            .unwrap_or(0);
        let mut lower = first.saturating_sub(300).max(previous_percent_boundary);
        if lower > 0
            && lower < first
            && !characters[lower - 1].is_whitespace()
            && !characters[lower].is_whitespace()
            && let Some(offset) = characters[lower..first]
                .iter()
                .position(|character| character.is_whitespace())
        {
            lower += offset + 1;
        }
        let upper = (second + 180).min(characters.len());
        let start = characters[lower..first]
            .iter()
            .rposition(|character| matches!(character, '.' | '?' | '!' | '#'))
            .map(|offset| lower + offset + 1)
            .unwrap_or(lower);
        let end = characters[second..upper]
            .iter()
            .position(|character| {
                matches!(character, '.' | '?' | '!' | '#' | '-' | ';' | '–' | '—')
            })
            .map(|offset| second + offset + 1)
            .unwrap_or(upper);
        let value: String = characters[start..end].iter().collect();
        windows.push(value);
    }
    windows
}

fn push_candidate(candidates: &mut Vec<String>, text: &str) {
    let cleaned = text
        .trim()
        .trim_start_matches('#')
        .trim_start_matches(['-', '*'])
        .trim()
        .replace(char::is_control, " ")
        .trim_end_matches(|character: char| {
            character.is_whitespace() || matches!(character, '#' | '-' | '*')
        })
        .trim()
        .to_string();
    if (25..=360).contains(&cleaned.chars().count())
        && !looks_truncated(&cleaned)
        && !candidates.contains(&cleaned)
    {
        candidates.push(cleaned);
    }
}

fn looks_truncated(text: &str) -> bool {
    let last = text
        .split_whitespace()
        .last()
        .unwrap_or("")
        .trim_matches(|character: char| !character.is_alphanumeric())
        .to_lowercase();
    matches!(
        last.as_str(),
        "a" | "ao"
            | "com"
            | "como"
            | "da"
            | "das"
            | "de"
            | "do"
            | "dos"
            | "e"
            | "em"
            | "entre"
            | "for"
            | "from"
            | "in"
            | "of"
            | "para"
            | "por"
            | "the"
            | "to"
    )
}

fn answer_keys(query: &str, text: &str) -> HashSet<String> {
    ordered_answer_keys(query, text).into_iter().collect()
}

fn ordered_answer_keys(query: &str, text: &str) -> Vec<String> {
    let query_terms: HashSet<_> = relevance_terms(query).into_iter().collect();
    let relation_adjacent = relation_adjacent_answer_keys(query, text, &query_terms);
    if !relation_adjacent.is_empty() {
        return relation_adjacent;
    }
    let mut keys = Vec::new();
    for term in text
        .split_whitespace()
        .enumerate()
        .filter(|(index, token)| {
            *index > 0
                && token
                    .trim_matches(|character: char| !character.is_alphanumeric())
                    .chars()
                    .next()
                    .is_some_and(char::is_uppercase)
        })
        .flat_map(|(_, token)| relevance_terms(token))
        .filter(|term| term.len() >= 4 && !query_terms.contains(term))
        .filter(|term| {
            ![
                "de acordo",
                "empresa",
                "fonte",
                "jornal",
                "pesquisa",
                "segundo",
            ]
            .contains(&term.as_str())
        })
    {
        if !keys.contains(&term) {
            keys.push(term);
        }
    }
    keys
}

fn relation_adjacent_answer_keys(
    query: &str,
    text: &str,
    query_terms: &HashSet<String>,
) -> Vec<String> {
    let factual_terms = crate::query_intent::factual_query_terms(query);
    let Some(relation) = factual_terms
        .iter()
        .find(|term| matches!(term.as_str(), "created" | "founded" | "invented"))
    else {
        return Vec::new();
    };
    let accepted_relations: &[&str] = match relation.as_str() {
        "created" => &["created", "founded"],
        "founded" => &["founded"],
        "invented" => &["invented", "created"],
        _ => return Vec::new(),
    };
    let entity_terms = factual_terms
        .iter()
        .filter(|term| !matches!(term.as_str(), "created" | "founded" | "invented"))
        .cloned()
        .collect::<Vec<_>>();
    let raw_tokens = text
        .split_whitespace()
        .map(|token| {
            token
                .trim_matches(|character: char| !character.is_alphanumeric() && character != '-')
                .to_string()
        })
        .collect::<Vec<_>>();
    let normalized = raw_tokens
        .iter()
        .map(|token| relevance_terms(token))
        .collect::<Vec<_>>();
    let relation_positions = normalized
        .iter()
        .enumerate()
        .filter(|(_, terms)| {
            terms
                .iter()
                .any(|term| accepted_relations.contains(&term.as_str()))
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();

    for relation_index in relation_positions {
        // Passive relation: "Pix foi desenvolvido pelo Banco Central".
        let passive_marker = normalized
            .iter()
            .enumerate()
            .skip(relation_index + 1)
            .take(4)
            .find(|(_, terms)| {
                terms
                    .iter()
                    .any(|term| matches!(term.as_str(), "by" | "pela" | "pelo" | "por"))
            })
            .map(|(index, _)| index);
        if let Some(marker_index) = passive_marker {
            let keys =
                collect_adjacent_name_keys(raw_tokens.iter().skip(marker_index + 1), query_terms);
            if !keys.is_empty() {
                return keys;
            }
        }

        // Active relation: "Banco Central criou o Pix". Only treat the
        // preceding proper-name span as the answer when the requested entity
        // appears immediately after the relation.
        let after_relation = normalized
            .iter()
            .skip(relation_index + 1)
            .take(5)
            .flatten()
            .collect::<Vec<_>>();
        if !entity_terms.is_empty()
            && entity_terms
                .iter()
                .all(|entity| after_relation.iter().any(|term| *term == entity))
        {
            let mut preceding = raw_tokens[..relation_index]
                .iter()
                .rev()
                .cloned()
                .collect::<Vec<_>>();
            let mut keys = collect_adjacent_name_keys(preceding.iter(), query_terms);
            keys.reverse();
            if !keys.is_empty() {
                return keys;
            }
            preceding.clear();
        }
    }
    Vec::new()
}

fn collect_adjacent_name_keys<'a>(
    tokens: impl Iterator<Item = &'a String>,
    query_terms: &HashSet<String>,
) -> Vec<String> {
    const CONNECTORS: &[&str] = &["and", "da", "das", "de", "do", "dos", "e", "of"];
    const GENERIC: &[&str] = &["empresa", "fonte", "jornal", "pesquisa", "segundo"];
    let mut keys = Vec::new();
    let mut started = false;
    for raw in tokens.take(12) {
        let terms = relevance_terms(raw);
        let connector_key = raw
            .trim_matches(|character: char| !character.is_alphanumeric())
            .to_ascii_lowercase();
        let connector = CONNECTORS.contains(&connector_key.as_str());
        if connector && started {
            continue;
        }
        let proper = raw.chars().next().is_some_and(char::is_uppercase);
        if !proper {
            if started {
                break;
            }
            continue;
        }
        let token_keys = terms
            .into_iter()
            .filter(|term| term.len() >= 4)
            .filter(|term| !query_terms.contains(term))
            .filter(|term| !GENERIC.contains(&term.as_str()))
            .collect::<Vec<_>>();
        if token_keys.is_empty() {
            // Short proper names/initials (for example "Sam") may sit
            // inside a longer name span even though keys shorter than four
            // characters are deliberately not used for identity matching.
            continue;
        }
        started = true;
        for key in token_keys {
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
    }
    keys
}

fn percent_signature(text: &str) -> Vec<String> {
    let mut values = Vec::new();
    for token in text.split_whitespace() {
        if !token.contains('%') {
            continue;
        }
        let raw: String = token
            .chars()
            .filter(|character| character.is_ascii_digit() || matches!(character, ',' | '.'))
            .collect();
        let value = raw
            .trim_matches(|character| matches!(character, ',' | '.'))
            .to_string();
        if !value.is_empty() && !values.contains(&value) {
            values.push(value);
            if values.len() == 2 {
                break;
            }
        }
    }
    values
}

fn has_repeated_leading_percent_pair(text: &str) -> bool {
    let mut values = Vec::new();
    for token in text.split_whitespace().filter(|token| token.contains('%')) {
        let value: String = token
            .chars()
            .filter(|character| character.is_ascii_digit() || matches!(character, ',' | '.'))
            .collect::<String>()
            .trim_matches(|character| matches!(character, ',' | '.'))
            .to_string();
        if !value.is_empty() {
            values.push(value);
        }
    }
    values.len() >= 4
        && values[2..]
            .windows(2)
            .any(|pair| pair[0] == values[0] && pair[1] == values[1])
}

fn poll_pair_crosses_section_boundary(text: &str) -> bool {
    let percent_positions = text
        .match_indices('%')
        .take(2)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if percent_positions.len() != 2 {
        return false;
    }
    // `percentage_windows` deliberately bridges short sentence boundaries so
    // list-formatted candidate/value pairs can be recovered. It must not,
    // however, bridge a Markdown heading into the next poll scenario: doing
    // so pairs the last percentage of one slate with the first percentage of
    // another.
    text[percent_positions[0] + 1..percent_positions[1]].contains('#')
}

fn canonical_percent_signature(signature: &[String]) -> Vec<String> {
    let mut canonical = signature.to_vec();
    canonical.sort_by(|left, right| {
        let left_value = left.replace(',', ".").parse::<f64>().unwrap_or_default();
        let right_value = right.replace(',', ".").parse::<f64>().unwrap_or_default();
        right_value.total_cmp(&left_value)
    });
    canonical
}

fn has_non_candidate_leading_percentage(text: &str) -> bool {
    let Some(percent_index) = text.find('%') else {
        return false;
    };
    let prefix_terms = relevance_terms(&text[..percent_index]);
    prefix_terms.iter().rev().take(8).any(|term| {
        matches!(
            term.as_str(),
            "branco"
                | "brancos"
                | "certeza"
                | "decidido"
                | "decididos"
                | "decisao"
                | "indeciso"
                | "indecisos"
                | "margem"
                | "nulo"
                | "nulos"
                | "rejeicao"
        )
    })
}

fn poll_candidate_keys(text: &str) -> HashSet<String> {
    ordered_poll_candidate_keys(text).into_iter().collect()
}

fn poll_candidate_identity_sets(text: &str) -> Vec<HashSet<String>> {
    let percent_positions: Vec<_> = text
        .match_indices('%')
        .take(2)
        .map(|(index, _)| index)
        .collect();
    if percent_positions.len() < 2 {
        return Vec::new();
    }
    let ranges = [
        (0usize, percent_positions[0]),
        (percent_positions[0] + 1, percent_positions[1]),
    ];
    ranges
        .into_iter()
        .map(|(start, end)| {
            let segment = strip_parenthetical(&text[start..end]);
            segment
                .split_whitespace()
                .filter(|token| {
                    token
                        .trim_matches(|character: char| !character.is_alphanumeric())
                        .chars()
                        .next()
                        .is_some_and(char::is_uppercase)
                })
                .flat_map(relevance_terms)
                .filter(|term| term.len() >= 3 && !POLL_NON_CANDIDATES.contains(&term.as_str()))
                .collect()
        })
        .collect()
}

fn poll_identities_compatible(
    left: &ExtractiveCandidate<'_>,
    right: &ExtractiveCandidate<'_>,
) -> bool {
    let (Some(left), Some(right)) = (&left.poll_identities, &right.poll_identities) else {
        return false;
    };
    poll_identity_sets_compatible(left, right)
}

fn poll_identity_sets_compatible(left: &[HashSet<String>], right: &[HashSet<String>]) -> bool {
    if left.len() != 2 || right.len() != 2 {
        return false;
    }
    let aligned = !left[0].is_disjoint(&right[0]) && !left[1].is_disjoint(&right[1]);
    let reversed = !left[0].is_disjoint(&right[1]) && !left[1].is_disjoint(&right[0]);
    aligned || reversed
}

fn ordered_poll_candidate_keys(text: &str) -> Vec<String> {
    let leading_pair = text
        .match_indices('%')
        .nth(1)
        .map(|(index, _)| &text[..=index])
        .unwrap_or(text);
    let identity_text = strip_parenthetical(leading_pair);
    let mut keys = Vec::new();
    for key in identity_text
        .split_whitespace()
        .filter(|token| {
            token
                .trim_matches(|character: char| !character.is_alphanumeric())
                .chars()
                .next()
                .is_some_and(char::is_uppercase)
        })
        .flat_map(relevance_terms)
        .filter(|term| term.len() >= 3 && !POLL_NON_CANDIDATES.contains(&term.as_str()))
    {
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    keys
}

const POLL_NON_CANDIDATES: &[&str] = &[
    "alternativo",
    "atlas",
    "bloomberg",
    "cenario",
    "datafolha",
    "eleicao",
    "eleitoral",
    "escritor",
    "espontaneo",
    "estimulado",
    "globo",
    "hipotese",
    "leia",
    "levantamento",
    "mais",
    "numero",
    "numeros",
    "pesquisa",
    "presidente",
    "presidencia",
    "quaest",
    "republica",
    "resultado",
    "resultados",
    "senador",
    "simulacao",
    "turno",
    "veja",
    "voto",
];

fn strip_parenthetical(text: &str) -> String {
    let mut depth = 0usize;
    text.chars()
        .filter(|character| match character {
            '(' => {
                depth += 1;
                false
            }
            ')' => {
                depth = depth.saturating_sub(1);
                false
            }
            _ => depth == 0,
        })
        .collect()
}

fn poll_group_score(group: &[ExtractiveCandidate<'_>]) -> usize {
    let mut score = group
        .iter()
        .flat_map(|candidate| candidate.answer_keys.iter().cloned())
        .collect::<HashSet<_>>()
        .len()
        .min(10);
    let folded: Vec<_> = group
        .iter()
        .map(|candidate| relevance_terms(&candidate.text).join(" "))
        .collect();
    if folded
        .iter()
        .any(|text| text.contains("intencao voto") || text.contains("intencoes voto"))
    {
        score += 20;
    }
    if folded.iter().any(|text| contains_first_round(text)) {
        score += 200;
    } else if folded.iter().any(|text| contains_second_round(text)) {
        score += 100;
    }
    // Within the same round, lead-candidate pairs are more useful than a
    // coincidental, heavily repeated pair near the bottom of a long slate.
    // This affects presentation order only; every emitted pair still needs
    // independent source-family corroboration.
    score += group
        .iter()
        .filter_map(|candidate| {
            let values = percent_signature(&candidate.text);
            (values.len() >= 2).then(|| {
                values
                    .iter()
                    .take(2)
                    .filter_map(|value| value.replace(',', ".").parse::<f64>().ok())
                    .sum::<f64>()
                    .round() as usize
            })
        })
        .max()
        .unwrap_or_default();
    score
}

fn poll_source_family_count(group: &[ExtractiveCandidate<'_>]) -> usize {
    group
        .iter()
        .map(|candidate| source_family(&candidate.result.url))
        .collect::<HashSet<_>>()
        .len()
}

/// Return poll-unit identities already supported by either retained text or
/// the Stage-2 lineage join. The latter matters for an editorial page that
/// reports the exact registered measurement but omits the registration code:
/// once the decision layer has matched its fingerprint and institute to one
/// verified unit, answer construction must not throw that dependency away.
fn poll_registration_ids_for_answer(result: &SearchResultItem) -> Vec<String> {
    let mut ids = crate::pesqele::registration_ids_for_result(result);
    if let Some(lineage) = result
        .decision
        .as_ref()
        .and_then(|decision| decision.evidence_lineage.as_ref())
        && lineage.unit_kind == "poll_measurement"
        && lineage.registry_verified
        && !lineage.unit_ref.is_empty()
        && !ids.contains(&lineage.unit_ref)
    {
        ids.push(lineage.unit_ref.clone());
    }
    ids
}

fn poll_primary_score(candidate: &ExtractiveCandidate<'_>) -> usize {
    let folded = relevance_terms(&candidate.text).join(" ");
    let mut score = 0usize;
    let has_navigation_prefix = folded.contains("leia tambem")
        || folded.contains("veja tambem")
        || folded.contains("conteudo relacionado");
    if contains_first_round(&folded) || contains_second_round(&folded) {
        score += 40;
    }
    if folded.contains("intencao voto")
        || folded.contains("intencoes voto")
        || folded.contains("cenario")
        || folded.contains("simulacao")
    {
        score += 20;
    }
    if has_navigation_prefix {
        score.saturating_sub(60)
    } else {
        score
    }
}

fn citation(result: &SearchResultItem, evidence_excerpt: &str) -> AnswerCitation {
    // Titles can seed extraction, but the citation must expose retained source
    // bytes rather than presenting a discovery headline as a body quotation.
    let excerpt = retained_citation_excerpt(&result.content_md, evidence_excerpt);
    AnswerCitation {
        evidence_role: "claim_text".to_string(),
        decision_rank: result.rank,
        title: result.title.clone(),
        url: result.url.clone(),
        source: result.source.clone(),
        evidence_excerpt: excerpt,
    }
}

fn retained_citation_excerpt(body: &str, requested: &str) -> Option<String> {
    let tokens = |text: &str| {
        text.split(|c: char| !c.is_alphanumeric())
            .filter(|t| !t.is_empty())
            .map(str::to_lowercase)
            .collect::<Vec<_>>()
    };
    let normalized_body = tokens(body).join(" ");
    let requested_tokens = tokens(requested);
    let normalized_requested = requested_tokens.join(" ");
    if normalized_body.is_empty() || normalized_requested.is_empty() {
        return None;
    }
    if normalized_body.contains(&normalized_requested) {
        return Some(shorten_excerpt(requested, 260));
    }
    // A locator, not a new entailment judgment. The answer constructor already
    // checks target relation and corroboration. Preserve a contiguous original
    // window, including negations/qualifiers, when its headline differs.
    let terms: HashSet<_> = requested_tokens
        .iter()
        .filter(|t| t.len() > 2 || t.chars().all(|c| c.is_ascii_digit()))
        .collect();
    let words: Vec<_> = body.split_whitespace().collect();
    let (start, score) = (0..words.len())
        .map(|start| {
            let text = words[start..(start + 34).min(words.len())].join(" ");
            let unique: HashSet<_> = tokens(&text).into_iter().collect();
            let score = terms
                .iter()
                .filter(|term| unique.contains(term.as_str()))
                .map(|term| {
                    if term.chars().all(|c| c.is_ascii_digit()) {
                        8
                    } else {
                        1
                    }
                })
                .sum::<usize>();
            (start, score)
        })
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))?;
    if score < 3 {
        return None;
    }
    Some(shorten_excerpt(
        &words[start..(start + 34).min(words.len())].join(" "),
        260,
    ))
}

fn registry_citation(record: &crate::pesqele::PesqEleRecord) -> AnswerCitation {
    let institute = record
        .institute
        .split_once(" - ")
        .map(|(_, name)| name.trim())
        .unwrap_or(record.institute.trim());
    let excerpt = format!(
        "Registro TSE {}: {}; escopo {}; cargo {}; {} entrevistados; campo de {} a {}; divulgação {}; contratantes {}; valor {}.",
        record.registration_id,
        institute,
        record.scope,
        record.office,
        record.respondents,
        record.fieldwork_start,
        record.fieldwork_end,
        record.disclosure_date,
        record.contractors,
        record.cost,
    );
    AnswerCitation {
        evidence_role: "registry_metadata".to_string(),
        decision_rank: 0,
        title: format!("TSE PesqEle — {}", record.registration_id),
        url: record.source_url.clone(),
        source: "tse_pesqele".to_string(),
        evidence_excerpt: Some(shorten_excerpt(&excerpt, 360)),
    }
}

fn shorten_excerpt(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }

    let mut clipped: String = text.chars().take(max_chars).collect();
    if let Some((index, _)) = clipped
        .char_indices()
        .rev()
        .find(|(_, character)| character.is_whitespace())
    {
        clipped.truncate(index);
    }
    clipped.push('…');
    clipped
}

fn dedup_citations(citations: &mut Vec<AnswerCitation>) {
    citations.sort_by_key(|citation| citation.decision_rank);
    let mut families = HashSet::new();
    citations.retain(|citation| families.insert(source_family(&citation.url)));
}

fn uncertainty_summary(results: &[SearchResultItem]) -> (String, Option<f64>) {
    let modes: HashSet<_> = results
        .iter()
        .filter_map(|result| result.decision.as_ref())
        .map(|decision| decision.uncertainty_mode.as_str())
        .collect();
    let mode = if modes.len() == 1 {
        modes.into_iter().next().unwrap_or("not_run").to_string()
    } else if modes.is_empty() {
        "not_run".to_string()
    } else {
        "mixed".to_string()
    };
    let ignorance: Vec<_> = results
        .iter()
        .filter_map(|result| result.decision.as_ref())
        .map(|decision| decision.ignorance)
        .collect();
    let average = (!ignorance.is_empty() && mode != "not_applicable")
        .then(|| ignorance.iter().sum::<f64>() / ignorance.len() as f64);
    (mode, average)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalized_invention_relation_is_also_accepted_in_retained_text() {
        let source = result(
            1,
            "https://archive.test/device",
            "Novel Device",
            "Alice Example invented Novel Device in 1991.",
        );
        assert!(best_factual_sentence("who invented Novel Device", &source).is_some());
        let denied = result(
            1,
            "https://archive.test/device",
            "Novel Device",
            "Alice Example never invented Novel Device.",
        );
        assert!(best_factual_sentence("who invented Novel Device", &denied).is_none());
        let developed = result(
            1,
            "https://archive.test/device",
            "Novel Device",
            "Alice Example developed Novel Device in 1991.",
        );
        assert!(best_factual_sentence("who developed Novel Device", &developed).is_some());
    }

    #[test]
    fn a_factual_headline_cannot_override_a_retained_denial() {
        let source = result(
            1,
            "https://archive.test/acme",
            "Acme was founded by Alice Example.",
            "Acme was not founded by Alice Example.",
        );
        assert!(best_factual_sentence("who founded Acme", &source).is_none());
    }

    #[test]
    fn unrelated_founder_enumeration_does_not_taint_the_target() {
        let source = result(
            1,
            "https://archive.test/acme",
            "Acme interview",
            "Acme announced a new product. OtherCompany was founded by Alice Example including other cofounders.",
        );
        assert!(!founder_enumeration_has_partial_cue(
            "who founded Acme",
            &[source]
        ));
        let relevant = result(
            1,
            "https://archive.test/acme",
            "Acme interview",
            "Acme was founded by Alice Example including other cofounders.",
        );
        assert!(founder_enumeration_has_partial_cue(
            "who founded Acme",
            &[relevant]
        ));
    }

    #[test]
    fn generic_founding_sentence_requires_matching_title_context() {
        let unrelated = result(
            1,
            "https://example.test/archive",
            "Archive of companies",
            "The company was founded by Ada Example and Ben Example.",
        );
        assert!(best_factual_sentence("who founded Acme", &unrelated).is_none());
        let matching = result(
            1,
            "https://example.test/acme",
            "Acme history",
            "The company was founded by Ada Example and Ben Example.",
        );
        assert!(best_factual_sentence("who founded Acme", &matching).is_some());
    }

    #[test]
    fn parenthetical_alias_does_not_erase_the_entity_relation() {
        let text = "ExampleSoft (pronounced according to the original regional dialect) was created by Alice Example.";
        let evidence = result(
            1,
            "https://example.test/software",
            "ExampleSoft history",
            text,
        );
        assert!(best_factual_sentence("who created ExampleSoft", &evidence).is_some());
        let wrong = result(
            1,
            "https://example.test/software",
            "ExampleSoft history",
            "ExampleSoft (a company) identifies content created by Alice Example.",
        );
        assert!(best_factual_sentence("who created ExampleSoft", &wrong).is_none());
        assert!(!factual_relation_direction_matches_entity(
            "who founded Some Long Institution Name",
            "(was created by Alice Example) Name"
        ));
        assert!(!factual_relation_header_matches_entity(
            "who founded Some Long Institution Name",
            "Founder"
        ));
    }

    #[test]
    fn negated_founding_is_not_an_answer_to_who_founded() {
        for text in [
            "Acme was not founded by Ada Example.",
            "Acme was never founded by Ada Example.",
            "Acme não foi fundada por Ada Example.",
        ] {
            let denied = result(1, "https://example.test/acme", "Acme history", text);
            assert!(
                best_factual_sentence("who founded Acme", &denied).is_none(),
                "{text}"
            );
        }
        let positive = result(
            1,
            "https://example.test/acme",
            "Acme history",
            "Acme was not only founded by Ada Example, but also funded by Ben Example.",
        );
        assert!(best_factual_sentence("who founded Acme", &positive).is_some());
    }

    #[test]
    fn citation_locator_preserves_retained_context_not_a_headline() {
        let body = "Investigators concluded Alpha did not found Beta. Beta was instead founded by Gamma, according to the archive.";
        let excerpt = retained_citation_excerpt(body, "Alpha founded Beta").unwrap();
        assert!(excerpt.contains("did not found"));
        assert!(body.contains(&excerpt));
        assert_ne!(excerpt, "Alpha founded Beta");
    }

    #[test]
    fn citation_locator_does_not_invent_missing_support() {
        assert!(retained_citation_excerpt("", "Alpha founded Beta").is_none());
        assert!(
            retained_citation_excerpt("Navigation footer and cookie policy", "Alpha founded Beta")
                .is_none()
        );
    }

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
            content_relevant: Some(content.to_string()),
            domain_first_seen_at: None,
            domain_url_count: None,
            domain_reputation_score: None,
            domain_total_votes: None,
            decision: None,
        }
    }

    fn registry_record() -> crate::pesqele::PesqEleRecord {
        crate::pesqele::PesqEleRecord {
            registration_id: "BR-04496/2026".to_string(),
            registered_at: "15/08/2026".to_string(),
            office: "Presidente".to_string(),
            disclosure_date: "21/08/2026".to_string(),
            institute: "CNPJ: 07630546000175 - DATAFOLHA INSTITUTO DE PESQUISAS LTDA.".to_string(),
            election: "Eleições Gerais 2026".to_string(),
            respondents: "2058".to_string(),
            fieldwork_start: "18/08/2026".to_string(),
            fieldwork_end: "20/08/2026".to_string(),
            statistician: "Renata Nunes Cesar".to_string(),
            statistician_registration: "7249A".to_string(),
            scope: "BRASIL".to_string(),
            cost: "R$ 307.641,60".to_string(),
            self_funded: "Não".to_string(),
            contractors: "EMPRESA FOLHA DA MANHA S.A.; GLOBO COMUNICACAO E PARTICIPACOES S/A"
                .to_string(),
            methodology: "Pesquisa quantitativa por amostragem.".to_string(),
            sampling_plan: "Margem de erro de 2 pontos; confiança de 95%.".to_string(),
            source_url: "https://pesqele-divulgacao.tse.jus.br/app/pesquisa/listar.xhtml"
                .to_string(),
            fetched_at: 0,
        }
    }

    #[test]
    fn factual_answer_requires_two_domains_with_compatible_names() {
        let results = vec![
            result(
                1,
                "https://news.test/anthropic",
                "Fundadores da Anthropic",
                "A Anthropic foi fundada por Dario Amodei e Daniela Amodei.",
            ),
            result(
                2,
                "https://reference.test/anthropic",
                "História da Anthropic",
                "A Anthropic foi fundada por Dario e Daniela Amodei.",
            ),
        ];
        let answer = construct_answer("quem fundou a Anthropic?", QueryIntent::Factual, &results);
        assert_eq!(answer.status, "extractive");
        assert_eq!(answer.claims[0].citations.len(), 2);
        let lineage = answer.claims[0].evidence_lineage.as_ref().unwrap();
        assert_eq!(lineage.underlying_unit_kind, "factual_relation_claim");
        assert_eq!(lineage.identity_status, "normalized_claim_signature_only");
        assert_eq!(lineage.underlying_unit_ids.len(), 1);
        assert_eq!(lineage.underlying_unit_count, None);
        assert_eq!(lineage.reporting_source_families, 2);
        assert_eq!(lineage.claim_text_citations, 2);
        assert_eq!(lineage.independence_status, "unknown_editorial_derivation");
        assert_eq!(lineage.known_independent_observations, None);
        assert!(
            lineage
                .unresolved_dependencies
                .contains(&"syndication".to_string())
        );
        assert_eq!(lineage.registry_metadata_records, 0);
    }

    #[test]
    fn destination_uses_hostname_when_crawled_title_is_empty() {
        let destination = result(1, "https://www.python.org/", "", "Python");
        let answer = construct_answer("python", QueryIntent::Navigational, &[destination]);
        assert_eq!(
            answer.summary,
            "Destino mais provável: python.org — https://www.python.org/"
        );
    }

    #[test]
    fn ambiguous_short_query_does_not_promote_an_unverified_host_as_official() {
        let candidate = result(
            1,
            "https://reference.example/rarecompound",
            "Rarecompound explained",
            "A reference article about the term.",
        );

        let answer = construct_answer("rarecompound", QueryIntent::Navigational, &[candidate]);

        assert_eq!(answer.status, "results_only");
        assert!(answer.summary.contains("nenhum host canônico"));
        assert!(answer.warning.is_some());
    }

    #[test]
    fn tool_query_returns_a_tool_slate_not_an_official_destination_claim() {
        let tool = result(
            1,
            "https://www.onlinegdb.com/online_python_compiler",
            "Online Python Compiler",
            "Execute Python in the browser.",
        );
        let answer = construct_answer("python online compiler", QueryIntent::Tool, &[tool]);
        assert_eq!(answer.status, "tool_results");
        assert_eq!(answer.intent, "tool");
        assert!(
            answer
                .summary
                .starts_with("Ferramentas funcionais encontradas")
        );
        assert!(answer.claims.is_empty());
    }

    #[test]
    fn founder_question_rejects_unrelated_mentions_of_founders() {
        assert!(!has_required_relation_cue(
            "quem fundou a Anthropic?",
            "A Anthropic é governada por seus fundadores e por um trust."
        ));
        assert!(has_required_relation_cue(
            "quem fundou a Anthropic?",
            "A Anthropic foi fundada por Dario e Daniela Amodei."
        ));
        assert!(!has_required_relation_cue(
            "quem criou a Anthropic?",
            "A Anthropic, criadora do modelo Claude, concorre com a OpenAI."
        ));
        assert!(has_required_relation_cue(
            "quem criou o Pix?",
            "O Pix foi criado pelo Banco Central do Brasil."
        ));
    }

    #[test]
    fn factual_answer_does_not_borrow_entity_context_for_an_unrelated_relation() {
        for unrelated in [
            result(
                1,
                "https://news.test/anthropic-content",
                "Anthropic vai identificar conteúdo gerado pelo Claude",
                "A estratégia diferencia o que é criado pelos modelos Claude do que é obra humana.",
            ),
            result(
                1,
                "https://paper.test/anthropic-content",
                "Anthropic vai identificar conteúdo gerado pelo Claude",
                "A Anthropic vai passar a identificar textos e imagens gerados pelo Claude para facilitar a detecção de conteúdo criado por IA.",
            ),
        ] {
            let answer = construct_answer(
                "quem criou a Anthropic?",
                QueryIntent::Factual,
                &[unrelated],
            );

            assert_eq!(answer.status, "insufficient_evidence");
            assert!(answer.claims.is_empty());
            assert!(!answer.summary.contains("conteúdo criado"));
        }
    }

    #[test]
    fn factual_discovery_surface_keeps_auditable_leads_and_rejects_relation_collisions() {
        for relevant in [
            result(
                1,
                "https://reference.test/anthropic-founders",
                "Who founded Anthropic and why?",
                "A profile of the company's founding team.",
            ),
            result(
                1,
                "https://profile.test/dario-amodei",
                "Dario Amodei: fundador da Anthropic",
                "Perfil do executivo.",
            ),
            result(
                1,
                "https://history.test/anthropic",
                "Anthropic Company History 2026: Founders and Investors",
                "A history of the company.",
            ),
        ] {
            assert!(factual_discovery_surface_matches_relation(
                "quem fundou a Anthropic?",
                &relevant,
            ));
        }

        for snippet in [
            "In 2021, he and his sister Daniela Amodei co-founded Anthropic, the company behind Claude.",
            "She co-founded Anthropic in 2021 with a group of former colleagues.",
            "Chris Olah is a Canadian researcher and a co-founder of Anthropic.",
        ] {
            let mut reference = result(
                1,
                "https://en.wikipedia.org/wiki/Profile",
                "Researcher profile",
                "",
            );
            reference.source = "wikipedia".to_string();
            reference.snippet = snippet.to_string();
            reference.content_relevant = None;
            assert!(factual_discovery_surface_matches_relation(
                "quem fundou a Anthropic?",
                &reference,
            ));
        }

        for collision in [
            result(
                1,
                "https://news.test/karpathy",
                "OpenAI co-founder Andrej Karpathy joins Anthropic's pre-training team",
                "Karpathy passou a integrar a equipe da Anthropic.",
            ),
            result(
                1,
                "https://news.test/content",
                "Anthropic identifica conteúdo criado por IA",
                "A empresa lançou um detector.",
            ),
            result(
                1,
                "https://answers.test/claude",
                "Quem criou o Claude, a inteligência artificial da Anthropic?",
                "Uma página sobre o produto Claude.",
            ),
            result(
                1,
                "https://en.wikipedia.org/wiki/Founders_Fund",
                "Founders Fund",
                "Anthropic: Founders Fund co-led its Series G funding round.",
            ),
            result(
                1,
                "https://anthropic.skilljar.com/",
                "Anthropic",
                "An accreditation program created for AWS, Anthropic launched a training course.",
            ),
        ] {
            assert!(!factual_discovery_surface_matches_relation(
                "quem criou a Anthropic?",
                &collision,
            ));
        }
    }

    #[test]
    fn factual_public_surface_is_stricter_than_retained_body_evidence() {
        let mut vague = result(
            1,
            "https://example.test/profile",
            "A closer look at the AI laboratory",
            "Background and analysis.",
        );
        vague.content_md =
            "Anthropic was founded by Dario Amodei and Daniela Amodei in 2021.".to_string();
        vague.content_relevant = Some(vague.content_md.clone());

        assert!(factual_discovery_surface_matches_relation(
            "quem fundou a Anthropic?",
            &vague,
        ));
        assert!(!factual_public_surface_matches_relation(
            "quem fundou a Anthropic?",
            &vague,
        ));
    }

    #[test]
    fn factual_question_heading_is_not_treated_as_an_answer() {
        let result = result(
            1,
            "https://example.test/pix",
            "Quem criou o Pix? Entenda a origem",
            "Quem criou o Pix? Entenda a origem do pagamento.",
        );
        let answer = construct_answer("quem criou o Pix?", QueryIntent::Factual, &[result]);
        assert_eq!(answer.status, "insufficient_evidence");
        assert!(answer.claims.is_empty());
    }

    #[test]
    fn factual_extraction_checks_full_retained_body_after_distillation() {
        let mut first = result(
            1,
            "https://news.test/pix",
            "A origem do Pix",
            "A ideia de criar o Pix surgiu de servidores do Banco Central do Brasil.",
        );
        first.content_relevant = Some("Pix é um sistema de pagamentos instantâneos.".to_string());
        let second = result(
            2,
            "https://reference.test/pix",
            "História do Pix",
            "O Pix foi criado pelo Banco Central do Brasil.",
        );
        let answer = construct_answer("quem criou o Pix?", QueryIntent::Factual, &[first, second]);
        assert_eq!(answer.status, "extractive");
        assert_eq!(answer.claims[0].citations.len(), 2);
    }

    #[test]
    fn factual_answer_accepts_creator_and_creation_wording_but_rejects_cut_fragments() {
        let first = result(
            1,
            "https://news.test/pix",
            "A origem do Pix",
            "O criador do Pix é o Banco Central do Brasil.",
        );
        let second = result(
            2,
            "https://reference.test/pix",
            "História do Pix",
            "O Banco Central iniciou o processo de criação do Pix em 2018.",
        );
        let answer = construct_answer("quem criou o Pix?", QueryIntent::Factual, &[first, second]);
        assert_eq!(answer.status, "extractive");
        assert!(answer.summary.ends_with('.'));
        assert!(looks_truncated(
            "O Pix é um sistema criado por técnicos do Banco Central, que opera em"
        ));
    }

    #[test]
    fn factual_answer_prefers_primary_source_wording_when_corroborated() {
        assert!(sentence_can_inherit_entity_context(
            "quem criou o Pix?",
            "### 2018 Início do processo de criação da plataforma pelo Banco Central."
        ));
        let secondary = result(
            1,
            "https://news.test/pix",
            "Linha do tempo da criação do Pix",
            "### 2018 Início do processo de criação da plataforma pelo Banco Central.",
        );
        let primary = result(
            2,
            "https://www.bcb.gov.br/estabilidadefinanceira/pix-sobre",
            "Pix",
            "O Pix é o meio de pagamento instantâneo criado pelo Banco Central.",
        );
        let answer = construct_answer(
            "quem criou o Pix?",
            QueryIntent::Factual,
            &[secondary, primary],
        );
        assert_eq!(answer.status, "extractive");
        assert_eq!(
            answer.claims[0].text,
            "O Pix é o meio de pagamento instantâneo criado pelo Banco Central."
        );
        assert!(
            answer.claims[0]
                .citations
                .iter()
                .any(|citation| citation.url.contains("bcb.gov.br/"))
        );
        assert!(!answer.claims[0].text.contains('#'));
    }

    #[test]
    fn factual_answer_joins_created_and_developed_origin_wording() {
        let editorial = result(
            1,
            "https://news.test/pix-origin",
            "A origem do Pix",
            "O Pix foi desenvolvido pelo Banco Central ao longo de cerca de 31 meses.",
        );
        let primary = result(
            2,
            "https://www.bcb.gov.br/estabilidadefinanceira/pix",
            "Pix",
            "Pix é um sistema de pagamento instantâneo criado pelo Banco Central.",
        );

        let answer = construct_answer(
            "quem criou o Pix?",
            QueryIntent::Factual,
            &[editorial, primary],
        );

        assert_eq!(answer.status, "extractive");
        assert!(answer.summary.contains("criado pelo Banco Central"));
        assert_eq!(answer.claims[0].citations.len(), 2);
        assert!(
            answer.claims[0]
                .citations
                .iter()
                .any(|citation| citation.url.contains("bcb.gov.br/"))
        );
    }

    #[test]
    fn factual_answer_accepts_direct_institutional_origin_wording() {
        assert!(factual_entity_terms_covered(
            "quem criou o Pix?",
            "A ideia de criar o Pix surgiu de servidores públicos do Banco Central do Brasil."
        ));
        assert!(sentence_can_inherit_entity_context(
            "quem criou o Pix?",
            "O sistema nasceu dentro do Banco Central do Brasil e foi lançado em 2020."
        ));
        let first = result(
            1,
            "https://news.test/pix",
            "A origem do Pix",
            "A ideia de criar o Pix surgiu de servidores públicos do Banco Central do Brasil.",
        );
        let second = result(
            2,
            "https://reference.test/pix",
            "Como surgiu o Pix",
            "O sistema nasceu dentro do Banco Central do Brasil e foi lançado em 2020.",
        );
        let answer = construct_answer("quem criou o Pix?", QueryIntent::Factual, &[first, second]);
        assert_eq!(answer.status, "extractive");
        assert_eq!(answer.claims[0].citations.len(), 2);
    }

    #[test]
    fn factual_creator_answer_handles_invented_and_answer_type_paraphrases() {
        let results = vec![
            result(
                1,
                "https://news.test/pix",
                "A origem do Pix",
                "A ideia de criar o Pix surgiu de servidores públicos do Banco Central do Brasil.",
            ),
            result(
                2,
                "https://reference.test/pix",
                "Como surgiu o Pix",
                "O Pix foi criado pelo Banco Central do Brasil.",
            ),
        ];
        for query in ["quem inventou o Pix?", "qual instituição criou o Pix?"] {
            let answer = construct_answer(query, QueryIntent::Factual, &results);
            assert_eq!(answer.status, "extractive", "query={query}");
            assert_eq!(answer.claims[0].citations.len(), 2, "query={query}");
        }
    }

    #[test]
    fn generic_current_query_never_uses_poll_extraction_or_expansion() {
        let poll = result(
            1,
            "https://news.test/eleicoes",
            "Pesquisa eleitoral",
            "Lula aparece com 39% e Flávio Bolsonaro com 33%.",
        );
        assert!(
            poll_corroboration_queries(
                "últimas notícias de inteligência artificial hoje",
                std::slice::from_ref(&poll)
            )
            .is_empty()
        );
        let answer = construct_answer(
            "últimas notícias de inteligência artificial hoje",
            QueryIntent::CurrentEvents,
            &[poll],
        );
        assert_eq!(answer.status, "insufficient_evidence");
        assert!(answer.claims.is_empty());
    }

    #[test]
    fn factual_corroboration_query_uses_extracted_answer_entity() {
        let evidence = result(
            1,
            "https://news.test/pix",
            "A origem do Pix",
            "A ideia de criar o Pix surgiu de servidores do Banco Central do Brasil.",
        );
        let queries = factual_corroboration_queries("quem criou o Pix?", &[evidence]);
        assert!(queries.iter().all(|query| query.contains("banco")));
        assert!(queries.iter().all(|query| query.contains("central")));
        assert_eq!(queries.len(), 2);
        assert!(queries.iter().any(|query| query.starts_with("site:gov.br")));

        let noisy = result(
            1,
            "https://news.test/pix-noisy",
            "Quem criou o Pix?",
            "Siga O TEMPO no Google e receba as principais notícias Seguir no Google Lançado em novembro de 2020, o Pix foi desenvolvido pelo Banco Central ao longo de cerca de 31 meses.",
        );
        let noisy_queries = factual_corroboration_queries("quem criou o Pix?", &[noisy]);
        assert!(
            noisy_queries
                .iter()
                .all(|query| query.contains("banco") && query.contains("central")),
            "{noisy_queries:#?}"
        );
        assert!(
            noisy_queries
                .iter()
                .all(|query| !query.contains("tempo google"))
        );
        assert!(factual_candidate_suggests_primary_authority(
            "quem criou o Pix?",
            &[result(
                1,
                "https://news.test/pix-primary-hint",
                "Quem criou o Pix?",
                "O Pix foi desenvolvido pelo Banco Central."
            )]
        ));

        let misleading_first = result(
            1,
            "https://news.test/pix-timeline",
            "Linha do tempo do Pix",
            "A Linha do Pix foi criada por especialistas para explicar o sistema.",
        );
        let authority_second = result(
            2,
            "https://news.test/pix-authority",
            "Origem do Pix",
            "O Pix foi desenvolvido pelo Banco Central do Brasil.",
        );
        let ranked_queries = factual_corroboration_queries(
            "quem criou o Pix?",
            &[misleading_first, authority_second],
        );
        assert!(
            ranked_queries
                .iter()
                .all(|query| query.contains("banco") && query.contains("central")),
            "{ranked_queries:#?}"
        );
    }

    #[test]
    fn factual_corroboration_recovers_when_only_unrelated_results_survive() {
        let unrelated = result(
            1,
            "https://news.test/anthropic-content",
            "Anthropic identifica conteúdo gerado pelo Claude",
            "A Anthropic identifica conteúdo criado por IA.",
        );
        let queries = factual_corroboration_queries("quem criou a Anthropic?", &[unrelated]);
        assert!(queries.iter().any(|query| query.contains("created by")));
        assert!(queries.iter().any(|query| query.contains("complete list")));

        let founder_queries = factual_corroboration_queries("who founded Anthropic?", &[]);
        assert!(
            founder_queries
                .iter()
                .any(|query| query.contains("founders cofounders complete list"))
        );
    }

    #[test]
    fn founder_discovery_combines_names_from_separate_retained_biographies() {
        let dario = result(
            1,
            "https://reference.test/dario",
            "Dario Amodei",
            "Dario Amodei co-founded Anthropic in 2021.",
        );
        let daniela = result(
            2,
            "https://profile.test/daniela",
            "Daniela Amodei",
            "Daniela Amodei founded Anthropic with a group of former OpenAI colleagues.",
        );
        let queries = factual_corroboration_queries("who founded Anthropic?", &[dario, daniela]);
        assert!(
            queries.iter().any(|query| {
                query.contains("\"anthropic\"")
                    && query.contains("\"dario\"")
                    && query.contains("\"daniela\"")
                    && query.contains("founders cofounders")
            }),
            "{queries:#?}"
        );
    }

    #[test]
    fn near_me_query_discloses_missing_location_instead_of_guessing() {
        let restaurant = result(
            1,
            "https://food.test/restaurante-japones",
            "Restaurante japonês",
            "Sushi e culinária japonesa.",
        );
        let answer = construct_answer(
            "restaurantes japoneses perto de mim",
            QueryIntent::Exploratory,
            &[restaurant],
        );
        assert_eq!(answer.status, "results_only");
        assert!(answer.summary.contains("sem localização explícita"));
        assert!(answer.warning.unwrap().contains("não está disponível"));
    }

    #[test]
    fn medical_exploration_discloses_unvalidated_source_authority() {
        let health = result(
            1,
            "https://health.test/dengue",
            "Sintomas de dengue",
            "Informações gerais sobre sintomas.",
        );
        let answer = construct_answer(
            "sintomas de dengue quando ir ao médico",
            QueryIntent::Exploratory,
            &[health],
        );
        assert_eq!(answer.status, "results_only");
        assert!(answer.warning.unwrap().contains("sem validação clínica"));
    }

    #[test]
    fn recommendation_exposes_unverified_commercial_dependencies() {
        let notebook = result(
            1,
            "https://reviews.test/notebooks",
            "Melhores notebooks para programar",
            "Guia comparativo.",
        );
        let answer = construct_answer(
            "melhor notebook para programar em 2026",
            QueryIntent::Exploratory,
            &[notebook],
        );
        assert_eq!(answer.status, "results_only");
        let warning = answer.warning.unwrap();
        assert!(warning.contains("comissões de afiliados"));
        assert!(warning.contains("independência editorial"));
    }

    #[test]
    fn live_value_answer_abstains_with_a_specific_validation_gap() {
        let quote = result(
            1,
            "https://quote.test/bitcoin",
            "Preço do Bitcoin hoje",
            "Cotação do Bitcoin.",
        );
        let answer = construct_answer(
            "preço do bitcoin hoje",
            QueryIntent::CurrentEvents,
            &[quote],
        );
        assert_eq!(answer.status, "insufficient_evidence");
        assert!(answer.summary.contains("horário da cotação"));
        assert!(answer.claims.is_empty());
    }

    #[test]
    fn live_value_answer_joins_two_timestamped_providers_without_claiming_independence() {
        let now = crate::market_data::unix_time_ms();
        let quote_page = result(
            1,
            "https://quote.test/bitcoin",
            "Preço do Bitcoin hoje",
            "Cotação do Bitcoin.",
        );
        let records = vec![
            crate::market_data::MarketQuoteRecord {
                provider: "coingecko".to_string(),
                asset_id: "bitcoin".to_string(),
                asset_symbol: "BTC".to_string(),
                quote_currency: "BRL".to_string(),
                value: 397_876.09,
                observed_at_unix_ms: now - 20_000,
                fetched_at_unix_ms: now,
                source_url: "https://api.coingecko.test/simple/price".to_string(),
                observation_kind: "aggregated_spot_price".to_string(),
            },
            crate::market_data::MarketQuoteRecord {
                provider: "binance".to_string(),
                asset_id: "bitcoin".to_string(),
                asset_symbol: "BTC".to_string(),
                quote_currency: "BRL".to_string(),
                value: 399_444.22,
                observed_at_unix_ms: now - 10_000,
                fetched_at_unix_ms: now,
                source_url: "https://data-api.binance.test/api/v3/avgPrice".to_string(),
                observation_kind: "five_minute_average_price".to_string(),
            },
        ];
        let answer = construct_answer_with_structured_evidence(
            "preço do bitcoin hoje",
            QueryIntent::CurrentEvents,
            &[quote_page],
            &[],
            &records,
        );
        assert_eq!(answer.status, "extractive");
        assert_eq!(answer.claims.len(), 1);
        assert_eq!(answer.claims[0].citations.len(), 2);
        assert!(answer.summary.contains("R$ 398.660,16"));
        assert!(answer.summary.contains("faixa observada"));
        let lineage = answer.claims[0].evidence_lineage.as_ref().unwrap();
        assert_eq!(lineage.underlying_unit_kind, "market_quote_snapshot");
        assert_eq!(lineage.underlying_unit_count, Some(2));
        assert_eq!(lineage.known_independent_observations, None);
        assert_eq!(
            lineage.independence_status,
            "cross_provider_dependency_unresolved"
        );
        assert!(
            answer
                .warning
                .as_deref()
                .is_some_and(|warning| warning.contains("não recomendação financeira"))
        );
    }

    #[test]
    fn current_office_holder_requires_official_and_independent_matching_names() {
        let official = result(
            1,
            "https://www.gov.br/planalto/pt-br/acesso-a-informacao/institucional/principais-cargos-e-respectivos-ocupantes-201cquem-e-quem201d",
            "Quem é Quem — Planalto",
            "PRESIDENTE DA REPÚBLICA Luiz Inácio Lula da Silva (mais conhecido como Lula) é o 39º presidente do Brasil.",
        );
        let mut independent = result(
            2,
            "https://news.test/brasil/presidencia",
            "Atual presidente participa de evento",
            "O atual presidente Luiz Inácio Lula da Silva participou do evento nesta sexta-feira.",
        );
        independent.published_at = Some(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64,
        );

        let answer = construct_answer(
            "quem é o presidente do Brasil?",
            QueryIntent::CurrentEvents,
            &[official.clone(), independent],
        );
        assert_eq!(answer.status, "extractive");
        assert_eq!(
            answer.summary,
            "O atual presidente do Brasil é Luiz Inácio Lula da Silva."
        );
        assert_eq!(answer.claims[0].citations.len(), 2);
        assert_eq!(
            answer.claims[0]
                .evidence_lineage
                .as_ref()
                .unwrap()
                .underlying_unit_kind,
            "current_office_holder_claim"
        );

        let mut incidental = result(
            3,
            "https://economia.test/2026/06/quem-criou-o-pix/",
            "Quem criou o Pix? A história do sistema",
            "O presidente Luiz Inácio Lula da Silva comentou o sistema de pagamentos.",
        );
        incidental.published_at = Some(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64,
        );
        let incidental_answer = construct_answer(
            "quem é o presidente do Brasil?",
            QueryIntent::CurrentEvents,
            &[official.clone(), incidental],
        );
        assert_eq!(incidental_answer.status, "insufficient_evidence");

        let single = construct_answer(
            "quem é o presidente do Brasil?",
            QueryIntent::CurrentEvents,
            &[official],
        );
        assert_eq!(single.status, "insufficient_evidence");
        assert!(single.claims.is_empty());
    }

    #[test]
    fn current_office_holder_accepts_explicit_english_editorial_relation() {
        let official = result(
            1,
            "https://www.gov.br/planalto/pt-br/acesso-a-informacao/institucional/principais-cargos-e-respectivos-ocupantes-201cquem-e-quem201d",
            "Quem é Quem — Planalto",
            "PRESIDENTE DA REPÚBLICA Luiz Inácio Lula da Silva é o 39º presidente do Brasil.",
        );
        let mut independent = result(
            2,
            "https://news.test/world/brazil-president-lula",
            "Brazil president Lula opens campaign",
            "Brazil president Lula opened his reelection campaign on Saturday.",
        );
        independent.published_at = Some(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64,
        );

        let answer = construct_answer(
            "quem é o presidente do Brasil?",
            QueryIntent::CurrentEvents,
            &[official, independent],
        );
        assert_eq!(answer.status, "extractive");
        assert!(answer.summary.contains("Luiz Inácio Lula da Silva"));
    }

    #[test]
    fn office_holder_corroboration_uses_name_extracted_from_official_record() {
        let official = result(
            1,
            "https://www.gov.br/planalto/pt-br/acesso-a-informacao/institucional/principais-cargos-e-respectivos-ocupantes-201cquem-e-quem201d",
            "Quem é Quem — Planalto",
            "PRESIDENTE DA REPÚBLICA Luiz Inácio Lula da Silva (nascido em 1945).",
        );
        let queries = current_office_holder_corroboration_queries(
            "quem é o presidente do Brasil?",
            &[official],
        );
        assert_eq!(queries.len(), 2);
        assert!(
            queries
                .iter()
                .all(|query| query.contains("Luiz Inácio Lula da Silva"))
        );
    }

    #[test]
    fn office_holder_name_specific_discovery_keeps_editorial_name_lead_only() {
        let official = result(
            1,
            "https://www.gov.br/planalto/pt-br/acesso-a-informacao/institucional/principais-cargos-e-respectivos-ocupantes-201cquem-e-quem201d",
            "Quem é Quem — Planalto",
            "PRESIDENTE DA REPÚBLICA Luiz Inácio Lula da Silva é o 39º presidente do Brasil.",
        );
        let editorial = result(
            2,
            "https://news.test/politica/lula-agenda",
            "Lula inaugura projeto nesta quinta-feira",
            "",
        );
        let social = result(
            3,
            "https://www.youtube.com/watch?v=example",
            "Lula participa de evento",
            "",
        );

        assert!(current_office_holder_discovery_surface_matches(
            "quem é o presidente do Brasil?",
            std::slice::from_ref(&official),
            &editorial,
        ));
        assert!(!current_office_holder_discovery_surface_matches(
            "quem é o presidente do Brasil?",
            std::slice::from_ref(&official),
            &social,
        ));
    }

    #[test]
    fn current_poll_accepts_same_measurement_across_captured_editorial_titles() {
        let mut g1 = result(
            1,
            "https://g1.globo.com/politica/eleicoes/2026/pesquisa-eleitoral/noticia/2026/08/22/datafolha-1o-turno.ghtml",
            "Pesquisa Datafolha: Lula tem 39% e Flávio Bolsonaro 33% no 1º turno | G1",
            "Pesquisa Datafolha divulgada nesta sexta-feira mostra Lula (PT) com 39% das intenções de voto no 1º turno, contra 33% de Flávio Bolsonaro (PL). O número de registro no TSE é BR-04496/2026.",
        );
        let mut r7 = result(
            2,
            "https://noticias.r7.com/eleicoes/2026/datafolha-lula-tem-39-e-flavio-33-no-primeiro-turno-21082026/",
            "Datafolha: Lula tem 39%, e Flávio, 33%, no primeiro turno",
            "Pesquisa Datafolha mostra que o presidente Luiz Inácio Lula da Silva tem 39% no 1º turno, enquanto o senador Flávio Bolsonaro (PL) tem 33%. O registro é BR-04496/2026.",
        );
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        g1.published_at = Some(now - 3_600);
        // Rolling 24-hour boundaries must not split reports of the same
        // measurement that were published on the previous calendar day.
        r7.published_at = Some(now - 27 * 3_600);

        let answer = construct_answer(
            "pesquisa eleitoral presidente brasil hoje",
            QueryIntent::CurrentEvents,
            &[g1, r7],
        );
        assert_eq!(answer.status, "extractive", "{answer:#?}");
        assert!(answer.summary.contains("39%"));
        assert!(answer.summary.contains("33%"));
    }

    #[test]
    fn office_holder_extractor_accepts_present_role_and_rejects_former_role() {
        assert_eq!(
            extract_office_holder_names(
                "O presidente Luiz Inácio Lula da Silva (PT) afirmou nesta sexta-feira que..."
            )
            .first()
            .map(String::as_str),
            Some("Luiz Inácio Lula da Silva")
        );
        assert!(
            extract_office_holder_names(
                "O ex-presidente Luiz Inácio Lula da Silva participou do arquivo histórico."
            )
            .is_empty()
        );
        assert!(
            extract_office_holder_names(
                "Former president Luiz Inácio Lula da Silva appeared in the archive."
            )
            .is_empty()
        );
        assert_eq!(
            extract_office_holder_names("presidente Lula")
                .first()
                .map(String::as_str),
            Some("Lula"),
            "a short excerpt must be handled without indexing past its tokens"
        );
    }

    #[test]
    fn founder_incompleteness_triggers_complete_list_discovery() {
        let first = result(
            1,
            "https://news.test/anthropic",
            "Fundadores da Anthropic",
            "Dario, Daniela e um grupo de colegas saíram para fundar a Anthropic.",
        );
        let second = result(
            2,
            "https://paper.test/anthropic",
            "História da Anthropic",
            "A Anthropic foi fundada por Dario e Daniela Amodei.",
        );
        let results = vec![first, second];
        let answer = construct_answer("quem fundou a Anthropic?", QueryIntent::Factual, &results);
        assert_eq!(answer.status, "insufficient_evidence");
        assert!(answer.claims.is_empty());
        assert!(answer.summary.contains("lista parcial"));
        let queries = factual_corroboration_queries("quem fundou a Anthropic?", &results);
        assert!(queries.iter().any(|query| query.contains("complete list")));

        let creator_answer =
            construct_answer("quem criou a Anthropic?", QueryIntent::Factual, &results);
        assert_eq!(creator_answer.status, "insufficient_evidence");
        assert!(creator_answer.claims.is_empty());
        assert!(creator_answer.summary.contains("lista parcial"));
        let creator_queries = factual_corroboration_queries("quem criou a Anthropic?", &results);
        assert!(
            creator_queries
                .iter()
                .any(|query| query.contains("complete list"))
        );
    }

    #[test]
    fn founder_incompleteness_detects_including_and_adjacent_other_cues() {
        let counted = result(
            1,
            "https://finance.test/anthropic",
            "Quem fundou a Anthropic?",
            "A empresa foi fundada em 2021 por sete ex-funcionários da OpenAI, incluindo os irmãos Dario Amodei e Daniela Amodei.",
        );
        let adjacent = result(
            2,
            "https://reference.test/anthropic",
            "História da Anthropic",
            "A Anthropic foi fundada em 2021 por Dario e Daniela Amodei, ambos ex-executivos da OpenAI. Eles foram acompanhados por outros pesquisadores seniores da OpenAI.",
        );
        let answer = construct_answer(
            "quem fundou a Anthropic?",
            QueryIntent::Factual,
            &[counted, adjacent],
        );
        assert_eq!(answer.status, "insufficient_evidence");
        assert!(answer.claims.is_empty());
        assert!(answer.summary.contains("lista parcial"));
    }

    #[test]
    fn founder_incompleteness_detects_explicit_founder_count_in_title() {
        let counted = result(
            1,
            "https://finance.test/anthropic",
            "Os 7 fundadores da Anthropic entram para a lista",
            "Dario e Daniela Amodei fundaram a Anthropic.",
        );
        let abbreviated = result(
            2,
            "https://reference.test/anthropic",
            "Quem fundou a Anthropic?",
            "A Anthropic foi fundada por Dario e Daniela Amodei.",
        );
        let answer = construct_answer(
            "quem fundou a Anthropic?",
            QueryIntent::Factual,
            &[counted, abbreviated],
        );
        assert_eq!(answer.status, "insufficient_evidence");
        assert!(answer.claims.is_empty());
    }

    #[test]
    fn founder_enumeration_accepts_two_near_identical_complete_lists() {
        assert!(sentence_can_inherit_entity_context(
            "quem fundou a Anthropic?",
            "A empresa foi fundada por Dario Amodei, Daniela Amodei, Jack Clark e Sam McCandlish."
        ));
        let partial = result(
            1,
            "https://news.test/anthropic",
            "História da Anthropic",
            "Dario, Daniela e um grupo de colegas fundaram a Anthropic.",
        );
        let first = result(
            2,
            "https://reference.test/anthropic",
            "Fundadores da Anthropic",
            "A Anthropic foi fundada por Dario Amodei, Daniela Amodei, Jack Clark e Sam McCandlish.",
        );
        let second = result(
            3,
            "https://paper.test/anthropic",
            "Quem criou a Anthropic",
            "A empresa foi fundada por Dario Amodei, Daniela Amodei, Jack Clark e Sam McCandlish.",
        );
        let first_keys = answer_keys("quem fundou a Anthropic?", &first.content_md);
        let second_keys = answer_keys("quem fundou a Anthropic?", &second.content_md);
        assert!(first_keys.len() >= 6, "first={first_keys:?}");
        assert!(second_keys.len() >= 6, "second={second_keys:?}");
        assert_eq!(first_keys, second_keys);
        let answer = construct_answer(
            "quem fundou a Anthropic?",
            QueryIntent::Factual,
            &[partial, first, second],
        );
        assert_eq!(answer.status, "extractive");
        assert!(answer.summary.contains("Jack Clark"));
        assert_eq!(answer.claims[0].citations.len(), 2);
    }

    #[test]
    fn factual_answer_rejects_meta_discussion_as_corroboration() {
        let first = result(
            1,
            "https://news.test/pix",
            "A origem do Pix",
            "A ideia de criar o Pix surgiu de servidores do Banco Central do Brasil.",
        );
        let second = result(
            2,
            "https://reference.test/pix",
            "Quem criou o Pix?",
            "O debate sobre quem criou o Pix voltou ao Brasil em 2026.",
        );
        let answer = construct_answer("quem criou o Pix?", QueryIntent::Factual, &[first, second]);
        assert_eq!(answer.status, "insufficient_evidence");
        assert!(answer.claims.is_empty());
    }

    #[test]
    fn current_answer_requires_repeated_percentage_signature() {
        let results = vec![
            result(
                1,
                "https://news.test/poll",
                "Pesquisa: Alice tem 39% e Bruno 33%",
                "No primeiro turno, Alice tem 39% e Bruno aparece com 33%.",
            ),
            result(
                2,
                "https://paper.test/poll",
                "Levantamento aponta Alice com 39% e Bruno com 33%",
                "O levantamento registrou Alice com 39% e Bruno com 33%.",
            ),
        ];
        let answer = construct_answer(
            "pesquisa eleitoral hoje",
            QueryIntent::CurrentEvents,
            &results,
        );
        assert_eq!(answer.status, "extractive");
        assert_eq!(answer.claims[0].citations.len(), 2);
    }

    #[test]
    fn current_answer_checks_full_retained_body_after_distillation() {
        let mut first = result(
            1,
            "https://news.test/poll",
            "Datafolha",
            "Lula tem 39% e Flávio Bolsonaro registra 33%.",
        );
        first.content_relevant = Some(
            "No cenário com outro candidato, Lula tem 39% e Flávio Bolsonaro 32%.".to_string(),
        );
        let second = result(
            2,
            "https://paper.test/poll",
            "Datafolha",
            "Lula aparece com 39% e Flávio Bolsonaro tem 33%.",
        );
        let answer = construct_answer(
            "pesquisa eleitoral presidente hoje",
            QueryIntent::CurrentEvents,
            &[first, second],
        );
        assert_eq!(answer.status, "extractive");
        assert!(answer.summary.contains("33%"));
    }

    #[test]
    fn current_answer_matches_prose_and_list_identity_formats() {
        let first = result(
            1,
            "https://bandab.test/poll",
            "Pesquisa Datafolha para presidente",
            "O presidente Lula (PT) aparece com 39% e o senador Flávio Bolsonaro (PL) com 33% das intenções de voto.",
        );
        let mut second = result(
            2,
            "https://g1.test/poll",
            "Datafolha: resultados para a Presidência",
            "Veja os números: - Lula (PT): 39% - Flávio Bolsonaro (PL): 33% - Ronaldo Caiado (PSD): 5%.",
        );
        second.content_relevant =
            Some("Cenário alternativo: Lula (PT): 39% - Flávio Bolsonaro (PL): 32%.".to_string());
        let answer = construct_answer(
            "pesquisa eleitoral presidente hoje",
            QueryIntent::CurrentEvents,
            &[first, second],
        );
        assert_eq!(
            answer.status, "extractive",
            "prose/list candidates did not match: {:?}",
            answer.summary
        );
        assert!(answer.summary.contains("33%"));
    }

    #[test]
    fn current_answer_does_not_treat_external_titles_as_retained_evidence() {
        let mut first = result(
            1,
            "https://news.test/poll",
            "Alice tem 39% e Bruno 33%",
            "Alice tem 39% e Bruno 33%.",
        );
        first.content_md.clear();
        first.content_relevant = None;
        let mut second = result(
            2,
            "https://paper.test/poll",
            "Alice tem 39% e Bruno 33%",
            "Alice tem 39% e Bruno 33%.",
        );
        second.content_md.clear();
        second.content_relevant = None;
        let answer = construct_answer(
            "pesquisa eleitoral presidente hoje",
            QueryIntent::CurrentEvents,
            &[first, second],
        );
        assert_eq!(answer.status, "insufficient_evidence");
        assert!(answer.claims.is_empty());
    }

    #[test]
    fn current_answer_does_not_merge_equal_percentages_for_different_people() {
        let results = vec![
            result(
                1,
                "https://news.test/poll",
                "Pesquisa presidencial",
                "Augusto Cury tem 2% e Samara Martins registra 1%.",
            ),
            result(
                2,
                "https://paper.test/poll",
                "Pesquisa presidencial",
                "Renan Santos tem 2% e Romeu Zema registra 1%.",
            ),
        ];
        let answer = construct_answer(
            "pesquisa eleitoral presidente hoje",
            QueryIntent::CurrentEvents,
            &results,
        );
        assert_eq!(answer.status, "insufficient_evidence");
    }

    #[test]
    fn party_labels_do_not_create_false_candidate_identity_overlap() {
        let results = vec![
            result(
                1,
                "https://news.test/poll",
                "Pesquisa presidencial",
                "Romeu Zema (Novo) tem 3% e Pablo Marçal (PRTB) registra 2%.",
            ),
            result(
                2,
                "https://paper.test/poll",
                "Pesquisa presidencial",
                "Romeu Zema (Novo) tem 3% e Augusto Cury (Avante) registra 2%.",
            ),
        ];
        let answer = construct_answer(
            "pesquisa eleitoral presidente hoje",
            QueryIntent::CurrentEvents,
            &results,
        );
        assert_eq!(answer.status, "insufficient_evidence");
        assert!(!ordered_poll_candidate_keys(&results[0].content_md).contains(&"novo".to_string()));
    }

    #[test]
    fn current_answer_rejects_a_repeated_pair_from_glued_dom_blocks() {
        let clean = result(
            1,
            "https://news.test/poll",
            "Pesquisa presidencial",
            "Lula aparece com 39% e Flávio Bolsonaro registra 33%.",
        );
        let glued = result(
            2,
            "https://paper.test/poll",
            "Pesquisa presidencial",
            "Lula aparece com 39% e Flávio Bolsonaro registra 33% Lula aparece com 39% e Flávio Bolsonaro registra 33%.",
        );
        let another_glued = result(
            3,
            "https://broadcast.test/poll",
            "Pesquisa presidencial",
            "Lula aparece com 39% e Flávio Bolsonaro registra 33% Lula aparece com 39% e Flávio Bolsonaro registra 33%.",
        );
        assert!(has_repeated_leading_percent_pair(&glued.content_md));
        let answer = construct_answer(
            "pesquisa eleitoral presidente hoje",
            QueryIntent::CurrentEvents,
            &[clean, glued, another_glued],
        );
        assert_eq!(answer.status, "extractive");
        assert_eq!(answer.claims.len(), 1);
        assert_eq!(answer.summary.matches("39%").count(), 1);
    }

    #[test]
    fn current_answer_does_not_join_indecision_to_a_second_round_candidate() {
        let first = result(
            1,
            "https://news.test/poll",
            "Datafolha",
            "Indecisos: 4% ## Segundo turno: Lula aparece com 47% e Flávio Bolsonaro tem 43%.",
        );
        let second = result(
            2,
            "https://paper.test/poll",
            "Datafolha",
            "Indecisos: 4% Segundo turno: Lula aparece com 47% e Flávio Bolsonaro tem 43%.",
        );
        assert!(has_non_candidate_leading_percentage(
            "Indecisos: 4% Segundo turno: Lula aparece com 47%"
        ));
        let answer = construct_answer(
            "pesquisa eleitoral presidente hoje",
            QueryIntent::CurrentEvents,
            &[first, second],
        );
        assert_eq!(answer.status, "extractive");
        assert_eq!(answer.claims.len(), 1);
        assert!(answer.summary.contains("47%"));
        assert!(!answer.summary.contains("4%"));
    }

    #[test]
    fn current_answer_joins_registry_without_counting_it_as_another_measurement() {
        let first = result(
            1,
            "https://news.test/poll",
            "Datafolha",
            "No segundo turno, Lula tem 47% e Flávio Bolsonaro 43%. Registro TSE BR-04496/2026.",
        );
        let second = result(
            2,
            "https://paper.test/poll",
            "Datafolha",
            "No segundo turno, Lula aparece com 47% e Flávio Bolsonaro tem 43%. A pesquisa está registrada sob BR-04496/2026.",
        );
        let answer = construct_answer_with_registry(
            "pesquisa eleitoral presidente hoje",
            QueryIntent::CurrentEvents,
            &[first, second],
            &[registry_record()],
        );
        assert_eq!(answer.status, "extractive");
        assert_eq!(answer.claims[0].citations.len(), 3);
        assert_eq!(
            answer.claims[0].citations[0].evidence_role,
            "registry_metadata"
        );
        let lineage = answer.claims[0].evidence_lineage.as_ref().unwrap();
        assert_eq!(lineage.identity_status, "official_registry_joined");
        assert_eq!(lineage.underlying_unit_count, Some(1));
        assert_eq!(lineage.reporting_source_families, 2);
        assert_eq!(lineage.registry_metadata_records, 1);
        assert!(
            lineage
                .registry_does_not_verify
                .contains(&"reported_percentages".to_string())
        );
        assert!(answer.warning.unwrap().contains("uma única medição"));
    }

    #[test]
    fn current_answer_preserves_verified_inferred_poll_lineage() {
        let seed = result(
            1,
            "https://seed.test/poll",
            "Datafolha segundo turno",
            "No segundo turno, Lula tem 47% e Flávio Bolsonaro 43%. Registro TSE BR-04496/2026.",
        );
        let inferred_a = result(
            2,
            "https://news.test/poll",
            "Datafolha: Lula 47% e Flávio 43%",
            "No segundo turno, Lula tem 47% e Flávio Bolsonaro 43%.",
        );
        let inferred_b = result(
            3,
            "https://paper.test/poll",
            "Pesquisa Datafolha de segundo turno",
            "No segundo turno, Lula aparece com 47% e Flávio Bolsonaro tem 43%.",
        );
        let registry = registry_record();
        let mut decided = vec![seed, inferred_a, inferred_b];
        crate::evidence::apply_decision_policy_with_registry(
            "pesquisa eleitoral presidente brasil hoje",
            &mut decided,
            std::slice::from_ref(&registry),
        );
        let inferred_only = decided
            .into_iter()
            .filter(|item| !item.url.contains("seed.test"))
            .collect::<Vec<_>>();
        assert_eq!(inferred_only.len(), 2);
        assert!(inferred_only.iter().all(|item| {
            item.decision
                .as_ref()
                .and_then(|decision| decision.evidence_lineage.as_ref())
                .is_some_and(|lineage| {
                    lineage.unit_ref == "BR-04496/2026" && lineage.registry_verified
                })
        }));

        let answer = construct_answer_with_registry(
            "pesquisa eleitoral presidente brasil hoje",
            QueryIntent::CurrentEvents,
            &inferred_only,
            &[registry],
        );
        assert_eq!(answer.status, "extractive");
        let lineage = answer.claims[0].evidence_lineage.as_ref().unwrap();
        assert_eq!(lineage.identity_status, "official_registry_joined");
        assert_eq!(lineage.underlying_unit_ids, vec!["BR-04496/2026"]);
        assert_eq!(lineage.underlying_unit_count, Some(1));
    }

    #[test]
    fn current_answer_emits_only_one_identity_for_the_same_numeric_signature() {
        let results = vec![
            result(
                1,
                "https://a.test/poll",
                "Pesquisa",
                "Alice tem 39% e Bruno 33%.",
            ),
            result(
                2,
                "https://b.test/poll",
                "Pesquisa",
                "Alice tem 39% e Bruno 33%.",
            ),
            result(
                3,
                "https://c.test/poll",
                "Pesquisa",
                "Carla tem 39% e Diego 33%.",
            ),
            result(
                4,
                "https://d.test/poll",
                "Pesquisa",
                "Carla tem 39% e Diego 33%.",
            ),
        ];
        let answer = construct_answer(
            "pesquisa eleitoral presidente hoje",
            QueryIntent::CurrentEvents,
            &results,
        );
        assert_eq!(answer.status, "extractive");
        assert_eq!(answer.claims.len(), 1);
    }

    #[test]
    fn current_answer_extracts_percentages_inside_long_distilled_blocks() {
        let advertisement = "Confira ofertas e produtos com até 71% de desconto hoje. ".repeat(12);
        let results = vec![
            result(
                1,
                "https://gazette.test/poll",
                "Nova pesquisa de intenção de voto",
                &format!(
                    "{advertisement} Cenário de primeiro turno. No cenário estimulado, Alice aparece com 39% das intenções de voto, seguida por Bruno, com 33%."
                ),
            ),
            result(
                2,
                "https://paper.test/poll",
                "Pesquisa traz Alice com 39% e Bruno com 33%",
                "Alice tem 39% e Bruno registra 33% no primeiro turno.",
            ),
        ];
        let answer = construct_answer(
            "pesquisa eleitoral presidente hoje",
            QueryIntent::CurrentEvents,
            &results,
        );
        assert_eq!(answer.status, "extractive");
        assert_eq!(answer.claims[0].citations.len(), 2);
    }

    #[test]
    fn current_answer_keeps_two_corroborated_scenarios_and_prefers_first_round() {
        let results = vec![
            result(
                1,
                "https://gazette.test/first-round",
                "Pesquisa de primeiro turno",
                "No primeiro turno, Alice tem 39% das intenções de voto e Bruno tem 33%.",
            ),
            result(
                2,
                "https://paper.test/first-round",
                "Intenção de voto no primeiro turno",
                "A pesquisa de primeiro turno registra Alice com 39% e Bruno com 33%.",
            ),
            result(
                3,
                "https://journal.test/runoff",
                "Simulação de segundo turno",
                "No segundo turno, Alice aparece com 47% e Bruno com 43%.",
            ),
            result(
                4,
                "https://radio.test/runoff",
                "Segundo turno entre Alice e Bruno",
                "A simulação de segundo turno aponta Alice com 47% e Bruno com 43%.",
            ),
        ];
        let answer = construct_answer(
            "pesquisa eleitoral presidente hoje",
            QueryIntent::CurrentEvents,
            &results,
        );
        assert_eq!(answer.status, "extractive");
        assert_eq!(answer.claims.len(), 2);
        assert!(answer.claims[0].text.contains("39%"));
        assert!(answer.claims[1].text.contains("47%"));
        assert_eq!(answer.claims[0].citations.len(), 2);
        assert_eq!(answer.claims[1].citations.len(), 2);
    }

    #[test]
    fn current_answer_preserves_heading_context_for_two_scenarios_in_one_poll() {
        let results = vec![
            result(
                1,
                "https://gazette.test/datafolha-main",
                "Datafolha: números da disputa presidencial",
                "## No cenário estimulado\nO presidente Lula aparece com 39% das intenções de voto e Flávio Bolsonaro tem 33%.",
            ),
            result(
                2,
                "https://paper.test/datafolha-main",
                "Datafolha divulga cenário estimulado",
                "## Cenário estimulado\nLula registra 39% e Flávio Bolsonaro marca 33%.",
            ),
            result(
                3,
                "https://gazette.test/datafolha-marcal",
                "Datafolha também testa Pablo Marçal",
                "## Pesquisa traz cenário com Pablo Marçal\nNo primeiro turno, Lula tem 39% das intenções de voto e Flávio Bolsonaro registra 32%.",
            ),
            result(
                4,
                "https://radio.test/datafolha-marcal",
                "Cenário com Pablo Marçal no Datafolha",
                "## Cenário com Pablo Marçal\nNo primeiro turno, Lula aparece com 39% e Flávio Bolsonaro tem 32%.",
            ),
        ];
        let answer = construct_answer(
            "pesquisa eleitoral presidente hoje",
            QueryIntent::CurrentEvents,
            &results,
        );
        assert_eq!(answer.status, "extractive");
        assert_eq!(answer.claims.len(), 2);
        let main = answer
            .claims
            .iter()
            .find(|claim| claim.text.contains("33%"))
            .expect("main scenario");
        let marcal = answer
            .claims
            .iter()
            .find(|claim| claim.text.contains("32%"))
            .expect("Pablo Marçal scenario");
        assert!(
            main.text.to_lowercase().contains("cenario")
                || main.text.to_lowercase().contains("cenário")
        );
        assert!(
            marcal.text.to_lowercase().contains("pablo marçal"),
            "second scenario lost its slate label: {}",
            marcal.text
        );
    }

    #[test]
    fn current_answer_labels_a_second_round_claim_from_nearby_heading() {
        let results = vec![
            result(
                1,
                "https://news.test/datafolha",
                "Datafolha: pesquisa presidencial",
                "## Segundo turno\nLula tem 47% e Flávio Bolsonaro tem 43%.",
            ),
            result(
                2,
                "https://reference.test/datafolha",
                "Datafolha: números para presidente",
                "## Segundo turno\nLula aparece com 47% e Flávio Bolsonaro registra 43%.",
            ),
        ];
        let answer = construct_answer(
            "pesquisa eleitoral presidente brasil hoje",
            QueryIntent::CurrentEvents,
            &results,
        );
        assert_eq!(answer.status, "extractive");
        assert!(
            answer.claims[0]
                .text
                .to_lowercase()
                .contains("segundo turno")
        );
        assert!(poll_answer_needs_scenario_expansion(
            "pesquisa eleitoral presidente brasil hoje",
            &answer
        ));
        assert!(
            answer
                .warning
                .as_deref()
                .is_some_and(|warning| { warning.contains("Cobertura parcial de cenários") })
        );
        let queries = poll_scenario_completion_queries(
            "pesquisa eleitoral presidente brasil hoje",
            &results,
            &answer,
        );
        assert!(queries.iter().any(|query| query.contains("primeiro turno")));
        assert!(!poll_answer_needs_scenario_expansion(
            "pesquisa eleitoral presidente segundo turno",
            &answer
        ));
    }

    #[test]
    fn cold_broad_brazilian_poll_expands_before_a_claim_is_corroborated() {
        let query = "pesquisa eleitoral presidente brasil hoje";
        let answer = SearchAnswer {
            status: "insufficient_evidence".to_string(),
            intent: "current_events".to_string(),
            summary: "Ainda sem duas famílias editoriais.".to_string(),
            claims: Vec::new(),
            uncertainty_mode: "evidential_only".to_string(),
            average_ignorance: Some(0.5),
            warning: None,
        };

        assert!(poll_answer_needs_scenario_expansion(query, &answer));
        let queries = poll_scenario_completion_queries(query, &[], &answer);
        assert!(queries.iter().any(|query| query.contains("primeiro turno")));
        assert!(queries.iter().any(|query| query.contains("segundo turno")));

        assert!(!poll_answer_needs_scenario_expansion(
            "pesquisa eleitoral presidente brasil segundo turno",
            &answer
        ));
    }

    #[test]
    fn complete_poll_slate_does_not_trigger_scenario_expansion() {
        let results = vec![
            result(
                1,
                "https://gazette.test/first-round",
                "Pesquisa de primeiro turno",
                "No primeiro turno, Alice tem 39% e Bruno tem 33%.",
            ),
            result(
                2,
                "https://paper.test/first-round",
                "Intenção de voto no primeiro turno",
                "No primeiro turno, Alice registra 39% e Bruno registra 33%.",
            ),
            result(
                3,
                "https://journal.test/runoff",
                "Simulação de segundo turno",
                "No segundo turno, Alice aparece com 47% e Bruno com 43%.",
            ),
            result(
                4,
                "https://radio.test/runoff",
                "Segundo turno entre Alice e Bruno",
                "No segundo turno, Alice registra 47% e Bruno marca 43%.",
            ),
        ];
        let query = "pesquisa eleitoral presidente hoje";
        let answer = construct_answer(query, QueryIntent::CurrentEvents, &results);
        assert_eq!(answer.claims.len(), 2);
        assert!(!poll_answer_needs_scenario_expansion(query, &answer));
        assert!(poll_scenario_completion_queries(query, &results, &answer).is_empty());
    }

    #[test]
    fn broad_poll_audits_an_alternate_slate_before_presenting_it_as_topline() {
        let results = vec![
            result(
                1,
                "https://news.test/datafolha",
                "Datafolha: Alice 39% e Bruno 33% no primeiro turno",
                "## Primeiro turno\nAlice tem 39% e Bruno tem 33%.\n\
                 ## Cenário com Carla\nAlice tem 39% e Bruno tem 32%.\n\
                 ## Segundo turno\nAlice tem 47% e Bruno tem 43%.",
            ),
            result(
                2,
                "https://paper.test/datafolha",
                "Datafolha: Alice lidera com 39% contra 32% de Bruno",
                "## Cenário com Carla\nAlice registra 39% e Bruno marca 32%.\n\
                 ## Segundo turno\nAlice registra 47% e Bruno marca 43%.",
            ),
        ];
        let query = "pesquisa eleitoral presidente brasil hoje";
        let answer = construct_answer(query, QueryIntent::CurrentEvents, &results);
        assert!(answer.claims.iter().any(|claim| {
            percent_signature(&claim.text) == vec!["39".to_string(), "32".to_string()]
        }));
        let queries = poll_primary_scenario_corroboration_queries(query, &results, &answer);
        assert!(!queries.is_empty());
        assert!(
            queries
                .iter()
                .any(|query| query.contains("\"39%\"") && query.contains("\"33%\""))
        );
        assert!(queries.iter().all(|query| !query.contains("\"32%\"")));
    }

    #[test]
    fn broad_poll_does_not_reaudit_an_already_corroborated_primary_slate() {
        let results = vec![
            result(
                1,
                "https://news.test/datafolha",
                "Datafolha: Alice 39% e Bruno 33% no primeiro turno",
                "## Primeiro turno\nAlice tem 39% e Bruno tem 33%.\n\
                 ## Cenário com Carla\nAlice tem 39% e Bruno tem 32%.\n\
                 ## Segundo turno\nAlice tem 47% e Bruno tem 43%.",
            ),
            result(
                2,
                "https://paper.test/datafolha",
                "Datafolha: Alice lidera com 39% contra 33% de Bruno",
                "## Primeiro turno\nAlice registra 39% e Bruno marca 33%.\n\
                 ## Segundo turno\nAlice registra 47% e Bruno marca 43%.",
            ),
        ];
        let query = "pesquisa eleitoral presidente brasil hoje";
        let answer = construct_answer(query, QueryIntent::CurrentEvents, &results);
        assert!(answer.claims.iter().any(|claim| {
            percent_signature(&claim.text) == vec!["39".to_string(), "33".to_string()]
        }));
        assert!(poll_primary_scenario_corroboration_queries(query, &results, &answer).is_empty());
    }

    #[test]
    fn poll_without_round_semantics_does_not_assume_a_runoff_model() {
        let query = "presidential election poll today";
        let answer = SearchAnswer {
            status: "extractive".to_string(),
            intent: "current_events".to_string(),
            summary: "Alice tem 39% e Bruno tem 33%.".to_string(),
            claims: vec![AnswerClaim {
                text: "Alice tem 39% e Bruno tem 33%.".to_string(),
                citations: Vec::new(),
                evidence_lineage: None,
            }],
            uncertainty_mode: "evidential_only".to_string(),
            average_ignorance: Some(0.5),
            warning: None,
        };
        assert!(!poll_answer_needs_scenario_expansion(query, &answer));
    }

    #[test]
    fn broad_brazilian_presidential_poll_requires_real_round_coverage() {
        let query = "resultado pesquisa eleitoral presidente brasil hoje";
        let answer = SearchAnswer {
            status: "extractive".to_string(),
            intent: "current_events".to_string(),
            summary: "cenário sem Carla: Alice tem 39% e Bruno tem 33%.".to_string(),
            claims: vec![AnswerClaim {
                text: "cenário sem Carla: Alice tem 39% e Bruno tem 33%.".to_string(),
                citations: Vec::new(),
                evidence_lineage: None,
            }],
            uncertainty_mode: "evidential_only".to_string(),
            average_ignorance: Some(0.5),
            warning: None,
        };
        assert!(poll_answer_needs_scenario_expansion(query, &answer));
        let queries = poll_scenario_completion_queries(query, &[], &answer);
        assert!(queries.iter().any(|query| query.contains("segundo turno")));
        assert!(!queries.iter().any(|query| query.contains("primeiro turno")));
    }

    #[test]
    fn current_answer_does_not_confuse_voter_certainty_with_vote_intention() {
        let results = vec![
            result(
                1,
                "https://news.test/certainty",
                "Datafolha mede decisão do eleitor",
                "O Datafolha diz que 72% estão decididos e 27% podem mudar o voto.",
            ),
            result(
                2,
                "https://paper.test/certainty",
                "Eleitor ainda pode mudar",
                "Segundo o Datafolha, 72% estão decididos e 27% ainda podem mudar.",
            ),
        ];
        let answer = construct_answer(
            "pesquisa eleitoral presidente hoje",
            QueryIntent::CurrentEvents,
            &results,
        );
        assert_eq!(answer.status, "insufficient_evidence");
    }

    #[test]
    fn current_answer_does_not_count_sister_domains_as_independent_sources() {
        let results = vec![
            result(
                1,
                "https://g1.globo.com/politica/poll.html",
                "Primeiro turno: Alice tem 39% e Bruno 33%",
                "No primeiro turno, Alice tem 39% e Bruno tem 33%.",
            ),
            result(
                2,
                "https://oglobo.globo.com/politica/poll.html",
                "Pesquisa aponta Alice com 39% e Bruno com 33%",
                "No primeiro turno, Alice registra 39% e Bruno registra 33%.",
            ),
        ];
        let answer = construct_answer(
            "pesquisa eleitoral presidente hoje",
            QueryIntent::CurrentEvents,
            &results,
        );
        assert_eq!(answer.status, "insufficient_evidence");
    }

    #[test]
    fn current_answer_does_not_treat_a_topic_hub_as_corroboration() {
        let results = vec![
            result(
                1,
                "https://paper.test/politica/poll.html",
                "Primeiro turno: Alice tem 39% e Bruno 33%",
                "No primeiro turno, Alice tem 39% e Bruno tem 33%.",
            ),
            result(
                2,
                "https://news.test/tudo-sobre/pesquisas-eleitorais/",
                "Pesquisas eleitorais",
                "Hoje: Alice tem 39% e Bruno tem 33% no primeiro turno.",
            ),
        ];
        let answer = construct_answer(
            "pesquisa eleitoral presidente hoje",
            QueryIntent::CurrentEvents,
            &results,
        );
        assert_eq!(answer.status, "insufficient_evidence");
    }

    #[test]
    fn current_answer_does_not_mix_a_fresh_article_with_an_old_poll_claim() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let mut fresh = result(
            1,
            "https://paper.test/politica/poll.html",
            "Primeiro turno: Alice tem 39% e Bruno 33%",
            "No primeiro turno, Alice tem 39% e Bruno tem 33%.",
        );
        fresh.published_at = Some(now);
        let mut old = result(
            2,
            "https://archive.test/politica/poll.html",
            "Rodada passada: Alice tinha 39% e Bruno 33%",
            "Na mesma rodada, Alice aparecia com 39% e Bruno com 33%.",
        );
        old.published_at = Some(now - 7 * 86_400);

        let answer = construct_answer(
            "pesquisa eleitoral presidente hoje",
            QueryIntent::CurrentEvents,
            &[fresh, old],
        );
        assert_eq!(answer.status, "insufficient_evidence");
    }

    #[test]
    fn poll_corroboration_query_uses_vote_pair_not_meta_percentages() {
        let results = vec![result(
            1,
            "https://news.test/poll",
            "Datafolha para presidente",
            "Veja a intenção de voto no primeiro turno: Alice tem 39% e Bruno tem 32%. O instituto também diz que 72% estão decididos e 27% podem mudar.",
        )];
        assert_eq!(
            poll_corroboration_queries("pesquisa eleitoral presidente hoje", &results)
                .first()
                .cloned(),
            Some("pesquisa eleitoral presidente hoje 39% 32% alice bruno datafolha".to_string())
        );
    }

    #[test]
    fn poll_corroboration_expansion_includes_both_long_candidate_names() {
        let results = vec![result(
            1,
            "https://news.test/poll",
            "Datafolha para presidente",
            "No primeiro turno, Luiz Inácio Lula da Silva tem 39% e Flávio Bolsonaro registra 33%.",
        )];
        let queries = poll_corroboration_queries("pesquisa eleitoral presidente hoje", &results);
        assert_eq!(queries.len(), 2);
        assert!(queries.iter().all(|query| query.contains("lula")));
        assert!(queries.iter().all(|query| query.contains("flavio")));
        assert!(queries.iter().all(|query| query.contains("39%")));
        assert!(queries.iter().all(|query| query.contains("33%")));
        assert!(queries.iter().any(|query| query.contains("datafolha")));
    }

    #[test]
    fn poll_breakdown_headline_can_seed_discovery_but_not_validate_the_topline() {
        let breakdown = result(
            1,
            "https://news.test/pesquisa-por-regiao-genero-idade",
            "Pesquisa Datafolha: Alice tem 39% e Bruno 33% no 1º turno",
            "## Região\nNorte: Alice 53% e Bruno 25%.\nSul: Alice 31% e Bruno 42%.",
        );
        let query = "pesquisa eleitoral presidente brasil hoje";
        let queries = poll_corroboration_queries(query, std::slice::from_ref(&breakdown));
        assert!(
            queries
                .iter()
                .any(|query| query.contains("39%") && query.contains("33%"))
        );
        assert!(queries.iter().all(|query| !query.contains("53%")));
        assert_eq!(
            construct_answer(query, QueryIntent::CurrentEvents, &[breakdown]).status,
            "insufficient_evidence"
        );
    }

    #[test]
    fn poll_claim_does_not_prefix_a_without_candidate_slate_as_with_candidate() {
        let results = vec![
            result(
                1,
                "https://paper.test/datafolha",
                "Pesquisa Datafolha traz Alice com 39% e Bruno com 33%",
                "Alice tem 39% e Bruno 33% no primeiro turno. No segundo turno, Alice tem 47% e Bruno 43%.",
            ),
            result(
                2,
                "https://journal.test/datafolha",
                "Nova pesquisa Datafolha: Alice lidera",
                "## Cenário com Carla\nAlice tem 39% e Bruno 32%.\n## Cenário sem Carla\nAlice também lidera no cenário sem Carla: Alice 39% e Bruno 33%.\n## Segundo turno\nAlice tem 47% e Bruno 43%.",
            ),
        ];
        let answer = construct_answer(
            "pesquisa eleitoral presidente brasil hoje",
            QueryIntent::CurrentEvents,
            &results,
        );
        let first_round = answer
            .claims
            .iter()
            .find(|claim| percent_signature(&claim.text) == vec!["39", "33"])
            .expect("corroborated first-round claim");
        let folded = relevance_terms(&first_round.text).join(" ");
        assert!(!(folded.contains("cenario com") && folded.contains("cenario sem")));
    }

    #[test]
    fn poll_cleanup_keeps_only_the_nearest_named_slate_before_the_pair() {
        let cleaned = clean_poll_candidate(
            "Datafolha — cenário com Carla: Alice também lidera no cenário sem Carla - Alice 39% - Bruno 33%.",
        );
        let folded = relevance_terms(&cleaned).join(" ");
        assert!(folded.starts_with("cenario sem carla"));
        assert!(!folded.contains("cenario com carla"));
    }

    #[test]
    fn poll_corroboration_uses_declared_registration_as_a_lineage_key() {
        let results = vec![result(
            1,
            "https://news.test/poll",
            "Datafolha para presidente",
            "No primeiro turno, Alice tem 39% e Bruno registra 33%. Pesquisa registrada no TSE sob BR-04496/2026.",
        )];
        let queries = poll_corroboration_queries("pesquisa eleitoral presidente hoje", &results);
        assert!(
            queries
                .iter()
                .any(|query| query == "\"BR-04496/2026\" pesquisa eleitoral resultado")
        );
    }

    #[test]
    fn percentage_signature_ignores_sentence_punctuation() {
        assert_eq!(
            percent_signature("Alice 39% e Bruno 33%."),
            vec!["39", "33"]
        );
        assert_eq!(percent_signature("Alice 39% e Bruno 33%"), vec!["39", "33"]);
    }

    #[test]
    fn measurement_fingerprint_matches_reordered_registered_claim_not_other_numbers() {
        let first = result(
            1,
            "https://news.test/first",
            "Pesquisa Datafolha",
            "Datafolha: Lula tem 39% e Flávio Bolsonaro registra 32% no primeiro turno.",
        );
        let reordered = result(
            2,
            "https://paper.test/reordered",
            "Nova rodada Datafolha",
            "No Datafolha, Flávio Bolsonaro soma 32%, enquanto Lula alcança 39%.",
        );
        let other = result(
            3,
            "https://paper.test/other",
            "Outra pesquisa Datafolha",
            "Datafolha: Lula tem 40% e Flávio Bolsonaro registra 31%.",
        );
        let institute = vec!["datafolha".to_string()];
        let first = poll_measurement_fingerprints(&first, &institute);
        let reordered = poll_measurement_fingerprints(&reordered, &institute);
        let other = poll_measurement_fingerprints(&other, &institute);
        assert!(!first.is_empty());
        assert!(
            first
                .iter()
                .any(|left| reordered.iter().any(|right| left.compatible_with(right)))
        );
        assert!(
            !first
                .iter()
                .any(|left| other.iter().any(|right| left.compatible_with(right)))
        );
    }

    #[test]
    fn poll_identity_ignores_names_after_the_leading_percentage_pair() {
        let keys = ordered_poll_candidate_keys(
            "Lula tem 39%; Flávio Bolsonaro, 33%; Caiado, 5%; Renan, 4%",
        );
        assert!(keys.contains(&"lula".to_string()));
        assert!(keys.contains(&"flavio".to_string()));
        assert!(!keys.contains(&"caiado".to_string()));
        assert!(!keys.contains(&"renan".to_string()));
    }

    #[test]
    fn candidate_cleanup_removes_trailing_markdown_and_citations_are_bounded() {
        let mut candidates = Vec::new();
        push_candidate(&mut candidates, "Alice tem 39% e Bruno tem 33% ###   ");
        assert_eq!(candidates, vec!["Alice tem 39% e Bruno tem 33%"]);

        let long = format!("Alice tem 39% e Bruno tem 33%. {}", "contexto ".repeat(50));
        let citation = citation(
            &result(1, "https://news.test/poll", "Pesquisa", &long),
            &long,
        );
        let excerpt = citation.evidence_excerpt.expect("excerpt");
        assert!(excerpt.chars().count() <= 260);
        assert!(excerpt.ends_with('…'));
    }

    #[test]
    fn poll_cleanup_removes_a_glued_advertisement_prefix() {
        assert_eq!(
            clean_poll_candidate(
                "OFF – confira a lista Cenário de primeiro turno No cenário estimulado, Alice tem 39% e Bruno 33%."
            ),
            "No cenário estimulado, Alice tem 39% e Bruno 33%."
        );
    }

    #[test]
    fn poll_claim_does_not_emit_an_uncorroborated_third_percentage() {
        let text = "Em um cenário alternativo, Lula mantém 39%, Flávio Bolsonaro oscila para 32%, e Marçal aparece com 2%.";
        let claim = poll_claim_with_supported_pair(text);
        assert_eq!(claim.matches('%').count(), 2);
        assert!(claim.ends_with("32%."));
        assert!(!claim.contains("Marçal"));
    }

    #[test]
    fn poll_claim_removes_a_glued_related_headline_after_the_pair() {
        let claim = poll_claim_with_supported_pair(
            "Pesquisa Datafolha: Lula tem 39% e Flávio Bolsonaro 33% no 1º turno | G1 Datafolha traz 2 cenários para eleições presidenciais;",
        );
        assert_eq!(
            claim,
            "Pesquisa Datafolha: Lula tem 39% e Flávio Bolsonaro 33% no 1º turno."
        );
    }

    #[test]
    fn poll_scenario_uses_the_last_round_cue_in_a_glued_block() {
        let glued = "Veja também: cenário no 2º turno O presidente Lula tem 39% e Flávio Bolsonaro 33% no primeiro turno da eleição.";
        assert_eq!(
            poll_scenario_label(glued).as_deref(),
            Some("primeiro turno")
        );
        assert!(has_conflicting_poll_round_context(glued));
        assert_eq!(
            poll_scenario_label(
                "Em um cenário alternativo no primeiro turno, Lula tem 39% e Flávio 32%."
            )
            .as_deref(),
            Some("cenário alternativo")
        );
    }

    #[test]
    fn poll_scenario_context_prefers_the_nearest_equally_specific_round() {
        let source = "## Segundo turno\nLula tem 47% e Flávio 43%.\n\
                      ## Primeiro turno\nSegundo o levantamento, Lula tem 39% das intenções de voto, enquanto Flávio Bolsonaro aparece com 33%. Caiado marcou 5%.";
        let candidate_start = source.find("39%").expect("candidate percentage");
        assert_eq!(
            poll_scenario_before(source, candidate_start).as_deref(),
            Some("Primeiro turno")
        );
    }

    #[test]
    fn brazilian_presidential_runoff_rejects_a_low_total_first_round_pair() {
        let query = "resultado pesquisa eleitoral presidente brasil hoje";
        assert!(!poll_round_values_plausible(
            query,
            "Datafolha — segundo turno: Flávio Bolsonaro aparece com 33%. Caiado marcou 5%.",
            Some("second")
        ));
        assert!(poll_round_values_plausible(
            query,
            "segundo turno: Lula tem 47% e Flávio Bolsonaro 43%.",
            Some("second")
        ));
    }

    #[test]
    fn broad_poll_does_not_promote_demographic_breakdown_as_the_topline() {
        let breakdown = result(
            1,
            "https://paper.test/datafolha-por-regiao-genero-idade",
            "Datafolha, 1º turno: intenção de voto por região, gênero e idade",
            "Entre eleitores brancos, Lula tem 34% e Flávio 37%.",
        );
        assert!(poll_breakdown_outside_query_scope(
            "pesquisa eleitoral presidente brasil hoje",
            &breakdown
        ));
        assert!(!poll_breakdown_outside_query_scope(
            "pesquisa eleitoral presidente por região",
            &breakdown
        ));
    }

    #[test]
    fn poll_registry_context_recovers_the_institute_from_breakdown_pages() {
        let results = vec![
            result(
                1,
                "https://paper.test/datafolha-por-regiao",
                "Datafolha, 1º turno: intenção de voto por região e gênero",
                "Recortes do levantamento.",
            ),
            result(
                2,
                "https://other.test/datafolha-resultado",
                "Datafolha divulga pesquisa presidencial",
                "Resultado nacional.",
            ),
            result(
                3,
                "https://third.test/analise",
                "O que dizem as pesquisas desta semana",
                "Análise agregada.",
            ),
        ];
        assert_eq!(
            poll_registry_context_terms(&results),
            vec!["datafolha".to_string()]
        );
    }

    #[test]
    fn emission_scenario_key_ignores_institute_prefix_but_keeps_named_slates() {
        assert_eq!(
            poll_emission_scenario_key("primeiro turno: Lula tem 39% e Flávio 33%."),
            poll_emission_scenario_key("Datafolha — primeiro turno: Lula tem 39% e Flávio 32%.")
        );
        assert_ne!(
            poll_emission_scenario_key("cenário com Pablo Marçal: Lula tem 39% e Flávio 32%."),
            poll_emission_scenario_key("cenário sem Pablo Marçal: Lula tem 39% e Flávio 33%.")
        );
    }

    #[test]
    fn explicit_runoff_near_the_values_beats_an_earlier_slate_heading() {
        let claim = "cenário sem Pablo Marçal: Mais destaques da pesquisa Datafolha - 2º turno: Lula tem 47%, e Flávio Bolsonaro, 43%.";
        assert_eq!(poll_scenario_label(claim).as_deref(), Some("segundo turno"));
    }

    #[test]
    fn spontaneous_poll_wording_cannot_be_relabelled_as_a_runoff() {
        assert_eq!(
            poll_scenario_label(
                "Lula fica na frente na pesquisa espontânea: Lula tem 32% e Flávio 19%."
            )
            .as_deref(),
            Some("cenário espontâneo")
        );
        assert_eq!(
            poll_scenario_specificity("cenário espontâneo"),
            5,
            "an explicit mode must outrank nearby generic round context"
        );
    }

    #[test]
    fn poll_primary_text_penalizes_related_link_prefixes() {
        let clean_result = result(
            1,
            "https://paper.test/poll",
            "Datafolha: Alice tem 39% no primeiro turno; Bruno tem 33%",
            "Leia também: veja simulações de segundo turno Segundo o levantamento, Alice tem 39% das intenções de voto e Bruno tem 33%.",
        );
        let candidates: Vec<_> = candidate_sentences(&clean_result)
            .into_iter()
            .filter(|text| percent_signature(text) == vec!["39", "33"])
            .map(|text| ExtractiveCandidate {
                answer_keys: poll_candidate_keys(&text),
                poll_identities: Some(poll_candidate_identity_sets(&text)),
                text,
                result: &clean_result,
            })
            .collect();
        let preferred = candidates
            .iter()
            .max_by_key(|candidate| poll_primary_score(candidate))
            .unwrap();
        assert!(!preferred.text.starts_with("Leia também"));
        assert!(preferred.text.contains("primeiro turno"));
    }

    #[test]
    fn due_diligence_abstains_without_structured_verification() {
        let results = vec![result(
            1,
            "https://entity.test/",
            "Entity",
            "A entidade diz que é legítima.",
        )];
        let answer = construct_answer("entity é legítima?", QueryIntent::DueDiligence, &results);
        assert_eq!(answer.status, "insufficient_evidence");
        assert!(answer.claims.is_empty());
    }
}
