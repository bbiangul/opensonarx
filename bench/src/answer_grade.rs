//! Grading sheets for answers a reader will grade one by one.
//!
//! A sheet lists every emitted response of one assembly variant with what a
//! reader needs to judge it — the request, the dataset's reference answers,
//! the response and the spans it cites — and nothing about its evidence state:
//! no status, no source-family count, no gap note, no subject. Items are
//! ordered by a hash of their key, so neighbours on the sheet share nothing.

use crate::answer_calibrate::label_key;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// Collect the sheet items of one `<report.json>:<cases.jsonl>` pair.
fn items(spec: &str, variant: &str) -> Result<Vec<(String, Value)>, String> {
    let (report_path, cases_path) = spec
        .split_once(':')
        .ok_or_else(|| format!("expected <report.json>:<cases.jsonl>, got {spec:?}"))?;
    let read = |path: &str| {
        std::fs::read_to_string(path).map_err(|error| format!("cannot read {path}: {error}"))
    };
    let report: Value =
        serde_json::from_str(&read(report_path)?).map_err(|error| error.to_string())?;
    let cases: BTreeMap<String, Value> = read(cases_path)?
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<Value>(line).map_err(|error| error.to_string()))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter_map(|case| Some((case["id"].as_str()?.to_string(), case)))
        .collect();
    let mut sheet = Vec::new();
    for result in report["results"].as_array().into_iter().flatten() {
        let outcome = &result["variants"][variant];
        let status = outcome["status"].as_str().unwrap_or_default();
        if status != "extractive" && status != "partial_answer" {
            continue;
        }
        let id = result["id"].as_str().unwrap_or_default();
        let case = cases
            .get(id)
            .ok_or_else(|| format!("{report_path}: case {id} is not in {cases_path}"))?;
        let key = label_key(report_path, id);
        sheet.push((
            key.clone(),
            json!({
                "key": key,
                "request": case["query"],
                "reference_answers": case["expect"]["answer_contains_any"],
                "response": outcome["summary"],
                "cited_spans": outcome["supporting_excerpts"],
            }),
        ));
    }
    Ok(sheet)
}

pub fn run(specs: &[String], variant: &str, output: &str) -> Result<(), String> {
    let mut sheet = Vec::new();
    for spec in specs {
        sheet.extend(items(spec, variant)?);
    }
    if sheet.is_empty() {
        return Err("no emitted responses to grade".to_string());
    }
    sheet.sort_by_cached_key(|(key, _)| Sha256::digest(key.as_bytes()).to_vec());
    let body: Vec<String> = sheet.iter().map(|(_, item)| item.to_string()).collect();
    std::fs::write(output, body.join("\n") + "\n")
        .map_err(|error| format!("cannot write {output}: {error}"))?;
    println!("wrote {} responses to grade to {output}", sheet.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sheet_lists_emitted_responses_without_their_evidence_state() {
        let directory = std::env::temp_dir().join(format!("oi-grade-sheet-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let report = directory.join("run-one.json");
        let cases = directory.join("cases.jsonl");
        std::fs::write(
            &report,
            json!({ "results": [
                { "id": "c1", "model_reported_gaps": ["x"], "variants": { "v": {
                    "status": "extractive", "summary": "Ada started it.",
                    "min_supporting_families": 2, "supporting_excerpts": ["started by Ada"] } } },
                { "id": "c2", "variants": { "v": {
                    "status": "partial_answer", "summary": "Partly.",
                    "min_supporting_families": 1, "supporting_excerpts": [] } } },
                { "id": "c3", "variants": { "v": {
                    "status": "insufficient_evidence", "summary": "", "supporting_excerpts": [] } } }
            ]})
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            &cases,
            [
                json!({ "id": "c1", "query": "who started it", "expect": { "answer_contains_any": ["Ada"] } }),
                json!({ "id": "c2", "query": "what else", "expect": { "not_status": "extractive" } }),
                json!({ "id": "c3", "query": "unused", "expect": {} }),
            ]
            .map(|case| case.to_string())
            .join("\n"),
        )
        .unwrap();
        let spec = format!("{}:{}", report.display(), cases.display());
        let sheet = items(&spec, "v").unwrap();
        assert_eq!(sheet.len(), 2, "abstentions are not graded");
        let first = &sheet.iter().find(|(key, _)| key == "run-one#c1").unwrap().1;
        assert_eq!(first["request"], "who started it");
        assert_eq!(first["reference_answers"], json!(["Ada"]));
        assert_eq!(first["response"], "Ada started it.");
        assert_eq!(first["cited_spans"], json!(["started by Ada"]));
        let text = first.to_string();
        for hidden in ["status", "families", "gap", "extractive"] {
            assert!(!text.contains(hidden), "sheet leaks {hidden}");
        }
        std::fs::remove_dir_all(&directory).ok();
    }
}
