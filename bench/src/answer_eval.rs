//! Offline evaluation of the answer path: model extraction, stance judgement
//! and deterministic assembly over fixed documents. No retrieval, no crawl.
//!
//! Each case supplies a request and the documents the pipeline would have
//! retained. The harness reports how often the assembled answer has the
//! expected status, how often an answer was emitted where none was supported,
//! and how many model quotes failed verbatim verification. Cases are data: a
//! new failure mode is a new line in the case file, not a new rule in code.

use oi_sdk::answer::{assemble_answer, build_passages, merge_repeated_claims};
use oi_sdk::{AnswerModelConfig, AnswerPolicy, OllamaChatModel};
use oi_types::{ClaimExtractor, SearchResultItem, StanceJudge};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
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
    model_error: Option<String>,
    latency_ms: f64,
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
    think: Option<String>,
    policy_id: String,
    cases: usize,
    passed: usize,
    /// Cases expected to answer that produced the expected answer.
    answerable_cases: usize,
    answerable_passed: usize,
    /// Cases where no answer is supported by the documents.
    unanswerable_cases: usize,
    /// Of those, cases where the pipeline still emitted an `extractive` answer.
    unsupported_answers_emitted: usize,
    rejected_quotes: usize,
    model_errors: usize,
    latency_ms_p50: f64,
    latency_ms_max: f64,
    limitations: Vec<&'static str>,
    results: Vec<CaseReport>,
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

fn evaluate(case: &Case, model: &OllamaChatModel, policy: &AnswerPolicy) -> CaseReport {
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
        .map(|claim| model.judge(&claim.text, &retained))
        .collect::<Result<_, _>>()
        .map_err(|error| model_error = Some(error.to_string()))
        .ok();
    let (answer, checklist, stats) = assemble_answer(
        &results,
        &passages,
        &extraction,
        judgments.as_deref(),
        policy,
    );

    let answer_text = std::iter::once(answer.summary.as_str())
        .chain(answer.claims.iter().map(|claim| claim.text.as_str()))
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let mut failures = Vec::new();
    if let Some(error) = &model_error {
        failures.push(format!("model error: {error}"));
    }
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
    if !alternatives.is_empty()
        && !alternatives
            .iter()
            .any(|needle| answer_text.contains(&needle.to_lowercase()))
    {
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
        model_error,
        latency_ms: started.elapsed().as_secs_f64() * 1_000.0,
    }
}

pub fn run(
    cases_path: &str,
    model: &str,
    base_url: Option<&str>,
    think: Option<&str>,
    min_source_families: Option<usize>,
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

    let mut config = AnswerModelConfig::new(model);
    if let Some(base_url) = base_url {
        config.base_url = base_url.to_string();
    }
    config.think = think.map(str::to_string);
    let chat_model = OllamaChatModel::new(config);
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

    let mut results = Vec::with_capacity(cases.len());
    for case in &cases {
        let report = evaluate(case, &chat_model, &policy);
        eprintln!(
            "{} {:<28} status={:<22} {:.1}s {}",
            if report.passed { "PASS" } else { "FAIL" },
            report.id,
            report.status,
            report.latency_ms / 1_000.0,
            report.failures.join("; ")
        );
        results.push(report);
    }

    let expects_answer = |case: &Case| case.expect.status.as_deref() == Some("extractive");
    let supports_no_answer = |case: &Case| case.expect.not_status.as_deref() == Some("extractive");
    let paired = || cases.iter().zip(&results);
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
        think: think.map(str::to_string),
        policy_id: policy.policy_id.clone(),
        cases: cases.len(),
        passed: results.iter().filter(|result| result.passed).count(),
        answerable_cases: cases.iter().filter(|case| expects_answer(case)).count(),
        answerable_passed: paired()
            .filter(|(case, result)| expects_answer(case) && result.passed)
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
        latency_ms_max: latencies[latencies.len() - 1],
        limitations: vec![
            "Cases were written by the implementer and have been inspected: a development regression, not evidence of generalization.",
            "Documents are supplied; retrieval, crawling and extraction from HTML are not exercised.",
            "One model, one run; model output is not deterministic across versions or servers.",
        ],
        results,
    };
    let json = serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?;
    std::fs::write(output, json).map_err(|error| format!("cannot write {output}: {error}"))?;
    println!(
        "answer-eval: {}/{} cases as expected; answerable {}/{}; unsupported answers emitted {}/{}; rejected quotes {}; model errors {}; p50 {:.1}s max {:.1}s → {output}",
        report.passed,
        report.cases,
        report.answerable_passed,
        report.answerable_cases,
        report.unsupported_answers_emitted,
        report.unanswerable_cases,
        report.rejected_quotes,
        report.model_errors,
        report.latency_ms_p50 / 1_000.0,
        report.latency_ms_max / 1_000.0,
    );
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
        let cases: Vec<Case> = crate::answer_eval_squad::convert(raw, 2, "v1")
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
