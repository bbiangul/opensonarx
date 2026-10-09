use ed25519_dalek::{Signer, SigningKey};
use oi_types::traits::StateChannel;
use oi_types::{Result, SignedTicket};

/// Build the canonical ticket payload used for signing and verification.
///
/// Format: `{payer_pubkey}|{payee_pubkey}|{amount_lamports}|{nonce}|{expiry_slot}`
pub fn canonical_ticket_payload(ticket: &SignedTicket) -> String {
    format!(
        "{}|{}|{}|{}|{}",
        ticket.payer_pubkey,
        ticket.payee_pubkey,
        ticket.amount_lamports,
        ticket.nonce,
        ticket.expiry_slot
    )
}

/// In-memory state channel for development/testing.
///
/// Signs tickets with a real Ed25519 key so P2P signature verification works.
pub struct InMemoryStateChannel {
    signing_key: SigningKey,
    payer_pubkey: String,
    nonce: std::sync::atomic::AtomicU64,
}

impl InMemoryStateChannel {
    pub fn new() -> Self {
        let signing_key = SigningKey::generate(&mut rand::thread_rng());
        let payer_pubkey = signing_key
            .verifying_key()
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        Self {
            signing_key,
            payer_pubkey,
            nonce: std::sync::atomic::AtomicU64::new(0),
        }
    }

    pub fn with_signing_key(signing_key: SigningKey) -> Self {
        let payer_pubkey = signing_key
            .verifying_key()
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        Self {
            signing_key,
            payer_pubkey,
            nonce: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Load a signing key from `~/.opensonarx/signing_key`, or generate and persist a new one.
    ///
    /// This ensures the gateway's identity survives restarts — tickets signed
    /// before a restart remain verifiable.
    pub fn load_or_generate() -> Self {
        if let Some(home) = dirs::home_dir() {
            let dir = home.join(".opensonarx");
            let path = dir.join("signing_key");

            // Try to load existing key
            if let Ok(bytes) = std::fs::read(&path)
                && bytes.len() == 32
            {
                let seed: [u8; 32] = bytes.try_into().unwrap();
                let signing_key = SigningKey::from_bytes(&seed);
                tracing::info!("Loaded signing key from {}", path.display());
                return Self::with_signing_key(signing_key);
            }

            // Generate new key and persist
            let signing_key = SigningKey::generate(&mut rand::thread_rng());
            let _ = std::fs::create_dir_all(&dir);
            if let Err(e) = std::fs::write(&path, signing_key.to_bytes()) {
                tracing::warn!("Failed to persist signing key to {}: {e}", path.display());
            } else {
                // Best-effort: restrict permissions on Unix
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
                }
                tracing::info!("Generated and saved signing key to {}", path.display());
            }
            Self::with_signing_key(signing_key)
        } else {
            tracing::warn!("Cannot determine home directory — using ephemeral signing key");
            Self::new()
        }
    }

    /// Get the payer's hex-encoded public key.
    pub fn payer_pubkey(&self) -> &str {
        &self.payer_pubkey
    }
}

impl Default for InMemoryStateChannel {
    fn default() -> Self {
        Self::new()
    }
}

impl StateChannel for InMemoryStateChannel {
    fn create_ticket(&self, amount_lamports: u64, payee_pubkey: &str) -> Result<SignedTicket> {
        let nonce = self.nonce.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        let mut ticket = SignedTicket {
            payer_pubkey: self.payer_pubkey.clone(),
            payee_pubkey: payee_pubkey.to_string(),
            amount_lamports,
            nonce,
            signature: Vec::new(),
            expiry_slot: u64::MAX,
        };

        // Sign the canonical payload with Ed25519
        let payload = canonical_ticket_payload(&ticket);
        ticket.signature = self
            .signing_key
            .sign(payload.as_bytes())
            .to_bytes()
            .to_vec();

        Ok(ticket)
    }

