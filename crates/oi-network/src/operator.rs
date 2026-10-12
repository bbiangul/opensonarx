//! Who operates a node, and whether that operator has a bond.
//!
//! A node's network identity is an ed25519 key, and so is a Solana wallet.
//! When an operator starts its node with the wallet's key, the node's peer
//! id carries the wallet's public key, so any peer can read the wallet from
//! the peer id it is already talking to. No extra message or signature binds
//! the two: the transport has authenticated the peer id.

use libp2p::PeerId;

/// Whether an operator may be copied from. Implemented outside this crate
/// (the node reads a mirror of the on-chain bond accounts).
pub trait OperatorBonds: Send + Sync {
    /// Whether the operator with this wallet has a bond in good standing.
    fn in_good_standing(&self, wallet: &[u8; 32]) -> bool;
}

/// The ed25519 public key inside a peer id: the operator's wallet, when the
/// node was started with it. `None` for a peer id of another key type or one
/// that carries only a hash of its key.
pub fn operator_wallet(peer: &PeerId) -> Option<[u8; 32]> {
    let multihash = peer.as_ref();
    // Code 0 is the identity "hash": the digest is the encoded public key.
    if multihash.code() != 0 {
        return None;
    }
    libp2p::identity::PublicKey::try_decode_protobuf(multihash.digest())
        .ok()?
        .try_into_ed25519()
        .ok()
        .map(|key| key.to_bytes())
}

/// The operator's wallet behind a node id, which is a peer id in text.
pub fn wallet_of_node(node_id: &str) -> Option<[u8; 32]> {
    operator_wallet(&node_id.parse::<PeerId>().ok()?)
}

/// A wallet's address as Solana writes it.
pub fn wallet_address(wallet: &[u8; 32]) -> String {
    bs58::encode(wallet).into_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use libp2p::identity::Keypair;

    #[test]
    fn the_wallet_is_read_back_from_the_peer_id() {
        let secret = [7u8; 32];
        let keypair = Keypair::ed25519_from_bytes(secret).unwrap();
        let public = keypair.public().try_into_ed25519().unwrap().to_bytes();
        let peer = PeerId::from(keypair.public());
        assert_eq!(operator_wallet(&peer), Some(public));
        // 32 bytes in base58 are 43 or 44 characters.
        assert!((43..=44).contains(&wallet_address(&public).len()));
        assert_ne!(operator_wallet(&PeerId::random()), Some(public));
    }
}
