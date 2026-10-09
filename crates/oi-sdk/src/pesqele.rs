//! Live adapter for Brazil's public TSE/PesqEle registry.
//!
//! PesqEle is a stateful JSF/PrimeFaces application rather than a stable JSON
//! API. This adapter deliberately follows the same public GET -> AJAX search ->
//! detail flow used by a browser. It never infers a registration from poll
//! numbers: IDs come either from retained editorial content or from a scoped
//! official listing (election + national jurisdiction). Official discovery
//! locates metadata only; editorial evidence must still support every number.

use oi_types::SearchResultItem;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Condvar, LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const LIST_URL: &str = "https://pesqele-divulgacao.tse.jus.br/app/pesquisa/listar.xhtml";
const DETAIL_URL: &str = "https://pesqele-divulgacao.tse.jus.br/app/pesquisa/detalhar.xhtml";
const SOURCE_URL: &str = LIST_URL;
const MAX_BODY_BYTES: u64 = 2 * 1024 * 1024;
const MAX_REGISTRATIONS_PER_ANSWER: usize = 4;
const POSITIVE_TTL_SECS: u64 = 24 * 60 * 60;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PesqEleRecord {
    pub registration_id: String,
    pub registered_at: String,
    pub office: String,
    pub disclosure_date: String,
    pub institute: String,
    pub election: String,
    pub respondents: String,
    pub fieldwork_start: String,
    pub fieldwork_end: String,
    pub statistician: String,
    pub statistician_registration: String,
    pub scope: String,
    pub cost: String,
    pub self_funded: String,
    pub contractors: String,
    pub methodology: String,
    pub sampling_plan: String,
    pub source_url: String,
    pub fetched_at: u64,
}

pub(crate) fn record_matches_query_scope(query: &str, record: &PesqEleRecord) -> bool {
    let query_terms = crate::query_intent::relevance_terms(query);
    let national_presidential = query_terms.iter().any(|term| term == "brasil")
        && query_terms
            .iter()
            .any(|term| matches!(term.as_str(), "presidente" | "presidencia" | "presidencial"));
    if !national_presidential {
        return true;
    }
    let methodology = format!("{} {}", record.methodology, record.sampling_plan).to_lowercase();
    let explicitly_local = [
        "eleitorado do estado",
        "eleitorado estadual",
        "eleitorado do município",
        "eleitorado do municipio",
        "universo do estado",
        "universo municipal",
    ]
    .iter()
    .any(|cue| methodology.contains(cue));
    !explicitly_local && record.scope.eq_ignore_ascii_case("BRASIL")
}

#[derive(Clone)]
struct CachedLookup {
    expires_at: Instant,
    value: Result<Option<PesqEleRecord>, String>,
}

#[derive(Default)]
struct LookupState {
    entries: HashMap<String, CachedLookup>,
    in_flight: HashSet<String>,
}

static LOOKUPS: LazyLock<(Mutex<LookupState>, Condvar)> =
    LazyLock::new(|| (Mutex::new(LookupState::default()), Condvar::new()));

#[derive(Clone)]
struct CachedDiscovery {
    expires_at: Instant,
    value: Result<Vec<RegistrationLead>, String>,
}

#[derive(Clone)]
struct RegistrationLead {
    registration_id: String,
    surface: String,
}

static BRAZIL_PRESIDENTIAL_DISCOVERY: LazyLock<Mutex<HashMap<String, CachedDiscovery>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Discover recent national registrations for the current general election
/// directly from the public registry. This is a bounded recall hint used to
/// find editorial result pages when a generic SERP has already shifted to
/// demographic breakdowns or announcement pages. It does not attest to any
/// reported percentage.
#[cfg(test)]
pub(crate) fn discover_brazil_presidential_registration_ids() -> Result<Vec<String>, String> {
    discover_brazil_presidential_registration_ids_for_context(&[])
}

pub(crate) fn discover_brazil_presidential_registration_ids_for_context(
    context_terms: &[String],
) -> Result<Vec<String>, String> {
    let mut cache_terms = context_terms.to_vec();
    cache_terms.sort();
    cache_terms.dedup();
    let cache_key = cache_terms.join("\u{1f}");
    if let Ok(cache) = BRAZIL_PRESIDENTIAL_DISCOVERY.lock()
        && let Some(cached) = cache.get(&cache_key)
        && cached.expires_at > Instant::now()
    {
        return cached
            .value
            .clone()
            .map(|leads| registration_ids_matching_context(leads, context_terms));
    }
    let value = discover_brazil_presidential_registration_ids_live(context_terms);
    let ttl = if value.is_ok() {
        Duration::from_secs(10 * 60)
    } else {
        Duration::from_secs(30)
    };
    if let Ok(mut cache) = BRAZIL_PRESIDENTIAL_DISCOVERY.lock() {
        cache.insert(
            cache_key,
            CachedDiscovery {
                expires_at: Instant::now() + ttl,
                value: value.clone(),
            },
        );
    }
    value.map(|leads| registration_ids_matching_context(leads, context_terms))
}

