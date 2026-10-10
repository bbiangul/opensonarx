//! Build `answer-eval` cases from MS MARCO QA v2.1 rows.
//!
//! Each MS MARCO query is a real search request with about ten web passages
//! from different sites and either human-written answers or the marker
//! "No Answer Present." That gives multi-source cases with self-contained
//! questions: the default two-family rule applies as it does in production,
//! and unanswerable cases are ones annotators could not answer from the
//! passages.
//!
//! Input is JSONL, one row per line, in the schema of the `microsoft/ms_marco`
//! `v2.1` dataset (`query_id`, `query`, `answers`, `wellFormedAnswers`,
//! `passages.{passage_text,url,is_selected}`). Reference answers are free
//! text rather than spans, so string matching undercounts correct answers;
//! the passages annotators selected are carried as `gold_evidence` so the
//! harness can also report whether a cited source was one of them.

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashSet;

const NO_ANSWER: &str = "No Answer Present.";

#[derive(Deserialize)]
struct Row {
    query_id: serde_json::Value,
    query: String,
    #[serde(default)]
    answers: Vec<String>,
    #[serde(default, rename = "wellFormedAnswers")]
    well_formed_answers: Vec<String>,
    passages: Passages,
}

#[derive(Deserialize)]
struct Passages {
    passage_text: Vec<String>,
    url: Vec<String>,
    #[serde(default)]
    is_selected: Vec<i64>,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn normalized(query: &str) -> String {
    query
        .to_lowercase()
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Convert JSONL `rows` into at most `sample` cases, half answerable and half
/// unanswerable, ordered by `sha256(label:query_id)`. A row whose normalized
/// query is in `exclude_queries` is never drawn.
///
/// With `subjects` (query id → subject), `sample` applies to each subject
/// separately, every case records its subject, and rows without a subject
/// are skipped: the result is balanced across subjects as well.
pub fn convert(
    rows: &str,
    sample: usize,
    label: &str,
    exclude_queries: &HashSet<String>,
    subjects: Option<&std::collections::HashMap<String, String>>,
) -> Result<Vec<String>, String> {
    let mut answerable: Vec<(String, String, serde_json::Value)> = Vec::new();
    let mut unanswerable: Vec<(String, String, serde_json::Value)> = Vec::new();
    let mut seen = HashSet::new();
    for (index, line) in rows.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let row: Row = serde_json::from_str(line)
            .map_err(|error| format!("row {}: not an MS MARCO v2.1 row: {error}", index + 1))?;
        let query = row.query.trim().trim_start_matches(['.', ' ']).to_string();
        let query_id = match &row.query_id {
            serde_json::Value::String(id) => id.clone(),
            other => other.to_string(),
        };
        if query.is_empty()
            || exclude_queries.contains(&normalized(&query))
            || !seen.insert(query_id.clone())
        {
            continue;
        }
        let documents: Vec<serde_json::Value> = row
            .passages
            .passage_text
            .iter()
            .zip(&row.passages.url)
            .enumerate()
            .filter(|(_, (text, url))| !text.trim().is_empty() && !url.trim().is_empty())
            .map(|(position, (text, url))| {
                serde_json::json!({
                    "url": url,
                    "title": "",
                    "content": text,
                    "gold_evidence": row.passages.is_selected.get(position) == Some(&1),
                })
            })
            .collect();
        if documents.is_empty() {
            continue;
        }
        let mut golds: Vec<&str> = row
            .answers
            .iter()
            .chain(&row.well_formed_answers)
            .map(|answer| answer.trim())
            .filter(|answer| !answer.is_empty() && *answer != NO_ANSWER)
            .collect();
        golds.sort_unstable();
        golds.dedup();
        let no_answer = row.answers.iter().any(|answer| answer.trim() == NO_ANSWER);
        let expect = match (no_answer, golds.is_empty()) {
            (true, true) => serde_json::json!({ "not_status": "extractive" }),
            (false, false) => {
                serde_json::json!({ "status": "extractive", "answer_contains_any": golds })
            }
            // Neither clearly answerable nor clearly unanswerable.
            _ => continue,
        };
        let subject = match subjects {
            Some(subjects) => match subjects.get(&query_id) {
                Some(subject) => subject.clone(),
                None => continue,
            },
            None => String::new(),
        };
        let mut case = serde_json::json!({
            "id": format!("msmarco-qa:{query_id}"),
            "query": query,
            "documents": documents,
            "expect": expect,
        });
        if !subject.is_empty() {
            case["subject"] = serde_json::Value::String(subject.clone());
        }
        let key = hex(&Sha256::digest(format!("{label}:{query_id}").as_bytes()));
        if no_answer {
            unanswerable.push((key, subject, case));
        } else {
            answerable.push((key, subject, case));
        }
    }
    answerable.sort_by(|left, right| left.0.cmp(&right.0));
    unanswerable.sort_by(|left, right| left.0.cmp(&right.0));
    let half = sample / 2;
    // Take the first `limit` of each subject, in hash order.
    let take = |pool: Vec<(String, String, serde_json::Value)>, limit: usize| {
        let mut taken: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        pool.into_iter()
            .filter(|(_, subject, _)| {
                let count = taken.entry(subject.clone()).or_default();
                *count += 1;
                *count <= limit
            })
            .collect::<Vec<_>>()
    };
    let mut chosen = take(answerable, sample - half);
    chosen.extend(take(unanswerable, half));
    chosen.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(chosen
        .into_iter()
        .map(|(_, _, case)| case.to_string())
        .collect())
}

pub fn run(
    input: &str,
    sample: usize,
    label: &str,
    exclude_queries_path: Option<&str>,
    subjects_path: Option<&str>,
    output: &str,
) -> Result<(), String> {
    let rows =
        std::fs::read_to_string(input).map_err(|error| format!("cannot read {input}: {error}"))?;
    let exclude: HashSet<String> = match exclude_queries_path {
        Some(path) => std::fs::read_to_string(path)
            .map_err(|error| format!("cannot read {path}: {error}"))?
            .lines()
            .map(normalized)
            .filter(|query| !query.is_empty())
            .collect(),
        None => HashSet::new(),
    };
    // Subject labels: JSONL lines of {"query_id": ..., "subject": ...}.
    let subjects = match subjects_path {
        Some(path) => Some(
            std::fs::read_to_string(path)
                .map_err(|error| format!("cannot read {path}: {error}"))?
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(|line| {
                    let value: serde_json::Value =
                        serde_json::from_str(line).map_err(|error| format!("{path}: {error}"))?;
                    let id = match &value["query_id"] {
                        serde_json::Value::String(id) => id.clone(),
                        other => other.to_string(),
                    };
                    let subject = value["subject"]
                        .as_str()
                        .ok_or_else(|| format!("{path}: query {id} has no subject"))?;
                    Ok((id, subject.to_string()))
                })
                .collect::<Result<std::collections::HashMap<_, _>, String>>()?,
        ),
        None => None,
    };
    let cases = convert(&rows, sample, label, &exclude, subjects.as_ref())?;
    if cases.is_empty() {
        return Err(format!("{input} produced no cases"));
    }
    let body = cases.join("\n") + "\n";
    std::fs::write(output, &body).map_err(|error| format!("cannot write {output}: {error}"))?;
    println!(
        "wrote {} cases to {output}\n  source sha256: {}\n  cases sha256:  {}\n  sample label:  {label}\n  excluded queries: {}",
        cases.len(),
        hex(&Sha256::digest(rows.as_bytes())),
        hex(&Sha256::digest(body.as_bytes())),
        exclude.len(),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: u32, query: &str, answers: &[&str], selected: &[i64]) -> String {
        serde_json::json!({
            "query_id": id,
            "query": query,
            "query_type": "DESCRIPTION",
            "answers": answers,
            "wellFormedAnswers": [],
            "passages": {
                "is_selected": selected,
                "passage_text": ["First passage text.", "Second passage text.", " "],
                "url": ["https://a.test/1", "https://b.test/2", "https://c.test/3"],
            },
        })
        .to_string()
    }

    fn parsed(rows: &[String], sample: usize, exclude: &[&str]) -> Vec<serde_json::Value> {
        let exclude = exclude.iter().map(|query| normalized(query)).collect();
        convert(&rows.join("\n"), sample, "v1", &exclude, None)
            .unwrap()
            .iter()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[test]
    fn rows_become_multi_source_cases_with_gold_evidence_flags() {
        let rows = [
            row(
                1,
                ". what is a corporation?",
                &["A legal entity."],
                &[0, 1, 0],
            ),
            row(
                2,
                "who painted the ceiling",
                &["No Answer Present."],
                &[0, 0, 0],
            ),
        ];
        let cases = parsed(&rows, 10, &[]);
        assert_eq!(cases.len(), 2);
        let answerable = cases
            .iter()
            .find(|case| case["id"] == "msmarco-qa:1")
            .unwrap();
        assert_eq!(answerable["query"], "what is a corporation?");
        assert_eq!(answerable["expect"]["status"], "extractive");
        assert_eq!(
            answerable["expect"]["answer_contains_any"],
            serde_json::json!(["A legal entity."])
        );
        let documents = answerable["documents"].as_array().unwrap();
        assert_eq!(documents.len(), 2, "the blank passage is dropped");
        assert_eq!(documents[0]["gold_evidence"], false);
        assert_eq!(documents[1]["gold_evidence"], true);
        assert_eq!(documents[1]["url"], "https://b.test/2");
        let unanswerable = cases
            .iter()
            .find(|case| case["id"] == "msmarco-qa:2")
            .unwrap();
        assert_eq!(unanswerable["expect"]["not_status"], "extractive");
    }

    #[test]
    fn excluded_duplicate_and_ambiguous_rows_are_skipped() {
        let rows = [
            row(1, "seen before query", &["x"], &[1, 0, 0]),
            row(2, "fresh query", &["y"], &[1, 0, 0]),
            row(2, "fresh query", &["y"], &[1, 0, 0]),
            // An empty answer list is neither answerable nor marked unanswerable.
            row(3, "no label", &[], &[0, 0, 0]),
            // Marked unanswerable yet carrying an answer.
            row(4, "mixed label", &["No Answer Present.", "z"], &[0, 0, 0]),
        ];
        let cases = parsed(&rows, 10, &["Seen before  QUERY!"]);
        let ids: Vec<_> = cases
            .iter()
            .map(|case| case["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, vec!["msmarco-qa:2"]);
    }

    #[test]
    fn sample_is_balanced_and_reproducible() {
        let mut rows = Vec::new();
        for id in 0..6 {
            rows.push(row(id, &format!("answerable {id}"), &["yes"], &[1, 0, 0]));
            rows.push(row(
                100 + id,
                &format!("unanswerable {id}"),
                &["No Answer Present."],
                &[0, 0, 0],
            ));
        }
        let first = parsed(&rows, 4, &[]);
        assert_eq!(first, parsed(&rows, 4, &[]));
        let answerable = first
            .iter()
            .filter(|case| case["expect"]["status"] == "extractive")
            .count();
        assert_eq!((first.len(), answerable), (4, 2));
    }

    #[test]
    fn with_subjects_the_sample_is_balanced_per_subject_and_cases_carry_it() {
        let mut rows = Vec::new();
        let mut subjects = std::collections::HashMap::new();
        for id in 0..12u32 {
            let answers: &[&str] = if id % 2 == 0 {
                &["yes"]
            } else {
                &["No Answer Present."]
            };
            rows.push(row(id, &format!("query {id}"), answers, &[1, 0, 0]));
            // Ten rows of one subject, two of another; one row left unlabelled.
            if id < 9 {
                subjects.insert(id.to_string(), "topic-science".to_string());
            } else if id < 11 {
                subjects.insert(id.to_string(), "topic-health".to_string());
            }
        }
        let cases: Vec<serde_json::Value> =
            convert(&rows.join("\n"), 4, "v1", &HashSet::new(), Some(&subjects))
                .unwrap()
                .iter()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
        let count = |subject: &str, answerable: bool| {
            cases
                .iter()
                .filter(|case| {
                    case["subject"] == subject
                        && (case["expect"]["status"] == "extractive") == answerable
                })
                .count()
        };
        assert_eq!(
            (count("topic-science", true), count("topic-science", false)),
            (2, 2)
        );
        assert_eq!(
            (count("topic-health", true), count("topic-health", false)),
            (1, 1)
        );
        assert_eq!(cases.len(), 6, "the unlabelled row is not drawn");
    }

    #[test]
    fn a_line_in_another_schema_is_an_error() {
        assert!(convert(r#"{"question":"x"}"#, 4, "v1", &HashSet::new(), None).is_err());
    }
}
