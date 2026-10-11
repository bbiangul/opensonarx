//! What an honest node's stored claims look like to a node that checks them
//! by fetching the page itself.
//!
//! A replica today checks each quote against the page text the sending node
//! shipped. The stronger check is to fetch the page independently, and its
//! cost falls on honest nodes: a page that changed, went away or is served
//! differently makes an honest claim fail. This measures that rate.
//!
//! The auditor is the runtime's own `Distiller` (fetch, extract, chunk) and
//! the check is the runtime's `locate_quote`. As a control, quotes taken from
//! a different page are checked against each audited page; they should fail.

use oi_crawl::Distiller;
use oi_sdk::claims::{ClaimPolicy, ClaimRecord};
use oi_types::quote::{locate_quote, plain_text};
use oi_types::traits::CrawlPipeline;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
const RESAMPLES: usize = 2000;

fn read_records(claims_path: &str) -> Result<Vec<ClaimRecord>, String> {
    let raw = std::fs::read_to_string(claims_path)
        .map_err(|error| format!("cannot read {claims_path}: {error}"))?;
    Ok(raw
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .flat_map(|line| {
            line["records"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter_map(|record| serde_json::from_value(record).ok())
        })
        .collect())
}

fn url_order_key(url: &str) -> String {
    format!("{:x}", Sha256::digest(url.as_bytes()))
}

/// What the auditor saw for one URL.
struct Audited {
    url: String,
    family: String,
    error: Option<String>,
    text_chars: usize,
    claims: usize,
    located_in_page: usize,
    located_in_a_chunk: usize,
    control: usize,
    control_located: usize,
}

fn count_located(texts: &[String], quotes: &[String], min_chars: usize) -> usize {
    quotes
        .iter()
        .filter(|quote| {
            texts
                .iter()
                .any(|text| locate_quote(text, quote, min_chars).is_some())
        })
        .count()
}

async fn audit_one(
    distiller: Arc<Distiller>,
    url: String,
    family: String,
    quotes: Vec<String>,
    control_quotes: Vec<String>,
    min_chars: usize,
) -> Audited {
    let mut audited = Audited {
        url: url.clone(),
        family,
        error: None,
        text_chars: 0,
        claims: quotes.len(),
        located_in_page: 0,
        located_in_a_chunk: 0,
        control: control_quotes.len(),
        control_located: 0,
    };
    match tokio::time::timeout(FETCH_TIMEOUT, distiller.crawl(&url)).await {
        Err(_) => audited.error = Some("timeout".to_string()),
        Ok(Err(error)) => audited.error = Some(error.to_string()),
        Ok(Ok(page)) => {
            let whole = vec![plain_text(&page.markdown)];
            let chunks: Vec<String> = page
                .chunks
                .iter()
                .map(|chunk| plain_text(&chunk.content))
                .collect();
            audited.text_chars = whole[0].len();
            audited.located_in_page = count_located(&whole, &quotes, min_chars);
            audited.located_in_a_chunk = count_located(&chunks, &quotes, min_chars);
            audited.control_located = count_located(&whole, &control_quotes, min_chars);
        }
    }
    audited
}

/// A fixed-seed generator: the interval is reproducible from the output file.
struct Lcg(u64);

impl Lcg {
    fn next_index(&mut self, len: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) as usize) % len
    }
}

/// Rate of `numerator / denominator` summed over URLs, with a 95% interval
/// from resampling URLs.
fn pooled_rate(pairs: &[(usize, usize)]) -> Value {
    let total: usize = pairs.iter().map(|pair| pair.1).sum();
    if pairs.is_empty() || total == 0 {
        return json!({ "rate": null, "numerator": 0, "denominator": total });
    }
    let hits: usize = pairs.iter().map(|pair| pair.0).sum();
    let mut generator = Lcg(20261011);
    let mut rates: Vec<f64> = (0..RESAMPLES)
        .filter_map(|_| {
            let (mut numerator, mut denominator) = (0usize, 0usize);
            for _ in 0..pairs.len() {
                let pair = pairs[generator.next_index(pairs.len())];
                numerator += pair.0;
                denominator += pair.1;
            }
            (denominator > 0).then(|| numerator as f64 / denominator as f64)
        })
        .collect();
    rates.sort_by(|a, b| a.total_cmp(b));
    let at = |quantile: f64| rates[((rates.len() - 1) as f64 * quantile).round() as usize];
    json!({
        "rate": hits as f64 / total as f64,
        "numerator": hits,
        "denominator": total,
        "low": at(0.025),
        "high": at(0.975),
    })
}