fn registration_ids_matching_context(
    leads: Vec<RegistrationLead>,
    context_terms: &[String],
) -> Vec<String> {
    let matching = leads
        .iter()
        .filter(|lead| {
            let surface_terms = crate::query_intent::relevance_terms(&lead.surface);
            context_terms
                .iter()
                .any(|term| surface_terms.iter().any(|surface| surface == term))
        })
        .map(|lead| lead.registration_id.clone())
        .collect::<Vec<_>>();
    if matching.is_empty() {
        leads.into_iter().map(|lead| lead.registration_id).collect()
    } else {
        matching
    }
}

fn discover_brazil_presidential_registration_ids_live(
    context_terms: &[String],
) -> Result<Vec<RegistrationLead>, String> {
    let first = discover_brazil_presidential_registration_ids_live_once(context_terms);
    match first {
        Ok(leads) => Ok(leads),
        Err(error)
            if error.contains("returned no registrations")
                || error.contains("did not contain a form update") =>
        {
            // PrimeFaces occasionally returns a valid 200 response with an
            // empty/stale partial update while the next fresh JSF session
            // succeeds. Retry exactly once with a new cookie jar and
            // ViewState; never loop or reuse the empty response as evidence.
            discover_brazil_presidential_registration_ids_live_once(context_terms)
                .map_err(|retry| format!("{error}; fresh-session retry failed: {retry}"))
        }
        Err(error) => Err(error),
    }
}

fn discover_brazil_presidential_registration_ids_live_once(
    context_terms: &[String],
) -> Result<Vec<RegistrationLead>, String> {
    let agent = ureq::Agent::new_with_config(
        ureq::config::Config::builder()
            .timeout_global(Some(Duration::from_secs(12)))
            .build(),
    );
    let list_html = get_text(&agent, LIST_URL)?;
    let list_document = Html::parse_document(&list_html);
    let form_selector = selector("form#formPesquisa")?;
    let form = list_document
        .select(&form_selector)
        .next()
        .ok_or_else(|| "PesqEle search form not found".to_string())?;
    let action = form
        .value()
        .attr("action")
        .map(absolute_pesqele_url)
        .transpose()?
        .unwrap_or_else(|| LIST_URL.to_string());
    let view_state = form_value(&form, "javax.faces.ViewState")
        .ok_or_else(|| "PesqEle JSF ViewState not found".to_string())?;
    let election =
        select_option_value(&form, "formPesquisa:eleicoes_input", "Eleições Gerais 2026")
            .ok_or_else(|| "PesqEle current general election option not found".to_string())?;
    let brazil = select_option_value(&form, "formPesquisa:filtroUF_input", "BRASIL")
        .ok_or_else(|| "PesqEle national jurisdiction option not found".to_string())?;
    let company =
        select_option_value_for_context(&form, "formPesquisa:empresas_input", context_terms);
    let search_button = form
        .select(&selector("a[id]")?)
        .find(|anchor| normalized_text(anchor.text()).contains("Pesquisar"))
        .and_then(|anchor| anchor.value().attr("id"))
        .ok_or_else(|| "PesqEle search action not found".to_string())?
        .to_string();

    let mut search_form = form_values(&form)?;
    set_form_value(&mut search_form, "formPesquisa:eleicoes_input", &election);
    set_form_value(&mut search_form, "formPesquisa:filtroUF_input", &brazil);
    if let Some(company) = company {
        set_form_value(&mut search_form, "formPesquisa:empresas_input", &company);
    }
    set_form_value(&mut search_form, "formPesquisa", "formPesquisa");
    set_form_value(&mut search_form, "javax.faces.partial.ajax", "true");
    set_form_value(&mut search_form, "javax.faces.source", &search_button);
    set_form_value(&mut search_form, "javax.faces.partial.execute", "@all");
    set_form_value(
        &mut search_form,
        "javax.faces.partial.render",
        "formPesquisa",
    );
    set_form_value(&mut search_form, &search_button, &search_button);
    set_form_value(&mut search_form, "javax.faces.ViewState", &view_state);

    let search_xml = post_form(&agent, &action, &search_form, LIST_URL)?;
    let search_fragment = first_cdata(&search_xml)
        .ok_or_else(|| "PesqEle discovery response did not contain a form update".to_string())?;
    let mut leads = parse_registration_leads(search_fragment)?;
    let mut page_view_state = partial_view_state(&search_xml).unwrap_or(view_state);
    // The registry renders ten rows at a time while the public query is
    // capped at fifty. Walk the bounded result set so an institute registered
    // several days earlier is not hidden by newer, not-yet-reported studies.
    for page in 1..5 {
        let first = page * 10;
        let table = "formPesquisa:tabelaPesquisas";
        let pagination_form = vec![
            ("formPesquisa".to_string(), "formPesquisa".to_string()),
            ("javax.faces.partial.ajax".to_string(), "true".to_string()),
            ("javax.faces.source".to_string(), table.to_string()),
            ("javax.faces.partial.execute".to_string(), table.to_string()),
            ("javax.faces.partial.render".to_string(), table.to_string()),
            ("javax.faces.behavior.event".to_string(), "page".to_string()),
            ("javax.faces.partial.event".to_string(), "page".to_string()),
            (format!("{table}_pagination"), "true".to_string()),
            (format!("{table}_first"), first.to_string()),
            (format!("{table}_rows"), "10".to_string()),
            (format!("{table}_skipChildren"), "true".to_string()),
            (format!("{table}_encodeFeature"), "true".to_string()),
            ("javax.faces.ViewState".to_string(), page_view_state.clone()),
        ];
        let page_xml = post_form(&agent, &action, &pagination_form, LIST_URL)?;
        let Some(page_fragment) = cdata_containing(&page_xml, "BR-") else {
            break;
        };
        let page_leads = parse_registration_leads(page_fragment)?;
        if page_leads.is_empty() {
            break;
        }
        let short_page = page_leads.len() < 10;
        for lead in page_leads {
            if !leads
                .iter()
                .any(|existing| existing.registration_id == lead.registration_id)
            {
                leads.push(lead);
            }
        }
        if let Some(updated) = partial_view_state(&page_xml) {
            page_view_state = updated;
        }
        if short_page {
            break;
        }
    }
    if leads.is_empty() {
        return Err(
            "PesqEle national presidential discovery returned no registrations".to_string(),
        );
    }
    leads.truncate(50);
    Ok(leads)
}

