//! Offline evaluation of the answer path: model extraction, stance judgement
//! and deterministic assembly over fixed documents. No retrieval, no crawl.
//!
//! Each case supplies a request and the documents the pipeline would have
//! retained. The harness reports how often the assembled answer has the
//! expected status, how often an answer was emitted where none was supported,
//! and how many model quotes failed verbatim verification. Cases are data: a
//! new failure mode is a new line in the case file, not a new rule in code.

use oi_sdk::answer::{
    assemble_answer, build_passages, merge_repeated_claims, responsiveness_decisions,
    ClaimJudgments, SearchAnswer,
};
use oi_sdk::{AnswerModelConfig, AnswerPolicy, ChatModel};
use oi_types::{ClaimExtractor, ResponsivenessJudge, SearchResultItem, StanceJudge};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::time::Instant;

#[derive(Debug, Deserialize)]
struct Case {
    id: String,
    query: String,
    documents: Vec<Document>,
    expect: Expectation,
}

#[derive(Debug, Deserialize)]
struct Document {
    url: String,
    title: String,
    /// Retained page text. Absent for a result known only by its snippet.
    #[serde(default)]
    content: String,
    #[serde(default)]
    snippet: String,
}

#[derive(Debug, Default, Deserialize)]
struct Expectation {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    not_status: Option<String>,
    /// Every string must occur in the summary or a claim (case-insensitive).
    #[serde(default)]
    answer_contains: Vec<String>,
    /// At least one string must occur in the summary or a claim
    /// (case-insensitive). Used when a dataset lists alternative gold answers.
    #[serde(default)]
    answer_contains_any: Vec<String>,
    /// No string may occur in the summary or a claim (case-insensitive).
    #[serde(default)]
    answer_excludes: Vec<String>,
    /// At least one claim must carry this per-claim status.
    #[serde(default)]
    claim_status: Option<String>,
}

#[derive(Debug, Serialize)]
struct CaseReport {
    id: String,
    query: String,
    passed: bool,
    failures: Vec<String>,
    request_kind: String,
    status: String,
    summary: String,
    claims: Vec<ClaimReport>,
    proposed_claims: usize,
    /// What the model said the documents lack for a complete answer.
    model_reported_gaps: Vec<String>,
    rejected_quotes: usize,
    complete_answer_supported: bool,
    /// The same model outputs assembled with and without each
    /// responsiveness source. `full` is the reported pipeline.
    variants: BTreeMap<String, VariantOutcome>,
    /// For cases with alternative gold answers: whether a gold answer occurs
    /// in a span the pipeline cited as support. A diagnostic for paraphrased
    /// claims; it never decides `passed`.
    gold_in_cited_span: Option<bool>,
    model_error: Option<String>,
    latency_ms: f64,
}

#[derive(Debug, Clone, Serialize)]
struct VariantOutcome {
    status: String,
    passed: bool,
}

/// Per-variant totals over all cases.
#[derive(Debug, Default, Serialize)]
struct VariantTotals {
    answerable_correct: usize,
    answerable_other_wording: usize,
    answerable_abstained: usize,
    unanswerable_answered: usize,
}

#[derive(Debug, Serialize)]
struct ClaimReport {
    text: String,
    status: String,
    supporting_urls: Vec<String>,
    contradicting_urls: Vec<String>,
}

#[derive(Debug, Serialize)]
struct Report {
    schema_version: u32,
    cases_path: String,
    cases_sha256: String,
    model: String,
    /// Model behind the stance and responsiveness judgments.
    judge_model: String,
    think: Option<String>,
    policy_id: String,
    cases: usize,
    passed: usize,
    /// Cases expected to answer that produced the expected answer.
    answerable_cases: usize,
    answerable_passed: usize,
    /// Answerable cases where the pipeline declined to answer.
    answerable_abstained: usize,
    /// Answerable cases where an answer was emitted without a gold answer.
    answerable_wrong_answer: usize,
    /// Answerable cases with a gold answer inside a cited supporting span.
    answerable_gold_in_cited_span: usize,
    /// Cases where no answer is supported by the documents.
    unanswerable_cases: usize,
    /// Of those, cases where the pipeline still emitted an `extractive` answer.
    unsupported_answers_emitted: usize,
    rejected_quotes: usize,
    model_errors: usize,
    latency_ms_p50: f64,
    latency_ms_p95: f64,
    latency_ms_max: f64,
    concurrency: usize,
    /// Whether the responsiveness judge ran (off = the earlier pipeline).
    responsiveness_judge: bool,
    /// Claims set aside as supported but not answering the request.
    off_request_claims: usize,
    /// Totals per assembly variant, all from the same model outputs.
    variants: BTreeMap<String, VariantTotals>,
    limitations: Vec<&'static str>,
    results: Vec<CaseReport>,
}

