use oi_types::Result;
#[cfg(test)]
use std::net::Ipv4Addr;
use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

const USER_AGENT: &str = "OpenSonarX/0.1 (https://opensonarx.com; search crawler)";
const TIMEOUT: Duration = Duration::from_secs(15);
/// Maximum response body size (10 MB).
const MAX_BODY_BYTES: usize = 10 * 1024 * 1024;
/// Maximum retry attempts for transient failures.
const MAX_RETRIES: u32 = 3;
/// Base delay for exponential backoff (doubles each retry).
const RETRY_BASE_DELAY: Duration = Duration::from_millis(500);

/// Maximum number of redirects to follow manually (with SSRF validation at each hop).
const MAX_REDIRECTS: u32 = 10;

/// Shared HTTP client — reused across all fetches to leverage connection pooling.
/// Redirects are disabled so we can validate each hop against SSRF blocklists.
static HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("failed to build HTTP client")
});

pub struct FetchResult {
    pub url: String,
    pub html: String,
    pub status: u16,
    pub content_type: String,
    pub fetch_time_ms: u64,
}

/// Whether an HTTP status code is retryable (server error or rate-limited).
fn is_retryable_status(status: u16) -> bool {
    matches!(status, 429 | 500 | 502 | 503 | 504)
}

/// Check if an IP address is in a blocked (private/internal) range.
fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_unspecified()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                // 100.64.0.0/10 (Carrier-grade NAT)
                || (v4.octets()[0] == 100 && (v4.octets()[1] & 0xC0) == 64)
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || is_ipv6_ula(&v6)       // fc00::/7
                || is_ipv6_link_local(&v6) // fe80::/10
                // IPv4-mapped IPv6 (::ffff:a.b.c.d) — check the inner v4
                || v6.to_ipv4_mapped().is_some_and(|v4| is_blocked_ip(IpAddr::V4(v4)))
        }
    }
}

/// Check if an IPv6 address is in the ULA range (fc00::/7).
fn is_ipv6_ula(v6: &Ipv6Addr) -> bool {
    (v6.segments()[0] & 0xFE00) == 0xFC00
}

/// Check if an IPv6 address is link-local (fe80::/10).
fn is_ipv6_link_local(v6: &Ipv6Addr) -> bool {
    (v6.segments()[0] & 0xFFC0) == 0xFE80
}

/// Validate the result of DNS resolution before opening a connection.
///
/// Every returned address must be public. Requiring all addresses to pass is
/// important because otherwise an attacker could mix a public address with a
/// private one and rely on the HTTP client selecting the unsafe address.
fn validate_resolved_addrs(host: &str, resolved: &[SocketAddr]) -> Result<()> {
    if resolved.is_empty() {
        return Err(oi_types::Error::Internal(format!(
            "DNS resolution returned no addresses for {host}"
        )));
    }

    for addr in resolved {
        if is_blocked_ip(addr.ip()) {
            return Err(oi_types::Error::Internal(format!(
                "blocked URL: {host} resolves to private/internal IP {}",
                addr.ip()
            )));
        }
    }

    Ok(())
}

/// Validate that a URL is safe to fetch (not targeting internal/private networks).
async fn validate_url_safe(url_str: &str) -> Result<()> {
    let parsed = url::Url::parse(url_str)
        .map_err(|e| oi_types::Error::Internal(format!("invalid URL: {e}")))?;

    // Only allow http and https schemes
    match parsed.scheme() {
        "http" | "https" => {}
        scheme => {
            return Err(oi_types::Error::Internal(format!(
                "blocked URL scheme: {scheme} (only http/https allowed)"
            )));
        }
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| oi_types::Error::Internal("URL has no host".to_string()))?;

    let port = parsed.port_or_known_default().unwrap_or(80);
    let addr_str = format!("{host}:{port}");

    // Resolve hostname to IP addresses
    let addrs = tokio::net::lookup_host(&addr_str)
        .await
        .map_err(|e| oi_types::Error::Internal(format!("DNS resolution failed for {host}: {e}")))?;

    let resolved: Vec<_> = addrs.collect();
    validate_resolved_addrs(host, &resolved)
}

