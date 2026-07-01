use oi_facts::MockFactExtractor;
use oi_types::traits::FactExtractionPipeline;
use std::collections::HashMap;
use std::sync::Arc;

use crate::facts_dataset;
use crate::facts_metrics::{
    self, DocResult, EntitySpan, FactsBenchmarkResult, NerMetrics, RelationSpan,
};

/// Get a fact extraction pipeline by name.
pub fn get_factifier(name: &str) -> Arc<dyn FactExtractionPipeline> {
    match name {
        "mock" => Arc::new(MockFactExtractor::new()),
        #[cfg(feature = "facts-onnx")]
        "gliner" => {
            tracing::info!("Loading GLiNER model...");
            Arc::new(oi_facts::GlinerFactExtractor::new().expect("Failed to load GLiNER"))
        }
        other => {
            eprintln!("Unknown facts pipeline: {other}. Available: mock, gliner");
            std::process::exit(1);
        }
    }
}

/// Run the full facts benchmark and return the result.
pub fn run_facts_bench(pipeline_name: &str, output_format: &str, skip_langs: &[String]) {
    let verbose = output_format == "verbose";
    let factifier = get_factifier(pipeline_name);
    let all_samples = facts_dataset::load_dataset();

    let samples: Vec<_> = all_samples
        .into_iter()
        .filter(|s| !skip_langs.iter().any(|l| l == &s.language))
        .collect();

    if !skip_langs.is_empty() {
        tracing::info!("Skipping languages: {:?}", skip_langs);
    }

    tracing::info!(
        "Running facts benchmark with pipeline '{}' on {} samples",
        factifier.name(),
        samples.len()
    );

    let mut doc_results = Vec::new();

    for sample in &samples {
        let extraction = match factifier.extract(&sample.text) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("Extraction failed for {}: {e}", sample.id);
                continue;
            }
        };

        // Verbose: print predicted vs expected for each sample
        if verbose {
            println!("\n--- {} [{}] ---", sample.id, sample.language);
            let preview: String = sample.text.chars().take(80).collect();
            println!("  Text: {preview}...");
            println!("  Expected entities:");
            for e in &sample.expected_entities {
                println!("    [{:>12}] \"{}\"", e.entity_type, e.mention);
            }
            println!("  Predicted entities:");
            if extraction.entities.is_empty() {
                println!("    (none)");
            }
            for e in &extraction.entities {
                let match_status = if sample
                    .expected_entities
                    .iter()
                    .any(|g| g.mention == e.mention)
                {
                    "EXACT"
                } else if sample
                    .expected_entities
                    .iter()
                    .any(|g| g.mention.contains(&e.mention) || e.mention.contains(&g.mention))
                {
                    "PARTIAL"
                } else {
                    "MISS"
                };
                println!(
                    "    [{:>12}] \"{}\"\t conf={:.3} {}",
                    e.entity_type, e.mention, e.confidence, match_status
                );
            }
        }

        // Convert predicted entities to spans
        let pred_entities: Vec<EntitySpan> = extraction
            .entities
            .iter()
            .map(|e| EntitySpan {
                start: e.start,
                end: e.end,
                entity_type: e.entity_type.clone(),
            })
            .collect();

        let gold_entities: Vec<EntitySpan> =
            sample.expected_entities.iter().map(|e| e.to_span()).collect();

        // Convert predicted facts to relations
        let pred_rels: Vec<RelationSpan> = extraction
            .facts
            .iter()
            .map(|f| RelationSpan {
                subject: f.subject.normalized.clone(),
                predicate: f.predicate.clone(),
                object: f.object.normalized.clone(),
            })
            .collect();

        let gold_rels: Vec<RelationSpan> =
            sample.expected_facts.iter().map(|f| f.to_rel()).collect();

        let ner = facts_metrics::evaluate_ner(&pred_entities, &gold_entities);
        let rel = facts_metrics::evaluate_relations(&pred_rels, &gold_rels);

        // Per entity-type NER
        let mut entity_type_ner: HashMap<String, NerMetrics> = HashMap::new();
        let all_types: Vec<String> = gold_entities
            .iter()
            .map(|e| e.entity_type.clone())
            .chain(pred_entities.iter().map(|e| e.entity_type.clone()))
            .collect();
        for etype in &all_types {
            if entity_type_ner.contains_key(etype) {
                continue;
            }
            let pred_for_type: Vec<EntitySpan> = pred_entities
                .iter()
                .filter(|e| e.entity_type == *etype)
                .cloned()
                .collect();
            let gold_for_type: Vec<EntitySpan> = gold_entities
                .iter()
                .filter(|e| e.entity_type == *etype)
                .cloned()
                .collect();
            let type_ner = facts_metrics::evaluate_ner(&pred_for_type, &gold_for_type);
            entity_type_ner.insert(etype.clone(), type_ner);
        }

        doc_results.push(DocResult {
            language: sample.language.clone(),
            ner,
            rel,
            entity_type_ner,
            extraction_time_ms: extraction.extraction_time_ms,
        });
    }

    let result = facts_metrics::aggregate_results(&doc_results);

    match output_format {
        "json" => print_json(pipeline_name, &result),
        _ => print_table(pipeline_name, &result),
    }
}

