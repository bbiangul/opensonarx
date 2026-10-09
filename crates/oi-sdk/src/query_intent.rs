use oi_types::SearchResultItem;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Coarse query intent used by the decision-ranking layer.
///
/// This is deliberately independent from trust and economic signals: it only
/// answers what kind of destination or information the query appears to seek.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryIntent {
    Navigational,
    Transactional,
    Tool,
    DueDiligence,
    CurrentEvents,
    Factual,
    Exploratory,
}

impl fmt::Display for QueryIntent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Navigational => "navigational",
            Self::Transactional => "transactional",
            Self::Tool => "tool",
            Self::DueDiligence => "due_diligence",
            Self::CurrentEvents => "current_events",
            Self::Factual => "factual",
            Self::Exploratory => "exploratory",
        };
        f.write_str(value)
    }
}

/// Explainable, stake-neutral adjustment applied after semantic retrieval.
#[derive(Debug, Clone, PartialEq)]
pub struct IntentAdjustment {
    pub intent: QueryIntent,
    pub score_delta: f64,
    pub signals: Vec<&'static str>,
}

const DUE_DILIGENCE_TERMS: &[&str] = &[
    "confiavel",
    "legitimo",
    "fraude",
    "golpe",
    "seguro",
    "trust",
    "scam",
    "reclamacao",
    "reputacao",
];

const TRANSACTION_TERMS: &[&str] = &[
    "acessar",
    "aplicar",
    "assinar",
    "baixar",
    "buy",
    "comprar",
    "download",
    "entrar",
    "inscrever",
    "inscricao",
    "login",
    "preco",
    "signup",
    "subscribe",
];

const CURRENT_TERMS: &[&str] = &[
    "agora",
    "atual",
    "eleicao",
    "hoje",
    "latest",
    "news",
    "noticia",
    "pesquisa eleitoral",
    "resultado",
    "ultima",
    "ultimas",
];

const FACTUAL_STARTS: &[&str] = &[
    "como ", "onde ", "o que ", "por que ", "qual ", "quando ", "quem ", "what ", "when ",
    "where ", "who ", "why ",
];

const FACTUAL_RELATION_TERMS: &[&str] = &[
    "cofounder",
    "cofounders",
    "created",
    "creator",
    "creators",
    "criado",
    "criada",
    "criador",
    "criadora",
    "criou",
    "founded",
    "founder",
    "founders",
    "fundado",
    "fundada",
    "fundador",
    "fundadora",
    "fundadores",
    "fundou",
    "invented",
    "inventor",
    "inventors",
    "inventou",
];

const CURRENT_OFFICES: &[&str] = &[
    "chanceler",
    "governador",
    "governadora",
    "mayor",
    "ministro",
    "ministra",
    "prefeito",
    "prefeita",
    "president",
    "presidente",
    "prime minister",
    "senador",
    "senadora",
];

const NAVIGATION_TERMS: &[&str] = &[
    "homepage",
    "official",
    "oficial",
    "pagina",
    "plataforma",
    "site",
    "website",
];
const ONLINE_EXECUTION_CONTEXT: &[&str] = &["browser", "navegador", "online"];
const ONLINE_TOOL_TERMS: &[&str] = &[
    "compiler",
    "compilador",
    "editor",
    "ide",
    "interpreter",
    "interpretador",
];

/// Classify a query using deterministic multilingual lexical signals.
///
/// The classifier intentionally returns a coarse class. Evidence sufficiency
/// and trust are later decisions and must not be inferred from this label.
pub fn classify_query(query: &str) -> QueryIntent {
    let folded = fold_text(query);
    let words: Vec<&str> = folded.split_whitespace().collect();

    if contains_any(&folded, DUE_DILIGENCE_TERMS) {
        return QueryIntent::DueDiligence;
    }
    // A short head-to-head surface ("A x B", "A vs B") normally asks for
    // the current match/event, not for a canonical website. This structural
    // signal must precede the <=3-word navigation fallback.
    if is_head_to_head_event_query(query) {
        return QueryIntent::CurrentEvents;
    }
    // "Preço" is normally an action/destination signal (for example,
    // "preço do Claude" -> pricing), but a quote explicitly requested for
    // today/now is mutable state. It must use temporal discovery and must not
    // be mistaken for a purchase route.
    if is_live_value_query(query) || is_current_office_holder_query(query) {
        return QueryIntent::CurrentEvents;
    }
    // A product-comparison request is exploratory even when it contains a
    // purchase verb. "Qual laptop comprar para programar" asks for evidence
    // and trade-offs; it is not the same intent as "comprar iPhone".
    if is_recommendation_query(query) {
        return QueryIntent::Exploratory;
    }
    // High-impact guidance still uses exploratory presentation until a
    // claim-specific answer policy exists, but it must never fall through to
    // short-query navigation or bypass Stage-2 reliance assessment.
    if is_high_stakes_guidance_query(query) {
        return QueryIntent::Exploratory;
    }
    // A short estimate request asks for information, not for a canonical
    // website or action route. Explicit buy/login/pricing actions remain
    // transactional through the existing action vocabulary.
    if contains_any_word(&words, &["cost", "custo"]) {
        return QueryIntent::Exploratory;
    }
    if contains_any_word(&words, TRANSACTION_TERMS) {
        return QueryIntent::Transactional;
    }
    if is_online_tool_query(query) {
        return QueryIntent::Tool;
    }
    // A methodological/definition question is not a request for the latest
    // event merely because its subject includes "election" or "results".
    // Keep explicit temporal requests on the current-state path.
    if is_explanatory_query(&folded)
        && !contains_any_word(
            &words,
            &["agora", "hoje", "today", "now", "latest", "atual"],
        )
    {
        return QueryIntent::Exploratory;
    }
    if contains_any(&folded, CURRENT_TERMS) {
        return QueryIntent::CurrentEvents;
    }
    // The current factual answerer supports bounded origin relations. Broad
    // definitions and procedures ("o que e...", "como declarar...") must
    // remain exploratory until a claim-specific synthesis policy exists;
    // routing them through the origin extractor produced empty or misleading
    // abstentions. Relation questions stay factual even when short.
    if supports_factual_relation(&folded) {
        // The current factual extractor supports bounded entity-origin
        // relations, not arbitrary authorship of creative works. Preserve a
        // useful result slate for those queries until a work/creator schema
        // can verify the requested predicate.
        return if is_creative_work_creator_query(&folded) {
            QueryIntent::Exploratory
        } else {
            QueryIntent::Factual
        };
    }
    // Short explanatory and local-discovery queries are not destinations.
    // This must precede the conservative <=3-word navigational fallback.
    if is_explanatory_query(&folded) || is_implicit_location_query(query) {
        return QueryIntent::Exploratory;
    }
    if FACTUAL_STARTS
        .iter()
        .any(|prefix| folded.starts_with(prefix))
        || starts_yes_no_question(&folded)
        || folded.ends_with('?')
    {
        return QueryIntent::Exploratory;
    }
    if contains_any_word(&words, NAVIGATION_TERMS) || words.len() <= 3 {
        return QueryIntent::Navigational;
    }
    QueryIntent::Exploratory
}

fn starts_yes_no_question(folded_query: &str) -> bool {
    [
        "are ", "can ", "could ", "did ", "do ", "does ", "is ", "should ", "was ", "were ",
        "will ", "would ",
    ]
    .iter()
    .any(|prefix| folded_query.starts_with(prefix))
}

fn supports_factual_relation(folded_query: &str) -> bool {
    let words = folded_query.split_whitespace().collect::<Vec<_>>();
    contains_any_word(&words, FACTUAL_RELATION_TERMS)
        || (contains_any_word(&words, &["who", "quem"])
            && contains_any_word(&words, &["developed", "desenvolveu"]))
}

fn is_creative_work_creator_query(folded_query: &str) -> bool {
    let terms = folded_query.split_whitespace().collect::<Vec<_>>();
    contains_any_word(
        &terms,
        &[
            "book", "film", "game", "movie", "painting", "poem", "song", "series",
        ],
    )
}

pub(crate) fn is_live_value_query(query: &str) -> bool {
    let folded = fold_text(query);
    let words = folded.split_whitespace().collect::<Vec<_>>();
    contains_any_word(&words, &["agora", "atual", "hoje", "today", "now"])
        && (contains_any_word(
            &words,
            &["cotacao", "preco", "price", "quote", "taxa", "valor"],
        ) || folded.contains("exchange rate"))
}

pub(crate) fn is_current_office_holder_query(query: &str) -> bool {
    // "Pesquisa eleitoral atual para presidente" asks about a measurement,
    // not the identity of the incumbent. Exclude the poll topic before
    // interpreting the temporal adjective as an office-holder relation.
    if is_poll_query(query) {
        return false;
    }
    let folded = fold_text(query);
    let asks_identity = folded.starts_with("quem e ")
        || folded.starts_with("quem eh ")
        || folded.starts_with("who is ")
        || folded.starts_with("atual ")
        || folded.starts_with("current ")
        || folded.contains(" atual ")
        || folded.ends_with(" atual")
        || folded.contains(" current ")
        || folded.ends_with(" current");
    asks_identity && CURRENT_OFFICES.iter().any(|office| folded.contains(office))
}

fn is_explanatory_query(folded_query: &str) -> bool {
    if [
        "o que significa ",
        "como identificar ",
        "how to identify ",
        "what does ",
    ]
    .iter()
    .any(|prefix| folded_query.starts_with(prefix))
    {
        return true;
    }
    let words = folded_query.split_whitespace().collect::<Vec<_>>();
    contains_any_word(
        &words,
        &[
            "define",
            "defined",
            "definicao",
            "definition",
            "meaning",
            "means",
            "explicada",
            "explicado",
            "explained",
        ],
    )
}

pub(crate) fn is_head_to_head_event_query(query: &str) -> bool {
    let folded = fold_text(query);
    let words = folded.split_whitespace().collect::<Vec<_>>();
    if words.len() < 3 || words.len() > 10 {
        return false;
    }
    // "What is X vs Y" and explicit difference/comparison wording describe
    // concepts, not a live match. This guard must precede the structural
    // connector check while leaving terse team-vs-team surfaces intact.
    if FACTUAL_STARTS
        .iter()
        .any(|prefix| folded.starts_with(prefix))
        || is_explanatory_query(&folded)
        || contains_any_word(
            &words,
            &["compare", "comparison", "difference", "diferenca"],
        )
    {
        return false;
    }
    words.iter().enumerate().any(|(index, word)| {
        matches!(*word, "x" | "vs" | "versus") && index > 0 && index + 1 < words.len()
    })
}

pub(crate) fn is_reference_exploration_query(query: &str) -> bool {
    let folded = fold_text(query);
    folded.starts_with("o que e ")
        || folded.starts_with("what is ")
        || folded.starts_with("como funciona ")
        || folded.starts_with("how does ")
        || is_explanatory_query(&folded)
}

pub(crate) fn is_implicit_location_query(query: &str) -> bool {
    let terms = relevance_terms(query);
    terms.iter().any(|term| {
        matches!(
            term.as_str(),
            "proximo" | "proxima" | "proximos" | "proximas"
        )
    }) || terms.windows(2).any(|window| {
        matches!(
            window,
            [left, right]
                if (left == "perto" && right == "mim")
                    || (left == "near" && right == "me")
        )
    })
}

pub(crate) fn current_query_allows_undated_display(query: &str) -> bool {
    let words = fold_text(query)
        .split_whitespace()
        .map(str::to_string)
        .collect::<Vec<_>>();
    let terms = words.iter().map(String::as_str).collect::<Vec<_>>();
    // Version/release pages are often mutable documents, not dated articles.
    // Admit scoped discovery leads without forcing the news corpus. This
    // does not authorize a version claim: generic current-state synthesis
    // still abstains until a timestamped release-specific policy exists.
    let release_lookup = contains_any_word(&terms, &["release", "version", "versao"])
        && contains_any_word(&terms, &["latest", "stable", "estavel", "current", "atual"]);
    is_live_value_query(query) || is_current_office_holder_query(query) || release_lookup
}

/// Curated query-relative primary-source entry points.
///
/// These URLs identify where to verify a role or obtain high-stakes official
/// guidance; they never encode the requested answer. A production deployment
/// should move this seed into a versioned public authority registry keyed by
/// jurisdiction, office, and guidance topic.
pub(crate) fn official_state_source_urls(query: &str) -> Vec<String> {
    let terms = relevance_terms(query);
    if is_current_office_holder_query(query)
        && terms
            .iter()
            .any(|term| matches!(term.as_str(), "presidente" | "president"))
        && terms
            .iter()
            .any(|term| matches!(term.as_str(), "brasil" | "brazil"))
    {
        return vec![
            "https://www.gov.br/planalto/pt-br/conheca-a-presidencia/biografia-do-presidente"
                .to_string(),
            "https://www.gov.br/planalto/pt-br/acesso-a-informacao/institucional/principais-cargos-e-respectivos-ocupantes-201cquem-e-quem201d"
                .to_string(),
        ];
    }
    if is_tax_guidance_query(query) {
        return vec![
            "https://www.gov.br/receitafederal/pt-br/acesso-a-informacao/perguntas-frequentes/imposto-de-renda/dirpf"
                .to_string(),
            "https://www.gov.br/receitafederal/pt-br/assuntos/meu-imposto-de-renda/preenchimento/por-onde-comecar"
                .to_string(),
            "https://www.gov.br/receitafederal/pt-br/assuntos/meu-imposto-de-renda/preenchimento/preenchimento/"
                .to_string(),
        ];
    }
    // The seed registry below is topic-specific. A generic health query must
    // not be redirected to the dengue entrypoint merely because both belong
    // to the same broad domain: doing so can erase relevant discovery results
    // after the foreground refresh and falsely label an unrelated page as
    // query-relative authority.
    if is_dengue_guidance_query(query) {
        return vec![
            "https://www.gov.br/saude/pt-br/assuntos/saude-de-a-a-z/d/dengue".to_string(),
            "https://www.gov.br/saude/pt-br/assuntos/saude-de-a-a-z/d/dengue/faq/dengue"
                .to_string(),
        ];
    }
    Vec::new()
}

pub(crate) fn official_registry_entry_for_query(query: &str, url: &str) -> bool {
    let candidate = url.trim_end_matches('/');
    official_state_source_urls(query).iter().any(|registered| {
        let registered = registered.trim_end_matches('/');
        candidate == registered
            || candidate
                .strip_prefix(registered)
                .is_some_and(|suffix| suffix.starts_with('/'))
    })
}

/// Calculate the intent relevance adjustment for one candidate.
///
/// An exact destination match is useful for navigational and transactional
/// queries, but it is not evidence that the destination is trustworthy. For a
/// due-diligence query such as "X e legitimo?", no destination boost is given:
/// independent corroboration must be handled by the evidence layer.
pub fn intent_adjustment(query: &str, result: &SearchResultItem) -> IntentAdjustment {
    let intent = classify_query(query);
    let query_words = entity_words(query, intent);
    let entity_compact = query_words.concat();
    let host_label = host_entity_label(&result.url);
    let title_words: Vec<String> = fold_text(&result.title)
        .split_whitespace()
        .map(str::to_string)
        .collect();
    let title_exact = !query_words.is_empty() && title_words == query_words;
    let title_prefix = !query_words.is_empty()
        && title_words.len() >= query_words.len()
        && title_words[..query_words.len()] == query_words;
    let registrable_origin = is_registrable_origin(&result.url) && !is_online_tool_query(query);
    let host_exact =
        !entity_compact.is_empty() && registrable_origin && host_label == entity_compact;
    let host_typo =
        !host_exact && registrable_origin && fuzzy_entity_label_match(&entity_compact, &host_label);
    let authority_path_exact = public_authority_entity_path_match(&entity_compact, &result.url);
    let root = is_root_destination(&result.url);

    let mut score_delta = 0.0;
    let mut signals = Vec::new();

    match intent {
        QueryIntent::Navigational => {
            if host_exact {
                // An exact entity hostname is the defining signal for a
                // navigational query. It must beat semantically strong
                // compound hosts such as `learnpython.org` for `python`.
                score_delta += 1.00;
                signals.push("exact_entity_host");
            } else if host_typo {
                // A one-edit typo in a sufficiently long alphabetic entity
                // should not make an event or directory outrank the canonical
                // root (`gama found` -> `gamafund.com`). Keep this below the
                // exact-host boost and confined to destination routing.
                score_delta += 0.85;
                signals.push("fuzzy_entity_host");
            }
            if title_exact {
                score_delta += 0.10;
                signals.push("exact_entity_title");
            } else if title_prefix {
                score_delta += 0.05;
                signals.push("entity_title_prefix");
            }
            if (host_exact || host_typo) && root {
                score_delta += 0.03;
                signals.push("root_destination");
            }
            if authority_path_exact {
                score_delta += 0.90;
                signals.push("public_authority_entity_path");
            }
        }
        QueryIntent::Transactional => {
            if host_exact {
                score_delta += 0.16;
                signals.push("exact_entity_host");
            } else if host_typo {
                score_delta += 0.14;
                signals.push("fuzzy_entity_host");
            }
            if title_exact || title_prefix {
                score_delta += 0.05;
                signals.push("entity_title_match");
            }
            let direct_destination = fold_text(&format!("{} {}", result.title, result.url));
            let retained_destination = fold_text(&format!(
                "{} {}",
                result.snippet,
                result.content_relevant.as_deref().unwrap_or("")
            ));
            if suspicious_transaction_path(query, &result.url) {
                // Search snippets occasionally invent action-looking paths
                // such as `/login-Claude`. An entity/action token mash-up is
                // not a safe destination; prefer a validated first-party root
                // or an exact action route instead.
                score_delta -= 0.50;
                signals.push("suspicious_transaction_path");
            } else if transaction_destination_matches_query(query, &direct_destination) {
                if result.extraction_quality == "verified_transaction_route" {
                    // A conventional first-party action route that was
                    // fetched successfully is more useful than a root page
                    // that merely mentions the action in retained content.
                    // The route verifier is deliberately outside ranking;
                    // this marker only carries its bounded HTTP result into
                    // the explainable intent score.
                    score_delta += 0.28;
                    signals.push("verified_transaction_route");
                } else {
                    score_delta += 0.12;
                }
                signals.push("transaction_destination");
            } else if host_exact
                && root
                && transaction_destination_matches_query(query, &retained_destination)
            {
                // Many consumer applications embed authentication or signup
                // directly in their root page. A retained first-party action
                // surface is safer than chasing an unverified action-looking
                // URL from a web snippet.
                score_delta += 0.16;
                signals.push("transaction_destination");
                signals.push("embedded_action_at_root");
            }
        }
        QueryIntent::Tool => {
            if online_tool_surface_matches(query, result) {
                score_delta += 0.18;
                signals.push("functional_tool_surface");
            } else {
                score_delta -= 0.30;
                signals.push("tool_surface_mismatch");
            }
        }
        QueryIntent::CurrentEvents => {
            let topic_coverage = current_topic_coverage(query, result);
            if topic_coverage >= 0.50 {
                if let Some((delta, signal)) = freshness_adjustment(result, unix_now()) {
                    score_delta += delta;
                    signals.push(signal);
                }
            } else if topic_coverage < 0.25 {
                score_delta -= 0.12;
                signals.push("current_topic_mismatch");
            }
        }
        QueryIntent::Factual => {
            let coverage = factual_answerability_coverage(query, result);
            if coverage >= 0.999 {
                score_delta += 0.12;
                signals.push("factual_terms_answered");
            } else if root && coverage < 0.999 {
                // A company/product homepage can be highly related to the
                // entity while still being a poor answer to a factual
                // predicate (for example, "who founded X?").
                score_delta -= 0.06;
                signals.push("root_lacks_factual_predicate");
            }
        }
        QueryIntent::DueDiligence => {
            // Keep the entity disambiguated without turning its own homepage
            // into trust evidence. Independent pages that name the exact
            // entity get a small relevance boost; homonymous funds/products
            // are penalized before the evidence layer decides sufficiency.
            if !host_exact {
                let entity_phrase = query_words.join(" ");
                let evidence = fold_text(&format!(
                    "{} {} {}",
                    result.title, result.snippet, result.url
                ));
                if !entity_phrase.is_empty() && evidence.contains(&entity_phrase) {
                    score_delta += 0.08;
                    signals.push("diligence_entity_phrase");
                } else if !entity_phrase.is_empty() {
                    score_delta -= 0.24;
                    signals.push("diligence_entity_mismatch");
                }
            }
        }
        QueryIntent::Exploratory => {
            if is_recommendation_query(query) {
                // Observation only: without merchant/affiliate disclosures,
                // relevance cannot be mistaken for editorial independence.
                signals.push("commercial_independence_unverified");
                if recommendation_merchant_surface(result) {
                    // A seller may still be a useful product lead, but its
                    // own buying guide has a direct commercial dependency and
                    // should not outrank an equally relevant editorial or
                    // testing surface merely through exact-match SEO.
                    score_delta -= 0.14;
                    signals.push("merchant_first_party_surface");
                }
            }
        }
    }

    if (intent == QueryIntent::Exploratory
        || (intent == QueryIntent::CurrentEvents && is_current_office_holder_query(query)))
        && authoritative_source_for_query(query, &result.url)
    {
        // Authority is query-relative. A government tax page or public-health
        // authority deserves precedence for guidance in its own remit; a
        // generic .gov page receives no universal truth bonus.
        score_delta += 0.32;
        signals.push("query_relative_primary_authority");
        if official_registry_entry_for_query(query, &result.url) {
            // The registry resolves the intended authority entrypoint within
            // a broad public-sector surface. This is a query/topic routing
            // prior, not a generic government truth bonus.
            score_delta += 0.18;
            signals.push("registered_authority_entrypoint");
        }
    }

    IntentAdjustment {
        intent,
        score_delta,
        signals,
    }
}

