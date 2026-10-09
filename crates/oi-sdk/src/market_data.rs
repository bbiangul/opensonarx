//! Typed market observations for one explicitly requested asset pair.
//!
//! The caller names the asset and currency; nothing here reads a query. Two
//! bounded public market-data observations are joined, their asset, currency
//! and time identity validated, and disagreement exposed instead of
//! manufacturing precision.

use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
const RESOLVE_TIMEOUT: Duration = Duration::from_millis(900);
const CONNECT_TIMEOUT: Duration = Duration::from_millis(900);
const MAX_QUOTE_AGE_MS: u64 = 5 * 60 * 1_000;
const MAX_FUTURE_SKEW_MS: u64 = 60 * 1_000;
const MAX_RELATIVE_SPREAD: f64 = 0.03;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MarketQuoteRecord {
    pub provider: String,
    pub asset_id: String,
    pub asset_symbol: String,
    pub quote_currency: String,
    pub value: f64,
    pub observed_at_unix_ms: u64,
    pub fetched_at_unix_ms: u64,
    pub source_url: String,
    pub observation_kind: String,
}

#[derive(Debug, Clone, Default)]
pub struct MarketQuoteLookup {
    pub records: Vec<MarketQuoteRecord>,
    pub errors: Vec<String>,
    pub lookup_ms: f64,
}

/// One asset pair, in each provider's own identifiers.
#[derive(Debug, Clone, Copy)]
pub struct QuoteSpec<'a> {
    pub asset_id: &'a str,
    pub asset_symbol: &'a str,
    pub quote_currency: &'a str,
    pub coingecko_id: &'a str,
    pub binance_symbol: &'a str,
}

/// Fetch the pair from both providers in parallel.
pub fn lookup(spec: QuoteSpec<'_>) -> MarketQuoteLookup {
    let started = Instant::now();
    let (coingecko, binance) = std::thread::scope(|scope| {
        let coingecko = scope.spawn(|| fetch_coingecko(spec));
        let binance = scope.spawn(|| fetch_binance_average(spec));
        (
            coingecko
                .join()
                .unwrap_or_else(|_| Err("coingecko worker panicked".to_string())),
            binance
                .join()
                .unwrap_or_else(|_| Err("binance worker panicked".to_string())),
        )
    });
    let mut lookup = MarketQuoteLookup {
        lookup_ms: started.elapsed().as_secs_f64() * 1_000.0,
        ..Default::default()
    };
    for result in [coingecko, binance] {
        match result {
            Ok(record) => lookup.records.push(record),
            Err(error) => lookup.errors.push(error),
        }
    }
    lookup
        .records
        .sort_by(|left, right| left.provider.cmp(&right.provider));
    lookup
}

/// Return the records only when they form a bounded, fresh cross-provider
/// observation. CoinGecko is an aggregator and may consume Binance, so two
/// records are not labelled independent observations; the answer lineage
/// keeps that dependency unresolved.
pub fn validated_quote_records(
    records: &[MarketQuoteRecord],
    now_ms: u64,
) -> Option<Vec<MarketQuoteRecord>> {
    let first = records.first()?;
    let eligible = records
        .iter()
        .filter(|record| {
            record.asset_id == first.asset_id
                && record.quote_currency == first.quote_currency
                && record.value.is_finite()
                && record.value > 0.0
                && record.observed_at_unix_ms <= now_ms.saturating_add(MAX_FUTURE_SKEW_MS)
                && now_ms.saturating_sub(record.observed_at_unix_ms) <= MAX_QUOTE_AGE_MS
        })
        .cloned()
        .collect::<Vec<_>>();
    let providers = eligible
        .iter()
        .map(|record| record.provider.as_str())
        .collect::<std::collections::HashSet<_>>();
    if eligible.len() < 2 || providers.len() < 2 {
        return None;
    }
    let min = eligible
        .iter()
        .map(|record| record.value)
        .fold(f64::INFINITY, f64::min);
    let max = eligible
        .iter()
        .map(|record| record.value)
        .fold(f64::NEG_INFINITY, f64::max);
    let midpoint = (min + max) / 2.0;
    if midpoint <= 0.0 || (max - min) / midpoint > MAX_RELATIVE_SPREAD {
        return None;
    }
    Some(eligible)
}

pub fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn fetch_coingecko(spec: QuoteSpec<'_>) -> Result<MarketQuoteRecord, String> {
    let currency = spec.quote_currency.to_ascii_lowercase();
    let url = format!(
        "https://api.coingecko.com/api/v3/simple/price?ids={}&vs_currencies={currency}&include_last_updated_at=true&precision=full",
        spec.coingecko_id
    );
    let value = get_json(&url)?;
    parse_coingecko(spec, &url, &value, unix_time_ms())
}

fn fetch_binance_average(spec: QuoteSpec<'_>) -> Result<MarketQuoteRecord, String> {
    let url = format!(
        "https://data-api.binance.vision/api/v3/avgPrice?symbol={}",
        spec.binance_symbol
    );
    let value = get_json(&url)?;
    parse_binance(spec, &url, &value, unix_time_ms())
}