/// Fetch raw bytes from a URL (for .gz sitemaps and other binary content).
/// Same SSRF protection and retry logic as `fetch_url`.
pub async fn fetch_bytes(url: &str) -> Result<Vec<u8>> {
    validate_url_safe(url).await?;

    let start = Instant::now();
    let mut last_err = None;

    for attempt in 0..=MAX_RETRIES {
        if attempt > 0 {
            let delay = RETRY_BASE_DELAY * 2u32.saturating_pow(attempt - 1);
            tracing::debug!("Retry {attempt}/{MAX_RETRIES} for {url} after {delay:?}");
            tokio::time::sleep(delay).await;
        }

        match fetch_bytes_once(url).await {
            Ok((bytes, status)) => {
                if is_retryable_status(status) && attempt < MAX_RETRIES {
                    tracing::debug!("Retryable status {status} for {url}");
                    last_err = Some(oi_types::Error::Internal(format!(
                        "HTTP {status} from {url}"
                    )));
                    continue;
                }
                tracing::debug!("fetch_bytes {url} in {}ms", start.elapsed().as_millis());
                return Ok(bytes);
            }
            Err(e) => {
                last_err = Some(e);
                if attempt < MAX_RETRIES {
                    tracing::debug!("Transient fetch error for {url}, will retry");
                }
            }
        }
    }

    Err(last_err.unwrap_or_else(|| oi_types::Error::Internal("fetch failed".to_string())))
}

/// Single bytes fetch attempt with redirect following and SSRF validation.
async fn fetch_bytes_once(url: &str) -> Result<(Vec<u8>, u16)> {
    let mut current_url = url.to_string();

    for redirect_count in 0..=MAX_REDIRECTS {
        let response = HTTP_CLIENT
            .get(&current_url)
            .send()
            .await
            .map_err(|e| oi_types::Error::Internal(format!("fetch failed: {e}")))?;

        let status = response.status().as_u16();

        if (300..400).contains(&status) {
            if redirect_count >= MAX_REDIRECTS {
                return Err(oi_types::Error::Internal(format!(
                    "too many redirects (max {MAX_REDIRECTS}) for {url}"
                )));
            }

            let location = response
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| {
                    oi_types::Error::Internal(format!("redirect {status} without Location header"))
                })?;

            let resolved = url::Url::parse(location)
                .or_else(|_| url::Url::parse(&current_url).and_then(|base| base.join(location)))
                .map_err(|e| {
                    oi_types::Error::Internal(format!("invalid redirect URL {location}: {e}"))
                })?;

            validate_url_safe(resolved.as_str()).await?;
            current_url = resolved.to_string();
            continue;
        }

        if let Some(len) = response.content_length()
            && len as usize > MAX_BODY_BYTES
        {
            return Err(oi_types::Error::Internal(format!(
                "response too large: {len} bytes (max {MAX_BODY_BYTES})"
            )));
        }

        let bytes = response
            .bytes()
            .await
            .map_err(|e| oi_types::Error::Internal(format!("body read failed: {e}")))?;

        if bytes.len() > MAX_BODY_BYTES {
            return Err(oi_types::Error::Internal(format!(
                "response too large: {} bytes (max {MAX_BODY_BYTES})",
                bytes.len()
            )));
        }

        return Ok((bytes.to_vec(), status));
    }

    Err(oi_types::Error::Internal(format!(
        "too many redirects (max {MAX_REDIRECTS}) for {url}"
    )))
}

pub async fn fetch_url(url: &str) -> Result<FetchResult> {
    // SSRF protection: validate URL before any network requests
    validate_url_safe(url).await?;

    if let Some(api_url) = bcb_page_api_url(url) {
        // bcb.gov.br serves an Angular shell to static clients. The public
        // page API used by that shell contains the canonical editorial HTML.
        // Fetch it directly to avoid paying for a known-empty shell first.
        // Keep the browser URL as provenance.
        if let Ok(api_result) = fetch_url_with_retries(&api_url).await
            && api_result.status < 400
            && let Some(expanded) = bcb_api_content(&api_result.html)
        {
            return Ok(FetchResult {
                url: url.to_string(),
                html: expanded,
                status: api_result.status,
                content_type: "text/html; source=bcb-page-api".to_string(),
                fetch_time_ms: api_result.fetch_time_ms,
            });
        }
    }
    fetch_url_with_retries(url).await
}