fn recommendation_merchant_surface(result: &SearchResultItem) -> bool {
    let Ok(parsed) = url::Url::parse(&result.url) else {
        return false;
    };
    let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
    let labels = host.split('.').collect::<Vec<_>>();
    labels.iter().any(|label| {
        [
            "amazon",
            "bestbuy",
            "kabum",
            "magazineluiza",
            "mercadolivre",
            "retail",
            "shop",
            "store",
            "varejo",
            "walmart",
        ]
        .iter()
        .any(|merchant| *label == *merchant || label.ends_with(merchant))
    })
}

pub(crate) fn apply_intent_adjustments(query: &str, results: &mut [SearchResultItem]) {
    for result in results {
        if result.published_at.is_none() {
            result.published_at = date_from_url(&result.url);
        }
        result.final_score += intent_adjustment(query, result).score_delta;
    }
}

/// Remove candidates that do not mention the requested entity/topic on the
/// surface relevant to the query intent.
///
/// Vector search always returns a nearest neighbour when an index is nonempty;
/// that does not mean the neighbour is a relevant result. This gate makes an
/// uncovered query return no result (and therefore abstain) instead of
/// presenting the least-unrelated document in the corpus.
pub(crate) fn retain_intent_scope(query: &str, results: &mut Vec<SearchResultItem>) {
    let intent = classify_query(query);
    results.retain(|result| match intent {
        QueryIntent::Navigational => {
            navigation_entity_host_match(query, result)
                // Navigation is decided from the public destination surface,
                // not arbitrary body/footer text. Otherwise a news article
                // whose related-links footer mentions the brand can leak into
                // an otherwise clean destination slate.
                || candidate_surface_covers_terms(result, &entity_words(query, intent))
        }
        QueryIntent::Tool => online_tool_surface_matches(query, result),
        QueryIntent::Transactional => {
            let entity_terms = entity_words(query, intent);
            !suspicious_transaction_path(query, &result.url)
                && candidate_covers_terms(result, &entity_terms, 1_200)
        }
        QueryIntent::DueDiligence => {
            // An entity's own site can identify the target, but cannot serve
            // as independent evidence that the target is legitimate.
            !exact_entity_host_match(query, result)
                && diligence_surface_matches_entity(query, result)
        }
        QueryIntent::CurrentEvents => {
            let freshness = freshness_adjustment(result, unix_now());
            let within_explicit_window = !query_requires_week_freshness(query)
                || freshness.is_some_and(|(_, signal)| {
                    matches!(
                        signal,
                        "published_within_1d" | "published_within_2d" | "published_within_7d"
                    )
                });
            current_topic_coverage(query, result) >= 0.50
                && current_office_holder_page_matches(query, result)
                && if result.source == "local" {
                    (freshness.is_some() && within_explicit_window)
                        || (current_query_allows_undated_display(query)
                            && !result.content_md.trim().is_empty())
                } else {
                    // External metadata is a discovery lead, not retained
                    // evidence. Allow an undated provider result to consume a
                    // bounded crawl slot; the answered-serving gate below
                    // requires an extracted/effective date before surfacing.
                    freshness.is_none() || within_explicit_window
                }
        }
        QueryIntent::Factual => {
            let entity_terms: Vec<_> = factual_query_terms(query)
                .into_iter()
                .filter(|term| !matches!(term.as_str(), "created" | "founded" | "invented"))
                .collect();
            // Provider snippets are discovery leads, not evidence. Factual
            // reference search often returns a person-profile title (for
            // example "Dario Amodei") whose snippet carries the requested
            // entity relation. Let it consume a bounded crawl slot; the final
            // answered-serving and relation-direction gates still require
            // retained page text before any claim can be emitted.
            let surface_terms = relevance_terms(&format!(
                "{} {} {}",
                result.title, result.snippet, result.url
            ));
            !entity_terms.is_empty() && entity_terms.iter().all(|term| surface_terms.contains(term))
        }
        QueryIntent::Exploratory => exploratory_topic_coverage(query, result) >= 0.50,
    });
}

/// Final answered-serving gate applied after synchronous hydration.
///
/// Discovery may admit an undated current-event lead or a factual page whose
/// title names the entity but whose answer predicate is only visible after a
/// fetch. Once an answer is served, current events need an effective date and
/// factual results need to expose both the entity and requested relation.
pub(crate) fn retain_answered_serving_scope(query: &str, results: &mut Vec<SearchResultItem>) {
    match classify_query(query) {
        QueryIntent::CurrentEvents => {
            let now = unix_now();
            let permits_undated_state = current_query_allows_undated_display(query);
            results.retain(|result| {
                let freshness = freshness_adjustment(result, now);
                let office_evidence_ready = !is_current_office_holder_query(query)
                    || (result.source == "local"
                        && (authoritative_source_for_query(query, &result.url)
                            || crate::answer::result_has_current_office_holder_candidate(
                                query, result,
                            )));
                current_topic_coverage(query, result) >= 0.50
                    && current_office_holder_page_matches(query, result)
                    && office_evidence_ready
                    && (freshness.is_some()
                        || (permits_undated_state
                            && result.source == "local"
                            && !result.content_md.trim().is_empty()))
                    && (!query_requires_week_freshness(query)
                        || permits_undated_state
                        || freshness.is_some_and(|(_, signal)| {
                            matches!(
                                signal,
                                "published_within_1d"
                                    | "published_within_2d"
                                    | "published_within_7d"
                            )
                        }))
            });
        }
        QueryIntent::Factual => {
            results.retain(|result| crate::answer::best_factual_sentence(query, result).is_some());
        }
        QueryIntent::Exploratory if is_recommendation_query(query) => {
            results.retain(|result| exploratory_topic_coverage(query, result) >= 0.50);
        }
        _ => {}
    }
}

fn query_requires_week_freshness(query: &str) -> bool {
    let folded = fold_text(query);
    let words: Vec<_> = folded.split_whitespace().collect();
    ["atual", "hoje", "latest", "today", "ultima", "ultimas"]
        .iter()
        .any(|cue| words.contains(cue))
}

fn diligence_surface_matches_entity(query: &str, result: &SearchResultItem) -> bool {
    let entity_terms = entity_words(query, QueryIntent::DueDiligence);
    if entity_terms.is_empty() {
        return false;
    }
    let surface_terms = relevance_terms(&format!(
        "{} {} {}",
        result.title, result.snippet, result.url
    ));
    surface_terms
        .windows(entity_terms.len())
        .any(|window| window == entity_terms)
}

fn candidate_surface_covers_terms(result: &SearchResultItem, required_terms: &[String]) -> bool {
    if required_terms.is_empty() {
        return false;
    }
    let candidate_terms = relevance_terms(&format!("{} {}", result.title, result.url));
    let surface_contains = |required: &str| {
        candidate_terms.iter().any(|candidate| {
            candidate == required
                || (required.len() >= 4 && candidate.strip_suffix('s') == Some(required))
                || (candidate.len() >= 4 && required.strip_suffix('s') == Some(candidate.as_str()))
        })
    };
    if required_terms.iter().all(|term| surface_contains(term)) {
        return true;
    }
    // A branded product query often contains a category synonym absent from
    // the canonical title ("protein shake" vs "nutritional shake"). Keep it
    // as an ambiguous result—not a verified destination—only when the first,
    // distinctive entity token is present in the registrable host and at
    // least half of the full query is visible in title/URL.
    let Some(entity) = required_terms.first() else {
        return false;
    };
    let host = host_entity_label(&result.url);
    let matches = required_terms
        .iter()
        .filter(|term| surface_contains(term))
        .count();
    entity.len() >= 3 && host.contains(entity) && matches >= required_terms.len().div_ceil(2)
}

fn candidate_covers_terms(
    result: &SearchResultItem,
    required_terms: &[String],
    body_limit: usize,
) -> bool {
    if required_terms.is_empty() {
        return false;
    }
    let body_prefix: String = result.content_md.chars().take(body_limit).collect();
    let candidate_terms = relevance_terms(&format!(
        "{} {} {} {} {}",
        result.title,
        result.snippet,
        result.url,
        result.content_relevant.as_deref().unwrap_or(""),
        body_prefix,
    ));
    required_terms
        .iter()
        .all(|term| candidate_terms.contains(term))
}

fn exploratory_topic_coverage(query: &str, result: &SearchResultItem) -> f64 {
    const QUERY_SCAFFOLD: &[&str] = &[
        "are",
        "best",
        "can",
        "como",
        "could",
        "did",
        "do",
        "does",
        "funciona",
        "funcionam",
        "how",
        "ir",
        "is",
        "keep",
        "like",
        "look",
        "melhor",
        "melhores",
        "mim",
        "near",
        "of",
        "on",
        "onde",
        "perto",
        "qual",
        "quais",
        "quando",
        "que",
        "should",
        "was",
        "were",
        "what",
        "where",
        "which",
        "who",
        "why",
        "will",
        "with",
        "would",
    ];
    let query_terms = relevance_terms(query)
        .into_iter()
        .filter(|term| !QUERY_SCAFFOLD.contains(&term.as_str()))
        .collect::<Vec<_>>();
    if query_terms.is_empty() {
        return 0.0;
    }
    let candidate_terms =
        if result.source == "local" && official_registry_entry_for_query(query, &result.url) {
            // A versioned, query-relative authority entry was selected before
            // crawl. Its own body is safe for topic coverage even when the title
            // is generic (for example "Dengue — Ministério da Saúde"). This
            // exception is unavailable to ordinary local pages, whose footers
            // could otherwise leak unrelated topics into a mixed index.
            let body_prefix: String = result.content_md.chars().take(4_000).collect();
            relevance_terms(&format!(
                "{} {} {} {}",
                result.title,
                result.url,
                result.content_relevant.as_deref().unwrap_or(""),
                body_prefix,
            ))
        } else if result.source == "local" {
            relevance_terms(&format!("{} {}", result.title, result.url))
        } else {
            relevance_terms(&format!(
                "{} {} {}",
                result.title, result.snippet, result.url
            ))
        };
    // A generic "Quantum" encyclopedia page is not an answer to "quantum
    // computing". Preserve the compound concept across languages and reject
    // candidates that expose only one half of it.
    let asks_quantum_computing = query_terms
        .iter()
        .any(|term| matches!(term.as_str(), "quantum" | "quantica" | "quantico"))
        && query_terms
            .iter()
            .any(|term| matches!(term.as_str(), "computacao" | "computing"));
    let covers_quantum_computing = candidate_terms
        .iter()
        .any(|term| matches!(term.as_str(), "quantum" | "quantica" | "quantico"))
        && candidate_terms
            .iter()
            .any(|term| matches!(term.as_str(), "computacao" | "computing"));
    if asks_quantum_computing && !covers_quantum_computing {
        return 0.0;
    }
    // Product-purpose recommendations are compound too. In a mixed resident
    // index, a programming-language page matches half of "laptop for
    // programming" and previously crossed the generic 0.50 threshold. It is
    // not a valid product result unless its public surface also names the
    // requested portable-computer category.
    let asks_portable_computer = query_terms.iter().any(|term| {
        matches!(
            term.as_str(),
            "laptop" | "laptops" | "notebook" | "notebooks"
        )
    });
    let covers_portable_computer = candidate_terms.iter().any(|term| {
        matches!(
            term.as_str(),
            "laptop" | "laptops" | "notebook" | "notebooks"
        )
    });
    if asks_portable_computer && !covers_portable_computer {
        return 0.0;
    }
    let matched_terms = query_terms
        .iter()
        .filter(|term| {
            candidate_terms.contains(term)
                || candidate_terms
                    .iter()
                    .any(|candidate| exploratory_term_equivalent(term, candidate))
        })
        .collect::<Vec<_>>();
    // A shared year alone is not a topic match. This matters in a mixed
    // resident index where, for example, an unrelated tax article and a
    // restaurant query can both mention 2026.
    let has_lexical_topic = matched_terms
        .iter()
        .any(|term| term.chars().any(|character| character.is_alphabetic()));
    if !has_lexical_topic {
        return 0.0;
    }
    let lexical_coverage = matched_terms.len() as f64 / query_terms.len() as f64;
    // Long natural-language questions routinely paraphrase the relation while
    // preserving two concrete subject terms. Requiring half of every token
    // rewards keyword repetition over answer relevance. Compound-product
    // guards above remain mandatory, and evidence policy still decides
    // whether an admitted lead can support an answer.
    if matched_terms.len() >= 2 {
        lexical_coverage.max(0.50)
    } else {
        lexical_coverage
    }
}

fn exploratory_term_equivalent(left: &str, right: &str) -> bool {
    let portable_computer =
        |term: &str| matches!(term, "laptop" | "laptops" | "notebook" | "notebooks");
    let software_development = |term: &str| {
        matches!(
            term,
            "coding"
                | "desenvolvedor"
                | "desenvolvedores"
                | "desenvolvimento"
                | "programacao"
                | "programar"
                | "programming"
                | "software"
        )
    };
    (portable_computer(left) && portable_computer(right))
        || (software_development(left) && software_development(right))
        || matches!(
            (left, right),
            ("computacao", "computing")
                | ("computing", "computacao")
                | ("quantica", "quantum")
                | ("quantico", "quantum")
                | ("quantum", "quantica")
                | ("quantum", "quantico")
        )
}