fn get_json(url: &str) -> Result<serde_json::Value, String> {
    let agent = ureq::Agent::new_with_config(
        ureq::config::Config::builder()
            .timeout_global(Some(REQUEST_TIMEOUT))
            .timeout_resolve(Some(RESOLVE_TIMEOUT))
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .build(),
    );
    let response = agent
        .get(url)
        .header("Accept", "application/json")
        .header("User-Agent", "OpenSonarX/0.1 (+https://opensonarx.com)")
        .call()
        .map_err(|error| format!("market-data request failed for {url}: {error}"))?;
    let bytes = response
        .into_body()
        .read_to_vec()
        .map_err(|error| format!("market-data response read failed for {url}: {error}"))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("market-data JSON parse failed for {url}: {error}"))
}

fn parse_coingecko(
    spec: QuoteSpec<'_>,
    url: &str,
    value: &serde_json::Value,
    fetched_at_unix_ms: u64,
) -> Result<MarketQuoteRecord, String> {
    let currency = spec.quote_currency.to_ascii_lowercase();
    let object = value
        .get(spec.coingecko_id)
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| "CoinGecko response omitted the requested asset".to_string())?;
    let price = object
        .get(&currency)
        .and_then(serde_json::Value::as_f64)
        .filter(|price| price.is_finite() && *price > 0.0)
        .ok_or_else(|| "CoinGecko response omitted a valid quote".to_string())?;
    let observed_at_unix_ms = object
        .get("last_updated_at")
        .and_then(serde_json::Value::as_u64)
        .and_then(|seconds| seconds.checked_mul(1_000))
        .ok_or_else(|| "CoinGecko response omitted last_updated_at".to_string())?;
    Ok(MarketQuoteRecord {
        provider: "coingecko".to_string(),
        asset_id: spec.asset_id.to_string(),
        asset_symbol: spec.asset_symbol.to_string(),
        quote_currency: spec.quote_currency.to_string(),
        value: price,
        observed_at_unix_ms,
        fetched_at_unix_ms,
        source_url: url.to_string(),
        observation_kind: "aggregated_spot_price".to_string(),
    })
}

fn parse_binance(
    spec: QuoteSpec<'_>,
    url: &str,
    value: &serde_json::Value,
    fetched_at_unix_ms: u64,
) -> Result<MarketQuoteRecord, String> {
    let price = value
        .get("price")
        .and_then(serde_json::Value::as_str)
        .and_then(|price| price.parse::<f64>().ok())
        .filter(|price| price.is_finite() && *price > 0.0)
        .ok_or_else(|| "Binance response omitted a valid average price".to_string())?;
    let observed_at_unix_ms = value
        .get("closeTime")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| "Binance response omitted closeTime".to_string())?;
    Ok(MarketQuoteRecord {
        provider: "binance".to_string(),
        asset_id: spec.asset_id.to_string(),
        asset_symbol: spec.asset_symbol.to_string(),
        quote_currency: spec.quote_currency.to_string(),
        value: price,
        observed_at_unix_ms,
        fetched_at_unix_ms,
        source_url: url.to_string(),
        observation_kind: "five_minute_average_price".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> QuoteSpec<'static> {
        QuoteSpec {
            asset_id: "bitcoin",
            asset_symbol: "BTC",
            quote_currency: "BRL",
            coingecko_id: "bitcoin",
            binance_symbol: "BTCBRL",
        }
    }

    #[test]
    fn parsers_require_provider_timestamps_and_preserve_pair_identity() {
        let coingecko = parse_coingecko(
            spec(),
            "https://api.coingecko.test/simple/price",
            &serde_json::json!({"bitcoin":{"brl":397876.09,"last_updated_at":1787429390}}),
            1_787_429_400_000,
        )
        .unwrap();
        let binance = parse_binance(
            spec(),
            "https://data-api.binance.test/api/v3/avgPrice",
            &serde_json::json!({"mins":5,"price":"399444.21508782","closeTime":1787429470264_u64}),
            1_787_429_480_000,
        )
        .unwrap();
        assert_eq!(coingecko.quote_currency, "BRL");
        assert_eq!(coingecko.observed_at_unix_ms, 1_787_429_390_000);
        assert_eq!(binance.asset_symbol, "BTC");
        assert_eq!(binance.observed_at_unix_ms, 1_787_429_470_264);
    }

    #[test]
    fn consensus_requires_two_fresh_close_providers() {
        let now = 1_787_429_500_000;
        let base = MarketQuoteRecord {
            provider: "coingecko".to_string(),
            asset_id: "bitcoin".to_string(),
            asset_symbol: "BTC".to_string(),
            quote_currency: "BRL".to_string(),
            value: 397_876.09,
            observed_at_unix_ms: now - 20_000,
            fetched_at_unix_ms: now,
            source_url: "https://api.coingecko.test".to_string(),
            observation_kind: "aggregated_spot_price".to_string(),
        };
        assert!(validated_quote_records(&[base.clone()], now).is_none());
        let mut second = base.clone();
        second.provider = "binance".to_string();
        second.value = 399_444.22;
        assert!(validated_quote_records(&[base.clone(), second.clone()], now).is_some());
        second.value = 450_000.0;
        assert!(validated_quote_records(&[base.clone(), second], now).is_none());
        let mut stale = base.clone();
        stale.provider = "binance".to_string();
        stale.observed_at_unix_ms = now - MAX_QUOTE_AGE_MS - 1;
        assert!(validated_quote_records(&[base, stale], now).is_none());
    }
}
