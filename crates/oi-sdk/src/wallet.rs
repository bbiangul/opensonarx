use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use sha2::{Digest, Sha256};
use std::fs;

/// Wallet for Ed25519 keypair management.
///
/// Signs messages with Ed25519 and derives pubkeys as hex-encoded
/// verifying key bytes (64 hex chars = 32 bytes).
pub struct Wallet {
    /// Hex-encoded Ed25519 public key (64 hex chars)
    pub pubkey: String,
    #[allow(dead_code)]
    keypair_path: Option<String>,
    /// Ed25519 signing key
    signing_key: SigningKey,
}

impl Wallet {
    /// Create a new random wallet using cryptographically secure entropy.
    pub fn new() -> Self {
        let signing_key = SigningKey::generate(&mut rand::thread_rng());
        let pubkey = Self::derive_pubkey(&signing_key);

        Self {
            pubkey,
            keypair_path: None,
            signing_key,
        }
    }

    /// Load wallet from a keypair file path.
    ///
    /// The file contents are SHA-256 hashed to derive a deterministic keypair.
    pub fn from_path(path: &str) -> std::io::Result<Self> {
        let data = fs::read(path)?;
        let hash = Sha256::digest(&data);
        let seed: [u8; 32] = hash.into();
        let signing_key = SigningKey::from_bytes(&seed);
        let pubkey = Self::derive_pubkey(&signing_key);

        Ok(Self {
            pubkey,
            keypair_path: Some(path.to_string()),
            signing_key,
        })
    }

    /// Get the public key string.
    pub fn address(&self) -> &str {
        &self.pubkey
    }

    /// Sign a message with Ed25519 (returns 64-byte signature).
    pub fn sign(&self, message: &[u8]) -> Vec<u8> {
        self.signing_key.sign(message).to_bytes().to_vec()
    }

    /// Get raw verifying key bytes (32 bytes) for callers that need them.
    pub fn verifying_key_bytes(&self) -> [u8; 32] {
        self.signing_key.verifying_key().to_bytes()
    }

    /// Derive hex-encoded pubkey from a signing key (64 hex chars).
    fn derive_pubkey(signing_key: &SigningKey) -> String {
        let vk: VerifyingKey = signing_key.verifying_key();
        vk.to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    }
}

impl Default for Wallet {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::Verifier;

    #[test]
    fn test_wallet_creation() {
        let wallet = Wallet::new();
        assert!(!wallet.address().is_empty());
        assert_eq!(wallet.address().len(), 64); // 32 bytes hex-encoded
    }

    #[test]
    fn test_wallet_unique() {
        let w1 = Wallet::new();
        let w2 = Wallet::new();
        assert_ne!(w1.address(), w2.address());
    }

    #[test]
    fn test_wallet_from_path() {
        let dir = std::env::temp_dir().join("oi_wallet_test");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("test_keypair.json");
        fs::write(&path, b"test-keypair-data-12345").unwrap();

        let wallet = Wallet::from_path(path.to_str().unwrap()).unwrap();
        assert!(!wallet.address().is_empty());
        assert_eq!(wallet.address().len(), 64);

        // Loading the same file should produce the same pubkey
        let wallet2 = Wallet::from_path(path.to_str().unwrap()).unwrap();
        assert_eq!(wallet.address(), wallet2.address());

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn test_wallet_sign() {
        let wallet = Wallet::new();
        let sig = wallet.sign(b"hello world");
        assert_eq!(sig.len(), 64); // Ed25519 signature

        // Same message produces same signature (Ed25519 is deterministic)
        let sig2 = wallet.sign(b"hello world");
        assert_eq!(sig, sig2);

        // Different message produces different signature
        let sig3 = wallet.sign(b"different message");
        assert_ne!(sig, sig3);
    }

    #[test]
    fn test_wallet_sign_verifiable() {
        let wallet = Wallet::new();
        let message = b"test message for verification";
        let sig_bytes = wallet.sign(message);

        // Verify the signature using the verifying key
        let vk = VerifyingKey::from_bytes(&wallet.verifying_key_bytes()).unwrap();
        let sig = ed25519_dalek::Signature::from_slice(&sig_bytes).unwrap();
        assert!(vk.verify(message, &sig).is_ok());

        // Verify fails with wrong message
        assert!(vk.verify(b"wrong message", &sig).is_err());
    }

    #[test]
    fn test_wallet_from_path_not_found() {
        let result = Wallet::from_path("/nonexistent/path/keypair.json");
        assert!(result.is_err());
    }

    #[test]
    fn test_verifying_key_bytes() {
        let wallet = Wallet::new();
        let vk_bytes = wallet.verifying_key_bytes();
        assert_eq!(vk_bytes.len(), 32);

        // Hex-encoding the verifying key bytes should match the pubkey
        let hex: String = vk_bytes.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, wallet.address());
    }
}