/// Retrieval rewrite for poll-result queries. Announcement pages often win a
/// literal "poll today" SERP before the result article is indexed; adding
/// result/numeric terms asks the external engine for answer-bearing pages.
/// The original query is still used for embeddings and ranking.
pub(crate) fn external_discovery_queries(query: &str) -> Vec<String> {
    let folded = fold_text(query);
    let intent = classify_query(query);
    let poll_query = is_poll_query(query);
    let already_result_focused = ["numero", "percent", "resultado"]
        .iter()
        .any(|term| folded.contains(term));
    if intent == QueryIntent::Tool {
        canonical_online_tool_query(query)
            .map(|canonical| vec![canonical])
            .unwrap_or_else(|| vec![query.to_string()])
    } else if matches!(
        intent,
        QueryIntent::Navigational | QueryIntent::Transactional
    ) {
        // Literal SERPs can rank press coverage or an event page above a
        // first-party destination. This is discovery only: the original query
        // still owns reranking, and host/entity matching decides whether the
        // candidate is a valid destination. Normalize navigational boilerplate
        // before issuing the provider query: Bing interprets a leading
        // "site oficial" as a request about website builders, and equivalent
        // variants otherwise create avoidable duplicate traffic under load.
        let entity = entity_words(query, intent).join(" ");
        if intent == QueryIntent::Transactional {
            // Equivalent action phrasings share one provider flight. During a
            // cold burst, "entrar/login/acessar" must not become independent
            // calls that exhaust a keyless provider before any destination is
            // retained. The original query still owns filtering and ranking.
            let action = canonical_transaction_action(query).unwrap_or("acessar");
            let base = if entity.is_empty() {
                query.to_string()
            } else {
                format!("{entity} {action}")
            };
            let official = if entity.is_empty() {
                format!("{base} site oficial")
            } else {
                format!("{entity} site oficial")
            };
            vec![base, official]
        } else {
            let base = if entity.is_empty() {
                query.to_string()
            } else {
                entity
            };
            vec![base.clone(), format!("{base} site oficial")]
        }
    } else if intent == QueryIntent::DueDiligence {
        // Normalize equivalent risk phrasings into one neutral entity query
        // and one bounded adverse-evidence query. Sending "legítimo",
        // "confiável", "fraude", and "seguro" independently caused a cold
        // concurrent burst to exhaust keyless providers even though all four
        // requests ask about the same entity. The original wording still owns
        // intent filtering and the answer policy; these rewrites only improve
        // shared discovery recall.
        let entity = entity_words(query, intent).join(" ");
        if entity.is_empty() {
            vec![query.to_string()]
        } else {
            vec![
                entity.clone(),
                format!("\"{entity}\" fraude golpe reclamações"),
            ]
        }
    } else if intent == QueryIntent::CurrentEvents && poll_query {
        let poll_terms = relevance_terms(query);
        let is_brazil_presidential_poll = poll_terms
            .iter()
            .any(|term| matches!(term.as_str(), "presidente" | "presidencial"))
            && poll_terms.iter().any(|term| term == "brasil");
        if is_brazil_presidential_poll {
            // Equivalent natural phrasings share one provider/crawl workload.
            // The rewrite names only the office and jurisdiction—never an
            // institute, candidate, or expected number—and the original query
            // still owns scope and ranking after discovery. The bounded set
            // covers generic, result-bearing, and first-round vocabulary;
            // per-provider backpressure limits the concurrent keyless load.
            // Claim, round, and registry-specific expansion remains available
            // later when retained evidence proves it is necessary.
            let base = "pesquisa eleitoral presidente brasil hoje";
            return vec![
                base.to_string(),
                format!("{base} números resultados"),
                format!("{base} intenção de voto primeiro turno percentuais"),
            ];
        }
        let mut queries = vec![query.to_string()];
        if !already_result_focused {
            queries.push(format!("{query} números resultados"));
        }
        if !folded.contains("intencao voto") && !folded.contains("percent") {
            // Publishers describe the same poll page using different surface
            // vocabulary. This recall rewrite is deliberately generic: no
            // candidate, institute, result, or outlet is baked in.
            queries.push(format!(
                "{query} intenção de voto primeiro turno percentuais"
            ));
        }
        queries
    } else if intent == QueryIntent::CurrentEvents && is_current_office_holder_query(query) {
        let folded = fold_text(query);
        if folded.contains("presidente") && folded.contains("brasil") {
            vec![
                query.to_string(),
                "\"atual presidente do Brasil\" nome".to_string(),
                "site:gov.br/planalto presidente da República".to_string(),
            ]
        } else {
            vec![query.to_string(), format!("{query} site oficial governo")]
        }
    } else if intent == QueryIntent::CurrentEvents
        && (folded.contains("inteligencia artificial")
            || folded.contains("artificial intelligence")
            || folded.split_whitespace().any(|term| term == "ia"))
    {
        // Normalize equivalent generic AI-news phrasings into a small shared
        // provider workload. Concurrent variants then coalesce in the engine
        // cache instead of independently exhausting keyless-provider limits,
        // and the explicit phrase avoids empty SERPs for the acronym "IA".
        if folded.contains("artificial intelligence") {
            vec![
                "latest artificial intelligence news".to_string(),
                "AI news today".to_string(),
            ]
        } else {
            vec![
                "notícias inteligência artificial hoje".to_string(),
                "novidades inteligência artificial hoje".to_string(),
            ]
        }
    } else if intent == QueryIntent::Factual
        && [
            "criou",
            "criada",
            "criado",
            "fundou",
            "fundador",
            "fundadores",
            "founder",
            "founders",
            "inventou",
            "who created",
            "who founded",
        ]
        .iter()
        .any(|cue| folded.contains(cue))
        && !folded.contains("fonte oficial")
    {
        // A relation query needs independent corroboration, but discovery
        // should also offer the first-party/primary record a chance to enter
        // the evidence set. "Fonte oficial" is a provider-side recall hint;
        // it receives no truth bonus and must still be fetched and entailed.
        let factual_terms = factual_query_terms(query);
        let entity = factual_terms
            .into_iter()
            .filter(|term| !matches!(term.as_str(), "created" | "founded" | "invented"))
            .collect::<Vec<_>>()
            .join(" ");
        if !entity.is_empty() {
            let founded = factual_query_terms(query)
                .iter()
                .any(|term| term == "founded");
            let (portuguese_relation, english_relation) = if founded {
                ("quem fundou", "who founded")
            } else {
                // "Inventou" and "created" are normalized to the same
                // institutional-origin relation. Keeping a shared canonical
                // rewrite gives low-recall paraphrases a provider-independent
                // fallback and lets concurrent variants coalesce in cache.
                ("quem criou", "who created")
            };
            // The query language is not necessarily the evidence language.
            // A Brazilian institution may have its primary record only in
            // Portuguese, while a global company may expose its best record
            // in English. Always issue the two bounded canonical rewrites so
            // correctness does not depend on a concurrent paraphrase warming
            // the local index. Equivalent variants still coalesce into four
            // shared provider flights per entity/relation.
            return vec![
                format!("{portuguese_relation} {entity}"),
                format!("{portuguese_relation} {entity} fonte oficial"),
                format!("{english_relation} {entity}"),
                format!("{english_relation} {entity} official source"),
            ];
        }
        vec![query.to_string(), format!("{query} fonte oficial")]
    } else if intent == QueryIntent::Exploratory && is_health_guidance_query(query) {
        vec![
            query.to_string(),
            format!("{query} site:gov.br saúde"),
            format!("{query} Ministério da Saúde sinais de alarme"),
        ]
    } else if intent == QueryIntent::Exploratory && is_tax_guidance_query(query) {
        vec![
            query.to_string(),
            format!("{query} site:gov.br Receita Federal"),
        ]
    } else {
        vec![query.to_string()]
    }
}

pub(crate) fn prefers_global_english_news(query: &str) -> bool {
    let folded = fold_text(query);
    classify_query(query) == QueryIntent::CurrentEvents
        && folded.contains("news")
        && (folded.contains("artificial intelligence")
            || folded.split_whitespace().any(|term| term == "ai"))
}

/// Build a bounded disambiguation query from metadata exposed by a likely
/// first-party destination. The destination is never returned as diligence
/// evidence; its title only contributes namespace terms that can help a web
/// index distinguish a new entity from older homonyms. Independent results
/// must still name the exact requested entity and pass the normal evidence
/// policy.
pub(crate) fn diligence_context_terms(query: &str, destination_title: &str) -> Vec<String> {
    const GENERIC: &[&str] = &[
        "and",
        "company",
        "future",
        "fund",
        "investment",
        "official",
        "program",
        "site",
        "website",
    ];
    let entity_words = entity_words(query, QueryIntent::DueDiligence);
    if entity_words.is_empty() {
        return Vec::new();
    }
    let mut context = Vec::new();
    for term in relevance_terms(destination_title) {
        if term.len() < 4
            || entity_words.contains(&term)
            || GENERIC.contains(&term.as_str())
            || context.contains(&term)
        {
            continue;
        }
        context.push(term);
        if context.len() == 2 {
            break;
        }
    }
    context
}

pub(crate) fn diligence_context_queries(query: &str, destination_title: &str) -> Vec<String> {
    let entity_words = entity_words(query, QueryIntent::DueDiligence);
    let context = diligence_context_terms(query, destination_title);
    if entity_words.is_empty() || context.is_empty() {
        return Vec::new();
    }
    vec![format!(
        "\"{}\" {}",
        entity_words.join(" "),
        context.join(" ")
    )]
}

/// Require independent diligence candidates to remain inside the namespace
/// established by first-party destination metadata. A literal entity name is
/// insufficient when an older fund, product, or organization has the same
/// name. Matching one bounded context term is intentionally conservative: the
/// first-party page identifies the entity, but never becomes trust evidence.
pub(crate) fn diligence_candidate_matches_context(
    query: &str,
    result: &SearchResultItem,
    context_terms: &[String],
) -> bool {
    if context_terms.is_empty() {
        return diligence_surface_matches_entity(query, result);
    }
    let surface = relevance_terms(&format!(
        "{} {} {} {}",
        result.title,
        result.snippet,
        result.url,
        result.content_relevant.as_deref().unwrap_or("")
    ));
    diligence_surface_matches_entity(query, result)
        && context_terms.iter().any(|term| surface.contains(term))
}

pub(crate) fn exact_entity_host_match(query: &str, result: &SearchResultItem) -> bool {
    let intent = classify_query(query);
    let entity_compact = entity_words(query, intent).concat();
    !entity_compact.is_empty()
        && is_registrable_origin(&result.url)
        && host_entity_label(&result.url) == entity_compact
}

/// Destination-only entity match with conservative one-edit typo tolerance.
///
/// This is deliberately not used as factual evidence. Both labels must be
/// alphabetic, begin with the same character, and contain at least five
/// characters; numeric lookalikes and short ambiguous names are rejected.
pub(crate) fn navigation_entity_host_match(query: &str, result: &SearchResultItem) -> bool {
    if is_online_tool_query(query) {
        return false;
    }
    if exact_entity_host_match(query, result) {
        return true;
    }
    let intent = classify_query(query);
    let entity_compact = entity_words(query, intent).concat();
    let host_label = host_entity_label(&result.url);
    is_registrable_origin(&result.url) && fuzzy_entity_label_match(&entity_compact, &host_label)
}

pub(crate) fn navigation_entity_key(query: &str) -> String {
    entity_words(query, classify_query(query)).concat()
}

/// Stable metadata-cache key for an action on an entity. Actions cannot share
/// a key: a verified `/login` route is not a valid answer to "download".
pub(crate) fn transaction_destination_key(query: &str) -> Option<String> {
    if classify_query(query) != QueryIntent::Transactional {
        return None;
    }
    let entity = navigation_entity_key(query);
    let action = canonical_transaction_action(query)?;
    (!entity.is_empty()).then(|| format!("transaction:{entity}:{action}"))
}

/// Conventional action paths that may be probed on an already-discovered,
/// entity-matching first-party origin. These are candidates, never evidence;
/// the caller must fetch and validate the final URL before serving one.
pub(crate) fn transaction_route_paths(query: &str) -> &'static [&'static str] {
    match canonical_transaction_action(query) {
        Some("login") => &["/login"],
        Some("inscrição") => &["/apply", "/application"],
        Some("download") => &["/download"],
        Some("preço") => &["/pricing"],
        _ => &[],
    }
}

/// Conservative `.com` labels for direct navigational resolution when every
/// external provider misses. Limited to multi-token, non-government entities;
/// the caller must still fetch the origin and validate the final host against
/// the original query before surfacing it.
pub(crate) fn navigation_domain_guesses(query: &str) -> Vec<String> {
    if is_online_tool_query(query) {
        return Vec::new();
    }
    let intent = classify_query(query);
    if !matches!(
        intent,
        QueryIntent::Navigational | QueryIntent::Transactional
    ) {
        return Vec::new();
    }
    let words = entity_words(query, intent);
    let single_transactional_brand = intent == QueryIntent::Transactional
        && words.len() == 1
        && words[0].len() >= 5
        && words[0].bytes().all(|byte| byte.is_ascii_alphabetic());
    if (words.len() < 2 && !single_transactional_brand)
        || words.iter().any(|word| {
            matches!(
                word.as_str(),
                "federal" | "governo" | "ministerio" | "prefeitura" | "tribunal"
            )
        })
    {
        return Vec::new();
    }
    if single_transactional_brand {
        return vec![words.concat()];
    }
    let mut labels = vec![words.concat()];
    for (word_index, word) in words.iter().enumerate() {
        let chars: Vec<char> = word.chars().collect();
        // Delete one vowel ("found" -> "fund").
        for (index, character) in chars.iter().enumerate() {
            if "aeiou".contains(*character) && chars.len() > 3 {
                let mut variant = chars.clone();
                variant.remove(index);
                let mut variant_words = words.clone();
                variant_words[word_index] = variant.into_iter().collect();
                labels.push(variant_words.concat());
            }
        }
        // Restore one omitted vowel only between consonants ("fnd" ->
        // "fund"). This keeps the candidate set bounded and explainable.
        for index in 1..chars.len() {
            if !"aeiou".contains(chars[index - 1]) && !"aeiou".contains(chars[index]) {
                for vowel in "aeiou".chars() {
                    let mut variant = chars.clone();
                    variant.insert(index, vowel);
                    let mut variant_words = words.clone();
                    variant_words[word_index] = variant.into_iter().collect();
                    labels.push(variant_words.concat());
                }
            }
        }
    }
    labels.sort();
    labels.dedup();
    labels.truncate(24);
    labels
}

pub(crate) fn navigation_title_supports_entity(query: &str, title: &str) -> bool {
    let words = entity_words(query, classify_query(query));
    let title_terms = relevance_terms(title);
    !words.is_empty()
        && title_terms.contains(&words[0])
        && words
            .iter()
            .filter(|word| title_terms.contains(word))
            .count()
            * 2
            >= words.len()
}

/// Primary-source hint used only to decide whether a factual answer needs an
/// answer-aware discovery pass. A public-authority record or the exact
/// first-party entity host qualifies; the fetched content must still entail
/// the claim before it can become a citation.
pub(crate) fn primary_source_url_matches_query(query: &str, url: &str) -> bool {
    if is_public_authority_url(url) {
        return true;
    }
    let relation_terms = factual_query_terms(query);
    let entity_compact = relation_terms
        .iter()
        .filter(|term| !matches!(term.as_str(), "created" | "founded" | "invented"))
        .cloned()
        .collect::<String>();
    !entity_compact.is_empty() && host_entity_label(url) == entity_compact
}

/// Claim-aware primary-source check.
///
/// A government domain is authoritative only for claims about the authority
/// represented by that URL. Treating every public record as interchangeable
/// lets an unrelated `.gov` result suppress the discovery of the institution
/// that actually owns the fact. First-party entity hosts retain the narrower
/// query match above; public-authority URLs must additionally identify an
/// institution named in the extracted claim.
pub(crate) fn primary_source_url_matches_claim(query: &str, claim: &str, url: &str) -> bool {
    if !primary_source_url_matches_query(query, url) {
        return false;
    }
    if !is_public_authority_url(url) {
        return true;
    }

    let identifiers = public_authority_url_identifiers(url);
    if identifiers.is_empty() {
        return false;
    }
    let claim_terms = relevance_terms(claim);
    identifiers.iter().any(|identifier| {
        claim_terms
            .iter()
            .any(|term| term.len() >= 4 && term == identifier)
            || (2..=4).any(|width| {
                claim_terms.windows(width).any(|window| {
                    let compact = window.concat();
                    let acronym: String = window
                        .iter()
                        .filter_map(|term| term.chars().next())
                        .collect();
                    compact == *identifier
                        || (acronym.len() >= 2
                            && identifier.len() <= 6
                            && (identifier == &acronym
                                || identifier.starts_with(&acronym)
                                || acronym.starts_with(identifier)))
                })
            })
    })
}

fn public_authority_url_identifiers(url: &str) -> Vec<String> {
    let without_scheme = url.split("//").nth(1).unwrap_or(url);
    let mut parts = without_scheme.split('/');
    let host = parts
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let host_labels = host.split('.').collect::<Vec<_>>();
    let authority_suffix = host_labels
        .iter()
        .position(|label| matches!(*label, "gov" | "gob" | "gouv"))
        .unwrap_or(host_labels.len());
    let mut identifiers = host_labels[..authority_suffix]
        .iter()
        .rev()
        .filter(|label| !matches!(**label, "www" | "portal" | "site"))
        .map(|label| compact_identifier(label))
        .filter(|label| label.len() >= 2)
        .collect::<Vec<_>>();
    for segment in parts.take(3) {
        let identifier = compact_identifier(segment);
        if identifier.len() >= 3 {
            identifiers.push(identifier);
        }
    }
    identifiers.sort();
    identifiers.dedup();
    identifiers
}

fn compact_identifier(value: &str) -> String {
    fold_text(value)
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .collect()
}

fn is_public_authority_url(url: &str) -> bool {
    let host = url
        .split("//")
        .nth(1)
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or("")
        .trim_start_matches("www.")
        .split(':')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    host == "gov.br"
        || host.ends_with(".gov.br")
        || host.ends_with(".gov")
        || host.ends_with(".gob")
        || host.ends_with(".gouv.fr")
}

/// Whether a local-only result set is sufficient to skip live discovery.
///
/// Score thresholds alone cannot distinguish a platform query from a corpus
/// full of homonyms. Current-events and due-diligence queries also require a
/// live evidence check even when the local semantic scores look strong.
pub(crate) fn local_coverage_satisfies_intent(query: &str, results: &[SearchResultItem]) -> bool {
    match classify_query(query) {
        QueryIntent::Tool => {
            let domains = results
                .iter()
                .filter(|result| {
                    result.source == "local"
                        && !result.content_md.trim().is_empty()
                        && online_tool_surface_matches(query, result)
                })
                .map(|result| source_family(&result.url))
                .collect::<std::collections::HashSet<_>>();
            domains.len() >= 3
        }
        QueryIntent::Navigational => results.iter().any(|result| {
            let signals = intent_adjustment(query, result).signals;
            ((signals.contains(&"exact_entity_host") || signals.contains(&"fuzzy_entity_host"))
                && (signals.contains(&"root_destination") || !result.content_md.trim().is_empty()))
                || signals.contains(&"public_authority_entity_path")
        }),
        QueryIntent::Transactional => results.iter().any(|result| {
            let signals = intent_adjustment(query, result).signals;
            signals.contains(&"exact_entity_host") && signals.contains(&"transaction_destination")
        }),
        QueryIntent::CurrentEvents => current_local_coverage(query, results, unix_now()),
        QueryIntent::DueDiligence | QueryIntent::Factual => false,
        QueryIntent::Exploratory => true,
    }
}