    fn settle_batch(&self, tickets: &[SignedTicket]) -> Result<String> {
        tracing::warn!(
            "On-chain settlement not implemented — {} tickets logged only",
            tickets.len()
        );
        Ok(format!("mock_tx_{}_tickets", tickets.len()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::Verifier;

    #[test]
    fn test_create_ticket() {
        let channel = InMemoryStateChannel::new();
        let ticket = channel.create_ticket(100, "payee_wallet_abc").unwrap();
        assert_eq!(ticket.amount_lamports, 100);
        assert_eq!(ticket.payee_pubkey, "payee_wallet_abc");
        assert_eq!(ticket.nonce, 0);
        assert_eq!(ticket.signature.len(), 64); // Real Ed25519 signature
    }

    #[test]
    fn test_ticket_nonce_increments() {
        let channel = InMemoryStateChannel::new();
        let t1 = channel.create_ticket(100, "node1").unwrap();
        let t2 = channel.create_ticket(200, "node2").unwrap();
        assert_eq!(t1.nonce, 0);
        assert_eq!(t2.nonce, 1);
    }

    #[test]
    fn test_ticket_signature_verifiable() {
        let channel = InMemoryStateChannel::new();
        let ticket = channel.create_ticket(500, "payee123").unwrap();

        // Decode payer pubkey from hex
        let pubkey_bytes: Vec<u8> = (0..ticket.payer_pubkey.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&ticket.payer_pubkey[i..i + 2], 16).unwrap())
            .collect();
        let vk =
            ed25519_dalek::VerifyingKey::from_bytes(&pubkey_bytes.try_into().unwrap()).unwrap();

        // Reconstruct canonical payload and verify
        let payload = canonical_ticket_payload(&ticket);
        let sig = ed25519_dalek::Signature::from_slice(&ticket.signature).unwrap();
        assert!(vk.verify(payload.as_bytes(), &sig).is_ok());
    }

    #[test]
    fn test_ticket_signature_rejects_tampered() {
        let channel = InMemoryStateChannel::new();
        let mut ticket = channel.create_ticket(500, "payee123").unwrap();

        // Decode payer pubkey
        let pubkey_bytes: Vec<u8> = (0..ticket.payer_pubkey.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&ticket.payer_pubkey[i..i + 2], 16).unwrap())
            .collect();
        let vk =
            ed25519_dalek::VerifyingKey::from_bytes(&pubkey_bytes.try_into().unwrap()).unwrap();

        // Tamper with the amount
        ticket.amount_lamports = 999999;
        let payload = canonical_ticket_payload(&ticket);
        let sig = ed25519_dalek::Signature::from_slice(&ticket.signature).unwrap();
        assert!(vk.verify(payload.as_bytes(), &sig).is_err());
    }

    #[test]
    fn test_settle_batch() {
        let channel = InMemoryStateChannel::new();
        let tickets = vec![
            channel.create_ticket(100, "node1").unwrap(),
            channel.create_ticket(200, "node2").unwrap(),
        ];
        let tx = channel.settle_batch(&tickets).unwrap();
        assert!(tx.contains("2_tickets"));
    }

    #[test]
    fn test_payer_pubkey_is_64_hex_chars() {
        let channel = InMemoryStateChannel::new();
        assert_eq!(channel.payer_pubkey().len(), 64);
        assert!(
            channel
                .payer_pubkey()
                .chars()
                .all(|c| c.is_ascii_hexdigit())
        );
    }

    #[test]
    fn test_canonical_ticket_payload() {
        let ticket = SignedTicket {
            payer_pubkey: "aabb".to_string(),
            payee_pubkey: "ccdd".to_string(),
            amount_lamports: 100,
            nonce: 5,
            signature: vec![],
            expiry_slot: 999,
        };
        assert_eq!(canonical_ticket_payload(&ticket), "aabb|ccdd|100|5|999");
    }
}
