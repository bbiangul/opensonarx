use oi_types::error::ErrorCode;
use oi_types::{AuthResponse, Error, MagicLinkRequest, Result, VerifyTokenRequest};
use std::time::Duration;

/// Request a magic link login email.
///
/// Sends a POST to `/v1/auth/magic-link` with the given email.
/// The gateway will send (or log in dev mode) a one-time token.
pub fn request_magic_link(gateway_url: &str, email: &str) -> Result<()> {
    let url = format!("{gateway_url}/v1/auth/magic-link");
    let body = MagicLinkRequest {
        email: email.to_string(),
    };

    let agent = ureq::Agent::new_with_config(
        ureq::config::Config::builder()
            .timeout_global(Some(Duration::from_secs(15)))
            .build(),
    );

    let json = serde_json::to_string(&body)
        .map_err(|e| Error::Internal(format!("JSON serialize: {e}")))?;

    agent
        .post(&url)
        .content_type("application/json")
        .send(json.as_bytes())
        .map_err(|e| match e {
            ureq::Error::StatusCode(status) => Error::protocol(
                ErrorCode::NodeUnreachable,
                format!("Gateway returned HTTP {status}"),
            ),
            _ => Error::Internal(format!("Magic link request failed: {e}")),
        })?;

    Ok(())
}

/// Verify a magic link token and obtain an API key.
///
/// Sends a POST to `/v1/auth/verify` with the token received via email.
/// On success, returns the API key, session token, user account, and quota.
pub fn verify_magic_link(gateway_url: &str, token: &str) -> Result<AuthResponse> {
    let url = format!("{gateway_url}/v1/auth/verify");
    let body = VerifyTokenRequest {
        token: token.to_string(),
    };

    let agent = ureq::Agent::new_with_config(
        ureq::config::Config::builder()
            .timeout_global(Some(Duration::from_secs(15)))
            .build(),
    );

    let json = serde_json::to_string(&body)
        .map_err(|e| Error::Internal(format!("JSON serialize: {e}")))?;

    let resp = agent
        .post(&url)
        .content_type("application/json")
        .send(json.as_bytes())
        .map_err(|e| match e {
            ureq::Error::StatusCode(401) => Error::protocol(
                ErrorCode::MagicLinkExpired,
                "Magic link token expired or invalid",
            ),
            ureq::Error::StatusCode(status) => Error::protocol(
                ErrorCode::NodeUnreachable,
                format!("Gateway returned HTTP {status}"),
            ),
            _ => Error::Internal(format!("Magic link verify failed: {e}")),
        })?;

    let resp_bytes = resp
        .into_body()
        .read_to_vec()
        .map_err(|e| Error::Internal(format!("Failed to read response: {e}")))?;

    serde_json::from_slice(&resp_bytes)
        .map_err(|e| Error::Internal(format!("Failed to parse auth response: {e}")))
}
