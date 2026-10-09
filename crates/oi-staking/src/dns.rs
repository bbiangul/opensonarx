use oi_types::Result;

/// Verify that a domain has a TXT record proving ownership.
/// Expected record: `opensonarx-verify=<wallet_address>`
pub fn verify_domain_ownership(domain: &str, expected_wallet: &str) -> Result<bool> {
    // Without the `dns` feature, this is a stub
    #[cfg(not(feature = "dns"))]
    {
        let _ = (domain, expected_wallet);
        Ok(false)
    }

    #[cfg(feature = "dns")]
    {
        verify_domain_ownership_real(domain, expected_wallet)
    }
}

#[cfg(feature = "dns")]
fn verify_domain_ownership_real(domain: &str, expected_wallet: &str) -> Result<bool> {
    use trust_dns_resolver::Resolver;
    use trust_dns_resolver::config::*;

    let resolver = Resolver::new(ResolverConfig::default(), ResolverOpts::default())
        .map_err(|e| oi_types::Error::Internal(format!("DNS resolver init: {e}")))?;

    let prefix = format!("opensonarx-verify={expected_wallet}");

    match resolver.txt_lookup(domain) {
        Ok(response) => {
            for record in response.iter() {
                let txt = record.to_string();
                if txt.contains(&prefix) {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        Err(_) => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verify_returns_false_for_unverified() {
        let result = verify_domain_ownership("example.com", "wallet123").unwrap();
        assert!(!result);
    }
}