/// The SQuAD evaluation normalization: lowercase, drop punctuation and the
/// English articles, collapse whitespace. It belongs to the dataset's metric,
/// not to the system under test.
fn normalize_answer(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || character.is_whitespace() {
                character
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .filter(|word| !matches!(*word, "a" | "an" | "the"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn contains_gold(text: &str, golds: &[String]) -> bool {
    let haystack = format!(" {} ", normalize_answer(text));
    golds.iter().any(|gold| {
        let gold = normalize_answer(gold);
        !gold.is_empty() && haystack.contains(&format!(" {gold} "))
    })
}

fn result_item(rank: usize, document: &Document) -> SearchResultItem {
    SearchResultItem {
        result_id: document.url.clone(),
        rank: rank as u32 + 1,
        url: document.url.clone(),
        title: document.title.clone(),
        snippet: if document.snippet.is_empty() {
            document.title.clone()
        } else {
            document.snippet.clone()
        },
        content_md: document.content.clone(),
        semantic_score: 0.8,
        bm25_score: 0.0,
        final_score: 0.8,
        stake_truth: 0,
        stake_usd: 0,
        stake_type: "none".to_string(),
        reputation: 0.0,
        content_hash: String::new(),
        extraction_quality: "full".to_string(),
        source: "fixture".to_string(),
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

/// What a case expected and the answer did not deliver.
fn expectation_failures(case: &Case, answer: &SearchAnswer) -> Vec<String> {
    let answer_text = std::iter::once(answer.summary.as_str())
        .chain(
            answer
                .claims
                .iter()
                .filter(|claim| claim.status != "off_request")
                .map(|claim| claim.text.as_str()),
        )
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let mut failures = Vec::new();
    if case
        .expect
        .status
        .as_ref()
        .is_some_and(|status| &answer.status != status)
    {
        failures.push(format!(
            "status {} != expected {}",
            answer.status,
            case.expect.status.as_deref().unwrap_or_default()
        ));
    }
    if case.expect.not_status.as_ref() == Some(&answer.status) {
        failures.push(format!("status must not be {}", answer.status));
    }
    for needle in &case.expect.answer_contains {
        if !answer_text.contains(&needle.to_lowercase()) {
            failures.push(format!("answer lacks {needle:?}"));
        }
    }
    let alternatives = &case.expect.answer_contains_any;
    if !alternatives.is_empty() && !contains_gold(&answer_text, alternatives) {
        failures.push(format!("answer has none of {alternatives:?}"));
    }
    for needle in &case.expect.answer_excludes {
        if answer.status == "extractive" && answer_text.contains(&needle.to_lowercase()) {
            failures.push(format!("answer asserts {needle:?}"));
        }
    }
    if let Some(status) = &case.expect.claim_status {
        if !answer.claims.iter().any(|claim| &claim.status == status) {
            failures.push(format!("no claim with status {status}"));
        }
    }
    failures
}

fn evaluate(
    case: &Case,
    model: &ChatModel,
    judge: &ChatModel,
    policy: &AnswerPolicy,
    responsiveness: bool,
) -> CaseReport {
    let started = Instant::now();
    let results: Vec<SearchResultItem> = case
        .documents
        .iter()
        .enumerate()
        .map(|(rank, document)| result_item(rank, document))
        .collect();
    let passages = build_passages(&results, policy);
    let retained: Vec<_> = passages
        .iter()
        .filter(|passage| passage.retained)
        .cloned()
        .collect();

    let mut model_error = None;
    let mut extraction = model
        .extract(&case.query, &passages)
        .unwrap_or_else(|error| {
            model_error = Some(error.to_string());
            Default::default()
        });
    let proposed_claims = extraction.claims.len();
    merge_repeated_claims(&mut extraction);
    let judgments: Option<Vec<_>> = extraction
        .claims
        .iter()
        .take(policy.max_claims)
        .map(|claim| judge.judge(&claim.text, &retained))
        .collect::<Result<_, _>>()
        .map_err(|error| model_error = Some(error.to_string()))
        .ok();
    let considered = &extraction.claims[..extraction.claims.len().min(policy.max_claims)];
    let claim_texts: Vec<&str> = considered.iter().map(|claim| claim.text.as_str()).collect();
    let verdicts = if responsiveness && !claim_texts.is_empty() {
        judge
            .answers_request(&case.query, &claim_texts)
            .map_err(|error| model_error = Some(error.to_string()))
            .ok()
    } else {
        None
    };

    // One model pass, several assemblies: each variant decides responsiveness
    // from a different subset of the same model outputs, so the variants
    // differ only by what the pipeline does with them.
    let assemble = |use_substitution: bool, use_judge: bool| {
        let mut claims = considered.to_vec();
        if !use_substitution {
            for claim in &mut claims {
                claim.substitution = None;
            }
        }
        let (decisions, _) = responsiveness_decisions(
            &case.query,
            &claims,
            verdicts.as_deref().filter(|_| use_judge),
        );
        assemble_answer(
            &results,
            &passages,
            &extraction,
            ClaimJudgments {
                stance: judgments.as_deref(),
                responsive: &decisions,
            },
            policy,
        )
    };
    let variants: BTreeMap<String, VariantOutcome> = [
        ("stance_only", false, false),
        ("plus_substitution", true, false),
        ("plus_judge", false, true),
        ("full", true, true),
    ]
    .into_iter()
    .map(|(name, use_substitution, use_judge)| {
        let (answer, _, _) = assemble(use_substitution, use_judge);
        (
            name.to_string(),
            VariantOutcome {
                passed: expectation_failures(case, &answer).is_empty(),
                status: answer.status,
            },
        )
    })
    .collect();
    let (answer, checklist, stats) = assemble(true, true);

    let mut failures = expectation_failures(case, &answer);
    if let Some(error) = &model_error {
        failures.push(format!("model error: {error}"));
    }
    let alternatives = &case.expect.answer_contains_any;
    let gold_in_cited_span = (!alternatives.is_empty()).then(|| {
        answer
            .claims
            .iter()
            .flat_map(|claim| &claim.citations)
            .filter(|citation| citation.evidence_role == "supports")
            .filter_map(|citation| citation.evidence_excerpt.as_deref())
            .any(|excerpt| contains_gold(excerpt, alternatives))
    });

    CaseReport {
        id: case.id.clone(),
        query: case.query.clone(),
        passed: failures.is_empty(),
        failures,
        request_kind: answer.intent.clone(),
        status: answer.status.clone(),
        summary: answer.summary.clone(),
        claims: answer
            .claims
            .iter()
            .map(|claim| {
                let urls = |role: &str| {
                    claim
                        .citations
                        .iter()
                        .filter(|citation| citation.evidence_role == role)
                        .map(|citation| citation.url.clone())
                        .collect()
                };
                ClaimReport {
                    text: claim.text.clone(),
                    status: claim.status.clone(),
                    supporting_urls: urls("supports"),
                    contradicting_urls: urls("contradicts"),
                }
            })
            .collect(),
        proposed_claims,
        model_reported_gaps: extraction.missing.clone(),
        rejected_quotes: stats.rejected_quotes,
        complete_answer_supported: checklist.complete_answer_supported,
        variants,
        gold_in_cited_span,
        model_error,
        latency_ms: started.elapsed().as_secs_f64() * 1_000.0,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn run(
    cases_path: &str,
    provider: &str,
    model: &str,
    judge_model: Option<&str>,
    base_url: Option<&str>,
    think: Option<&str>,
    min_source_families: Option<usize>,
    concurrency: usize,
    responsiveness: bool,
    output: &str,
) -> Result<(), String> {
    let raw = std::fs::read_to_string(cases_path)
        .map_err(|error| format!("cannot read {cases_path}: {error}"))?;
    let cases = raw
        .lines()
        .filter(|line| !line.trim().is_empty())
        .enumerate()
        .map(|(index, line)| {
            serde_json::from_str::<Case>(line)
                .map_err(|error| format!("{cases_path}:{}: {error}", index + 1))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if cases.is_empty() {
        return Err(format!("{cases_path} has no cases"));
    }

    let mut config = match provider {
        "ollama" => AnswerModelConfig::new(model),
        "openrouter" => AnswerModelConfig::openrouter(model),
        other => return Err(format!("unknown --provider {other:?} (ollama, openrouter)")),
    };
    if let Some(base_url) = base_url {
        config.base_url = Some(base_url.to_string());
    }
    config.think = think.map(str::to_string);
    let mut judge_config = config.clone();
    if let Some(judge_model) = judge_model {
        judge_config.model = judge_model.to_string();
    }
    let chat_model = ChatModel::new(config);
    let judge = ChatModel::new(judge_config);
    let mut policy = AnswerPolicy::default();
    if let Some(families) = min_source_families {
        // Single-document datasets cannot meet the default two-family rule;
        // the override is recorded in the report's policy fields.
        policy.min_source_families = families.max(1);
        policy.policy_id = format!(
            "{}+min_source_families={}",
            policy.policy_id,
            families.max(1)
        );
    }

    let concurrency = concurrency.max(1);
    let mut results = Vec::with_capacity(cases.len());
    for batch in cases.chunks(concurrency) {
        let reports: Vec<CaseReport> = std::thread::scope(|scope| {
            let workers: Vec<_> = batch
                .iter()
                .map(|case| {
                    scope.spawn(|| evaluate(case, &chat_model, &judge, &policy, responsiveness))
                })
                .collect();
            workers
                .into_iter()
                .map(|worker| worker.join().expect("evaluation worker panicked"))
                .collect()
        });
        for report in reports {
            eprintln!(
                "{} {:<28} status={:<22} {:.1}s {}",
                if report.passed { "PASS" } else { "FAIL" },
                report.id,
                report.status,
                report.latency_ms / 1_000.0,
                report
                    .failures
                    .iter()
                    .map(|failure| failure.chars().take(60).collect::<String>())
                    .collect::<Vec<_>>()
                    .join("; ")
            );
            results.push(report);
        }
    }

    let expects_answer = |case: &Case| case.expect.status.as_deref() == Some("extractive");
    let supports_no_answer = |case: &Case| case.expect.not_status.as_deref() == Some("extractive");
    let paired = || cases.iter().zip(&results);
    let mut variant_totals: BTreeMap<String, VariantTotals> = BTreeMap::new();
    for (case, result) in paired() {
        for (name, outcome) in &result.variants {
            let totals = variant_totals.entry(name.clone()).or_default();
            let answered = outcome.status == "extractive";
            if expects_answer(case) {
                match (answered, outcome.passed) {
                    (_, true) => totals.answerable_correct += 1,
                    (true, false) => totals.answerable_other_wording += 1,
                    (false, false) => totals.answerable_abstained += 1,
                }
            } else if supports_no_answer(case) && answered {
                totals.unanswerable_answered += 1;
            }
        }
    }
    let mut latencies: Vec<f64> = results.iter().map(|result| result.latency_ms).collect();
    latencies.sort_by(f64::total_cmp);
    let report = Report {
        schema_version: 1,
        cases_path: cases_path.to_string(),
        cases_sha256: Sha256::digest(raw.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
        model: ClaimExtractor::model_version(&chat_model),
        judge_model: StanceJudge::model_version(&judge),
        think: think.map(str::to_string),
        policy_id: policy.policy_id.clone(),
        cases: cases.len(),
        passed: results.iter().filter(|result| result.passed).count(),
        answerable_cases: cases.iter().filter(|case| expects_answer(case)).count(),
        answerable_passed: paired()
            .filter(|(case, result)| expects_answer(case) && result.passed)
            .count(),
        answerable_abstained: paired()
            .filter(|(case, result)| expects_answer(case) && result.status != "extractive")
            .count(),
        answerable_wrong_answer: paired()
            .filter(|(case, result)| {
                expects_answer(case) && result.status == "extractive" && !result.passed
            })
            .count(),
        answerable_gold_in_cited_span: results
            .iter()
            .filter(|result| result.gold_in_cited_span == Some(true))
            .count(),
        unanswerable_cases: cases.iter().filter(|case| supports_no_answer(case)).count(),
        unsupported_answers_emitted: paired()
            .filter(|(case, result)| supports_no_answer(case) && result.status == "extractive")
            .count(),
        rejected_quotes: results.iter().map(|result| result.rejected_quotes).sum(),
        model_errors: results
            .iter()
            .filter(|result| result.model_error.is_some())
            .count(),
        latency_ms_p50: latencies[latencies.len() / 2],
        latency_ms_p95: latencies[(latencies.len() * 95 / 100).min(latencies.len() - 1)],
        latency_ms_max: latencies[latencies.len() - 1],
        concurrency,
        responsiveness_judge: responsiveness,
        off_request_claims: results
            .iter()
            .flat_map(|result| &result.claims)
            .filter(|claim| claim.status == "off_request")
            .count(),
        variants: variant_totals,
        limitations: vec![
            "Documents are supplied; retrieval, crawling and extraction from HTML are not exercised.",
            "One model, one run; model output is not deterministic across versions or servers.",
        ],
        results,
    };
    let json = serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?;
    std::fs::write(output, json).map_err(|error| format!("cannot write {output}: {error}"))?;
    println!(
        "answer-eval: {}/{} cases as expected\n  answerable: {} correct, {} wrong answer, {} abstained (of {}); gold in cited span {}\n  unanswerable: {} answered anyway (of {})\n  rejected quotes {}; model errors {}; latency p50 {:.1}s p95 {:.1}s max {:.1}s\n  → {output}",
        report.passed,
        report.cases,
        report.answerable_passed,
        report.answerable_wrong_answer,
        report.answerable_abstained,
        report.answerable_cases,
        report.answerable_gold_in_cited_span,
        report.unsupported_answers_emitted,
        report.unanswerable_cases,
        report.rejected_quotes,
        report.model_errors,
        report.latency_ms_p50 / 1_000.0,
        report.latency_ms_p95 / 1_000.0,
        report.latency_ms_max / 1_000.0,
    );
    for (name, totals) in &report.variants {
        println!(
            "  variant {name:<18} answerable: {} correct, {} other wording, {} abstained | unanswerable answered: {}",
            totals.answerable_correct,
            totals.answerable_other_wording,
            totals.answerable_abstained,
            totals.unanswerable_answered,
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imported_squad_cases_load_as_eval_cases() {
        let raw = r#"{"data":[{"title":"T","paragraphs":[{"context":"The bridge opened in 1932.","qas":[
            {"id":"a","question":"When did the bridge open?","is_impossible":false,"answers":[{"text":"1932"}]},
            {"id":"u","question":"Who built the bridge?","is_impossible":true,"answers":[]}]}]}]}"#;
        let cases: Vec<Case> = crate::answer_eval_squad::convert(raw, 2, "v1", &Default::default())
            .unwrap()
            .iter()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(cases.len(), 2);
        let answerable = cases.iter().find(|case| case.id == "squad2:a").unwrap();
        assert_eq!(answerable.expect.status.as_deref(), Some("extractive"));
        assert_eq!(answerable.expect.answer_contains_any, vec!["1932"]);
        assert_eq!(
            answerable.documents[0].content,
            "The bridge opened in 1932."
        );
        let unanswerable = cases.iter().find(|case| case.id == "squad2:u").unwrap();
        assert_eq!(
            unanswerable.expect.not_status.as_deref(),
            Some("extractive")
        );
        assert!(unanswerable.expect.answer_contains_any.is_empty());
    }

    #[test]
    fn gold_answers_match_after_squad_normalization_on_word_boundaries() {
        let golds = vec!["The 10th century".to_string(), "1932".to_string()];
        assert!(contains_gold("It opened in 1932.", &golds));
        assert!(contains_gold(
            "Founded in 10th-century Normandy? No: 10th century.",
            &golds
        ));
        assert!(!contains_gold("It opened in 19320.", &golds));
        assert!(!contains_gold("Nothing relevant.", &golds));
        assert!(!contains_gold("anything", &["the".to_string()]));
    }

    #[test]
    fn the_bundled_case_file_parses() {
        let raw = include_str!("../fixtures/answer-eval-cases-v1.jsonl");
        let cases: Vec<Case> = raw
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(cases.len(), 20);
        assert!(cases.iter().all(|case| !case.documents.is_empty()));
    }
}
