//! The node's view of operators' on-chain bonds: a mirror file written by
//! `chain_node_bond show --mirror`, read again whenever it changes. The node
//! does not query the chain itself; a bond or a slash reaches it when the
//! mirror is refreshed.

use oi_network::operator::OperatorBonds;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::SystemTime;

pub struct BondMirror {
    path: PathBuf,
    /// When the file was last read, and the wallets in good standing then.
    loaded: Mutex<(Option<SystemTime>, HashSet<[u8; 32]>)>,
}

impl BondMirror {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            loaded: Mutex::new((None, HashSet::new())),
        }
    }
}

/// The wallets whose bond, after what was slashed, is at least the minimum
/// stake (and more than nothing).
fn in_good_standing(mirror: &serde_json::Value) -> HashSet<[u8; 32]> {
    let minimum = mirror["min_stake"].as_u64().unwrap_or(0).max(1);
    mirror["operators"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(_, bond)| {
            let staked = bond["staked"].as_u64().unwrap_or(0);
            let slashed = bond["slashed"].as_u64().unwrap_or(0);
            staked.saturating_sub(slashed) >= minimum
        })
        .filter_map(|(address, _)| bs58::decode(address).into_vec().ok()?.try_into().ok())
        .collect()
}

impl OperatorBonds for BondMirror {
    fn in_good_standing(&self, wallet: &[u8; 32]) -> bool {
        let Ok(mut loaded) = self.loaded.lock() else {
            return false;
        };
        let modified = std::fs::metadata(&self.path)
            .and_then(|meta| meta.modified())
            .ok();
        if modified != loaded.0 {
            let mirror = std::fs::read_to_string(&self.path)
                .ok()
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
                .unwrap_or_default();
            *loaded = (modified, in_good_standing(&mirror));
        }
        loaded.1.contains(wallet)
    }
}

/// The 32-byte secret of a Solana keypair file: a JSON array of 64 numbers,
/// the secret followed by the public key.
pub fn read_wallet_secret(path: &str) -> Result<[u8; 32], String> {
    let raw = std::fs::read_to_string(path).map_err(|error| format!("{path}: {error}"))?;
    let bytes: Vec<u8> = serde_json::from_str(&raw)
        .map_err(|error| format!("{path}: not a keypair file: {error}"))?;
    bytes
        .get(..32)
        .filter(|_| bytes.len() == 64)
        .and_then(|secret| secret.try_into().ok())
        .ok_or_else(|| {
            format!(
                "{path}: a keypair file holds 64 bytes, this one {}",
                bytes.len()
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standing_is_the_bond_left_after_slashing_against_the_minimum() {
        let address = |byte: u8| bs58::encode([byte; 32]).into_string();
        let mirror = serde_json::json!({
            "min_stake": 1000,
            "operators": {
                address(1): { "staked": 1000, "slashed": 0 },
                address(2): { "staked": 1000, "slashed": 1 },
                address(3): { "staked": 5000, "slashed": 4000 },
                "not-an-address": { "staked": 9000, "slashed": 0 },
            }
        });
        let good = in_good_standing(&mirror);
        assert_eq!(good, HashSet::from([[1u8; 32], [3u8; 32]]));
        assert!(in_good_standing(&serde_json::json!({})).is_empty());
    }

    #[test]
    fn the_mirror_is_read_again_when_the_file_changes() {
        let dir = std::env::temp_dir().join(format!("oi-bonds-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("operator_bonds.json");
        let mirror = BondMirror::new(path.clone());
        let wallet = [9u8; 32];
        assert!(!mirror.in_good_standing(&wallet), "no file, no bond");

        let write = |slashed: u64| {
            let body = serde_json::json!({
                "min_stake": 10,
                "operators": { bs58::encode(wallet).into_string(): { "staked": 10, "slashed": slashed } }
            });
            // A new file each time, so the modification time moves even on
            // a coarse clock.
            std::thread::sleep(std::time::Duration::from_millis(1100));
            std::fs::write(&path, body.to_string()).unwrap();
        };
        write(0);
        assert!(mirror.in_good_standing(&wallet));
        write(10);
        assert!(!mirror.in_good_standing(&wallet), "slashed");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_keypair_file_gives_its_first_32_bytes() {
        let dir = std::env::temp_dir().join(format!("oi-wallet-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("id.json");
        let bytes: Vec<u8> = (0..64).collect();
        std::fs::write(&path, serde_json::to_string(&bytes).unwrap()).unwrap();
        let secret = read_wallet_secret(path.to_str().unwrap()).unwrap();
        assert_eq!(secret[0], 0);
        assert_eq!(secret[31], 31);
        std::fs::write(&path, "[1,2,3]").unwrap();
        assert!(read_wallet_secret(path.to_str().unwrap()).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
