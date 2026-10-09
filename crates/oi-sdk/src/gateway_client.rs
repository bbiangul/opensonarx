use oi_types::error::ErrorCode;
use oi_types::gateway::{
    FeedbackStats, FiatDisputeOpenRequest, FiatDisputeOpenResponse, GatewayDisputeInfoResponse,
    GatewayDisputeListResponse,
};
use oi_types::{
    DisputeInfo, Error, GatewaySearchRequest, GatewaySearchResponse, QueryFilters, QueryQuota,
    Result, SearchResultItem, TrustCheckResult, UserAccount,
};
use std::time::Duration;

/// HTTP client for the OpenSonarX gateway (web2 mode).
///
/// Communicates with the gateway service over REST, optionally using a Bearer
/// API key for authentication. When `api_key` is `None`, requests are sent
/// without auth (anonymous mode — IP-based rate limiting).
pub struct GatewayClient {
    base_url: String,
    api_key: Option<String>,
    http: ureq::Agent,
}

impl GatewayClient {
    /// Create a new gateway client.
    ///
    /// * `base_url` — e.g. `"https://api.opensonarx.com"` (no trailing slash)
    /// * `api_key` — the `oik_...` key obtained via `oi login`, or `None` for anonymous
    /// * `timeout_ms` — request timeout in milliseconds
    pub fn new(base_url: String, api_key: Option<String>, timeout_ms: u64) -> Self {
        let http = ureq::Agent::new_with_config(
            ureq::config::Config::builder()
                .timeout_global(Some(Duration::from_millis(timeout_ms)))
                .build(),
        );
        Self {
            base_url,
            api_key,
            http,
        }
    }

    /// Add auth header to a request builder if an API key is configured.
    fn auth_post(&self, url: &str) -> ureq::RequestBuilder<ureq::typestate::WithBody> {
        let mut req = self.http.post(url).content_type("application/json");
        if let Some(key) = &self.api_key {
            req = req.header("Authorization", &format!("Bearer {key}"));
        }
        req
    }

    /// Add auth header to a GET request builder if an API key is configured.
    fn auth_get(&self, url: &str) -> ureq::RequestBuilder<ureq::typestate::WithoutBody> {
        let mut req = self.http.get(url);
        if let Some(key) = &self.api_key {
            req = req.header("Authorization", &format!("Bearer {key}"));
        }
        req
    }

    /// Search via the gateway. Returns up to `top_k` results.
    pub fn search(
        &self,
        query: &str,
        top_k: usize,
        filters: &QueryFilters,
    ) -> Result<Vec<SearchResultItem>> {
        let url = format!("{}/v1/search", self.base_url);
        let body = GatewaySearchRequest {
            query: query.to_string(),
            top_k,
            min_stake: filters.min_stake_usd,
            freshness: filters.freshness.clone(),
            types: filters.types.clone(),
            locale: filters.locale.clone(),
            gl: filters.gl.clone(),
            site: filters.site.clone(),
            min_trust: filters.min_trust.clone(),
            fields: None,
            wait_for_content: None,
        };

        let json = serde_json::to_string(&body)
            .map_err(|e| Error::Internal(format!("JSON serialize: {e}")))?;

        let resp = self
            .auth_post(&url)
            .send(json.as_bytes())
            .map_err(map_ureq_error)?;

        let gateway_resp: GatewaySearchResponse = read_json_body(resp)?;
        Ok(gateway_resp.results)
    }

    /// Submit feedback on a search result.
    ///
    /// For maximum data moat value, include the original query, result URL, and rank
    /// so the system can build the Query -> Solved Problem mapping.
    pub fn feedback(&self, result_id: &str, useful: bool) -> Result<()> {
        self.feedback_with_context(result_id, useful, None, None, None)
    }

    /// Submit feedback with full query context (maximises data moat accumulation).
    pub fn feedback_with_context(
        &self,
        result_id: &str,
        useful: bool,
        query: Option<&str>,
        result_url: Option<&str>,
        result_rank: Option<u32>,
    ) -> Result<()> {
        let url = format!("{}/v1/feedback", self.base_url);
        let mut body = serde_json::json!({
            "result_id": result_id,
            "useful": useful,
        });

        if let Some(q) = query {
            body["query"] = serde_json::Value::String(q.to_string());
        }
        if let Some(u) = result_url {
            body["result_url"] = serde_json::Value::String(u.to_string());
        }
        if let Some(r) = result_rank {
            body["result_rank"] = serde_json::Value::Number(r.into());
        }

        let json = serde_json::to_string(&body)
            .map_err(|e| Error::Internal(format!("JSON serialize: {e}")))?;

        self.auth_post(&url)
            .send(json.as_bytes())
            .map_err(map_ureq_error)?;

        Ok(())
    }

    /// Get aggregate feedback statistics (data moat metrics).
    pub fn feedback_stats(&self) -> Result<FeedbackStats> {
        let url = format!("{}/v1/feedback/stats", self.base_url);

        let resp = self.auth_get(&url).call().map_err(map_ureq_error)?;

        read_json_body(resp)
    }