fn current_local_coverage(query: &str, results: &[SearchResultItem], now: i64) -> bool {
    const MAX_QUERY_CACHE_AGE_SECS: i64 = 300;
    let source_families: std::collections::HashSet<_> = results
        .iter()
        .filter(|result| !is_hub_url(&result.url))
        .filter(|result| current_topic_coverage(query, result) >= 0.50)
        .filter(|result| {
            result.crawled_at.is_some_and(|crawled_at| {
                now.saturating_sub(crawled_at as i64) <= MAX_QUERY_CACHE_AGE_SECS
            })
        })
        .filter(|result| {
            freshness_adjustment(result, now).is_some_and(|(_, signal)| {
                matches!(signal, "published_within_1d" | "published_within_2d")
            })
        })
        .map(|result| source_family(&result.url))
        .collect();
    let required_families = if is_poll_query(query) { 2 } else { 3 };
    source_families.len() >= required_families
}

fn current_topic_coverage(query: &str, result: &SearchResultItem) -> f64 {
    const TEMPORAL_TERMS: &[&str] = &[
        "agora", "atual", "hoje", "latest", "news", "noticia", "noticias", "now", "today",
        "ultima", "ultimas",
    ];
    const QUESTION_TERMS: &[&str] = &[
        "como", "qual", "quais", "que", "quem", "what", "which", "who",
    ];
    let query_terms: Vec<_> = relevance_terms(query)
        .into_iter()
        .filter(|term| {
            !TEMPORAL_TERMS.contains(&term.as_str()) && !QUESTION_TERMS.contains(&term.as_str())
        })
        .collect();
    if query_terms.is_empty() {
        return 1.0;
    }
    // Current-event relevance must be visible in the result's public search
    // surface. A crawled page body often contains unrelated navigation,
    // recommended stories, or newsletter copy; allowing `content_relevant`
    // here let an election article rank for an AI-news query merely because
    // its footer linked to an artificial-intelligence story.
    let evidence_terms = if result.source == "local"
        && is_current_office_holder_query(query)
        && authoritative_source_for_query(query, &result.url)
    {
        // A URL from the versioned office/jurisdiction registry has already
        // been scoped before crawling. Its retained primary text may expose
        // the role even when a generic title such as "Quem é Quem" does not.
        // This exception is deliberately unavailable to ordinary local pages
        // because footer text would otherwise reintroduce topic leakage.
        let body_prefix: String = result.content_md.chars().take(2_000).collect();
        relevance_terms(&format!("{} {} {}", result.title, result.url, body_prefix))
    } else if result.source == "local" {
        // A retained snippet is generated from page content and may include
        // related-story/footer text. For a current local document, require
        // the topic on its own title/URL surface.
        relevance_terms(&format!("{} {}", result.title, result.url))
    } else {
        relevance_terms(&format!(
            "{} {} {}",
            result.title, result.snippet, result.url
        ))
    };
    if poll_scope_conflicts(&query_terms, &evidence_terms) {
        return 0.0;
    }
    if live_value_scope_conflicts(query, &query_terms, &evidence_terms) {
        return 0.0;
    }
    if current_office_holder_scope_conflicts(query, &evidence_terms) {
        return 0.0;
    }
    if current_subject_scope_conflicts(query, &query_terms, &evidence_terms) {
        return 0.0;
    }
    let matches = query_terms
        .iter()
        .filter(|term| {
            evidence_terms.contains(term)
                || (is_current_office_holder_query(query)
                    && evidence_terms
                        .iter()
                        .any(|evidence| current_office_term_equivalent(term, evidence)))
                || (is_live_value_query(query)
                    && evidence_terms
                        .iter()
                        .any(|evidence| current_value_term_equivalent(term, evidence)))
        })
        .count();
    let lexical_coverage = matches as f64 / query_terms.len() as f64;

    // Poll coverage is expressed with several interchangeable surface forms:
    // "pesquisa eleitoral", "levantamento presidencial", "intenção de
    // voto" and "primeiro turno" all denote the same result family. Without
    // this small topic equivalence, a relevant result that only names the
    // candidates is denied freshness, while a generic fresh page can win.
    if is_poll_topic(&query_terms) && is_poll_topic(&evidence_terms) {
        lexical_coverage.max(0.50)
    } else {
        lexical_coverage
    }
}

fn current_subject_scope_conflicts(
    query: &str,
    query_terms: &[String],
    evidence_terms: &[String],
) -> bool {
    let folded = fold_text(query);
    let separator = [" x ", " vs ", " versus "]
        .into_iter()
        .find(|separator| folded.contains(separator));
    if let Some(separator) = separator {
        let Some((left, right)) = folded.split_once(separator) else {
            return false;
        };
        let left_terms = relevance_terms(left);
        let right_terms = relevance_terms(right);
        return left_terms.is_empty()
            || right_terms.is_empty()
            || !left_terms.iter().any(|term| evidence_terms.contains(term))
            || !right_terms.iter().any(|term| evidence_terms.contains(term));
    }

    let event_words = ["game", "jogo", "match", "partida"];
    if !query_terms
        .iter()
        .any(|term| event_words.contains(&term.as_str()))
    {
        return false;
    }
    let subjects = query_terms
        .iter()
        .filter(|term| !event_words.contains(&term.as_str()))
        .collect::<Vec<_>>();
    !subjects.is_empty()
        && subjects
            .iter()
            .any(|subject| !evidence_terms.contains(*subject))
}

fn current_office_term_equivalent(left: &str, right: &str) -> bool {
    matches!(
        (left, right),
        ("brasil", "brazil")
            | ("brazil", "brasil")
            | ("presidente", "president")
            | ("president", "presidente")
    )
}

fn current_value_term_equivalent(left: &str, right: &str) -> bool {
    matches!(
        (left, right),
        ("price", "preco")
            | ("price", "cotacao")
            | ("price", "valor")
            | ("preco", "price")
            | ("cotacao", "price")
            | ("quote", "cotacao")
            | ("cotacao", "quote")
            | ("valor", "price")
    )
}

fn live_value_scope_conflicts(
    query: &str,
    query_terms: &[String],
    evidence_terms: &[String],
) -> bool {
    if !is_live_value_query(query) {
        return false;
    }
    let subject_terms = query_terms.iter().filter(|term| {
        !matches!(
            term.as_str(),
            "cotacao" | "exchange" | "price" | "preco" | "quote" | "rate" | "taxa" | "valor"
        )
    });
    subject_terms
        .into_iter()
        .any(|subject| !evidence_terms.contains(subject))
}

fn current_office_holder_scope_conflicts(query: &str, evidence_terms: &[String]) -> bool {
    if !is_current_office_holder_query(query) {
        return false;
    }
    let explicitly_seeking_reelection = evidence_terms.iter().any(|term| term == "reeleicao")
        && evidence_terms
            .iter()
            .any(|term| matches!(term.as_str(), "president" | "presidente"));
    evidence_terms.iter().any(|term| {
        matches!(
            term.as_str(),
            "candidato"
                | "candidata"
                | "candidatura"
                | "disputa"
                | "eleicao"
                | "eleicoes"
                | "ex-presidente"
                | "former"
                | "futuro"
                | "jovem"
                | "pesquisa"
                | "pesquisas"
                | "presidentes"
                | "sera"
                | "tentou"
                | "tenta"
        )
    }) && !explicitly_seeking_reelection
}

fn current_office_holder_page_matches(query: &str, result: &SearchResultItem) -> bool {
    if !is_current_office_holder_query(query) {
        return true;
    }
    if is_hub_url(&result.url) {
        return false;
    }
    if authoritative_source_for_query(query, &result.url) {
        return true;
    }
    // A generic role explainer ("what a president does") is on-topic but
    // cannot identify the current holder. Independent pages must state the
    // temporal relation explicitly; name extraction and corroboration remain
    // separate answer-layer responsibilities.
    let body_prefix: String = result.content_md.chars().take(6_000).collect();
    let surface = fold_text(&format!(
        "{} {} {} {} {}",
        result.title,
        result.snippet,
        result.url,
        result.content_relevant.as_deref().unwrap_or(""),
        body_prefix,
    ));
    if [
        "atual presidente",
        "atualmente o presidente",
        "current president",
        "incumbent president",
        "presidente atual",
    ]
    .iter()
    .any(|cue| surface.contains(cue))
    {
        return true;
    }
    let terms = relevance_terms(&surface);
    let has_role = terms
        .iter()
        .any(|term| matches!(term.as_str(), "president" | "presidente"));
    let has_jurisdiction = terms
        .iter()
        .any(|term| matches!(term.as_str(), "brasil" | "brazil"));
    let is_former = terms
        .iter()
        .any(|term| matches!(term.as_str(), "former" | "ex-presidente"));
    has_role && has_jurisdiction && !is_former
}

pub(crate) fn is_health_guidance_query(query: &str) -> bool {
    let folded = fold_text(query);
    let terms = relevance_terms(query);
    let explicit_health_claim = [
        "benefits of ",
        "beneficios de ",
        "health benefits",
        "medical benefits",
    ]
    .iter()
    .any(|cue| folded.contains(cue));
    explicit_health_claim
        || terms.iter().any(|term| {
            matches!(
                term.as_str(),
                "alcool"
                    | "alcohol"
                    | "autoimmune"
                    | "blood"
                    | "cancer"
                    | "dengue"
                    | "diagnostico"
                    | "diagnosis"
                    | "disease"
                    | "doenca"
                    | "drug"
                    | "drugs"
                    | "health"
                    | "infection"
                    | "medicacao"
                    | "medicacoes"
                    | "medica"
                    | "medico"
                    | "medication"
                    | "medications"
                    | "medicine"
                    | "pain"
                    | "pregnancy"
                    | "sangue"
                    | "sintoma"
                    | "sintomas"
                    | "symptom"
                    | "symptoms"
                    | "syndrome"
                    | "therapy"
                    | "therapies"
                    | "tratamento"
                    | "treatment"
                    | "treatments"
                    | "viral"
                    | "virus"
            ) || [
                "contraindicat",
                "diagnos",
                "disease",
                "infect",
                "pregnan",
                "symptom",
                "therap",
                "treat",
            ]
            .iter()
            .any(|prefix| term.starts_with(prefix))
        })
}

fn is_dengue_guidance_query(query: &str) -> bool {
    relevance_terms(query).iter().any(|term| term == "dengue")
}

fn is_legal_financial_guidance_query(query: &str) -> bool {
    relevance_terms(query).iter().any(|term| {
        matches!(
            term.as_str(),
            "annuity"
                | "bankruptcy"
                | "benefit"
                | "credito"
                | "credit"
                | "imposto"
                | "insurance"
                | "law"
                | "legal"
                | "loan"
                | "mortgage"
                | "regulated"
                | "regulation"
                | "regulatory"
                | "routing"
                | "salary"
                | "tax"
                | "trust"
        )
    })
}

pub(crate) fn is_high_stakes_guidance_query(query: &str) -> bool {
    is_health_guidance_query(query) || is_legal_financial_guidance_query(query)
}

pub(crate) fn is_recommendation_query(query: &str) -> bool {
    let terms = relevance_terms(query);
    let explicit_comparison = terms.iter().any(|term| {
        matches!(
            term.as_str(),
            "best"
                | "melhor"
                | "melhores"
                | "recomenda"
                | "recomendacao"
                | "recommend"
                | "recommended"
        )
    });
    let asks_which_to_buy = terms
        .iter()
        .any(|term| matches!(term.as_str(), "qual" | "quais" | "what" | "which"))
        && terms
            .iter()
            .any(|term| matches!(term.as_str(), "buy" | "comprar"));
    let portable_computer = terms.iter().any(|term| {
        matches!(
            term.as_str(),
            "laptop" | "laptops" | "notebook" | "notebooks"
        )
    });
    let software_development = terms.iter().any(|term| {
        matches!(
            term.as_str(),
            "coding"
                | "desenvolvedor"
                | "desenvolvedores"
                | "desenvolvimento"
                | "programacao"
                | "programar"
                | "programming"
                | "software"
        )
    });
    explicit_comparison || asks_which_to_buy || (portable_computer && software_development)
}

fn is_tax_guidance_query(query: &str) -> bool {
    let terms = relevance_terms(query);
    terms.iter().any(|term| term == "imposto")
        && terms
            .iter()
            .any(|term| matches!(term.as_str(), "declarar" | "declaracao" | "renda"))
}

pub(crate) fn authoritative_source_for_query(query: &str, url: &str) -> bool {
    if is_current_office_holder_query(query) {
        return official_registry_entry_for_query(query, url);
    }
    if is_tax_guidance_query(query) {
        return is_public_authority_url(url);
    }
    if is_health_guidance_query(query) {
        let host = normalized_host(url);
        return is_public_authority_url(url)
            || host == "fiocruz.br"
            || host.ends_with(".fiocruz.br")
            || host == "paho.org"
            || host.ends_with(".paho.org")
            || host == "who.int"
            || host.ends_with(".who.int");
    }
    false
}

/// Do not let a semantically similar poll for a different elected office
/// satisfy the query merely because both pages say "Datafolha", "turno" and
/// "intenção de voto". We only reject explicit office conflicts; a headline
/// that omits the office remains eligible and must still pass the other topic
/// and evidence gates.
fn poll_scope_conflicts(query_terms: &[String], evidence_terms: &[String]) -> bool {
    let asks_president = query_terms
        .iter()
        .any(|term| matches!(term.as_str(), "presidente" | "presidencial"));
    if !asks_president {
        return false;
    }
    let mentions_president = evidence_terms
        .iter()
        .any(|term| matches!(term.as_str(), "presidente" | "presidencial" | "presidencia"));
    let mentions_other_office = evidence_terms.iter().any(|term| {
        matches!(
            term.as_str(),
            "deputado"
                | "deputados"
                | "governador"
                | "governadora"
                | "governo"
                | "prefeito"
                | "prefeita"
                | "senador"
                | "senadora"
        )
    });
    mentions_other_office && !mentions_president
}

fn is_poll_topic(terms: &[String]) -> bool {
    let joined = terms.join(" ");
    joined.contains("pesquisa eleitoral")
        || joined.contains("levantamento presidencial")
        || joined.contains("intencao voto")
        || joined.contains("primeiro turno")
        || joined.contains("segundo turno")
        || (terms.iter().any(|term| term == "pesquisa")
            && terms
                .iter()
                .any(|term| matches!(term.as_str(), "presidente" | "presidencial")))
        || terms
            .iter()
            .any(|term| matches!(term.as_str(), "eleicao" | "eleitoral" | "poll" | "polling"))
            && terms.iter().any(|term| {
                matches!(
                    term.as_str(),
                    "presidente" | "presidencial" | "pesquisa" | "resultado" | "voto"
                )
            })
}

pub(crate) fn is_poll_query(query: &str) -> bool {
    is_poll_topic(&relevance_terms(query))
}

/// Coarse publisher-ownership family used only to avoid treating sister
/// domains as independent corroboration. Unknown publishers stay separated
/// by hostname; a production deployment should replace this small seed map
/// with a versioned ownership/derivation graph.
pub(crate) fn source_family(url: &str) -> String {
    let domain = url
        .split("//")
        .nth(1)
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or("")
        .trim_start_matches("www.")
        .to_ascii_lowercase();
    if domain == "globo.com" || domain.ends_with(".globo.com") {
        return "grupo-globo".to_string();
    }
    if domain == "techtudo.com.br" || domain.ends_with(".techtudo.com.br") {
        return "grupo-globo".to_string();
    }
    if domain == "uol.com.br" || domain.ends_with(".uol.com.br") {
        return "grupo-uol-folha".to_string();
    }
    domain
}

pub(crate) fn entity_words(query: &str, intent: QueryIntent) -> Vec<String> {
    let words: Vec<String> = fold_text(query)
        .split_whitespace()
        .map(str::to_string)
        .collect();
    match intent {
        QueryIntent::Transactional => words
            .into_iter()
            .filter(|word| !TRANSACTION_TERMS.contains(&word.as_str()))
            .collect(),
        QueryIntent::Navigational => words
            .into_iter()
            .filter(|word| !NAVIGATION_TERMS.contains(&word.as_str()))
            .collect(),
        QueryIntent::Tool => words
            .into_iter()
            .filter(|word| {
                !ONLINE_EXECUTION_CONTEXT.contains(&word.as_str())
                    && !ONLINE_TOOL_TERMS.contains(&word.as_str())
            })
            .collect(),
        QueryIntent::DueDiligence => words
            .into_iter()
            .filter(|word| {
                !DUE_DILIGENCE_TERMS.contains(&word.as_str())
                    && !["e", "eh", "is", "trusted"].contains(&word.as_str())
            })
            .collect(),
        _ => words,
    }
}

fn contains_any(text: &str, terms: &[&str]) -> bool {
    terms.iter().any(|term| text.contains(term))
}

fn contains_any_word(words: &[&str], terms: &[&str]) -> bool {
    words.iter().any(|word| terms.contains(word))
}

fn canonical_transaction_action(query: &str) -> Option<&'static str> {
    let query = fold_text(query);
    let categories: &[(&[&str], &str)] = &[
        (
            &["acessar", "entrar", "login", "sign in", "signup"],
            "login",
        ),
        (
            &["aplicar", "application", "inscrever", "inscricao"],
            "inscrição",
        ),
        (&["baixar", "download"], "download"),
        (
            &["assinar", "buy", "comprar", "preco", "subscribe"],
            "preço",
        ),
    ];
    categories
        .iter()
        .find(|(terms, _)| terms.iter().any(|term| query.contains(term)))
        .map(|(_, canonical)| *canonical)
}

/// Normalize short “run this language in the browser” queries without
/// confusing them with navigation to the language homepage. The original
/// query still owns embedding/ranking; this only coalesces discovery and the
/// multilingual serving-scope terms.
pub(crate) fn is_online_tool_query(query: &str) -> bool {
    canonical_online_tool_query(query).is_some()
}