#[cfg(test)]
fn parse_presidential_registration_ids(html: &str) -> Result<Vec<String>, String> {
    Ok(parse_registration_leads(html)?
        .into_iter()
        .map(|lead| lead.registration_id)
        .collect())
}

fn parse_registration_leads(html: &str) -> Result<Vec<RegistrationLead>, String> {
    // PrimeFaces pagination updates contain bare row siblings. HTML5 parsing
    // drops those outside a table context, so provide the minimal container
    // for page fragments while leaving the initial full form alone.
    let wrapped;
    let source = if html.trim_start().starts_with("<tr") {
        wrapped = format!("<table><tbody>{html}</tbody></table>");
        wrapped.as_str()
    } else {
        html
    };
    let document = Html::parse_fragment(source);
    let row_selector = selector("tr")?;
    let mut leads = Vec::new();
    for row in document.select(&row_selector) {
        let text = normalized_text(row.text());
        for id in registration_ids_in_text(&text) {
            if id.starts_with("BR-")
                && !leads
                    .iter()
                    .any(|lead: &RegistrationLead| lead.registration_id == id)
            {
                leads.push(RegistrationLead {
                    registration_id: id,
                    surface: text.clone(),
                });
            }
        }
    }
    // The live form is already scoped to UF=BR. Keep a markup-change fallback
    // for a result grid that stops using table rows, but never accept a state
    // registration in this national path.
    if leads.is_empty() {
        let text = normalized_text(document.root_element().text());
        for id in registration_ids_in_text(&text) {
            if id.starts_with("BR-")
                && !leads
                    .iter()
                    .any(|lead: &RegistrationLead| lead.registration_id == id)
                && let Some(index) = text.find(&id)
            {
                let before = text[..index].chars().count().saturating_sub(80);
                let surface = text.chars().skip(before).take(260).collect::<String>();
                leads.push(RegistrationLead {
                    registration_id: id,
                    surface,
                });
            }
        }
    }
    leads.truncate(50);
    Ok(leads)
}

/// Resolve registrations only from pages that actually support the emitted
/// claim. Other polls retained in the broad candidate set must not inflate the
/// response payload or the official-registry join.
pub(crate) fn registration_ids_for_claim_urls(
    results: &[SearchResultItem],
    claim_urls: &HashSet<String>,
) -> Vec<String> {
    collect_registration_ids(
        results
            .iter()
            .filter(|result| claim_urls.contains(&result.url)),
    )
}

fn collect_registration_ids<'a>(
    results: impl Iterator<Item = &'a SearchResultItem>,
) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut ids = Vec::new();
    for result in results.take(40) {
        for text in [
            result.content_md.as_str(),
            result.content_relevant.as_deref().unwrap_or_default(),
        ] {
            for id in registration_ids_in_text(text) {
                if seen.insert(id.clone()) {
                    ids.push(id);
                    if ids.len() == MAX_REGISTRATIONS_PER_ANSWER {
                        return ids;
                    }
                }
            }
        }
    }
    ids
}