async fn fetch_url_with_retries(url: &str) -> Result<FetchResult> {
    validate_url_safe(url).await?;

    let start = Instant::now();
    let mut last_err = None;

    for attempt in 0..=MAX_RETRIES {
        if attempt > 0 {
            let delay = RETRY_BASE_DELAY * 2u32.saturating_pow(attempt - 1);
            tracing::debug!("Retry {attempt}/{MAX_RETRIES} for {url} after {delay:?}");
            tokio::time::sleep(delay).await;
        }

        match fetch_once(url).await {
            Ok(result) => {
                if is_retryable_status(result.status) && attempt < MAX_RETRIES {
                    tracing::debug!("Retryable status {} for {url}", result.status);
                    last_err = Some(oi_types::Error::Internal(format!(
                        "HTTP {} from {url}",
                        result.status
                    )));
                    continue;
                }
                let mut result = result;
                result.fetch_time_ms = start.elapsed().as_millis() as u64;
                return Ok(result);
            }
            Err(e) => {
                last_err = Some(e);
                if attempt < MAX_RETRIES {
                    tracing::debug!("Transient fetch error for {url}, will retry");
                }
            }
        }
    }

    Err(last_err.unwrap_or_else(|| oi_types::Error::Internal("fetch failed".to_string())))
}

fn bcb_page_api_url(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?.trim_start_matches("www.");
    if host != "bcb.gov.br" || parsed.path().starts_with("/api/") {
        return None;
    }
    let path = parsed.path().trim_matches('/');
    let mut segments = path.split('/');
    let trunk = segments.next()?;
    let slug = segments.collect::<Vec<_>>().join("/");
    if trunk.is_empty() || slug.is_empty() {
        return None;
    }
    Some(format!(
        "{}://{}/api/paginasite/sitebcb/{trunk}/{slug}",
        parsed.scheme(),
        parsed.host_str()?
    ))
}

fn bcb_api_content(json: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let html = value.get("conteudo")?.as_str()?.trim();
    if html.is_empty() {
        return None;
    }

    // Some editorial blocks are HTML stored inside Angular input attributes
    // such as [card_body]="'<p>...</p>'". A static HTML extractor cannot see
    // attribute text, so append those same retained fragments as ordinary DOM
    // nodes. This is deterministic decoding of the official API response.
    let fragment = scraper::Html::parse_fragment(html);
    let selector = scraper::Selector::parse("*").ok()?;
    let mut embedded = String::new();
    for element in fragment.select(&selector) {
        for (name, attribute) in element.value().attrs() {
            if matches!(name, "[card_body]" | "[texto]" | "card_body" | "texto") {
                let decoded = attribute
                    .trim()
                    .trim_matches('\'')
                    .replace("\\&#34;", "&#34;");
                if decoded.contains('<') && decoded.contains('>') {
                    embedded.push_str(&decoded);
                    embedded.push('\n');
                }
            }
        }
    }
    Some(if embedded.is_empty() {
        html.to_string()
    } else {
        format!("{html}\n<section data-opensonarx-expanded=\"bcb-api\">{embedded}</section>")
    })
}

