//! DNS-based domain ownership verification for staking.
//!
//! A publisher proves control of a domain by publishing a TXT record:
//!
//! ```text
//! oi-verify=<wallet address>
//! ```
//!
//! Stakes on unverified domains are still accepted (the bond is real and
//! visible), but their trust verdict is capped at "neutral" — anyone can
//! bond capital to any name; only the DNS proof ties the bond to the domain
//! owner. Lookup uses DNS-over-HTTPS (Cloudflare, then Google as fallback)
//! so no system resolver integration is needed.

/// Expected TXT record content for a wallet.
pub fn expected_txt(wallet_address: &str) -> String {
    format!("oi-verify={wallet_address}")
}

/// Check whether `domain` publishes a TXT record proving ownership by
/// `wallet_address`. Network errors return false (unverified, not fatal).
pub fn verify_domain_ownership(domain: &str, wallet_address: &str) -> bool {
    let expected = expected_txt(wallet_address);
    for resolver in [
        "https://cloudflare-dns.com/dns-query",
        "https://dns.google/resolve",
    ] {
        let url = format!("{resolver}?name={domain}&type=TXT");
        let response = ureq::get(&url)
            .header("accept", "application/dns-json")
            .call();
        let Ok(mut resp) = response else { continue };
        let Ok(body) = resp.body_mut().read_to_vec() else {
            continue;
        };
        let Ok(json) = serde_json::from_slice::<serde_json::Value>(&body) else {
            continue;
        };
        if txt_answers_contain(&json, &expected) {
            return true;
        }
    }
    false
}

/// Does a dns-json response contain the expected TXT value?
/// TXT data arrives quoted (`"\"oi-verify=abc\""`) and possibly chunked.
fn txt_answers_contain(json: &serde_json::Value, expected: &str) -> bool {
    json.get("Answer")
        .and_then(|a| a.as_array())
        .is_some_and(|answers| {
            answers.iter().any(|ans| {
                ans.get("data")
                    .and_then(|d| d.as_str())
                    .map(|s| s.replace('"', ""))
                    .is_some_and(|s| s.trim() == expected)
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_txt_answers_contain_match() {
        let json: serde_json::Value = serde_json::from_str(
            r#"{"Status":0,"Answer":[
                {"name":"example.com","type":16,"data":"\"v=spf1 -all\""},
                {"name":"example.com","type":16,"data":"\"oi-verify=wallet123\""}
            ]}"#,
        )
        .unwrap();
        assert!(txt_answers_contain(&json, "oi-verify=wallet123"));
        assert!(!txt_answers_contain(&json, "oi-verify=other"));
    }

    #[test]
    fn test_txt_answers_no_answer_section() {
        let json: serde_json::Value = serde_json::from_str(r#"{"Status":3}"#).unwrap();
        assert!(!txt_answers_contain(&json, "oi-verify=wallet123"));
    }

    #[test]
    fn test_expected_txt_format() {
        assert_eq!(expected_txt("abc"), "oi-verify=abc");
    }
}
