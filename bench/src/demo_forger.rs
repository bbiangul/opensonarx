//! A dishonest node, for demonstrating what a replica refuses.
//!
//! It fetches one real page, joins the network as a peer of the given shard,
//! and offers that page with two claims: one whose quote is in the page, and
//! one the operator made up, with a quote the page does not contain. It
//! writes both straight into its own claim index, which is what a modified
//! node could do: nothing on its side checks them.
//!
//! An honest replica of the same shard pulls from it like from any peer. It
//! is expected to keep the first claim and refuse the second, because it
//! looks for each quote in the page text it received.
//!
//! This is a demonstration tool; it measures nothing.

use oi_crawl::Distiller;
use oi_index::HybridIndex;
use oi_network::config::NetworkConfig;
use oi_network::Node;
use oi_sdk::claims::ClaimRecord;
use oi_types::quote::{locate_quote, plain_text};
use oi_types::traits::{CrawlPipeline, SearchIndex};
use sha2::{Digest, Sha256};
use std::sync::Arc;

/// The first stretch of the text that reads as a sentence: from the start of
/// a sentence to its full stop, long enough to be a quote.
fn first_sentence(plain: &str, min_chars: usize) -> Option<String> {
    plain
        .split_inclusive(". ")
        .map(str::trim)
        .find(|sentence| sentence.chars().count() >= min_chars.max(40) && sentence.ends_with('.'))
        .map(str::to_string)
}

#[allow(clippy::too_many_arguments)]
pub async fn run(
    url: &str,
    shard: &str,
    bootstrap: &str,
    port: u16,
    forged_claim: &str,
    forged_quote: &str,
    seconds: u64,
) -> Result<(), String> {
    let policy = oi_sdk::claims::ClaimPolicy::default();
    let embedder = oi_sdk::OpenSonarXClient::default_embedder();
    let page = Distiller::new()
        .with_embedder(embedder.clone())
        .crawl(url)
        .await
        .map_err(|error| format!("cannot fetch {url}: {error}"))?;

    // The first chunk that yields a quotable sentence.
    let (mut chunk, honest) = page
        .chunks
        .into_iter()
        .filter(|chunk| chunk.embedding.is_some())
        .find_map(|chunk| {
            let plain = plain_text(&chunk.content);
            let sentence = first_sentence(&plain, policy.min_quote_chars)?;
            let located = locate_quote(&plain, &sentence, policy.min_quote_chars)?;
            Some((chunk, located))
        })
        .ok_or_else(|| format!("{url} has no chunk with a sentence to quote"))?;
    let plain = plain_text(&chunk.content);
    if locate_quote(&plain, forged_quote, policy.min_quote_chars).is_some() {
        return Err("the forged quote is in the page; it would not be a forgery".to_string());
    }
    if let Some(meta) = chunk.meta.as_mut() {
        meta.shard_id = Some(shard.to_string());
        meta.content_hash = format!("{:x}", Sha256::digest(chunk.content.as_bytes()));
    }

    let family = oi_types::source::source_family(url);
    let record = |claim: &str, quote: &str| ClaimRecord {
        claim: claim.to_string(),
        quote: quote.to_string(),
        url: url.to_string(),
        source_family: family.clone(),
        extractor: "forger".to_string(),
    };
    let vectors = embedder
        .embed_text_batch(&[honest.as_str(), forged_claim])
        .map_err(|error| format!("embedding: {error}"))?;
    let index = Arc::new(HybridIndex::new(embedder.dimension()));
    index
        .ingest(std::slice::from_ref(&chunk))
        .map_err(|error| format!("ingest: {error}"))?;
    index
        .ingest_claims(vec![
            (record(&honest, &honest), vectors[0].clone()),
            (record(forged_claim, forged_quote), vectors[1].clone()),
        ])
        .map_err(|error| format!("ingest claims: {error}"))?;

    let node = Node::new(NetworkConfig {
        listen_port: port,
        bootstrap_peers: vec![bootstrap.to_string()],
        enable_mdns: false,
        heartbeat_interval_secs: 5,
        routing_table_path: None,
        ..Default::default()
    })
    .with_index(index, vec![shard.to_string()]);
    let peer_id = node.peer_id;
    let handle = node
        .start()
        .await
        .map_err(|error| format!("node start: {error}"))?;

    println!("dishonest node {peer_id} joined shard {shard}");
    println!("  page:          {url}");
    println!("  honest claim:  {honest}");
    println!("  forged claim:  {forged_claim}");
    println!("  forged quote:  {forged_quote}  (not in the page)");
    println!("  offering both for {seconds} s");
    tokio::time::sleep(std::time::Duration::from_secs(seconds)).await;
    handle.shutdown();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_sentence_long_enough_to_quote_is_taken() {
        let text = "Deserts. Antarctica is the largest desert on Earth by a wide margin. More text follows here.";
        assert_eq!(
            first_sentence(text, 20).as_deref(),
            Some("Antarctica is the largest desert on Earth by a wide margin.")
        );
        assert!(first_sentence("Too short. No.", 20).is_none());
    }
}