fn canonical_online_tool_query(query: &str) -> Option<String> {
    let terms = relevance_terms(query);
    if !terms
        .iter()
        .any(|term| ONLINE_EXECUTION_CONTEXT.contains(&term.as_str()))
        || !terms
            .iter()
            .any(|term| ONLINE_TOOL_TERMS.contains(&term.as_str()))
    {
        return None;
    }
    let entity = terms
        .into_iter()
        .filter(|term| {
            !ONLINE_EXECUTION_CONTEXT.contains(&term.as_str())
                && !ONLINE_TOOL_TERMS.contains(&term.as_str())
        })
        .collect::<Vec<_>>()
        .join(" ");
    (!entity.is_empty()).then(|| format!("{entity} online compiler"))
}

fn online_tool_surface_matches(query: &str, result: &SearchResultItem) -> bool {
    let Some(canonical) = canonical_online_tool_query(query) else {
        return false;
    };
    let entity_terms = relevance_terms(&canonical)
        .into_iter()
        .filter(|term| !matches!(term.as_str(), "online" | "compiler"))
        .collect::<Vec<_>>();
    let surface_terms = relevance_terms(&format!(
        "{} {} {}",
        result.title, result.snippet, result.url
    ));
    !entity_terms.is_empty()
        && entity_terms.iter().all(|term| surface_terms.contains(term))
        && surface_terms
            .iter()
            .any(|term| ONLINE_TOOL_TERMS.contains(&term.as_str()))
}

/// Reject a deep action-looking URL when its path only contains an action as
/// part of a larger slug (`/login-Claude`) rather than as a conventional route
/// (`/login`, `/auth/login`, `/sign-in`). Search-engine snippets can synthesize
/// such URLs; routing to the verified first-party root is safer.
pub(crate) fn suspicious_transaction_path(query: &str, url: &str) -> bool {
    let Some((_, rest)) = url.split_once("://") else {
        return true;
    };
    let path = rest.split_once('/').map(|(_, path)| path).unwrap_or("");
    let path = path
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .trim_matches('/');
    if path.is_empty() {
        return false;
    }

    let Some(action) = canonical_transaction_action(query) else {
        return false;
    };
    let accepted: &[&str] = match action {
        "login" => &[
            "auth", "entrar", "login", "log in", "sign in", "signin", "sso",
        ],
        "inscrição" => &[
            "aplicar",
            "application",
            "apply",
            "candidatar",
            "inscrever",
            "inscricao",
        ],
        "download" => &["baixar", "download"],
        "preço" => &["buy", "comprar", "preco", "pricing", "subscribe"],
        _ => &[],
    };
    let segments = path
        .split('/')
        .map(fold_text)
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    let contains_action = segments
        .iter()
        .any(|segment| accepted.iter().any(|term| segment.contains(term)));
    let exact_action = segments
        .iter()
        .any(|segment| accepted.iter().any(|term| segment == term));
    contains_action && !exact_action
}

pub(crate) fn transaction_destination_matches_query(query: &str, destination: &str) -> bool {
    let query = fold_text(query);
    let categories: &[(&[&str], &[&str])] = &[
        (
            &["acessar", "entrar", "login", "sign in", "signup"],
            &[
                "acessar",
                "continue with",
                "continuar com",
                "entrar",
                "login",
                "log in",
                "sign in",
                "signup",
                "sso",
            ],
        ),
        (
            &["aplicar", "application", "inscrever", "inscricao"],
            &[
                "aplicar",
                "application",
                "apply",
                "candidate se",
                "candidatar",
                "inscrever",
                "inscricao",
                "inscricoes",
            ],
        ),
        (&["baixar", "download"], &["baixar", "download"]),
        (
            &["assinar", "buy", "comprar", "preco", "subscribe"],
            &[
                "assinar",
                "buy",
                "comprar",
                "preco",
                "pricing",
                "subscribe",
                "subscription",
            ],
        ),
    ];
    categories.iter().any(|(query_terms, destination_terms)| {
        query_terms.iter().any(|term| query.contains(term))
            && destination_terms
                .iter()
                .any(|term| destination.contains(term))
    })
}