/// Single fetch attempt (no retry). Manually follows redirects with SSRF
/// validation at each hop to prevent redirect-based SSRF bypasses.
async fn fetch_once(url: &str) -> Result<FetchResult> {
    let start = Instant::now();
    let mut current_url = url.to_string();

    for redirect_count in 0..=MAX_REDIRECTS {
        let response = HTTP_CLIENT
            .get(&current_url)
            .send()
            .await
            .map_err(|e| oi_types::Error::Internal(format!("fetch failed: {e}")))?;

        let status = response.status().as_u16();

        // Handle redirects: 301, 302, 303, 307, 308
        if (300..400).contains(&status) {
            if redirect_count >= MAX_REDIRECTS {
                return Err(oi_types::Error::Internal(format!(
                    "too many redirects (max {MAX_REDIRECTS}) for {url}"
                )));
            }

            let location = response
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| {
                    oi_types::Error::Internal(format!("redirect {status} without Location header"))
                })?;

            // Resolve relative URLs against the current URL
            let resolved = url::Url::parse(location)
                .or_else(|_| url::Url::parse(&current_url).and_then(|base| base.join(location)))
                .map_err(|e| {
                    oi_types::Error::Internal(format!("invalid redirect URL {location}: {e}"))
                })?;

            // SSRF check on the redirect target
            validate_url_safe(resolved.as_str()).await?;

            current_url = resolved.to_string();
            continue;
        }

        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("text/html")
            .to_string();

        // Enforce body size limit to prevent OOM from malicious/huge pages
        if let Some(len) = response.content_length()
            && len as usize > MAX_BODY_BYTES
        {
            return Err(oi_types::Error::Internal(format!(
                "response too large: {len} bytes (max {MAX_BODY_BYTES})"
            )));
        }

        let bytes = response
            .bytes()
            .await
            .map_err(|e| oi_types::Error::Internal(format!("body read failed: {e}")))?;

        if bytes.len() > MAX_BODY_BYTES {
            return Err(oi_types::Error::Internal(format!(
                "response too large: {} bytes (max {MAX_BODY_BYTES})",
                bytes.len()
            )));
        }

        let html = String::from_utf8_lossy(&bytes).into_owned();
        let fetch_time_ms = start.elapsed().as_millis() as u64;

        return Ok(FetchResult {
            url: current_url,
            html,
            status,
            content_type,
            fetch_time_ms,
        });
    }

    Err(oi_types::Error::Internal(format!(
        "too many redirects (max {MAX_REDIRECTS}) for {url}"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_agent_set() {
        assert!(USER_AGENT.contains("OpenSonarX"));
    }

    #[test]
    fn test_timeout_is_15s() {
        assert_eq!(TIMEOUT, Duration::from_secs(15));
    }

    #[test]
    fn test_retryable_statuses() {
        assert!(is_retryable_status(429));
        assert!(is_retryable_status(500));
        assert!(is_retryable_status(502));
        assert!(is_retryable_status(503));
        assert!(is_retryable_status(504));
        assert!(!is_retryable_status(200));
        assert!(!is_retryable_status(404));
        assert!(!is_retryable_status(403));
    }

    #[test]
    fn test_retry_constants() {
        assert_eq!(MAX_RETRIES, 3);
        assert_eq!(RETRY_BASE_DELAY, Duration::from_millis(500));
    }

    #[test]
    fn bcb_angular_page_maps_to_its_public_editorial_api() {
        assert_eq!(
            bcb_page_api_url("https://www.bcb.gov.br/estabilidadefinanceira/pix-sobre"),
            Some(
                "https://www.bcb.gov.br/api/paginasite/sitebcb/estabilidadefinanceira/pix-sobre"
                    .to_string()
            )
        );
        assert!(bcb_page_api_url("https://example.com/estabilidadefinanceira/pix-sobre").is_none());
        assert!(bcb_page_api_url("https://www.bcb.gov.br/api/config").is_none());
    }

    #[test]
    fn bcb_api_exposes_html_stored_in_angular_attributes() {
        let json = serde_json::json!({
            "conteudo": r#"<h1>Sobre o Pix</h1><bcb-accordion-page [card_body]="'<p>O meio de pagamento criado pelo Banco Central.</p>'"></bcb-accordion-page>"#
        })
        .to_string();
        let expanded = bcb_api_content(&json).expect("expanded BCB content");
        let markdown = crate::extractor::html_to_markdown(&expanded).markdown;
        assert!(
            markdown.contains("O meio de pagamento criado pelo Banco Central"),
            "Angular attribute content was lost: {markdown}"
        );
    }

    #[tokio::test]
    #[ignore = "requires public Banco Central website"]
    async fn live_bcb_angular_page_fetches_canonical_editorial_content() {
        let result = fetch_url("https://www.bcb.gov.br/estabilidadefinanceira/pix-sobre")
            .await
            .expect("live BCB page");
        assert!(result.content_type.contains("bcb-page-api"));
        assert!(result.html.contains("criado pelo Banco Central"));
        assert!(result.html.contains("O BC criou e opera a infraestrutura"));
    }

    // ── SSRF protection tests ──

    #[test]
    fn test_blocked_ip_loopback() {
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))));
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2))));
        assert!(is_blocked_ip(IpAddr::V6(Ipv6Addr::LOCALHOST)));
    }

    #[test]
    fn test_blocked_ip_unspecified() {
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::UNSPECIFIED)));
        assert!(is_blocked_ip(IpAddr::V6(Ipv6Addr::UNSPECIFIED)));
    }

    #[test]
    fn test_blocked_ip_private_ranges() {
        // 10.0.0.0/8
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(10, 255, 255, 255))));
        // 172.16.0.0/12
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(172, 16, 0, 1))));
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(172, 31, 255, 255))));
        // 192.168.0.0/16
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(192, 168, 0, 1))));
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(192, 168, 255, 255))));
    }

    #[test]
    fn test_blocked_ip_link_local() {
        // 169.254.0.0/16 (cloud metadata endpoint lives here)
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254))));
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(169, 254, 0, 1))));
    }

    #[test]
    fn test_blocked_ip_ipv6_ula() {
        // fc00::/7
        let ula: Ipv6Addr = "fd00::1".parse().unwrap();
        assert!(is_blocked_ip(IpAddr::V6(ula)));
        let ula2: Ipv6Addr = "fc00::1".parse().unwrap();
        assert!(is_blocked_ip(IpAddr::V6(ula2)));
    }

    #[test]
    fn test_blocked_ip_ipv6_link_local() {
        // fe80::/10
        let ll: Ipv6Addr = "fe80::1".parse().unwrap();
        assert!(is_blocked_ip(IpAddr::V6(ll)));
    }

    #[test]
    fn test_blocked_ip_carrier_grade_nat() {
        // 100.64.0.0/10
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(100, 64, 0, 1))));
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(100, 127, 255, 255))));
        // Not in CGNAT range
        assert!(!is_blocked_ip(IpAddr::V4(Ipv4Addr::new(100, 128, 0, 1))));
    }

    #[test]
    fn test_allowed_ip_public() {
        // Public IPs should NOT be blocked
        assert!(!is_blocked_ip(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
        assert!(!is_blocked_ip(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))));
        assert!(!is_blocked_ip(IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34))));
        // Public IPv6
        let public_v6: Ipv6Addr = "2607:f8b0:4004:800::200e".parse().unwrap();
        assert!(!is_blocked_ip(IpAddr::V6(public_v6)));
    }

    #[test]
    fn test_validate_resolved_addrs_allows_only_public_results() {
        let resolved = [
            "93.184.216.34:443".parse().unwrap(),
            "[2607:f8b0:4004:800::200e]:443".parse().unwrap(),
        ];
        assert!(validate_resolved_addrs("example.com", &resolved).is_ok());
    }

    #[test]
    fn test_validate_resolved_addrs_rejects_empty_and_mixed_results() {
        assert!(validate_resolved_addrs("empty.example", &[]).is_err());

        let mixed = [
            "93.184.216.34:443".parse().unwrap(),
            "127.0.0.1:443".parse().unwrap(),
        ];
        assert!(validate_resolved_addrs("rebinding.example", &mixed).is_err());
    }

    #[test]
    fn test_blocked_ip_ipv4_mapped_v6() {
        // ::ffff:127.0.0.1 should be blocked (maps to loopback)
        let mapped: Ipv6Addr = "::ffff:127.0.0.1".parse().unwrap();
        assert!(is_blocked_ip(IpAddr::V6(mapped)));
        // ::ffff:10.0.0.1 should be blocked (maps to private)
        let mapped_priv: Ipv6Addr = "::ffff:10.0.0.1".parse().unwrap();
        assert!(is_blocked_ip(IpAddr::V6(mapped_priv)));
        // ::ffff:8.8.8.8 should be allowed (maps to public)
        let mapped_pub: Ipv6Addr = "::ffff:8.8.8.8".parse().unwrap();
        assert!(!is_blocked_ip(IpAddr::V6(mapped_pub)));
    }

    #[tokio::test]
    async fn test_validate_url_rejects_non_http_schemes() {
        assert!(validate_url_safe("ftp://example.com/file").await.is_err());
        assert!(validate_url_safe("file:///etc/passwd").await.is_err());
        assert!(validate_url_safe("gopher://evil.com").await.is_err());
    }

    #[tokio::test]
    async fn test_validate_url_rejects_metadata_endpoint() {
        // AWS/GCP metadata endpoint
        let result = validate_url_safe("http://169.254.169.254/latest/meta-data/").await;
        assert!(result.is_err());
        let err_msg = format!("{:?}", result.unwrap_err());
        assert!(err_msg.contains("private/internal") || err_msg.contains("blocked"));
    }

    #[tokio::test]
    async fn test_validate_url_rejects_localhost() {
        assert!(
            validate_url_safe("http://127.0.0.1:8080/admin")
                .await
                .is_err()
        );
        assert!(validate_url_safe("http://[::1]/secret").await.is_err());
    }

    #[tokio::test]
    async fn test_validate_url_rejects_private_ips() {
        assert!(validate_url_safe("http://10.0.0.1/internal").await.is_err());
        assert!(
            validate_url_safe("http://192.168.1.1/router")
                .await
                .is_err()
        );
        assert!(validate_url_safe("http://172.16.0.1/").await.is_err());
    }

    #[tokio::test]
    #[ignore = "live test requires public DNS"]
    async fn test_validate_url_allows_public_domains_live() {
        // The deterministic address-policy path is tested above. This test is
        // intentionally opt-in because it verifies the host DNS environment.
        assert!(validate_url_safe("https://example.com").await.is_ok());
    }

    #[tokio::test]
    async fn test_fetch_bytes_rejects_private_ip() {
        let result = fetch_bytes("http://127.0.0.1:9999/test").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_fetch_bytes_rejects_non_http() {
        let result = fetch_bytes("ftp://example.com/file.gz").await;
        assert!(result.is_err());
    }
}