    /// Trust-check a URL (stake lookup + risk assessment).
    pub fn verify_url(&self, url: &str) -> Result<TrustCheckResult> {
        let endpoint = format!("{}/v1/verify?url={}", self.base_url, urlencoded(url));

        let resp = self.auth_get(&endpoint).call().map_err(map_ureq_error)?;

        read_json_body(resp)
    }

    /// Get current quota status.
    pub fn quota(&self) -> Result<QueryQuota> {
        let url = format!("{}/v1/quota", self.base_url);

        let resp = self.auth_get(&url).call().map_err(map_ureq_error)?;

        read_json_body(resp)
    }

    /// Open a fiat-backed dispute via the gateway.
    pub fn open_fiat_dispute(
        &self,
        domain: &str,
        reason: &str,
        payment_method_id: &str,
    ) -> Result<FiatDisputeOpenResponse> {
        let url = format!("{}/v1/disputes/open", self.base_url);
        let body = FiatDisputeOpenRequest {
            domain: domain.to_string(),
            reason: reason.to_string(),
            payment_method_id: payment_method_id.to_string(),
        };

        let json = serde_json::to_string(&body)
            .map_err(|e| Error::Internal(format!("JSON serialize: {e}")))?;

        let resp = self
            .auth_post(&url)
            .send(json.as_bytes())
            .map_err(map_ureq_error)?;

        read_json_body(resp)
    }

    /// List active disputes via the gateway.
    pub fn list_disputes(&self) -> Result<Vec<DisputeInfo>> {
        let url = format!("{}/v1/disputes", self.base_url);

        let resp = self.auth_get(&url).call().map_err(map_ureq_error)?;

        let list: GatewayDisputeListResponse = read_json_body(resp)?;
        Ok(list.disputes)
    }

    /// Get dispute details via the gateway.
    pub fn get_dispute(&self, dispute_id: &str) -> Result<GatewayDisputeInfoResponse> {
        let url = format!("{}/v1/disputes/{}", self.base_url, urlencoded(dispute_id));

        let resp = self.auth_get(&url).call().map_err(map_ureq_error)?;

        read_json_body(resp)
    }

    /// Get the most-queried-domains leaderboard from the gateway.
    pub fn leaderboard(&self, limit: usize) -> Result<Vec<oi_types::LeaderboardEntry>> {
        let url = format!("{}/v1/leaderboard?limit={limit}", self.base_url);

        let resp = self.auth_get(&url).call().map_err(map_ureq_error)?;

        read_json_body(resp)
    }

    /// Get account information.
    pub fn account(&self) -> Result<UserAccount> {
        let url = format!("{}/v1/account", self.base_url);

        let resp = self.auth_get(&url).call().map_err(map_ureq_error)?;

        read_json_body(resp)
    }
}

/// Read and deserialize a JSON response body.
fn read_json_body<T: serde::de::DeserializeOwned>(
    resp: ureq::http::Response<ureq::Body>,
) -> Result<T> {
    let bytes = resp
        .into_body()
        .read_to_vec()
        .map_err(|e| Error::Internal(format!("Failed to read response: {e}")))?;

    serde_json::from_slice(&bytes)
        .map_err(|e| Error::Internal(format!("Failed to parse response JSON: {e}")))
}

/// Minimal percent-encoding for URL query parameter values.
fn urlencoded(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push('%');
                out.push(char::from(HEX[(b >> 4) as usize]));
                out.push(char::from(HEX[(b & 0xf) as usize]));
            }
        }
    }
    out
}

const HEX: [u8; 16] = *b"0123456789ABCDEF";

/// Map ureq errors to oi_types::Error with appropriate error codes.
fn map_ureq_error(err: ureq::Error) -> Error {
    match err {
        ureq::Error::StatusCode(status) => {
            let code = match status {
                401 => ErrorCode::InvalidApiKey,
                429 => ErrorCode::QuotaExceeded,
                503 => ErrorCode::FreeTierExhausted,
                _ => ErrorCode::NodeUnreachable,
            };
            Error::protocol(code, format!("Gateway returned HTTP {status}"))
        }
        _ => Error::Internal(format!("Gateway request failed: {err}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_urlencoded() {
        assert_eq!(urlencoded("hello"), "hello");
        assert_eq!(
            urlencoded("https://example.com/path?q=1"),
            "https%3A%2F%2Fexample.com%2Fpath%3Fq%3D1"
        );
    }

    #[test]
    fn test_map_ureq_error_codes() {
        let err = map_ureq_error(ureq::Error::StatusCode(401));
        assert_eq!(err.code(), Some(ErrorCode::InvalidApiKey));

        let err = map_ureq_error(ureq::Error::StatusCode(429));
        assert_eq!(err.code(), Some(ErrorCode::QuotaExceeded));

        let err = map_ureq_error(ureq::Error::StatusCode(503));
        assert_eq!(err.code(), Some(ErrorCode::FreeTierExhausted));
    }
}
