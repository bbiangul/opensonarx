//! Read-only inspection of an existing real index, without discovery/crawling.
//! This reports candidate extraction, not ground-truth accuracy.
use oi_sdk::query_intent::classify_query;
use oi_sdk::{OpenSonarXClient, SdkConfig, construct_answer};
use oi_types::QueryFilters;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() != 2 {
        return Err("usage: audit_retained INDEX_DIR QUERY".into());
    }
    let client = OpenSonarXClient::new(SdkConfig {
        local_index_path: Some(args[0].clone()),
        search_engines: Some(vec![]),
        ..Default::default()
    });
    let model = client.embedder().name();
    if model.to_ascii_lowercase().contains("mock") {
        return Err("real embedder required; compile with --features onnx".into());
    }
    let intent = classify_query(&args[1]);
    let results = client.search(&args[1], 40, &QueryFilters::default())?;
    let rows = results.iter().map(|result| {
        let candidate = construct_answer(&args[1], intent, std::slice::from_ref(result));
        serde_json::json!({
            "url": result.url, "title": result.title, "source": result.source,
            "retained_chars": result.content_md.chars().count(),
            "single_source_status": candidate.status,
            "single_source_diagnostic": candidate.summary,
            "origin_candidates": oi_sdk::answer::audit_factual_candidates(&args[1], result)
                .into_iter().filter(|audit| audit.required_relation).take(8).collect::<Vec<_>>(),
        })
    }).collect::<Vec<_>>();
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "model": model, "query": args[1], "index_documents": client.indexed_document_count(),
            "network_calls": false, "truth_judged": false,
            "unfiltered_local_answer": construct_answer(&args[1], intent, &results), "rows": rows,
        }))?
    );
    Ok(())
}