pub async fn run(
    claims_path: &str,
    urls: usize,
    concurrency: usize,
    output: &str,
) -> Result<(), String> {
    let policy = ClaimPolicy::default();
    let mut by_url: BTreeMap<String, Vec<ClaimRecord>> = BTreeMap::new();
    for record in read_records(claims_path)? {
        by_url.entry(record.url.clone()).or_default().push(record);
    }
    let mut ordered: Vec<(String, Vec<ClaimRecord>)> = by_url.into_iter().collect();
    ordered.sort_by_key(|(url, _)| url_order_key(url));
    let stored_urls = ordered.len();
    let stored_claims: usize = ordered.iter().map(|(_, records)| records.len()).sum();
    let sample = urls.min(stored_urls);

    // The control for a URL: the quotes of the next URL in the same order
    // that belongs to another source family.
    let jobs: Vec<(String, String, Vec<String>, Vec<String>)> = (0..sample)
        .map(|position| {
            let (url, records) = &ordered[position];
            let family = records[0].source_family.clone();
            let control = (1..stored_urls)
                .map(|step| &ordered[(position + step) % stored_urls].1)
                .find(|other| other[0].source_family != family)
                .map(|other| other.iter().map(|record| record.quote.clone()).collect())
                .unwrap_or_default();
            let quotes = records.iter().map(|record| record.quote.clone()).collect();
            (url.clone(), family, quotes, control)
        })
        .collect();

    let min_chars = policy.min_quote_chars;
    let mut audited: Vec<Audited> = {
        let distiller = Arc::new(Distiller::new());
        let mut pending = jobs.into_iter();
        let mut active = tokio::task::JoinSet::new();
        let mut done = Vec::new();
        loop {
            while active.len() < concurrency.max(1) {
                let Some((url, family, quotes, control)) = pending.next() else {
                    break;
                };
                active.spawn(audit_one(
                    distiller.clone(),
                    url,
                    family,
                    quotes,
                    control,
                    min_chars,
                ));
            }
            match active.join_next().await {
                Some(Ok(result)) => {
                    done.push(result);
                    if done.len() % 25 == 0 {
                        eprintln!("audited {} of {sample}", done.len());
                    }
                }
                Some(Err(error)) => eprintln!("audit task failed: {error}"),
                None => break,
            }
        }
        done
    };
    audited.sort_by_key(|entry| url_order_key(&entry.url));

    let fetched: Vec<&Audited> = audited
        .iter()
        .filter(|entry| entry.error.is_none())
        .collect();
    let pairs = |pick: fn(&Audited) -> (usize, usize)| -> Vec<(usize, usize)> {
        fetched.iter().map(|entry| pick(entry)).collect()
    };
    let fully_located = fetched
        .iter()
        .filter(|entry| entry.located_in_page == entry.claims)
        .count();
    let none_located = fetched
        .iter()
        .filter(|entry| entry.located_in_page == 0)
        .count();
    let mut errors: BTreeMap<String, usize> = BTreeMap::new();
    for entry in audited.iter().filter(|entry| entry.error.is_some()) {
        let message = entry.error.clone().unwrap_or_default();
        // Group by what went wrong, not by which URL it went wrong for.
        let kind = message
            .split(" for ")
            .next()
            .unwrap_or(&message)
            .chars()
            .take(60)
            .collect::<String>();
        *errors.entry(kind).or_default() += 1;
    }

    let report = json!({
        "schema": "opensonarx.claim_audit.v1",
        "claims": claims_path,
        "stored_urls": stored_urls,
        "stored_claims": stored_claims,
        "sample_order": "sha256(url) ascending",
        "min_quote_chars": min_chars,
        "fetch_timeout_secs": FETCH_TIMEOUT.as_secs(),
        "summary": {
            "urls_audited": audited.len(),
            "urls_fetched": fetched.len(),
            "fetch_errors": errors,
            "honest_claims_located_in_page": pooled_rate(&pairs(|e| (e.located_in_page, e.claims))),
            "honest_claims_located_in_a_chunk": pooled_rate(&pairs(|e| (e.located_in_a_chunk, e.claims))),
            "urls_with_every_claim_located": fully_located,
            "urls_with_no_claim_located": none_located,
            "foreign_quotes_located_in_page": pooled_rate(&pairs(|e| (e.control_located, e.control))),
        },
        "urls": audited.iter().map(|entry| json!({
            "url": entry.url,
            "source_family": entry.family,
            "error": entry.error,
            "text_chars": entry.text_chars,
            "claims": entry.claims,
            "located_in_page": entry.located_in_page,
            "located_in_a_chunk": entry.located_in_a_chunk,
            "control": entry.control,
            "control_located": entry.control_located,
        })).collect::<Vec<_>>(),
    });
    std::fs::write(
        output,
        serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("cannot write {output}: {error}"))?;
    println!(
        "{}",
        serde_json::to_string_pretty(&report["summary"]).unwrap_or_default()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pooled_rate_sums_over_urls_and_brackets_the_point() {
        let rate = pooled_rate(&[(9, 10), (10, 10), (0, 10), (10, 10)]);
        assert_eq!(rate["numerator"], 29);
        assert_eq!(rate["denominator"], 40);
        let (low, high) = (
            rate["low"].as_f64().unwrap(),
            rate["high"].as_f64().unwrap(),
        );
        assert!(low <= 0.725 && 0.725 <= high);
    }

    #[test]
    fn a_quote_counts_when_any_text_holds_it() {
        let texts = vec![
            "The Sahara is the largest hot desert in the world.".to_string(),
            "Antarctica is the largest desert on Earth by area.".to_string(),
        ];
        let quotes = vec![
            "Antarctica is the largest desert on Earth".to_string(),
            "The Gobi is the largest desert in Asia by far".to_string(),
        ];
        assert_eq!(count_located(&texts, &quotes, 20), 1);
    }
}
