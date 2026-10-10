//! Real-model positive controls; synthetic texts, not a mock extractor.
use oi_types::FactExtractionPipeline;
use serde_json::json;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let root = std::path::Path::new(args.get(1).ok_or("model directory required")?);
    let model = oi_facts::GlinerFactExtractor::from_paths(
        &root.join("model_quantized.onnx"),
        &root.join("tokenizer.json"),
    )?;
    let controls = [
        (
            "Microsoft was founded by Bill Gates and Paul Allen in 1975.",
            vec!["Microsoft", "Bill Gates", "Paul Allen"],
        ),
        (
            "Barack Obama was born in Honolulu, Hawaii, on August 4, 1961.",
            vec!["Barack Obama", "Honolulu", "Hawaii"],
        ),
        (
            "A Petrobras tem sede no Rio de Janeiro, Brasil.",
            vec!["Petrobras", "Rio de Janeiro", "Brasil"],
        ),
        (
            "Maria Silva trabalha na Microsoft em São Paulo.",
            vec!["Maria Silva", "Microsoft", "São Paulo"],
        ),
    ];
    let mut rows = vec![];
    for (text, expected) in controls {
        let extraction = model.extract(text)?;
        let hits = expected
            .iter()
            .filter(|mention| extraction.entities.iter().any(|e| e.mention == **mention))
            .count();
        rows.push(
            json!({"text":text,"expected_mentions":expected,"exact_mentions_found":hits,
            "actual":extraction}),
        );
    }
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"status":"positive_control_not_general_quality_eval","results":rows})
        )?
    );
    Ok(())
}
