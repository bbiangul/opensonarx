//! Build `answer-eval` cases from a SQuAD 2.0 file.
//!
//! SQuAD 2.0 pairs each question with one context paragraph and marks
//! questions the paragraph cannot answer. That gives the answer path cases
//! nobody on this project wrote: an answerable question must yield a claim
//! containing a gold answer, and an unanswerable one must yield no answer.
//!
//! The sample is chosen by hashing each question id with a caller-supplied
//! label, so it is reproducible and nobody picks questions by looking at them.
//! Each case has a single document, so run `answer-eval` on the output with
//! `--min-source-families 1`.

use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Deserialize)]
struct Squad {
    data: Vec<Article>,
}

#[derive(Deserialize)]
struct Article {
    title: String,
    paragraphs: Vec<Paragraph>,
}

#[derive(Deserialize)]
struct Paragraph {
    context: String,
    qas: Vec<Question>,
}

#[derive(Deserialize)]
struct Question {
    id: String,
    question: String,
    #[serde(default)]
    is_impossible: bool,
    #[serde(default)]
    answers: Vec<Answer>,
}

#[derive(Deserialize)]
struct Answer {
    text: String,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Convert `raw` into at most `sample` JSONL cases, half answerable and half
/// unanswerable, ordered by `sha256(label:id)`. Case ids in `exclude` (from
/// earlier samples) are never drawn again.
pub fn convert(
    raw: &str,
    sample: usize,
    label: &str,
    exclude: &std::collections::HashSet<String>,
) -> Result<Vec<String>, String> {
    let squad: Squad =
        serde_json::from_str(raw).map_err(|error| format!("not a SQuAD 2.0 file: {error}"))?;
    let mut answerable = Vec::new();
    let mut unanswerable = Vec::new();
    for article in &squad.data {
        for (index, paragraph) in article.paragraphs.iter().enumerate() {
            for question in &paragraph.qas {
                let mut golds: Vec<&str> = question
                    .answers
                    .iter()
                    .map(|answer| answer.text.trim())
                    .filter(|text| !text.is_empty())
                    .collect();
                golds.sort_unstable();
                golds.dedup();
                if !question.is_impossible && golds.is_empty() {
                    continue;
                }
                if exclude.contains(&format!("squad2:{}", question.id)) {
                    continue;
                }
                let expect = if question.is_impossible {
                    serde_json::json!({ "not_status": "extractive" })
                } else {
                    serde_json::json!({ "status": "extractive", "answer_contains_any": golds })
                };
                let case = serde_json::json!({
                    "id": format!("squad2:{}", question.id),
                    "query": question.question.trim(),
                    "documents": [{
                        "url": format!("https://squad.test/{}/{index}", article.title),
                        "title": article.title.replace('_', " "),
                        "content": paragraph.context,
                    }],
                    "expect": expect,
                });
                let key = hex(&Sha256::digest(
                    format!("{label}:{}", question.id).as_bytes(),
                ));
                if question.is_impossible {
                    unanswerable.push((key, case));
                } else {
                    answerable.push((key, case));
                }
            }
        }
    }
    answerable.sort_by(|left, right| left.0.cmp(&right.0));
    unanswerable.sort_by(|left, right| left.0.cmp(&right.0));
    let half = sample / 2;
    let mut chosen: Vec<_> = answerable
        .into_iter()
        .take(sample - half)
        .chain(unanswerable.into_iter().take(half))
        .collect();
    chosen.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(chosen
        .into_iter()
        .map(|(_, case)| case.to_string())
        .collect())
}

pub fn run(
    input: &str,
    sample: usize,
    label: &str,
    exclude_paths: &[String],
    output: &str,
) -> Result<(), String> {
    let raw =
        std::fs::read_to_string(input).map_err(|error| format!("cannot read {input}: {error}"))?;
    let mut exclude = std::collections::HashSet::new();
    for path in exclude_paths {
        let earlier = std::fs::read_to_string(path)
            .map_err(|error| format!("cannot read {path}: {error}"))?;
        for line in earlier.lines().filter(|line| !line.trim().is_empty()) {
            let case: serde_json::Value =
                serde_json::from_str(line).map_err(|error| format!("{path}: {error}"))?;
            if let Some(id) = case["id"].as_str() {
                exclude.insert(id.to_string());
            }
        }
    }
    let cases = convert(&raw, sample, label, &exclude)?;
    if cases.is_empty() {
        return Err(format!("{input} produced no cases"));
    }
    let body = cases.join("\n") + "\n";
    std::fs::write(output, &body).map_err(|error| format!("cannot write {output}: {error}"))?;
    println!(
        "wrote {} cases to {output}\n  source sha256: {}\n  cases sha256:  {}\n  sample label:  {label}\n  excluded ids:  {}\nRun: oi-bench answer-eval --cases {output} --min-source-families 1 --model <model>",
        cases.len(),
        hex(&Sha256::digest(raw.as_bytes())),
        hex(&Sha256::digest(body.as_bytes())),
        exclude.len(),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"version":"v2.0","data":[{"title":"Test_Article","paragraphs":[
      {"context":"The river is 120 km long and ends at the sea.","qas":[
        {"id":"a1","question":"How long is the river?","is_impossible":false,
         "answers":[{"text":"120 km","answer_start":13},{"text":"120 km","answer_start":13},{"text":"120 km long","answer_start":13}]},
        {"id":"u1","question":"Who discovered the river?","is_impossible":true,"answers":[],
         "plausible_answers":[{"text":"the sea","answer_start":37}]}]},
      {"context":"The bridge opened in 1932.","qas":[
        {"id":"a2","question":"When did the bridge open?","is_impossible":false,"answers":[{"text":"1932","answer_start":21}]},
        {"id":"a3","question":"Empty gold","is_impossible":false,"answers":[{"text":"  ","answer_start":0}]},
        {"id":"u2","question":"Who built the bridge?","is_impossible":true,"answers":[]}]}]}]}"#;

    fn parsed(sample: usize, label: &str) -> Vec<serde_json::Value> {
        convert(SAMPLE, sample, label, &Default::default())
            .unwrap()
            .iter()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[test]
    fn answerable_and_unanswerable_questions_get_opposite_expectations() {
        let cases = parsed(10, "v1");
        assert_eq!(
            cases.len(),
            4,
            "the question with an empty gold answer is skipped"
        );
        let by_id = |id: &str| {
            cases
                .iter()
                .find(|case| case["id"] == format!("squad2:{id}"))
                .unwrap()
                .clone()
        };
        let answerable = by_id("a1");
        assert_eq!(answerable["expect"]["status"], "extractive");
        assert_eq!(
            answerable["expect"]["answer_contains_any"],
            serde_json::json!(["120 km", "120 km long"])
        );
        assert_eq!(answerable["documents"][0]["title"], "Test Article");
        assert_eq!(
            answerable["documents"][0]["content"],
            "The river is 120 km long and ends at the sea."
        );
        let unanswerable = by_id("u1");
        assert_eq!(unanswerable["expect"]["not_status"], "extractive");
        assert!(unanswerable["expect"].get("answer_contains_any").is_none());
        assert_eq!(
            by_id("a2")["documents"][0]["url"],
            "https://squad.test/Test_Article/1"
        );
    }

    #[test]
    fn sample_is_balanced_reproducible_and_depends_on_the_label() {
        let first = convert(SAMPLE, 2, "v1", &Default::default()).unwrap();
        assert_eq!(
            first,
            convert(SAMPLE, 2, "v1", &Default::default()).unwrap()
        );
        let cases = parsed(2, "v1");
        let answerable = cases
            .iter()
            .filter(|case| case["expect"]["status"] == "extractive")
            .count();
        assert_eq!((cases.len(), answerable), (2, 1));
        let labels = ["v1", "v2", "v3", "v4", "v5", "v6"];
        assert!(
            labels
                .iter()
                .any(|label| convert(SAMPLE, 2, label, &Default::default()).unwrap() != first),
            "the label must change which questions are sampled"
        );
    }

    #[test]
    fn ids_from_earlier_samples_are_never_drawn_again() {
        let all = parsed(10, "v1");
        let exclude: std::collections::HashSet<String> =
            ["squad2:a1".to_string(), "squad2:u2".to_string()].into();
        let remaining: Vec<serde_json::Value> = convert(SAMPLE, 10, "v1", &exclude)
            .unwrap()
            .iter()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(remaining.len(), all.len() - 2);
        assert!(remaining
            .iter()
            .all(|case| case["id"] != "squad2:a1" && case["id"] != "squad2:u2"));
    }

    #[test]
    fn a_file_in_another_format_is_rejected() {
        assert!(convert(r#"{"rows":[]}"#, 4, "v1", &Default::default()).is_err());
    }
}
