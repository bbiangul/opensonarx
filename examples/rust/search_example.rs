//! Example: Search the OpenSonarX network and provide feedback
//!
//! ```bash
//! cargo run --example search_example
//! ```

fn main() {
    // Create SDK client
    let _client = oi_sdk::OpenSonarXClient::new();

    // In a real app, you'd use tokio and call:
    // let results = client.search("rust programming", 10, &Default::default()).await?;
    // for result in results {
    //     println!("{}: {} (score: {:.3})", result.url, result.title, result.score);
    // }
    //
    // // Submit feedback
    // client.feedback(&results[0].result_id, true).await?;

    println!("OpenSonarX Rust SDK example");
    println!("See oi-sdk crate for full API documentation");
}
