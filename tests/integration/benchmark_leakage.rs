//! No benchmark query may appear in production code.
//!
//! A query that an evaluation asks must never be a string the system was
//! written around. This test collects every query in `bench/fixtures/` and
//! fails if one of three or more words occurs anywhere in the non-test part
//! of a file under `crates/`, comments included.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn fold(text: &str) -> String {
    let mut folded = String::with_capacity(text.len());
    let mut previous_space = true;
    for character in text.chars().flat_map(char::to_lowercase) {
        let character = match character {
            'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
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
    folded.trim_end().to_string()
}

fn collect_queries(value: &serde_json::Value, queries: &mut BTreeSet<String>) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                match (key.as_str(), value) {
                    (
                        "query" | "google_query" | "opensonarx_query",
                        serde_json::Value::String(q),
                    ) => {
                        queries.insert(q.clone());
                    }
                    ("queries", serde_json::Value::Array(items)) => {
                        for item in items {
                            match item {
                                serde_json::Value::String(q) => {
                                    queries.insert(q.clone());
                                }
                                other => collect_queries(other, queries),
                            }
                        }
                    }
                    _ => collect_queries(value, queries),
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_queries(item, queries);
            }
        }
        _ => {}
    }
}

fn rust_files(directory: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path
                .file_name()
                .is_some_and(|name| name != "target" && name != "tests")
            {
                rust_files(&path, files);
            }
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

#[test]
fn no_benchmark_query_appears_in_production_code() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut queries = BTreeSet::new();
    let fixtures = std::fs::read_dir(root.join("bench/fixtures")).expect("bench/fixtures exists");
    for entry in fixtures.flatten() {
        let path = entry.path();
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        match path.extension().and_then(|extension| extension.to_str()) {
            Some("json") => {
                if let Ok(value) = serde_json::from_str(&raw) {
                    collect_queries(&value, &mut queries);
                }
            }
            Some("jsonl") => {
                for line in raw.lines() {
                    if let Ok(value) = serde_json::from_str(line) {
                        collect_queries(&value, &mut queries);
                    }
                }
            }
            _ => {}
        }
    }
    let needles: BTreeSet<String> = queries
        .iter()
        .map(|query| fold(query))
        .filter(|query| query.split(' ').count() >= 3)
        .collect();
    assert!(
        needles.len() > 100,
        "expected the fixture queries to be found, got {}",
        needles.len()
    );

    let mut files = Vec::new();
    rust_files(&root.join("crates"), &mut files);
    assert!(
        files.len() > 50,
        "expected crate sources, got {}",
        files.len()
    );

    let mut leaks = Vec::new();
    for file in files {
        let Ok(source) = std::fs::read_to_string(&file) else {
            continue;
        };
        // Unit tests live in a trailing `mod tests`; only the code before it
        // ships.
        let production = source
            .find("#[cfg(test)]\nmod tests")
            .map_or(source.as_str(), |end| &source[..end]);
        let haystack = format!(" {} ", fold(production));
        for needle in &needles {
            if haystack.contains(&format!(" {needle} ")) {
                leaks.push(format!(
                    "{}: {needle:?}",
                    file.strip_prefix(&root).unwrap_or(&file).display()
                ));
            }
        }
    }
    assert!(
        leaks.is_empty(),
        "benchmark queries found in production code:\n{}",
        leaks.join("\n")
    );
}