fn print_json(pipeline_name: &str, result: &FactsBenchmarkResult) {
    let output = serde_json::json!({
        "pipeline": pipeline_name,
        "overall_ner": {
            "exact_f1": result.overall_ner.exact_f1,
            "partial_f1": result.overall_ner.partial_f1,
            "exact_precision": result.overall_ner.exact_precision,
            "exact_recall": result.overall_ner.exact_recall,
        },
        "overall_rel": {
            "f1": result.overall_rel.f1,
            "precision": result.overall_rel.precision,
            "recall": result.overall_rel.recall,
        },
        "per_language": result.per_language.iter().map(|(lang, (ner, rel))| {
            (lang.clone(), serde_json::json!({
                "ner_exact_f1": ner.exact_f1,
                "ner_partial_f1": ner.partial_f1,
                "rel_f1": rel.f1,
            }))
        }).collect::<serde_json::Map<String, serde_json::Value>>(),
        "per_entity_type": result.per_entity_type.iter().map(|(ty, ner)| {
            (ty.clone(), serde_json::json!({
                "exact_f1": ner.exact_f1,
                "partial_f1": ner.partial_f1,
            }))
        }).collect::<serde_json::Map<String, serde_json::Value>>(),
        "avg_extraction_time_ms": result.avg_extraction_time_ms,
    });
    println!("{}", serde_json::to_string_pretty(&output).unwrap());
}

fn print_table(pipeline_name: &str, result: &FactsBenchmarkResult) {
    println!("\n=== Facts Benchmark Results ===");
    println!("Pipeline: {pipeline_name}\n");

    println!("Overall NER:");
    println!(
        "  Exact:   P={:.3}  R={:.3}  F1={:.3}",
        result.overall_ner.exact_precision,
        result.overall_ner.exact_recall,
        result.overall_ner.exact_f1
    );
    println!(
        "  Partial: P={:.3}  R={:.3}  F1={:.3}",
        result.overall_ner.partial_precision,
        result.overall_ner.partial_recall,
        result.overall_ner.partial_f1
    );

    println!("\nOverall Relations:");
    println!(
        "  P={:.3}  R={:.3}  F1={:.3}",
        result.overall_rel.precision, result.overall_rel.recall, result.overall_rel.f1
    );

    println!("\nPer Language:");
    println!(
        "  {:<6} {:>10} {:>10} {:>10}",
        "Lang", "NER-F1(E)", "NER-F1(P)", "REL-F1"
    );
    println!("  {}", "-".repeat(40));
    let mut langs: Vec<_> = result.per_language.keys().collect();
    langs.sort();
    for lang in langs {
        let (ner, rel) = &result.per_language[lang];
        println!(
            "  {:<6} {:>10.3} {:>10.3} {:>10.3}",
            lang, ner.exact_f1, ner.partial_f1, rel.f1
        );
    }

    println!("\nPer Entity Type:");
    println!("  {:<12} {:>10} {:>10}", "Type", "NER-F1(E)", "NER-F1(P)");
    println!("  {}", "-".repeat(34));
    let mut types: Vec<_> = result.per_entity_type.keys().collect();
    types.sort();
    for ty in types {
        let ner = &result.per_entity_type[ty];
        println!(
            "  {:<12} {:>10.3} {:>10.3}",
            ty, ner.exact_f1, ner.partial_f1
        );
    }

    println!(
        "\nAvg extraction time: {:.2}ms",
        result.avg_extraction_time_ms
    );
    println!();
}