fn host_entity_label(url: &str) -> String {
    let host = normalized_host(url);
    let labels = host.split('.').collect::<Vec<_>>();
    let Some(registrable_index) = registrable_label_index(&labels) else {
        return String::new();
    };
    labels[registrable_index]
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn normalized_host(url: &str) -> &str {
    url.split("//")
        .nth(1)
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("")
        .trim_start_matches("www.")
        .trim_end_matches('.')
}

fn registrable_label_index(labels: &[&str]) -> Option<usize> {
    if labels.len() < 2 {
        return None;
    }
    // Match the registrable-domain label, never an arbitrary subdomain. The
    // old first-label rule let `python.codeutility.io` impersonate
    // `python.org` after it entered the resident index. Cover the common
    // country-code second-level namespaces conservatively; an unknown suffix
    // falls back to the label immediately before the TLD.
    let generic_second_level = ["ac", "co", "com", "edu", "gov", "mil", "net", "nom", "org"];
    let tld = labels[labels.len() - 1];
    let second_level = labels[labels.len() - 2];
    let registrable_index =
        if labels.len() >= 3 && tld.len() == 2 && generic_second_level.contains(&second_level) {
            labels.len() - 3
        } else {
            labels.len() - 2
        };
    Some(registrable_index)
}

fn is_registrable_origin(url: &str) -> bool {
    let host = normalized_host(url);
    let labels = host.split('.').collect::<Vec<_>>();
    registrable_label_index(&labels) == Some(0)
}

fn fuzzy_entity_label_match(entity: &str, host_label: &str) -> bool {
    const MIN_LABEL_LEN: usize = 5;
    if entity.len() < MIN_LABEL_LEN
        || host_label.len() < MIN_LABEL_LEN
        || !entity.bytes().all(|byte| byte.is_ascii_alphabetic())
        || !host_label.bytes().all(|byte| byte.is_ascii_alphabetic())
        || entity.as_bytes().first() != host_label.as_bytes().first()
        || entity.len().abs_diff(host_label.len()) > 1
    {
        return false;
    }
    bounded_levenshtein_one(entity.as_bytes(), host_label.as_bytes())
}

fn bounded_levenshtein_one(left: &[u8], right: &[u8]) -> bool {
    if left == right || left.len().abs_diff(right.len()) > 1 {
        return false;
    }
    if left.len() == right.len() {
        return left.iter().zip(right).filter(|(a, b)| a != b).count() == 1;
    }
    let (shorter, longer) = if left.len() < right.len() {
        (left, right)
    } else {
        (right, left)
    };
    let mut short_index = 0;
    let mut long_index = 0;
    let mut skipped = false;
    while short_index < shorter.len() && long_index < longer.len() {
        if shorter[short_index] == longer[long_index] {
            short_index += 1;
            long_index += 1;
        } else if skipped {
            return false;
        } else {
            skipped = true;
            long_index += 1;
        }
    }
    true
}

fn is_root_destination(url: &str) -> bool {
    let without_scheme = url.split("//").nth(1).unwrap_or(url);
    without_scheme.split_once('/').is_none_or(|(_, path)| {
        let path = path.trim_matches('/');
        path.is_empty()
            || (!path.contains('/')
                && (path.len() == 2
                    || (path.len() == 5 && matches!(path.as_bytes().get(2), Some(b'-' | b'_')))))
    })
}

/// Government portals often host many official entities under one authority
/// domain (`gov.br/receitafederal/pt-br`). Match the entity path only on a
/// public-authority host and only when the remaining path is a locale marker;
/// a random `/claude` page must never impersonate Claude's destination.
fn public_authority_entity_path_match(entity_compact: &str, url: &str) -> bool {
    if entity_compact.is_empty() {
        return false;
    }
    let without_scheme = url.split("//").nth(1).unwrap_or(url);
    let mut parts = without_scheme.split('/');
    let host = parts
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("")
        .trim_start_matches("www.")
        .to_ascii_lowercase();
    let public_authority = host == "gov.br"
        || host.ends_with(".gov.br")
        || host.ends_with(".gov")
        || host.contains(".gov.")
        || host.contains(".gob.");
    if !public_authority {
        return false;
    }

    let path: Vec<&str> = parts.filter(|segment| !segment.is_empty()).collect();
    let Some((index, _)) = path.iter().enumerate().find(|(_, segment)| {
        segment
            .chars()
            .filter(|character| character.is_ascii_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect::<String>()
            == entity_compact
    }) else {
        return false;
    };
    path[index + 1..]
        .iter()
        .all(|segment| is_locale_path_segment(segment))
}

fn is_locale_path_segment(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    bytes.len() == 2
        || (bytes.len() == 5
            && matches!(bytes[2], b'-' | b'_')
            && bytes[..2].iter().all(u8::is_ascii_alphabetic)
            && bytes[3..].iter().all(u8::is_ascii_alphabetic))
}

fn freshness_adjustment(result: &SearchResultItem, now: i64) -> Option<(f64, &'static str)> {
    // A topic/tag/search hub can expose the date of its newest child in JSON-LD.
    // Treating that as the hub's own publication date makes an index page beat
    // the actual article, so hubs are ineligible for the strong freshness boost.
    if is_hub_url(&result.url) {
        return None;
    }
    // A date encoded in a URL is usually day-granularity (midnight), while
    // article metadata can carry the actual publication time. Prefer the most
    // recent non-future value so midnight UTC does not make an evening article
    // look one day older a few hours after publication.
    let published_at = [date_from_url(&result.url), result.published_at]
        .into_iter()
        .flatten()
        .filter(|timestamp| *timestamp <= now.saturating_add(3_600))
        .max()?;
    let age = now.saturating_sub(published_at).max(0);
    match age {
        0..=86_400 => Some((0.26, "published_within_1d")),
        86_401..=172_800 => Some((0.20, "published_within_2d")),
        172_801..=604_800 => Some((0.14, "published_within_7d")),
        604_801..=2_592_000 => Some((0.08, "published_within_30d")),
        2_592_001..=15_552_000 => Some((0.03, "published_within_180d")),
        _ => None,
    }
}

pub(crate) fn is_hub_url(url: &str) -> bool {
    let folded = url.to_ascii_lowercase();
    [
        "/assunto/",
        "/category/",
        "/categoria/",
        "/search/",
        "/tag/",
        "/tags/",
        "/topic/",
        "/topics/",
        "/tudo-sobre/",
    ]
    .iter()
    .any(|marker| folded.contains(marker))
}

/// Whether a current-events result is a preview/announcement rather than the
/// published outcome. These pages can be fresh and contain numbers from an
/// older round, so they must not become answer evidence.
pub(crate) fn is_current_preview(result: &SearchResultItem) -> bool {
    let title = relevance_terms(&result.title).join(" ");
    let words = relevance_terms(&format!(
        "{} {} {}",
        result.title, result.snippet, result.url
    ))
    .join(" ");
    [
        "que horas sai",
        "quando sai",
        "horario pesquisa",
        "previsao",
    ]
    .iter()
    .any(|phrase| title.contains(phrase) || words.contains(phrase))
        || [
            "resultados saem",
            "resultados devem",
            "sai hoje",
            "deve sair",
            "deve ser divulgada",
            "deve ser divulgado",
            "sera divulgada",
            "sera divulgado",
            "vai divulgar",
            "will be released",
        ]
        .iter()
        .any(|phrase| words.contains(phrase))
}

fn date_from_url(url: &str) -> Option<i64> {
    let bytes = url.as_bytes();
    for window in bytes.windows(10) {
        let year = window[..4].iter().all(u8::is_ascii_digit);
        let month = window[5..7].iter().all(u8::is_ascii_digit);
        let day = window[8..10].iter().all(u8::is_ascii_digit);
        let separators =
            (window[4] == b'-' && window[7] == b'-') || (window[4] == b'/' && window[7] == b'/');
        if year && month && day && separators {
            let mut date = String::from_utf8(window.to_vec()).ok()?;
            date.replace_range(4..5, "-");
            date.replace_range(7..8, "-");
            if let Some(timestamp) = oi_crawl::extractor::parse_date_to_timestamp(&date) {
                return Some(timestamp);
            }
        }
    }
    None
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

pub(crate) fn relevance_terms(text: &str) -> Vec<String> {
    const STOP_WORDS: &[&str] = &[
        "a", "as", "da", "das", "de", "do", "dos", "e", "em", "for", "in", "o", "os", "para",
        "the", "um", "uma",
    ];
    let folded = fold_text(text);
    let mut terms = Vec::new();
    for word in folded
        .split_whitespace()
        .filter(|word| word.len() > 1 && !STOP_WORDS.contains(word))
    {
        // Portuguese users overwhelmingly use "IA" for inteligência
        // artificial. Treating the two-letter acronym as a generic token made
        // "notícias de IA" topic-empty after temporal stop-word removal, so
        // any fresh article (including election coverage) could pass.
        if word == "ia" {
            terms.push("inteligencia".to_string());
            terms.push("artificial".to_string());
        } else if word == "ai" {
            terms.push("artificial".to_string());
            terms.push("intelligence".to_string());
        } else {
            terms.push(normalize_relevance_term(word));
        }
    }
    terms
}

fn normalize_relevance_term(word: &str) -> String {
    // Small, explainable cross-language relation normalization. Semantic
    // similarity recognizes the entity, but factual ranking also needs to
    // distinguish a page that answers the predicate from an entity homepage.
    // Keep this deliberately narrow rather than pretending to be a stemmer.
    if matches!(
        word,
        "fundou"
            | "fundar"
            | "fundaram"
            | "fundado"
            | "fundada"
            | "fundados"
            | "fundadas"
            | "fundador"
            | "fundadora"
            | "fundadores"
            | "fundadoras"
            | "cofundador"
            | "cofundadora"
            | "cofundadores"
            | "cofundadoras"
            | "cofundaram"
            | "founded"
            | "founder"
            | "founders"
            | "cofounder"
            | "cofounders"
            | "founding"
    ) {
        return "founded".to_string();
    }
    if matches!(
        word,
        "criou"
            | "criar"
            | "criado"
            | "criada"
            | "criador"
            | "criadora"
            | "criadores"
            | "criadoras"
            | "criacao"
            | "criacoes"
            | "created"
            | "creator"
            | "creators"
            | "desenvolveu"
            | "desenvolver"
            | "desenvolvido"
            | "desenvolvida"
            | "desenvolvidos"
            | "desenvolvidas"
            | "developed"
            | "developer"
            | "developers"
    ) {
        return "created".to_string();
    }
    if matches!(
        word,
        "inventou"
            | "inventado"
            | "inventada"
            | "inventor"
            | "inventora"
            | "inventores"
            | "inventoras"
            | "invented"
            | "inventors"
    ) {
        // For factual retrieval, "invented" and "created" ask for the same
        // origin relation. Evidence often uses the institutional wording
        // "created/developed by" even when a user asks colloquially who
        // invented a system such as Pix.
        return "created".to_string();
    }
    if let Some(stem) = word.strip_suffix("ais")
        && stem.len() >= 4
    {
        return format!("{stem}al");
    }
    word.to_string()
}

pub(crate) fn factual_answerability_coverage(query: &str, result: &SearchResultItem) -> f64 {
    let body_prefix: String = result.content_md.chars().take(6_000).collect();
    let evidence = format!(
        "{} {} {} {}",
        result.title,
        result.snippet,
        result.content_relevant.as_deref().unwrap_or(""),
        body_prefix,
    );
    factual_answerability_coverage_text(query, &evidence)
}

pub(crate) fn factual_query_terms(query: &str) -> Vec<String> {
    const QUESTION_WORDS: &[&str] = &[
        "como", "onde", "por", "porque", "qual", "quais", "quando", "quem", "what", "when",
        "where", "who", "why",
    ];
    const AUXILIARIES: &[&str] = &["era", "esta", "estao", "foi", "is", "sao", "was", "were"];
    const ANSWER_TYPE_WORDS: &[&str] = &[
        "company",
        "empresa",
        "institution",
        "instituicao",
        "organization",
        "organizacao",
        "orgao",
    ];

    relevance_terms(query)
        .into_iter()
        .filter(|term| {
            !QUESTION_WORDS.contains(&term.as_str())
                && !AUXILIARIES.contains(&term.as_str())
                && !ANSWER_TYPE_WORDS.contains(&term.as_str())
        })
        .collect()
}

pub(crate) fn factual_answerability_coverage_text(query: &str, evidence: &str) -> f64 {
    let query_terms = factual_query_terms(query);
    if query_terms.len() < 2 {
        return 0.0;
    }
    let candidate_terms = relevance_terms(evidence);
    let matches = query_terms
        .iter()
        .filter(|query_term| candidate_terms.contains(query_term))
        .count();
    matches as f64 / query_terms.len() as f64
}

fn fold_text(text: &str) -> String {
    let mut folded = String::with_capacity(text.len());
    let mut previous_space = true;
    for character in text.chars() {
        let character = match character.to_ascii_lowercase() {
            'Á' | 'À' | 'Â' | 'Ã' | 'Ä' | 'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
            'É' | 'È' | 'Ê' | 'Ë' | 'é' | 'è' | 'ê' | 'ë' => 'e',
            'Í' | 'Ì' | 'Î' | 'Ï' | 'í' | 'ì' | 'î' | 'ï' => 'i',
            'Ó' | 'Ò' | 'Ô' | 'Õ' | 'Ö' | 'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
            'Ú' | 'Ù' | 'Û' | 'Ü' | 'ú' | 'ù' | 'û' | 'ü' => 'u',
            'Ç' | 'ç' => 'c',
            value => value,
        };
        if character.is_alphanumeric() {
            folded.push(character);
            previous_space = false;
        } else if !previous_space {
            folded.push(' ');
            previous_space = true;
        }
    }
    folded.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn development_origin_questions_are_not_development_topic_queries() {
        assert_eq!(
            classify_query("who developed ExampleSoft"),
            QueryIntent::Factual
        );
        assert_eq!(
            classify_query("quem desenvolveu ExampleSoft"),
            QueryIntent::Factual
        );
        assert_eq!(
            classify_query("how do developed economies compare"),
            QueryIntent::Exploratory
        );
    }

    #[test]
    fn explicit_web_destinations_are_language_neutral() {
        for query in [
            "Apache Maven official website",
            "Krita site oficial",
            "OpenStreetMap official homepage",
        ] {
            assert_eq!(classify_query(query), QueryIntent::Navigational);
        }
        assert_eq!(navigation_entity_key("Krita official website"), "krita");
        assert_eq!(
            classify_query("what is an official website"),
            QueryIntent::Exploratory
        );
        assert_eq!(
            classify_query("is this official website a scam"),
            QueryIntent::DueDiligence
        );
    }

    #[test]
    fn definitions_do_not_trigger_live_event_audits() {
        for query in [
            "o que significa resultado de uma eleição",
            "como identificar o período de uma pesquisa eleitoral",
            "what does an election result mean",
        ] {
            assert_eq!(classify_query(query), QueryIntent::Exploratory);
        }
        assert_eq!(
            classify_query("resultado da eleição hoje"),
            QueryIntent::CurrentEvents
        );
        assert_eq!(
            classify_query("quem é o presidente atualmente"),
            QueryIntent::CurrentEvents
        );
    }

    #[test]
    fn mutable_release_discovery_is_not_forced_into_news() {
        for query in [
            "latest stable release of Elixir",
            "versão estável atual do FreeBSD",
            "latest release of JupyterLab",
        ] {
            assert!(current_query_allows_undated_display(query));
            assert_eq!(classify_query(query), QueryIntent::CurrentEvents);
        }
        assert!(!current_query_allows_undated_display(
            "latest news about an election"
        ));
    }

    fn result(url: &str, title: &str) -> SearchResultItem {
        SearchResultItem {
            result_id: url.to_string(),
            rank: 0,
            url: url.to_string(),
            title: title.to_string(),
            snippet: String::new(),
            content_md: String::new(),
            semantic_score: 0.5,
            bm25_score: 0.0,
            final_score: 0.5,
            stake_truth: 0,
            stake_usd: 0,
            stake_type: "domain".to_string(),
            reputation: 0.0,
            content_hash: String::new(),
            extraction_quality: "snippet".to_string(),
            source: "test".to_string(),
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
    fn classifies_representative_queries() {
        assert_eq!(classify_query("claude"), QueryIntent::Navigational);
        assert_eq!(classify_query("gama fund"), QueryIntent::Navigational);
        assert_eq!(
            classify_query("compilador python online"),
            QueryIntent::Tool
        );
        assert_eq!(
            classify_query("gama fund inscrição"),
            QueryIntent::Transactional
        );
        assert_eq!(
            classify_query("gama fund é legítimo?"),
            QueryIntent::DueDiligence
        );
        assert_eq!(
            classify_query("pesquisa eleitoral hoje"),
            QueryIntent::CurrentEvents
        );
        assert_eq!(
            classify_query("última pesquisa para presidente do brasil"),
            QueryIntent::CurrentEvents
        );
        assert!(is_poll_query("última pesquisa para presidente do brasil"));
        assert!(!is_current_office_holder_query(
            "pesquisa eleitoral atual para presidente do brasil"
        ));
        assert_eq!(
            classify_query("como funciona busca vetorial?"),
            QueryIntent::Exploratory
        );
        assert_eq!(
            classify_query("o que é computação quântica"),
            QueryIntent::Exploratory
        );
        assert_eq!(
            classify_query("como declarar imposto de renda 2026"),
            QueryIntent::Exploratory
        );
        assert_eq!(
            classify_query("preço do bitcoin hoje"),
            QueryIntent::CurrentEvents
        );
        assert_eq!(
            classify_query("quem é o presidente do Brasil?"),
            QueryIntent::CurrentEvents
        );
        assert_eq!(
            classify_query("preço do Claude"),
            QueryIntent::Transactional
        );
        assert_eq!(
            classify_query("who founded Anthropic?"),
            QueryIntent::Factual
        );
        for query in [
            "computação quântica explicada",
            "restaurante japonês próximo",
            "notebook para desenvolvimento de software 2026",
            "qual laptop comprar para programação",
            "best laptop for programming 2026",
        ] {
            assert_eq!(classify_query(query), QueryIntent::Exploratory, "{query}");
        }
        assert_eq!(classify_query("comprar iPhone"), QueryIntent::Transactional);
        assert_eq!(
            classify_query("Alpha United vs Beta City"),
            QueryIntent::CurrentEvents
        );
        for query in [
            "kinetic energy vs potential energy definition",
            "comet versus asteroid meaning",
            "latency vs throughput explained",
        ] {
            assert_eq!(classify_query(query), QueryIntent::Exploratory, "{query}");
        }
        assert_eq!(
            classify_query("neurological syndrome symptoms"),
            QueryIntent::Exploratory
        );
        assert!(is_high_stakes_guidance_query(
            "neurological syndrome symptoms"
        ));
        for query in [
            "what is flornex",
            "is acme regulated",
            "40x70 foundation cost",
            "rare autoimmune condition",
            "term defined",
            "viral replication name",
            "nutrient contraindication",
        ] {
            assert_eq!(classify_query(query), QueryIntent::Exploratory, "{query}");
        }
        assert!(is_high_stakes_guidance_query("is acme regulated"));
        assert!(is_high_stakes_guidance_query("rare autoimmune condition"));
        assert_eq!(
            classify_query("meaning of luminance"),
            QueryIntent::Exploratory
        );
        assert_eq!(
            classify_query("what is cohort analysis vs case review"),
            QueryIntent::Exploratory
        );
        assert_eq!(
            classify_query("who created the song blue horizon"),
            QueryIntent::Exploratory
        );
        for query in [
            "o que é computação quântica",
            "what is quantum computing",
            "computação quântica explicada",
            "como funciona busca vetorial",
        ] {
            assert!(is_reference_exploration_query(query), "{query}");
        }
        assert!(!is_reference_exploration_query(
            "sintomas de dengue quando ir ao médico"
        ));
        for query in [
            "atual presidente do Brasil",
            "who is the current president of Brazil?",
        ] {
            assert_eq!(classify_query(query), QueryIntent::CurrentEvents, "{query}");
            assert!(is_current_office_holder_query(query), "{query}");
        }
    }

    #[test]
    fn current_event_subject_guard_rejects_cross_query_contamination() {
        let wrong = result(
            "https://sports.example/beta-game-today",
            "Beta City game today: line-up and score",
        );
        let right = result(
            "https://sports.example/alpha-game-today",
            "Alpha United game today: line-up and score",
        );

        assert_eq!(
            current_topic_coverage("Alpha United game today", &wrong),
            0.0
        );
        assert!(current_topic_coverage("Alpha United game today", &right) >= 0.50);
    }

    #[test]
    fn head_to_head_event_requires_both_named_sides() {
        let one_side = result(
            "https://sports.example/alpha-news",
            "Alpha United prepares for the next round",
        );
        let both = result(
            "https://sports.example/alpha-beta",
            "Alpha United vs Beta City: live score",
        );

        assert_eq!(
            current_topic_coverage("Alpha United vs Beta City", &one_side),
            0.0
        );
        assert!(current_topic_coverage("Alpha United vs Beta City", &both) >= 0.50);
    }

    #[test]
    fn quantum_computing_scope_preserves_compound_concept_across_languages() {
        let broad = result("https://en.wikipedia.org/wiki/Quantum", "Quantum");
        let english = result(
            "https://example.test/quantum-computing",
            "Quantum computing explained",
        );
        let portuguese = result(
            "https://example.test/computacao-quantica",
            "Computação quântica explicada",
        );

        assert_eq!(
            exploratory_topic_coverage("what is quantum computing", &broad),
            0.0
        );
        assert!(exploratory_topic_coverage("what is quantum computing", &english) >= 0.99);
        assert!(exploratory_topic_coverage("what is quantum computing", &portuguese) >= 0.99);
    }

    #[test]
    fn recommendation_scope_requires_the_product_and_its_purpose() {
        let language = result(
            "https://en.wikipedia.org/wiki/Java_(programming_language)",
            "Java (programming language)",
        );
        let compiler = result(
            "https://www.programiz.com/python-programming/online-compiler/",
            "Online Python Compiler",
        );
        let notebook = result(
            "https://reviews.test/best-laptops-for-programming",
            "Best laptops for programming",
        );

        assert_eq!(
            exploratory_topic_coverage("best laptops for programming", &language),
            0.0
        );
        assert_eq!(
            exploratory_topic_coverage("best laptops for programming", &compiler),
            0.0
        );
        assert!(exploratory_topic_coverage("best laptops for programming", &notebook) >= 0.99);

        let mut serving = vec![language, compiler, notebook];
        retain_answered_serving_scope("melhor notebook para programar em 2026", &mut serving);
        assert_eq!(serving.len(), 1);
        assert!(serving[0].url.contains("laptops"));
    }

    #[test]
    fn recommendation_demotes_a_merchants_own_buying_guide() {
        let merchant = result(
            "https://www.terabyteshop.com.br/melhor/notebook-para-programacao",
            "Melhor notebook para programação",
        );
        let editorial = result(
            "https://reviews.test/best-laptops-for-programming",
            "Best laptops for programming",
        );

        let merchant_adjustment = intent_adjustment("melhor notebook para programar", &merchant);
        let editorial_adjustment = intent_adjustment("melhor notebook para programar", &editorial);
        assert!(merchant_adjustment.score_delta < editorial_adjustment.score_delta);
        assert!(
            merchant_adjustment
                .signals
                .contains(&"merchant_first_party_surface")
        );
        assert!(
            !editorial_adjustment
                .signals
                .contains(&"merchant_first_party_surface")
        );
    }

    #[test]
    fn live_price_scope_matches_portuguese_quote_surface_for_english_query() {
        let portuguese = result(
            "https://example.test/cotacao-bitcoin",
            "Cotação do Bitcoin em tempo real",
        );
        let unrelated = result(
            "https://example.test/cotacao-dolar",
            "Cotação do dólar em tempo real",
        );

        assert!(current_topic_coverage("bitcoin price today", &portuguese) >= 0.99);
        assert!(current_topic_coverage("bitcoin price today", &unrelated) < 0.50);
    }

    #[test]
    fn exploratory_scope_rejects_semantic_leakage_from_a_mixed_index() {
        let mut results = vec![
            result(
                "https://food.test/restaurantes-japoneses",
                "Melhores restaurantes japoneses perto de você",
            ),
            result(
                "https://tax.test/imposto-de-renda-2026",
                "Como declarar imposto de renda em 2026",
            ),
        ];
        retain_intent_scope("restaurantes japoneses perto de mim", &mut results);
        assert_eq!(results.len(), 1);
        assert!(results[0].url.contains("food.test"));
    }

    #[test]
    fn exploratory_scope_accepts_paraphrased_relations_with_two_named_subjects() {
        let relevant = result(
            "https://cinema.test/alpha-beta-duel",
            "Alpha and Beta duel in the final chapter",
        );
        let unrelated = result(
            "https://cinema.test/gamma-delta",
            "Gamma and Delta appear in another series",
        );
        let query = "what movie does Alpha fight Beta in";

        assert!(exploratory_topic_coverage(query, &relevant) >= 0.50);
        assert_eq!(exploratory_topic_coverage(query, &unrelated), 0.0);
    }

    #[test]
    fn query_relative_authority_boosts_official_tax_guidance() {
        let official = intent_adjustment(
            "como declarar imposto de renda 2026",
            &result(
                "https://www.gov.br/receitafederal/pt-br/assuntos/meu-imposto-de-renda",
                "Meu Imposto de Renda",
            ),
        );
        let generic = intent_adjustment(
            "como declarar imposto de renda 2026",
            &result(
                "https://blog.test/imposto-de-renda-2026",
                "Como declarar Imposto de Renda 2026",
            ),
        );
        assert!(
            official
                .signals
                .contains(&"query_relative_primary_authority")
        );
        assert!(official.score_delta > generic.score_delta);
    }

    #[test]
    fn registered_health_entrypoint_beats_an_unregistered_public_authority() {
        let query = "sintomas de dengue quando ir ao médico";
        let registered = intent_adjustment(
            query,
            &result(
                "https://www.gov.br/saude/pt-br/assuntos/saude-de-a-a-z/d/dengue",
                "Dengue — Ministério da Saúde",
            ),
        );
        let generic_public = intent_adjustment(
            query,
            &result(
                "https://www.dengue.pr.gov.br/Pagina/Dengue-Sintomas-e-Sinais",
                "Dengue: sintomas e sinais",
            ),
        );
        assert!(
            registered
                .signals
                .contains(&"registered_authority_entrypoint")
        );
        assert!(registered.score_delta > generic_public.score_delta);
    }

    #[test]
    fn current_office_scope_rejects_candidates_and_keeps_current_office_pages() {
        let mut candidate = result(
            "https://news.test/eleicao",
            "Candidato tenta ser presidente do Brasil",
        );
        candidate.published_at = Some(unix_now());
        let mut current = result(
            "https://www.gov.br/planalto/pt-br/conheca-a-presidencia/biografia-do-presidente",
            "Biografia do Presidente da República do Brasil",
        );
        current.source = "external".to_string();
        let mut results = vec![candidate, current];
        retain_intent_scope("quem é o presidente do Brasil?", &mut results);
        assert_eq!(results.len(), 1);
        assert!(results[0].url.contains("gov.br/planalto"));
    }

    #[test]
    fn current_office_scope_accepts_english_role_and_jurisdiction_surface() {
        let mut current = result(
            "https://news.test/world/brazil-president-lula",
            "Brazil president Lula opens campaign",
        );
        current.source = "guardian".to_string();
        current.published_at = Some(unix_now());
        let mut former = result(
            "https://news.test/world/former-brazil-president",
            "Former Brazil president visits Europe",
        );
        former.source = "guardian".to_string();
        former.published_at = Some(unix_now());

        let mut results = vec![current, former];
        retain_intent_scope("quem é o presidente do Brasil?", &mut results);
        assert_eq!(results.len(), 1);
        assert!(results[0].url.contains("brazil-president-lula"));
    }

    #[test]
    fn current_office_scope_keeps_incumbent_reelection_reporting_for_name_corroboration() {
        let mut current = result(
            "https://news.test/reeleicao-lula-presidente-brasil",
            "Reeleição de Lula: o que a lei permite ao presidente do Brasil",
        );
        current.source = "guardian".to_string();
        current.published_at = Some(unix_now());

        let mut results = vec![current];
        retain_intent_scope("quem é o presidente do Brasil?", &mut results);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn answered_current_office_scope_requires_crawled_named_relation() {
        let mut named = result(
            "https://news.test/world/brazil-president-lula",
            "Brazil president Lula opens campaign",
        );
        named.source = "local".to_string();
        named.content_md =
            "Brazil president Lula opened his campaign at an event on Saturday.".to_string();
        let mut generic = result(
            "https://education.test/presidente-do-brasil",
            "Presidente do Brasil: funções e mandato",
        );
        generic.source = "local".to_string();
        generic.content_md =
            "O presidente da República é o chefe de Estado e de governo.".to_string();

        let mut results = vec![named, generic];
        retain_answered_serving_scope("quem é o presidente do Brasil?", &mut results);
        assert_eq!(results.len(), 1);
        assert!(results[0].url.contains("brazil-president-lula"));
    }

    #[test]
    fn mutable_state_discovery_seeks_an_official_office_holder_record() {
        let queries = external_discovery_queries("quem é o presidente do Brasil?");
        assert!(
            queries
                .iter()
                .any(|query| query.contains("site:gov.br/planalto"))
        );
        assert!(
            queries
                .iter()
                .any(|query| query.contains("atual presidente do Brasil"))
        );
    }

    #[test]
    fn current_office_primary_authority_is_exact_registry_entry_not_entire_domain() {
        let query = "quem é o presidente do Brasil?";
        assert!(authoritative_source_for_query(
            query,
            "https://www.gov.br/planalto/pt-br/conheca-a-presidencia/biografia-do-presidente"
        ));
        assert!(!authoritative_source_for_query(
            query,
            "https://www.gov.br/planalto/pt-br/acesso-a-informacao/institucional/gabinete-pessoal-do-presidente-da-republica/estrutura-de-pessoal"
        ));
    }

    #[test]
    fn high_stakes_guidance_has_query_relative_official_entrypoints() {
        let tax = official_state_source_urls("como declarar imposto de renda 2026");
        assert!(tax.len() >= 2);
        assert!(tax.iter().all(|url| url.contains("gov.br/receitafederal")));

        let health = official_state_source_urls("sintomas de dengue quando ir ao médico");
        assert!(health.len() >= 2);
        assert!(health.iter().all(|url| url.contains("gov.br/saude")));

        // A registry entry is a topic-specific join, not a broad domain
        // prior. Synthetic non-dengue guidance must remain on ordinary
        // discovery until a matching authority entry is registered.
        assert!(official_state_source_urls("muscle infection symptoms").is_empty());
        assert!(official_state_source_urls("cardiac therapy options").is_empty());
    }

    #[test]
    fn structural_health_claims_enter_reliance_assessment() {
        for query in [
            "how does luminex therapy treat ravel syndrome",
            "benefits of moringa and ginger",
            "respiratory infections explained",
        ] {
            assert!(is_health_guidance_query(query), "{query}");
            assert_eq!(classify_query(query), QueryIntent::Exploratory, "{query}");
        }
    }

    #[test]
    fn registered_health_entry_can_use_its_body_for_topic_scope_only() {
        let query = "sintomas de dengue quando ir ao médico";
        let mut official = result(
            "https://www.gov.br/saude/pt-br/assuntos/saude-de-a-a-z/d/dengue",
            "Dengue — Ministério da Saúde",
        );
        official.source = "local".to_string();
        official.content_md =
            "Sintomas de dengue e sinais para procurar atendimento médico.".to_string();
        let mut generic = result("https://blog.test/dengue", "Dengue");
        generic.source = "local".to_string();
        generic.content_md =
            "Sintomas de dengue e sinais para procurar atendimento médico.".to_string();

        let mut results = vec![official, generic];
        retain_intent_scope(query, &mut results);
        assert_eq!(results.len(), 1);
        assert!(results[0].url.contains("gov.br/saude"));
    }

    #[test]
    fn boosts_exact_destination_without_calling_it_trustworthy() {
        let exact = intent_adjustment("gama fund", &result("https://gamafund.com/", "Gama Fund"));
        let homonym = intent_adjustment(
            "gama fund",
            &result(
                "https://gama-asset.com/funds",
                "GAMA Asset Management Funds",
            ),
        );
        assert_eq!(exact.intent, QueryIntent::Navigational);
        assert!(exact.signals.contains(&"exact_entity_host"));
        assert!(exact.score_delta > homonym.score_delta);
    }

    #[test]
    fn branded_product_surface_can_remain_an_ambiguous_result() {
        let product = result(
            "https://zenu-nutrition.com/products/zenu-nutritional-shakes",
            "Zenu Nutritional Shakes",
        );
        let unrelated = result(
            "https://zenu-nutrition.com/news/company-update",
            "Zenu company update",
        );
        let required = vec![
            "zenu".to_string(),
            "protein".to_string(),
            "shake".to_string(),
        ];

        assert!(candidate_surface_covers_terms(&product, &required));
        assert!(!candidate_surface_covers_terms(&unrelated, &required));
        assert!(!navigation_entity_host_match(
            "zenu protein shake",
            &product
        ));
    }

    #[test]
    fn one_edit_navigation_typo_routes_to_canonical_host_only() {
        let official = result("https://gamafund.com/", "Gama Fund");
        let event = result(
            "https://luma.com/gama-fund-event",
            "Gama Fund - Founders Brasileiros",
        );
        let official_adjustment = intent_adjustment("gama found", &official);
        let event_adjustment = intent_adjustment("gama found", &event);

        assert!(official_adjustment.signals.contains(&"fuzzy_entity_host"));
        assert!(official_adjustment.score_delta > event_adjustment.score_delta + 0.25);
        assert!(navigation_entity_host_match("gama found", &official));
        assert!(local_coverage_satisfies_intent("gama found", &[official]));
        assert!(!navigation_entity_host_match("gama found", &event));
    }

    #[test]
    fn direct_navigation_guesses_cover_bounded_vowel_typos_only_for_brands() {
        assert!(
            navigation_domain_guesses("gama found")
                .iter()
                .any(|label| label == "gamafund")
        );
        assert!(
            navigation_domain_guesses("gama fnd")
                .iter()
                .any(|label| label == "gamafund")
        );
        assert!(navigation_domain_guesses("python").is_empty());
        assert_eq!(navigation_domain_guesses("claude login"), vec!["claude"]);
        assert!(navigation_domain_guesses("receita federal").is_empty());
        assert!(navigation_domain_guesses("gama fund é legítimo").is_empty());
        assert!(navigation_title_supports_entity(
            "gama fund",
            "Gama Fund — The Monashees' and Google's AI program"
        ));
        assert!(navigation_title_supports_entity(
            "gama found",
            "Gama Fund — The Monashees' and Google's AI program"
        ));
        assert!(!navigation_title_supports_entity(
            "gama fund",
            "GAMFUND.COM"
        ));
    }

    #[test]
    fn fuzzy_navigation_rejects_short_numeric_and_compound_hosts() {
        assert!(!navigation_entity_host_match(
            "paypa1",
            &result("https://paypal.com/", "PayPal")
        ));
        assert!(!navigation_entity_host_match(
            "python",
            &result("https://learnpython.org/", "Learn Python")
        ));
        assert!(!navigation_entity_host_match(
            "python",
            &result(
                "https://python.codeutility.io/pt/",
                "Compilador Python online"
            )
        ));
        assert!(!navigation_entity_host_match(
            "perplexity",
            &result("https://social.perplexity.ai/", "Perplexity Social")
        ));
        assert!(!navigation_entity_host_match(
            "nubank",
            &result("https://nuapp.nubank.com.br/login", "Nubank login")
        ));
        assert!(!navigation_entity_host_match(
            "java",
            &result("https://jaxa.com/", "Jaxa")
        ));
    }

    #[test]
    fn exact_entity_host_beats_a_compound_tutorial_host() {
        let exact = intent_adjustment(
            "python",
            &result("https://www.python.org/", "Welcome to Python.org"),
        );
        let compound = intent_adjustment(
            "python",
            &result("https://www.learnpython.org/", "Learn Python"),
        );
        assert!(exact.score_delta >= compound.score_delta + 0.30);
    }

    #[test]
    fn factual_predicate_beats_related_entity_homepage() {
        let homepage = result("https://claude.com/", "Claude by Anthropic");
        let mut answer = result(
            "https://example.test/anthropic-founders",
            "Quem são os fundadores da Anthropic",
        );
        answer.snippet = "A Anthropic foi fundada pelos irmãos Dario e Daniela Amodei.".to_string();

        let homepage_adjustment = intent_adjustment("quem fundou a Anthropic?", &homepage);
        let answer_adjustment = intent_adjustment("quem fundou a Anthropic?", &answer);

        assert_eq!(homepage_adjustment.intent, QueryIntent::Factual);
        assert!(homepage_adjustment.score_delta < 0.0);
        assert!(
            homepage_adjustment
                .signals
                .contains(&"root_lacks_factual_predicate")
        );
        assert_eq!(answer_adjustment.score_delta, 0.12);
        assert!(
            answer_adjustment
                .signals
                .contains(&"factual_terms_answered")
        );
    }

    #[test]
    fn factual_relation_normalization_matches_inflections_and_languages() {
        assert_eq!(normalize_relevance_term("fundou"), "founded");
        assert_eq!(normalize_relevance_term("fundadores"), "founded");
        assert_eq!(normalize_relevance_term("fundaram"), "founded");
        assert_eq!(normalize_relevance_term("founders"), "founded");
        assert_eq!(normalize_relevance_term("cofounder"), "founded");
        assert_eq!(normalize_relevance_term("criar"), "created");
        assert_eq!(normalize_relevance_term("desenvolvido"), "created");
        assert_eq!(normalize_relevance_term("developed"), "created");
        assert_eq!(normalize_relevance_term("inventou"), "created");
        assert_eq!(
            factual_query_terms("qual instituição criou o Pix?"),
            vec!["created", "pix"]
        );
    }

    #[test]
    fn known_sister_publications_share_a_source_family() {
        assert_eq!(
            source_family("https://valor.globo.com/financas/noticia.html"),
            source_family("https://g1.globo.com/economia/noticia.html")
        );
        assert_eq!(
            source_family("https://techtudo.com.br/noticias/pix.html"),
            source_family("https://valor.globo.com/financas/noticia.html")
        );
    }

    #[test]
    fn extracts_entity_from_transactional_query() {
        let adjustment = intent_adjustment(
            "claude login",
            &result("https://claude.com/login", "Claude Login"),
        );
        assert_eq!(adjustment.intent, QueryIntent::Transactional);
        assert!(adjustment.signals.contains(&"exact_entity_host"));
        assert!(adjustment.signals.contains(&"transaction_destination"));

        let root = intent_adjustment("claude login", &result("https://claude.com/", "Claude"));
        assert!(adjustment.score_delta > root.score_delta);
    }

    #[test]
    fn retained_root_auth_surface_beats_an_unverified_action_looking_path() {
        let mut root = result("https://claude.com/", "Claude");
        root.snippet = "Continue with Google, email, or SSO".to_string();
        let odd_path = result("https://claude.ai/login-Claude", "Claude login");
        let root_adjustment = intent_adjustment("claude login", &root);
        let path_adjustment = intent_adjustment("claude login", &odd_path);
        assert!(root_adjustment.score_delta > path_adjustment.score_delta);
        assert!(root_adjustment.signals.contains(&"embedded_action_at_root"));
        assert!(
            path_adjustment
                .signals
                .contains(&"suspicious_transaction_path")
        );
        assert!(local_coverage_satisfies_intent("claude login", &[root]));

        let mut unsafe_only = vec![odd_path];
        retain_intent_scope("claude login", &mut unsafe_only);
        assert!(unsafe_only.is_empty());
    }

    #[test]
    fn verified_action_route_beats_embedded_root_and_has_action_specific_cache_key() {
        let mut root = result("https://claude.com/", "Claude");
        root.snippet = "Continue with Google, email, or SSO".to_string();
        let mut direct = result("https://claude.ai/login", "Claude login");
        direct.extraction_quality = "verified_transaction_route".to_string();

        let root_adjustment = intent_adjustment("claude login", &root);
        let direct_adjustment = intent_adjustment("claude login", &direct);
        assert!(direct_adjustment.score_delta > root_adjustment.score_delta);
        assert!(
            direct_adjustment
                .signals
                .contains(&"verified_transaction_route")
        );
        assert_eq!(
            transaction_destination_key("entrar claude").as_deref(),
            Some("transaction:claude:login")
        );
        assert_eq!(transaction_route_paths("claude login"), &["/login"]);
        assert_ne!(
            transaction_destination_key("claude download"),
            transaction_destination_key("claude login")
        );
    }

    #[test]
    fn transaction_surface_must_match_the_requested_action_category() {
        assert!(!transaction_destination_matches_query(
            "claude login",
            "download desktop app"
        ));
        assert!(transaction_destination_matches_query(
            "claude login",
            "continue with google or sso"
        ));
        assert!(transaction_destination_matches_query(
            "gama fund inscricao",
            "candidate se inscricoes abertas"
        ));
    }

    #[test]
    fn due_diligence_does_not_boost_self_declared_destination() {
        let adjustment = intent_adjustment(
            "gama fund é legítimo?",
            &result("https://gamafund.com/", "Gama Fund"),
        );
        assert_eq!(adjustment.intent, QueryIntent::DueDiligence);
        assert_eq!(adjustment.score_delta, 0.0);
        assert!(adjustment.signals.is_empty());
    }

    #[test]
    fn due_diligence_removes_the_entitys_own_site_from_evidence_results() {
        let mut results = vec![
            result("https://gamafund.com/", "Gama Fund"),
            result(
                "https://news.test/gama-fund-google-monashees",
                "Gama Fund de Google e Monashees",
            ),
        ];
        retain_intent_scope("gama fund é legítimo?", &mut results);
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].url,
            "https://news.test/gama-fund-google-monashees"
        );
    }

    #[test]
    fn due_diligence_rejects_a_homonymous_fund_ticker() {
        let mut results = vec![
            result(
                "https://uk.finance.yahoo.com/quote/BRAMGAMAFIMC.SA/holdings/",
                "GAMA FUNDO DE INVESTIMENTO MULTIMERCADO holdings",
            ),
            result(
                "https://startups.com.br/negocios/gama-fund-google-monashees/",
                "Gama Fund de Google e Monashees estende inscrições",
            ),
        ];
        retain_intent_scope("gama fund é legítimo?", &mut results);
        assert_eq!(results.len(), 1);
        assert!(results[0].url.contains("startups.com.br"));
    }

    #[test]
    fn due_diligence_disambiguates_the_named_entity_from_homonymous_funds() {
        let intended = intent_adjustment(
            "gama fund é legítimo?",
            &result(
                "https://news.test/gama-fund-google-monashees",
                "Gama Fund de Google e Monashees",
            ),
        );
        let homonym = intent_adjustment(
            "gama fund é legítimo?",
            &result(
                "https://gama-am.ch/en/bond-funds/",
                "Our funds — GAMA Asset Management",
            ),
        );
        assert!(intended.score_delta > homonym.score_delta);
        assert!(intended.signals.contains(&"diligence_entity_phrase"));
        assert!(homonym.signals.contains(&"diligence_entity_mismatch"));
    }

    #[test]
    fn freshness_does_not_make_an_unrelated_current_article_relevant() {
        let now = unix_now();
        let mut election = result(
            "https://news.test/2026/08/21/pesquisa-eleitoral",
            "Pesquisa eleitoral divulgada hoje",
        );
        election.published_at = Some(now);
        let mut ai = result(
            "https://tech.test/2026/08/21/inteligencia-artificial",
            "Novidades de inteligência artificial hoje",
        );
        ai.published_at = Some(now);

        let election_adjustment = intent_adjustment(
            "últimas notícias de inteligência artificial hoje",
            &election,
        );
        let ai_adjustment =
            intent_adjustment("últimas notícias de inteligência artificial hoje", &ai);
        assert!(election_adjustment.score_delta < 0.0);
        assert!(
            election_adjustment
                .signals
                .contains(&"current_topic_mismatch")
        );
        assert!(ai_adjustment.score_delta > election_adjustment.score_delta);
    }

    #[test]
    fn current_topic_coverage_ignores_unrelated_body_and_footer_copy() {
        let mut election = result(
            "https://news.test/2026/08/21/pesquisa-eleitoral",
            "Pesquisa eleitoral divulgada hoje",
        );
        election.content_relevant =
            Some("Leia também as últimas notícias de inteligência artificial hoje".to_string());
        election.source = "local".to_string();
        election.snippet = "Rodapé: leia também notícias de inteligência artificial".to_string();
        assert!(
            current_topic_coverage(
                "últimas notícias de inteligência artificial hoje",
                &election
            ) < 0.25
        );
        assert!(is_poll_query("pesquisa eleitoral presidente hoje"));
        assert!(!is_poll_query(
            "últimas notícias de inteligência artificial hoje"
        ));
    }

    #[test]
    fn portuguese_ai_acronym_keeps_current_event_topic_scope() {
        let election = result(
            "https://news.test/2026/08/21/pesquisa-eleitoral",
            "Pesquisa eleitoral divulgada hoje",
        );
        let ai = result(
            "https://tech.test/2026/08/21/inteligencia-artificial",
            "Notícias de inteligência artificial",
        );
        assert_eq!(
            relevance_terms("últimas notícias sobre IA"),
            vec!["ultimas", "noticias", "sobre", "inteligencia", "artificial"]
        );
        assert_eq!(
            relevance_terms("latest AI news"),
            vec!["latest", "artificial", "intelligence", "news"]
        );
        assert!(current_topic_coverage("últimas notícias sobre IA", &election) < 0.25);
        assert!(current_topic_coverage("últimas notícias sobre IA", &ai) >= 0.50);
        let english_ai = result(
            "https://tech.test/2026/08/21/ai-news",
            "Latest AI news today",
        );
        assert!(current_topic_coverage("latest artificial intelligence news", &english_ai) >= 0.50);
    }

    #[test]
    fn current_event_scope_drops_undated_local_evergreen_pages() {
        let mut stale = result(
            "https://reference.test/wiki/artificial-intelligence",
            "Artificial intelligence encyclopedia",
        );
        stale.source = "local".to_string();
        let mut fresh = result(
            "https://tech.test/artificial-intelligence-news",
            "Latest artificial intelligence news",
        );
        fresh.source = "local".to_string();
        fresh.published_at = Some(unix_now());
        let mut results = vec![stale, fresh];
        retain_intent_scope("latest artificial intelligence news", &mut results);
        assert_eq!(results.len(), 1);
        assert!(results[0].url.contains("tech.test"));
    }

    #[test]
    fn undated_external_current_lead_can_be_crawled_but_not_served() {
        let lead = result(
            "https://tech.test/artificial-intelligence-breaking-news",
            "Latest artificial intelligence news",
        );
        let mut discovery = vec![lead.clone()];
        retain_intent_scope("latest artificial intelligence news today", &mut discovery);
        assert_eq!(discovery.len(), 1);

        let mut serving = vec![lead];
        retain_answered_serving_scope("latest artificial intelligence news today", &mut serving);
        assert!(serving.is_empty());
    }

    #[test]
    fn freshly_crawled_mutable_state_page_can_be_served_without_article_date() {
        let mut official = result(
            "https://www.gov.br/planalto/pt-br/conheca-a-presidencia/biografia-do-presidente",
            "Biografia do Presidente da República — Planalto",
        );
        official.source = "local".to_string();
        official.content_md =
            "Página institucional do ocupante da Presidência da República.".to_string();
        official.crawled_at = Some(unix_now() as u64);
        let mut results = vec![official];

        retain_intent_scope("quem é o presidente do Brasil?", &mut results);
        retain_answered_serving_scope("quem é o presidente do Brasil?", &mut results);

        assert_eq!(results.len(), 1);
    }

    #[test]
    fn factual_serving_drops_entity_pages_that_do_not_expose_the_relation() {
        let support = result(
            "https://support.claude.com/pt/articles/login",
            "Faça login em sua conta Claude | Anthropic Help Center",
        );
        let mut founder = result(
            "https://reference.test/anthropic-founders",
            "Who founded Anthropic?",
        );
        founder.content_md = "Anthropic was founded by former OpenAI researchers.".to_string();
        let mut results = vec![support, founder];

        retain_answered_serving_scope("who founded Anthropic?", &mut results);

        assert_eq!(results.len(), 1);
        assert!(results[0].url.contains("anthropic-founders"));
    }

    #[test]
    fn factual_discovery_can_use_an_entity_relation_in_a_provider_snippet() {
        let mut profile = result("https://en.wikipedia.org/wiki/Dario_Amodei", "Dario Amodei");
        profile.source = "wikipedia".to_string();
        profile.snippet = "In 2021, he and Daniela Amodei co-founded Anthropic.".to_string();
        let unrelated = result("https://en.wikipedia.org/wiki/Dario_Fo", "Dario Fo");
        let mut results = vec![profile, unrelated];

        retain_intent_scope("who founded Anthropic?", &mut results);

        assert_eq!(results.len(), 1);
        assert!(results[0].url.contains("Dario_Amodei"));
    }

    #[test]
    fn url_date_is_materialized_as_result_metadata() {
        let mut results = vec![result(
            "https://tech.test/2026/08/21/artificial-intelligence-news",
            "Artificial intelligence news",
        )];
        apply_intent_adjustments("latest artificial intelligence news", &mut results);
        assert!(results[0].published_at.is_some());
    }

    #[test]
    fn explicit_today_query_drops_articles_older_than_one_week() {
        let mut recent = result(
            "https://tech.test/artificial-intelligence-yesterday",
            "Notícias de inteligência artificial",
        );
        recent.source = "local".to_string();
        recent.published_at = Some(unix_now() - 86_400);
        let mut old = result(
            "https://tech.test/artificial-intelligence-last-month",
            "Notícias de inteligência artificial",
        );
        old.source = "local".to_string();
        old.published_at = Some(unix_now() - 20 * 86_400);
        let mut results = vec![old, recent];
        retain_intent_scope(
            "últimas notícias de inteligência artificial hoje",
            &mut results,
        );
        assert_eq!(results.len(), 1);
        assert!(results[0].url.contains("yesterday"));
    }

    #[test]
    fn homonyms_do_not_satisfy_navigational_coverage() {
        let homonyms = vec![
            result("https://example.com/georges-claude", "Georges Claude"),
            result("https://example.org/claude-shannon", "Claude Shannon"),
        ];
        assert!(!local_coverage_satisfies_intent("claude", &homonyms));

        let mut with_destination = homonyms;
        with_destination.push(result("https://claude.com/", "Claude"));
        assert!(local_coverage_satisfies_intent("claude", &with_destination));

        let download_only = vec![result("https://claude.com/download", "Download Claude")];
        assert!(!local_coverage_satisfies_intent("claude", &download_only));
    }

    #[test]
    fn intent_scope_drops_nearest_neighbours_that_do_not_cover_the_query() {
        let mut navigational = vec![
            result(
                "https://news.test/anthropic",
                "Anthropic criada por ex-membros da OpenAI",
            ),
            result(
                "https://blog.nubank.test/login",
                "Como acessar sua conta Nubank",
            ),
        ];
        retain_intent_scope("python", &mut navigational);
        assert!(navigational.is_empty());

        let mut footer_only = result(
            "https://news.test/election",
            "Pesquisa eleitoral divulgada hoje",
        );
        footer_only.content_md = "Leia também: novidades sobre ChatGPT".to_string();
        let mut navigational = vec![footer_only];
        retain_intent_scope("chatgpt", &mut navigational);
        assert!(navigational.is_empty());

        let mut typo_destination = vec![result("https://gamafund.com/", "Gama Fund")];
        retain_intent_scope("gama found", &mut typo_destination);
        assert_eq!(typo_destination.len(), 1);

        let mut factual = vec![
            result(
                "https://reference.test/anthropic",
                "Quem fundou a Anthropic?",
            ),
            result("https://reference.test/pix", "A origem do Pix"),
        ];
        retain_intent_scope("quem criou o Pix?", &mut factual);
        assert_eq!(factual.len(), 1);
        assert!(factual[0].url.ends_with("/pix"));

        let mut tangential = vec![result(
            "https://play.test/store/apps/nubank",
            "Nubank — conta e cartão",
        )];
        tangential[0].content_md = "O aplicativo permite fazer Pix.".to_string();
        retain_intent_scope("quem criou o Pix?", &mut tangential);
        assert!(tangential.is_empty());
    }

    #[test]
    fn intent_scope_keeps_paraphrased_poll_topic_results() {
        let mut poll = result(
            "https://news.test/eleicoes/levantamento",
            "Levantamento presidencial mostra intenção de voto",
        );
        poll.published_at = Some(unix_now());
        let mut results = vec![poll];
        retain_intent_scope("pesquisa eleitoral presidente brasil hoje", &mut results);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn presidential_poll_scope_rejects_an_explicit_state_government_poll() {
        let mut state = result(
            "https://news.test/politica/governo-sao-paulo",
            "Datafolha divulga nova pesquisa para o governo de São Paulo",
        );
        state.published_at = Some(unix_now());
        let mut presidential = result(
            "https://news.test/eleicoes/presidente",
            "Datafolha divulga pesquisa presidencial de primeiro turno",
        );
        presidential.published_at = Some(unix_now());
        let mut results = vec![state, presidential];
        retain_intent_scope("pesquisa eleitoral presidente brasil hoje", &mut results);
        assert_eq!(results.len(), 1);
        assert!(results[0].url.ends_with("/presidente"));
    }

    #[test]
    fn retained_exact_host_deep_page_satisfies_navigation() {
        let mut feature_page = result("https://chatgpt.com/features", "ChatGPT features");
        feature_page.content_md = "Official ChatGPT product features.".to_string();
        assert!(local_coverage_satisfies_intent("chatgpt", &[feature_page]));

        let unretained = result("https://chatgpt.com/unverified-looking-path", "ChatGPT");
        assert!(!local_coverage_satisfies_intent("chatgpt", &[unretained]));
    }

    #[test]
    fn diligence_and_current_events_require_live_discovery() {
        let candidates = vec![result("https://gamafund.com/", "Gama Fund")];
        assert!(!local_coverage_satisfies_intent(
            "gama fund é legítimo?",
            &candidates
        ));
        assert!(!local_coverage_satisfies_intent(
            "pesquisa eleitoral hoje",
            &candidates
        ));
        assert!(!local_coverage_satisfies_intent(
            "quem fundou a Anthropic?",
            &candidates
        ));
    }

    #[test]
    fn current_events_can_use_a_five_minute_multi_domain_snapshot() {
        let now = unix_now();
        let mut first = result(
            "https://news-one.test/politica/pesquisa",
            "Pesquisa eleitoral divulgada hoje",
        );
        first.crawled_at = Some(now as u64);
        first.published_at = Some(now);
        let mut second = result(
            "https://news-two.test/eleicoes/levantamento",
            "Novo levantamento presidencial",
        );
        second.crawled_at = Some(now as u64);
        second.published_at = Some(now);

        assert!(local_coverage_satisfies_intent(
            "pesquisa eleitoral hoje",
            &[first.clone(), second]
        ));
        first.crawled_at = Some((now - 301) as u64);
        assert!(!local_coverage_satisfies_intent(
            "pesquisa eleitoral hoje",
            &[first]
        ));
    }

    #[test]
    fn generic_news_snapshot_requires_three_fresh_source_families() {
        let now = unix_now();
        let fresh = |url: &str, title: &str| {
            let mut item = result(url, title);
            item.crawled_at = Some(now as u64);
            item.published_at = Some(now);
            item
        };
        let first = fresh(
            "https://news-one.test/ai/update",
            "Notícias de inteligência artificial hoje",
        );
        let second = fresh(
            "https://news-two.test/tecnologia/ai",
            "Últimas notícias sobre inteligência artificial",
        );
        let third = fresh(
            "https://news-three.test/inovacao/ai",
            "Inteligência artificial: novidades de hoje",
        );
        let query = "últimas notícias de inteligência artificial hoje";
        assert!(!local_coverage_satisfies_intent(
            query,
            &[first.clone(), second.clone()]
        ));
        assert!(local_coverage_satisfies_intent(
            query,
            &[first, second, third]
        ));
    }

    #[test]
    fn sister_domains_do_not_satisfy_independent_current_coverage() {
        let now = unix_now();
        let mut g1 = result(
            "https://g1.globo.com/politica/pesquisa.html",
            "Pesquisa divulgada hoje",
        );
        g1.crawled_at = Some(now as u64);
        g1.published_at = Some(now);
        let mut oglobo = result(
            "https://oglobo.globo.com/politica/pesquisa.html",
            "Resultado da pesquisa presidencial",
        );
        oglobo.crawled_at = Some(now as u64);
        oglobo.published_at = Some(now);

        assert!(!local_coverage_satisfies_intent(
            "pesquisa eleitoral hoje",
            &[g1, oglobo]
        ));
    }

    #[test]
    fn poll_discovery_rewrite_seeks_answer_pages_without_changing_other_news() {
        assert_eq!(
            external_discovery_queries("pesquisa eleitoral presidente brasil hoje"),
            vec![
                "pesquisa eleitoral presidente brasil hoje",
                "pesquisa eleitoral presidente brasil hoje números resultados",
                "pesquisa eleitoral presidente brasil hoje intenção de voto primeiro turno percentuais"
            ]
        );
        assert_eq!(
            external_discovery_queries("pesquisa eleitoral atual para presidente do brasil"),
            external_discovery_queries("pesquisa eleitoral presidente brasil hoje")
        );
        assert_eq!(
            external_discovery_queries("pesquisa eleitoral hoje resultados"),
            vec![
                "pesquisa eleitoral hoje resultados",
                "pesquisa eleitoral hoje resultados intenção de voto primeiro turno percentuais"
            ]
        );
        assert_eq!(
            external_discovery_queries("últimas notícias sobre inteligência artificial"),
            vec![
                "notícias inteligência artificial hoje",
                "novidades inteligência artificial hoje"
            ]
        );
        assert_eq!(
            external_discovery_queries("últimas notícias sobre IA"),
            vec![
                "notícias inteligência artificial hoje",
                "novidades inteligência artificial hoje"
            ]
        );
        assert_eq!(
            external_discovery_queries("latest artificial intelligence news"),
            vec!["latest artificial intelligence news", "AI news today"]
        );
        assert!(prefers_global_english_news(
            "latest artificial intelligence news"
        ));
        assert!(!prefers_global_english_news(
            "últimas notícias de inteligência artificial hoje"
        ));
    }

    #[test]
    fn factual_discovery_also_seeks_a_primary_record() {
        assert_eq!(
            external_discovery_queries("quem criou o Pix?"),
            vec![
                "quem criou pix",
                "quem criou pix fonte oficial",
                "who created pix",
                "who created pix official source"
            ]
        );
        assert_eq!(
            external_discovery_queries("who founded Anthropic?"),
            vec![
                "quem fundou anthropic",
                "quem fundou anthropic fonte oficial",
                "who founded anthropic",
                "who founded anthropic official source"
            ]
        );
        assert_eq!(
            external_discovery_queries("quem são os fundadores da Anthropic?"),
            vec![
                "quem fundou anthropic",
                "quem fundou anthropic fonte oficial",
                "who founded anthropic",
                "who founded anthropic official source"
            ]
        );
        assert_eq!(
            external_discovery_queries("who created Pix?"),
            vec![
                "quem criou pix",
                "quem criou pix fonte oficial",
                "who created pix",
                "who created pix official source"
            ]
        );
        assert_eq!(
            external_discovery_queries("quem inventou o Pix?"),
            vec![
                "quem criou pix",
                "quem criou pix fonte oficial",
                "who created pix",
                "who created pix official source"
            ]
        );
    }

    #[test]
    fn destination_discovery_also_seeks_the_official_site() {
        assert_eq!(
            external_discovery_queries("gama fund"),
            vec!["gama fund", "gama fund site oficial"]
        );
        assert_eq!(
            external_discovery_queries("gama fund inscrição"),
            vec!["gama fund inscrição", "gama fund site oficial"]
        );
        assert_eq!(
            external_discovery_queries("inscrição gama fund"),
            external_discovery_queries("gama fund aplicar")
        );
        assert_eq!(
            external_discovery_queries("login claude"),
            external_discovery_queries("entrar claude")
        );
        assert_eq!(
            external_discovery_queries("claude site oficial"),
            vec!["claude", "claude site oficial"]
        );
    }

    #[test]
    fn online_tool_variants_share_discovery_and_multilingual_scope() {
        let canonical = vec!["python online compiler".to_string()];
        for query in [
            "python online compiler",
            "online python compiler",
            "compilador python online",
            "python browser compiler",
        ] {
            assert_eq!(classify_query(query), QueryIntent::Tool, "query={query}");
            assert_eq!(
                external_discovery_queries(query),
                canonical,
                "query={query}"
            );
        }

        let tool = result(
            "https://www.onlinegdb.com/online_python_compiler",
            "Online Python Compiler",
        );
        let portuguese_tool = result(
            "https://ferramentas.test/python",
            "Compilador Python no navegador",
        );
        assert!(online_tool_surface_matches(
            "python online compiler",
            &portuguese_tool
        ));
        let keyword_domain = result(
            "https://onlinepythoncompiler.com/en",
            "Python Online Compiler",
        );
        assert!(!navigation_entity_host_match(
            "online python compiler",
            &keyword_domain
        ));
        assert!(
            !intent_adjustment("online python compiler", &keyword_domain)
                .signals
                .contains(&"exact_entity_host")
        );
        assert!(navigation_domain_guesses("online python compiler").is_empty());
        let homepage = result("https://www.python.org/", "Welcome to Python.org");
        let mut results = vec![tool, homepage];
        retain_intent_scope("compilador python online", &mut results);
        assert_eq!(results.len(), 1);
        assert!(results[0].url.contains("onlinegdb.com/"));

        let mut local_tools = [
            result("https://onecompiler.com/python", "Python Online Compiler"),
            result(
                "https://www.online-python.com/",
                "Online Python Interpreter",
            ),
            result(
                "https://www.programiz.com/python-programming/online-compiler/",
                "Online Python Compiler",
            ),
        ];
        for local in &mut local_tools {
            local.source = "local".to_string();
            local.content_md = "Run Python code in this online editor.".to_string();
        }
        assert!(!local_coverage_satisfies_intent(
            "python online compiler",
            &local_tools[..2]
        ));
        assert!(local_coverage_satisfies_intent(
            "python online compiler",
            &local_tools
        ));
    }

    #[test]
    fn diligence_discovery_canonicalizes_equivalent_risk_phrasings() {
        assert_eq!(
            external_discovery_queries("gama fund é fraude"),
            vec!["gama fund", "\"gama fund\" fraude golpe reclamações"]
        );
        assert_eq!(
            external_discovery_queries("gama fund é confiável"),
            external_discovery_queries("gama fund é seguro")
        );
    }

    #[test]
    fn diligence_context_uses_first_party_metadata_only_as_disambiguation() {
        assert_eq!(
            diligence_context_queries(
                "gama fund é legítimo?",
                "Gama Fund — The Monashees' and Google's AI Future Fund co-investment program"
            ),
            vec!["\"gama fund\" monashees google"]
        );
        assert!(diligence_context_queries("gama fund é legítimo?", "Gama Fund").is_empty());
    }

    #[test]
    fn diligence_context_rejects_exact_name_homonyms() {
        let context = diligence_context_terms(
            "gama fund é legítimo?",
            "Gama Fund — The Monashees' and Google's AI Future Fund co-investment program",
        );
        assert_eq!(context, vec!["monashees", "google"]);

        let intended = result(
            "https://news.test/google-gama-fund-brazil",
            "Google Gama Fund invests in Brazilian AI startups",
        );
        let homonym = result(
            "https://pitchbook.com/profiles/fund/30122-38F",
            "Gama Fund: Performance | PitchBook",
        );
        assert!(diligence_candidate_matches_context(
            "gama fund é legítimo?",
            &intended,
            &context
        ));
        assert!(!diligence_candidate_matches_context(
            "gama fund é legítimo?",
            &homonym,
            &context
        ));
    }

    #[test]
    fn factual_primary_source_recognizes_public_records_and_first_party_hosts() {
        assert!(primary_source_url_matches_query(
            "quem criou o Pix?",
            "https://www.bcb.gov.br/estabilidadefinanceira/pix-sobre"
        ));
        assert!(primary_source_url_matches_query(
            "quem fundou a Anthropic?",
            "https://www.anthropic.com/company"
        ));
        assert!(!primary_source_url_matches_query(
            "quem criou o Pix?",
            "https://example.com/quem-criou-o-pix"
        ));
    }

    #[test]
    fn factual_primary_source_must_match_the_institution_named_by_the_claim() {
        let query = "quem criou o Pix?";
        let claim = "O Pix foi criado pelo Banco Central do Brasil.";
        assert!(primary_source_url_matches_claim(
            query,
            claim,
            "https://www.bcb.gov.br/estabilidadefinanceira/pix-sobre"
        ));
        assert!(!primary_source_url_matches_claim(
            query,
            claim,
            "https://www.camara.gov.br/noticias/pix"
        ));
        assert!(!primary_source_url_matches_claim(
            query,
            claim,
            "https://www.gov.br/pt-br/noticias/pix"
        ));
        assert!(primary_source_url_matches_claim(
            "quem fundou a Anthropic?",
            "A Anthropic foi fundada por Dario e Daniela Amodei.",
            "https://www.anthropic.com/company"
        ));
    }

    #[test]
    fn locale_landing_page_counts_as_a_root_destination() {
        let adjustment = intent_adjustment("java", &result("https://www.java.com/pt_BR/", "Java"));
        assert!(adjustment.signals.contains(&"root_destination"));
    }

    #[test]
    fn official_entity_path_on_public_authority_is_a_destination() {
        let official = result("https://www.gov.br/receitafederal/pt-br", "Receita Federal");
        let impersonator = result(
            "https://example.com/receitafederal/pt-br",
            "Receita Federal",
        );

        let official_adjustment = intent_adjustment("receita federal", &official);
        let impersonator_adjustment = intent_adjustment("receita federal", &impersonator);
        assert!(
            official_adjustment
                .signals
                .contains(&"public_authority_entity_path")
        );
        assert!(
            !impersonator_adjustment
                .signals
                .contains(&"public_authority_entity_path")
        );
        assert!(local_coverage_satisfies_intent(
            "receita federal",
            &[official]
        ));
        assert!(!local_coverage_satisfies_intent(
            "receita federal",
            &[impersonator]
        ));
    }

    #[test]
    fn current_events_use_dates_encoded_in_urls() {
        let article = result(
            "https://example.com/noticias/2026/08/21/pesquisa-eleitoral.html",
            "Pesquisa eleitoral",
        );
        let now = oi_crawl::extractor::parse_date_to_timestamp("2026-08-22").unwrap();
        let adjustment = freshness_adjustment(&article, now).unwrap();
        assert_eq!(adjustment, (0.26, "published_within_1d"));
    }

    #[test]
    fn precise_metadata_beats_a_coarse_midnight_url_date() {
        let now = unix_now();
        let mut article = result(
            "https://news.test/2026/08/21/pesquisa-eleitoral",
            "Pesquisa eleitoral",
        );
        article.published_at = Some(now - 3_600);
        assert_eq!(
            freshness_adjustment(&article, now),
            Some((0.26, "published_within_1d"))
        );
    }

    #[test]
    fn topic_hub_cannot_impersonate_a_fresh_article() {
        let mut hub = result(
            "https://example.com/tudo-sobre/pesquisas-eleitorais/",
            "Pesquisas eleitorais",
        );
        hub.published_at =
            Some(oi_crawl::extractor::parse_date_to_timestamp("2026-08-21").unwrap());
        let now = oi_crawl::extractor::parse_date_to_timestamp("2026-08-22").unwrap();
        assert_eq!(freshness_adjustment(&hub, now), None);
    }
}