pub(crate) fn registration_ids_for_result(result: &SearchResultItem) -> Vec<String> {
    let mut ids = registration_ids_in_text(&result.content_md);
    if let Some(relevant) = &result.content_relevant {
        for id in registration_ids_in_text(relevant) {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    ids
}

/// Start an official lookup as soon as a crawled page exposes a registration
/// ID. The lookup runs while the remaining evidence pages are still being
/// fetched; final answer construction will join the same singleflight and
/// still requires that the ID survive in retained result content.
pub(crate) fn prefetch_from_retained_text(text: &str, cache_dir: Option<PathBuf>) {
    for registration_id in registration_ids_in_text(text)
        .into_iter()
        .take(MAX_REGISTRATIONS_PER_ANSWER)
    {
        let (state_mutex, _) = &*LOOKUPS;
        let Ok(mut state) = state_mutex.lock() else {
            return;
        };
        let fresh = state
            .entries
            .get(&registration_id)
            .is_some_and(|cached| cached.expires_at > Instant::now());
        if fresh || state.in_flight.contains(&registration_id) {
            continue;
        }
        state.entries.remove(&registration_id);
        state.in_flight.insert(registration_id.clone());
        drop(state);
        let cache_dir = cache_dir.clone();
        std::thread::spawn(move || {
            let value = lookup_disk_or_live(&registration_id, cache_dir.as_deref());
            store_lookup(registration_id, value);
        });
    }
}

/// Resolve one exact registration through the live public PesqEle application.
/// Concurrent lookups for the same ID are coalesced because a single JSF flow
/// requires three network round trips and its session state cannot be shared.
#[cfg(test)]
pub(crate) fn lookup(registration_id: &str) -> Result<Option<PesqEleRecord>, String> {
    lookup_with_cache(registration_id, None)
}

pub(crate) fn lookup_with_cache(
    registration_id: &str,
    cache_dir: Option<&Path>,
) -> Result<Option<PesqEleRecord>, String> {
    let Some(registration_id) = normalize_registration_id(registration_id) else {
        return Ok(None);
    };
    let (state_mutex, changed) = &*LOOKUPS;
    let mut state = state_mutex
        .lock()
        .map_err(|_| "PesqEle lookup cache lock poisoned".to_string())?;
    loop {
        if let Some(cached) = state.entries.get(&registration_id)
            && cached.expires_at > Instant::now()
        {
            return cached.value.clone();
        }
        state.entries.remove(&registration_id);
        if !state.in_flight.contains(&registration_id) {
            state.in_flight.insert(registration_id.clone());
            break;
        }
        state = changed
            .wait(state)
            .map_err(|_| "PesqEle lookup wait lock poisoned".to_string())?;
    }
    drop(state);

    let value = lookup_disk_or_live(&registration_id, cache_dir);
    store_lookup(registration_id, value.clone());
    value
}

fn lookup_disk_or_live(
    registration_id: &str,
    cache_dir: Option<&Path>,
) -> Result<Option<PesqEleRecord>, String> {
    if let Some(record) = cache_dir.and_then(|dir| read_persistent(dir, registration_id)) {
        return Ok(Some(record));
    }
    let value = lookup_live(registration_id);
    if let (Some(cache_dir), Ok(Some(record))) = (cache_dir, &value)
        && let Err(error) = write_persistent(cache_dir, record)
    {
        tracing::warn!(
            registration_id,
            error,
            "could not persist PesqEle record cache"
        );
    }
    value
}

fn cache_file(cache_dir: &Path, registration_id: &str) -> PathBuf {
    cache_dir.join(format!("{}.json", registration_id.replace('/', "_")))
}

fn read_persistent(cache_dir: &Path, registration_id: &str) -> Option<PesqEleRecord> {
    let bytes = std::fs::read(cache_file(cache_dir, registration_id)).ok()?;
    let record: PesqEleRecord = serde_json::from_slice(&bytes).ok()?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    (record.registration_id == registration_id
        && now.saturating_sub(record.fetched_at) <= POSITIVE_TTL_SECS
        && absolute_pesqele_url(&record.source_url).is_ok())
    .then_some(record)
}

fn write_persistent(cache_dir: &Path, record: &PesqEleRecord) -> Result<(), String> {
    std::fs::create_dir_all(cache_dir)
        .map_err(|error| format!("create cache directory: {error}"))?;
    let destination = cache_file(cache_dir, &record.registration_id);
    let temporary = cache_dir.join(format!(
        ".{}.{}.tmp",
        record.registration_id.replace('/', "_"),
        std::process::id()
    ));
    let bytes = serde_json::to_vec(record).map_err(|error| format!("serialize cache: {error}"))?;
    std::fs::write(&temporary, bytes).map_err(|error| format!("write cache: {error}"))?;
    std::fs::rename(&temporary, destination).map_err(|error| format!("publish cache: {error}"))
}

fn store_lookup(registration_id: String, value: Result<Option<PesqEleRecord>, String>) {
    let ttl = match &value {
        Ok(Some(_)) => Duration::from_secs(POSITIVE_TTL_SECS),
        Ok(None) => Duration::from_secs(10 * 60),
        Err(_) => Duration::from_secs(30),
    };
    let (state_mutex, changed) = &*LOOKUPS;
    let Ok(mut state) = state_mutex.lock() else {
        return;
    };
    state.in_flight.remove(&registration_id);
    state.entries.insert(
        registration_id,
        CachedLookup {
            expires_at: Instant::now() + ttl,
            value,
        },
    );
    changed.notify_all();
}

fn lookup_live(registration_id: &str) -> Result<Option<PesqEleRecord>, String> {
    let agent = ureq::Agent::new_with_config(
        ureq::config::Config::builder()
            .timeout_global(Some(Duration::from_secs(12)))
            .build(),
    );
    let list_html = get_text(&agent, LIST_URL)?;
    let list_document = Html::parse_document(&list_html);
    let form_selector = selector("form#formPesquisa")?;
    let form = list_document
        .select(&form_selector)
        .next()
        .ok_or_else(|| "PesqEle search form not found".to_string())?;
    let action = form
        .value()
        .attr("action")
        .map(absolute_pesqele_url)
        .transpose()?
        .unwrap_or_else(|| LIST_URL.to_string());
    let view_state = form_value(&form, "javax.faces.ViewState")
        .ok_or_else(|| "PesqEle JSF ViewState not found".to_string())?;
    let number_input = form
        .select(&selector("input[placeholder]")?)
        .find(|input| {
            input
                .value()
                .attr("placeholder")
                .is_some_and(|value| value.contains("Informe o n"))
        })
        .and_then(|input| input.value().attr("name"))
        .ok_or_else(|| "PesqEle registration input not found".to_string())?
        .to_string();
    let search_button = form
        .select(&selector("a[id]")?)
        .find(|anchor| normalized_text(anchor.text()).contains("Pesquisar"))
        .and_then(|anchor| anchor.value().attr("id"))
        .ok_or_else(|| "PesqEle search action not found".to_string())?
        .to_string();

    let mut search_form = form_values(&form)?;
    set_form_value(&mut search_form, &number_input, registration_id);
    set_form_value(&mut search_form, "formPesquisa", "formPesquisa");
    set_form_value(&mut search_form, "javax.faces.partial.ajax", "true");
    set_form_value(&mut search_form, "javax.faces.source", &search_button);
    set_form_value(&mut search_form, "javax.faces.partial.execute", "@all");
    set_form_value(
        &mut search_form,
        "javax.faces.partial.render",
        "formPesquisa",
    );
    set_form_value(&mut search_form, &search_button, &search_button);

    let search_xml = post_form(&agent, &action, &search_form, LIST_URL)?;
    let search_fragment = first_cdata(&search_xml)
        .ok_or_else(|| "PesqEle search response did not contain a form update".to_string())?;
    let search_document = Html::parse_fragment(search_fragment);
    let detail_selector = selector("a[aria-label=\"Visualizar dados da pesquisa\"]")?;
    let Some(detail_button) = search_document
        .select(&detail_selector)
        .find_map(|anchor| anchor.value().attr("id"))
    else {
        return Ok(None);
    };
    if !normalized_text(search_document.root_element().text()).contains(registration_id) {
        return Err(format!(
            "PesqEle returned a row that did not contain {registration_id}"
        ));
    }
    let updated_view_state = partial_view_state(&search_xml).unwrap_or(view_state);
    let detail_form = vec![
        ("javax.faces.partial.ajax".to_string(), "true".to_string()),
        ("javax.faces.source".to_string(), detail_button.to_string()),
        (
            "javax.faces.partial.execute".to_string(),
            "@all".to_string(),
        ),
        (detail_button.to_string(), detail_button.to_string()),
        ("formPesquisa".to_string(), "formPesquisa".to_string()),
        ("formPesquisa_SUBMIT".to_string(), "1".to_string()),
        ("javax.faces.ViewState".to_string(), updated_view_state),
    ];
    let detail_redirect = post_form(&agent, LIST_URL, &detail_form, LIST_URL)?;
    let redirect = partial_redirect(&detail_redirect)
        .ok_or_else(|| "PesqEle detail action did not redirect".to_string())?;
    let detail_url = absolute_pesqele_url(&redirect)?;
    if detail_url != DETAIL_URL {
        return Err(format!("unexpected PesqEle detail URL: {detail_url}"));
    }
    let detail_html = get_text(&agent, &detail_url)?;
    let record = parse_detail(&detail_html)?;
    if record.registration_id != registration_id {
        return Err(format!(
            "PesqEle detail mismatch: requested {registration_id}, received {}",
            record.registration_id
        ));
    }
    Ok(Some(record))
}

fn get_text(agent: &ureq::Agent, url: &str) -> Result<String, String> {
    let mut response = agent
        .get(url)
        .header("User-Agent", "OpenSonarX/0.1 (public TSE registry adapter)")
        .call()
        .map_err(|error| format!("PesqEle GET failed: {error}"))?;
    response
        .body_mut()
        .with_config()
        .limit(MAX_BODY_BYTES)
        .lossy_utf8(true)
        .read_to_string()
        .map_err(|error| format!("PesqEle response read failed: {error}"))
}

fn post_form(
    agent: &ureq::Agent,
    url: &str,
    form: &[(String, String)],
    referer: &str,
) -> Result<String, String> {
    let mut response = agent
        .post(url)
        .header("User-Agent", "OpenSonarX/0.1 (public TSE registry adapter)")
        .header("Faces-Request", "partial/ajax")
        .header("X-Requested-With", "XMLHttpRequest")
        .header("Origin", "https://pesqele-divulgacao.tse.jus.br")
        .header("Referer", referer)
        .send_form(
            form.iter()
                .map(|(key, value)| (key.as_str(), value.as_str())),
        )
        .map_err(|error| format!("PesqEle POST failed: {error}"))?;
    response
        .body_mut()
        .with_config()
        .limit(MAX_BODY_BYTES)
        .lossy_utf8(true)
        .read_to_string()
        .map_err(|error| format!("PesqEle response read failed: {error}"))
}

fn parse_detail(html: &str) -> Result<PesqEleRecord, String> {
    let document = Html::parse_document(html);
    let registration_id = field_text(&document, "form:lblNumeroProtocolo")
        .and_then(|value| normalize_registration_id(&value))
        .ok_or_else(|| "PesqEle detail registration ID missing".to_string())?;
    let scope = document
        .select(&selector("#print h5")?)
        .next()
        .map(|node| normalized_text(node.text()))
        .unwrap_or_default();
    let (cost, self_funded) = detail_cost_and_funding(&document)?;
    let contractors = row_value(&document, "Contratante(s):")?.unwrap_or_default();
    Ok(PesqEleRecord {
        registration_id,
        registered_at: field_text(&document, "form:lblDataRegistro").unwrap_or_default(),
        office: field_text(&document, "form:lblDescricaoCargos").unwrap_or_default(),
        disclosure_date: field_text(&document, "form:lblDataPublicacaoPesquisa")
            .unwrap_or_default(),
        institute: field_text(&document, "form:lblEmpresa").unwrap_or_default(),
        election: field_text(&document, "form:lblEleicao").unwrap_or_default(),
        respondents: field_text(&document, "form:lblEntrevistados").unwrap_or_default(),
        fieldwork_start: field_text(&document, "form:lblDataInicio").unwrap_or_default(),
        fieldwork_end: field_text(&document, "form:lblDataFinal").unwrap_or_default(),
        statistician: field_text(&document, "form:lblNomeEstatistico").unwrap_or_default(),
        statistician_registration: field_text(&document, "form:lblNumeroRegistroEstatistico")
            .unwrap_or_default(),
        scope,
        cost,
        self_funded,
        contractors,
        methodology: field_text(&document, "form:lblMetodologia").unwrap_or_default(),
        sampling_plan: field_text(&document, "form:lblPlanoAmostral").unwrap_or_default(),
        source_url: SOURCE_URL.to_string(),
        fetched_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    })
}

fn detail_cost_and_funding(document: &Html) -> Result<(String, String), String> {
    let row_selector = selector("table[id=\"form:camposPesquisa\"] tr")?;
    let cell_selector = selector("td")?;
    for row in document.select(&row_selector) {
        let cells: Vec<_> = row
            .select(&cell_selector)
            .map(|cell| normalized_text(cell.text()))
            .collect();
        if cells.len() >= 4 && cells[0] == "Valor:" {
            return Ok((cells[1].clone(), cells[3].clone()));
        }
    }
    Ok((String::new(), String::new()))
}

fn row_value(document: &Html, label: &str) -> Result<Option<String>, String> {
    let row_selector = selector("tr")?;
    let cell_selector = selector("td")?;
    for row in document.select(&row_selector) {
        let cells: Vec<_> = row
            .select(&cell_selector)
            .map(|cell| normalized_text(cell.text()))
            .collect();
        if cells.len() >= 2 && cells[0] == label {
            return Ok(Some(cells[1].clone()));
        }
    }
    Ok(None)
}

fn field_text(document: &Html, id: &str) -> Option<String> {
    let selector = Selector::parse(&format!("[id=\"{id}\"]")).ok()?;
    document
        .select(&selector)
        .next()
        .map(|node| normalized_text(node.text()))
        .filter(|value| !value.is_empty())
}

fn form_values(form: &scraper::ElementRef<'_>) -> Result<Vec<(String, String)>, String> {
    let field_selector = selector("input[name], select[name]")?;
    let option_selector = selector("option")?;
    let mut values = Vec::new();
    for field in form.select(&field_selector) {
        let Some(name) = field.value().attr("name") else {
            continue;
        };
        let value = if field.value().name() == "select" {
            field
                .select(&option_selector)
                .find(|option| option.value().attr("selected").is_some())
                .and_then(|option| option.value().attr("value"))
                .unwrap_or_default()
        } else {
            field.value().attr("value").unwrap_or_default()
        };
        set_form_value(&mut values, name, value);
    }
    Ok(values)
}

fn form_value(form: &scraper::ElementRef<'_>, name: &str) -> Option<String> {
    let selector = Selector::parse(&format!("input[name=\"{name}\"]")).ok()?;
    form.select(&selector)
        .next()
        .and_then(|input| input.value().attr("value"))
        .map(str::to_string)
}

fn select_option_value(
    form: &scraper::ElementRef<'_>,
    name: &str,
    expected_label: &str,
) -> Option<String> {
    let selector = Selector::parse(&format!("select[name=\"{name}\"] option")).ok()?;
    form.select(&selector).find_map(|option| {
        (normalized_text(option.text()) == expected_label)
            .then(|| option.value().attr("value"))
            .flatten()
            .map(str::to_string)
    })
}

fn select_option_value_for_context(
    form: &scraper::ElementRef<'_>,
    name: &str,
    context_terms: &[String],
) -> Option<String> {
    if context_terms.is_empty() {
        return None;
    }
    let selector = Selector::parse(&format!("select[name=\"{name}\"] option")).ok()?;
    form.select(&selector)
        .filter_map(|option| {
            let value = option.value().attr("value")?.trim();
            if value.is_empty() {
                return None;
            }
            let label = normalized_text(option.text());
            let label_terms = crate::query_intent::relevance_terms(&label);
            let score = context_terms
                .iter()
                .filter(|context| label_terms.iter().any(|term| term == *context))
                .count();
            (score > 0).then(|| (score, value.to_string()))
        })
        .max_by_key(|(score, _)| *score)
        .map(|(_, value)| value)
}

fn set_form_value(values: &mut Vec<(String, String)>, key: &str, value: &str) {
    if let Some(entry) = values.iter_mut().find(|(existing, _)| existing == key) {
        entry.1 = value.to_string();
    } else {
        values.push((key.to_string(), value.to_string()));
    }
}

fn first_cdata(xml: &str) -> Option<&str> {
    let start = xml.find("<![CDATA[")? + "<![CDATA[".len();
    let end = xml[start..].find("]]>")? + start;
    Some(&xml[start..end])
}

fn cdata_containing<'a>(xml: &'a str, needle: &str) -> Option<&'a str> {
    let mut remaining = xml;
    while let Some(start) = remaining.find("<![CDATA[") {
        let content_start = start + "<![CDATA[".len();
        let tail = &remaining[content_start..];
        let end = tail.find("]]>")?;
        let content = &tail[..end];
        if content.contains(needle) {
            return Some(content);
        }
        remaining = &tail[end + "]]>".len()..];
    }
    None
}

