//! Claims extracted when a page enters the index, and the corroboration
//! signal computed from them at query time without any model call.
//!
//! Ingestion: a model lists the statements a passage makes, each with a
//! quote. A claim is kept only if its quote is located verbatim in the
//! passage by the same deterministic check the answer path uses. The kept
//! claims are embedded and stored with their source family.
//!
//! Query: the request is embedded and the nearest stored claims are fetched.
//! The best match is the head; stored claims close enough to the head say
//! the same thing, and the number of distinct source families among them is
//! the corroboration count. The whole query step is one embedding and a
//! vector lookup.
//!
//! Similarity of two claims here is embedding similarity. It does not tell
//! agreement from contradiction on the same subject; stance still needs a
//! model and is not part of this signal.
//!
//! The data types live in `oi-types` and the index in `oi-index`, so a
//! network node can store and serve claims; this module adds the step that
//! needs a model and the quote check.

pub use oi_index::claims::ClaimIndex;
pub use oi_types::claims::{
    ClaimPolicy, ClaimRecord, Corroboration, MatchedClaim, claims_from_results, corroboration,
    supporting_families,
};
use oi_types::{EvidencePassage, PassageClaimExtractor, Result};

/// Extract and verify the claims of one passage. Returns the kept records
/// and how many proposed claims were dropped because their quote was not
/// found in the passage.
pub fn ingest_passage(
    extractor: &dyn PassageClaimExtractor,
    passage: &EvidencePassage,
    policy: &ClaimPolicy,
) -> Result<(Vec<ClaimRecord>, usize)> {
    let proposed = extractor.passage_claims(passage, policy.max_claims_per_passage)?;
    let extractor_version = extractor.model_version();
    let family = crate::source::source_family(&passage.url);
    let mut kept = Vec::new();
    let mut rejected = 0;
    for claim in proposed.into_iter().take(policy.max_claims_per_passage) {
        match crate::answer::locate_quote(&passage.text, &claim.quote, policy.min_quote_chars) {
            Some(span) => kept.push(ClaimRecord {
                claim: claim.text,
                quote: span,
                url: passage.url.clone(),
                source_family: family.clone(),
                extractor: extractor_version.clone(),
            }),
            None => rejected += 1,
        }
    }
    Ok((kept, rejected))
}

#[cfg(test)]
mod tests {
    use super::*;
    use oi_types::PassageClaim;

    struct Scripted(Vec<PassageClaim>);

    impl PassageClaimExtractor for Scripted {
        fn passage_claims(
            &self,
            _passage: &EvidencePassage,
            _max_claims: usize,
        ) -> Result<Vec<PassageClaim>> {
            Ok(self.0.clone())
        }

        fn model_version(&self) -> String {
            "scripted".to_string()
        }
    }

    fn passage(url: &str, text: &str) -> EvidencePassage {
        EvidencePassage {
            id: "p1".to_string(),
            url: url.to_string(),
            title: String::new(),
            text: text.to_string(),
            retained: true,
            published_at: None,
            observed_at: None,
        }
    }

    #[test]
    fn a_claim_is_kept_only_when_its_quote_is_in_the_passage() {
        let text = "Antarctica is the largest desert on Earth. It covers 14 million square km.";
        let extractor = Scripted(vec![
            PassageClaim {
                text: "Antarctica is the largest desert.".to_string(),
                quote: "Antarctica is the largest desert on Earth.".to_string(),
            },
            PassageClaim {
                text: "The Sahara is the largest desert.".to_string(),
                quote: "The Sahara is the largest desert on Earth.".to_string(),
            },
        ]);
        let (kept, rejected) = ingest_passage(
            &extractor,
            &passage("https://www.example.com/deserts", text),
            &ClaimPolicy::default(),
        )
        .unwrap();
        assert_eq!(rejected, 1);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].quote, "Antarctica is the largest desert on Earth.");
        assert_eq!(kept[0].source_family, "example.com");
        assert_eq!(kept[0].extractor, "scripted");
    }
}