fn partial_view_state(xml: &str) -> Option<String> {
    let marker = xml.find("javax.faces.ViewState")?;
    let cdata = xml[marker..].find("<![CDATA[")? + marker + "<![CDATA[".len();
    let end = xml[cdata..].find("]]>")? + cdata;
    Some(xml[cdata..end].trim().to_string())
}

fn partial_redirect(xml: &str) -> Option<String> {
    let marker = "<redirect url=\"";
    let start = xml.find(marker)? + marker.len();
    let end = xml[start..].find('"')? + start;
    Some(xml[start..end].replace("&amp;", "&"))
}

fn absolute_pesqele_url(path: &str) -> Result<String, String> {
    if path == "/app/pesquisa/detalhar.xhtml" {
        return Ok(DETAIL_URL.to_string());
    }
    if path == "/app/pesquisa/listar.xhtml" {
        return Ok(LIST_URL.to_string());
    }
    if path.starts_with("/app/pesquisa/listar.xhtml;jsessionid=")
        && path
            .split_once(";jsessionid=")
            .is_some_and(|(_, id)| !id.is_empty() && id.bytes().all(is_session_id_byte))
    {
        return Ok(format!("https://pesqele-divulgacao.tse.jus.br{path}"));
    }
    if matches!(path, LIST_URL | DETAIL_URL) {
        return Ok(path.to_string());
    }
    Err(format!("unexpected PesqEle URL: {path}"))
}

fn is_session_id_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')
}

fn selector(value: &str) -> Result<Selector, String> {
    Selector::parse(value).map_err(|_| format!("invalid internal CSS selector: {value}"))
}

fn normalized_text<'a>(parts: impl Iterator<Item = &'a str>) -> String {
    parts
        .flat_map(str::split_whitespace)
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_registration_id(value: &str) -> Option<String> {
    registration_ids_in_text(value).into_iter().next()
}

pub(crate) fn registration_ids_in_text(text: &str) -> Vec<String> {
    let upper = text.to_ascii_uppercase();
    let bytes = upper.as_bytes();
    let mut ids = Vec::new();
    let mut index = 0;
    while index + 13 <= bytes.len() {
        let boundary_before = index == 0 || !bytes[index - 1].is_ascii_alphanumeric();
        let candidate = &bytes[index..];
        let matches = boundary_before
            && candidate[0].is_ascii_uppercase()
            && candidate[1].is_ascii_uppercase()
            && candidate[2] == b'-'
            && candidate[3..8].iter().all(u8::is_ascii_digit)
            && candidate[8] == b'/'
            && candidate[9..13].iter().all(u8::is_ascii_digit)
            && (index + 13 == bytes.len() || !bytes[index + 13].is_ascii_alphanumeric());
        if matches {
            let id = upper[index..index + 13].to_string();
            if !ids.contains(&id) {
                ids.push(id);
            }
            index += 13;
        } else {
            index += 1;
        }
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_only_canonical_registration_ids() {
        assert_eq!(
            registration_ids_in_text(
                "Registro TSE: BR-04496/2026. Compare com PA-04548/2026; ignore BR044962026."
            ),
            vec!["BR-04496/2026", "PA-04548/2026"]
        );
        assert!(registration_ids_in_text("XBR-04496/2026Y").is_empty());
    }

    #[test]
    fn official_discovery_keeps_only_national_presidential_rows() {
        let html = r#"
        <table>
          <tr><td>BR-04496/2026</td><td>Presidente</td><td>BRASIL</td></tr>
          <tr><td>SP-01806/2026</td><td>Governador</td><td>SÃO PAULO</td></tr>
          <tr><td>BR-09956/2026</td><td>Presidente</td><td>BRASIL</td></tr>
        </table>
        "#;
        assert_eq!(
            parse_presidential_registration_ids(html).unwrap(),
            vec!["BR-04496/2026", "BR-09956/2026"]
        );
    }

    #[test]
    fn official_discovery_prefers_the_institute_seen_in_editorial_results() {
        let leads = vec![
            RegistrationLead {
                registration_id: "BR-01882/2026".to_string(),
                surface: "RANKING BRASIL INTELIGENCIA LTDA".to_string(),
            },
            RegistrationLead {
                registration_id: "BR-04496/2026".to_string(),
                surface: "DATAFOLHA INSTITUTO DE PESQUISAS LTDA".to_string(),
            },
        ];
        assert_eq!(
            registration_ids_matching_context(leads, &["datafolha".to_string()]),
            vec!["BR-04496/2026"]
        );
    }

    #[test]
    #[ignore = "requires the public TSE/PesqEle service"]
    fn live_discovery_finds_current_national_presidential_registrations() {
        let ids = discover_brazil_presidential_registration_ids().unwrap();
        eprintln!("PesqEle presidential registrations: {ids:?}");
        assert!(!ids.is_empty());
        assert!(ids.iter().all(|id| id.starts_with("BR-")));
        let datafolha =
            discover_brazil_presidential_registration_ids_for_context(&["datafolha".to_string()])
                .unwrap();
        eprintln!("PesqEle Datafolha registrations: {datafolha:?}");
        assert!(datafolha.iter().any(|id| id == "BR-04496/2026"));
    }

    #[test]
    fn parses_observed_pesqele_detail_shape() {
        let html = r#"
        <div id="print"><h4>Visualizar Pesquisa Eleitoral - BR-04496/2026</h4><h5>BRASIL</h5>
        <table id="form:camposPesquisa"><tr><td>Valor:</td><td>R$ 307.641,60</td><td>Pesquisa é realizada com recursos próprios?</td><td>Não</td></tr></table>
        <span id="form:lblNumeroProtocolo">BR-04496/2026</span>
        <span id="form:lblDataRegistro">15/08/2026</span><span id="form:lblDescricaoCargos">Presidente</span>
        <span id="form:lblDataPublicacaoPesquisa">21/08/2026</span><span id="form:lblEmpresa">CNPJ: 07630546000175 - DATAFOLHA INSTITUTO DE PESQUISAS LTDA.</span>
        <span id="form:lblEleicao">Eleições Gerais 2026</span><span id="form:lblEntrevistados">2058</span>
        <span id="form:lblDataInicio">18/08/2026</span><span id="form:lblDataFinal">20/08/2026</span>
        <span id="form:lblNomeEstatistico">Renata Nunes Cesar</span><span id="form:lblNumeroRegistroEstatistico">7249A</span>
        <table><tr><td>Contratante(s):</td><td>EMPRESA FOLHA DA MANHA S.A.; GLOBO COMUNICACAO E PARTICIPACOES S/A</td></tr></table>
        <span id="form:lblMetodologia">Pesquisa quantitativa por amostragem.</span>
        <span id="form:lblPlanoAmostral">Margem de erro de 2 pontos; confiança de 95%.</span></div>"#;
        let record = parse_detail(html).expect("observed detail shape");
        assert_eq!(record.registration_id, "BR-04496/2026");
        assert_eq!(record.respondents, "2058");
        assert_eq!(record.scope, "BRASIL");
        assert_eq!(record.cost, "R$ 307.641,60");
        assert!(record.institute.contains("DATAFOLHA"));
        assert!(record.contractors.contains("GLOBO"));
        assert!(record_matches_query_scope(
            "pesquisa eleitoral presidente brasil hoje",
            &record
        ));
        let mut regional = record.clone();
        regional.methodology =
            "O eleitorado do estado do Rio de Janeiro foi tomado como universo da pesquisa."
                .to_string();
        assert!(!record_matches_query_scope(
            "pesquisa eleitoral presidente brasil hoje",
            &regional
        ));

        let cache_dir = std::env::temp_dir().join(format!(
            "opensonarx-pesqele-cache-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&cache_dir);
        write_persistent(&cache_dir, &record).expect("persist record");
        assert_eq!(
            read_persistent(&cache_dir, "BR-04496/2026")
                .expect("reload fresh record")
                .registration_id,
            "BR-04496/2026"
        );
        std::fs::remove_dir_all(&cache_dir).expect("remove isolated test cache");
    }

    #[test]
    fn accepts_only_expected_pesqele_routes() {
        assert_eq!(
            absolute_pesqele_url("/app/pesquisa/detalhar.xhtml").unwrap(),
            DETAIL_URL
        );
        assert!(absolute_pesqele_url("https://attacker.example/detail").is_err());
        assert!(absolute_pesqele_url("/app/admin/detail.xhtml").is_err());
    }

    #[test]
    #[ignore = "requires the public TSE/PesqEle service"]
    fn live_lookup_resolves_current_datafolha_registration() {
        let record = lookup("BR-04496/2026")
            .expect("live lookup")
            .expect("registered poll");
        assert_eq!(record.registration_id, "BR-04496/2026");
        assert_eq!(record.respondents, "2058");
        assert_eq!(record.scope, "BRASIL");
        assert!(record.institute.contains("DATAFOLHA"));
    }
}
