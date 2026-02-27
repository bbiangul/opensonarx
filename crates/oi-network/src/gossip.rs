use prost::Message;
use oi_types::proto::opensonarx::{ContentAnnouncement, GossipMessage, Heartbeat, ShardAnnounce};

/// Encode a heartbeat message for gossip
pub fn encode_heartbeat(node_id: &str, shard_count: u32, vectors_stored: u64, queries_served: u32) -> Vec<u8> {
    let msg = GossipMessage {
        payload: Some(
            oi_types::proto::opensonarx::gossip_message::Payload::Heartbeat(Heartbeat {
                node_id: node_id.to_string(),
                timestamp_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64,
                shard_count,
                vectors_stored,
                queries_served,
                avg_latency_ms: 0.0,
                uptime_pct: 100.0,
                min_replicas: 0,
            }),
        ),
    };
    msg.encode_to_vec()
}

/// Encode a content announcement for gossip.
/// Sentinels broadcast this when they discover or update content.
#[allow(clippy::too_many_arguments)]
pub fn encode_content_announcement(
    sentinel_id: &str,
    url: &str,
    domain: &str,
    content_hash: &str,
    chunk_count: u32,
    shard_id: &str,
    is_update: bool,
    entity: &str,
) -> Vec<u8> {
    let msg = GossipMessage {
        payload: Some(
            oi_types::proto::opensonarx::gossip_message::Payload::ContentAnnouncement(
                ContentAnnouncement {
                    sentinel_id: sentinel_id.to_string(),
                    url: url.to_string(),
                    domain: domain.to_string(),
                    content_hash: content_hash.to_string(),
                    crawled_at: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs(),
                    chunk_count,
                    shard_id: shard_id.to_string(),
                    is_update,
                    entity: entity.to_string(),
                },
            ),
        ),
    };
    msg.encode_to_vec()
}

/// Encode a shard announcement for gossip.
/// Nodes broadcast this on startup and when their shard assignments change.
pub fn encode_shard_announce(node_id: &str, shard_ids: &[String], vector_count: u64, replica_count: u32) -> Vec<u8> {
    let msg = GossipMessage {
        payload: Some(
            oi_types::proto::opensonarx::gossip_message::Payload::ShardAnnounce(ShardAnnounce {
                node_id: node_id.to_string(),
                shard_ids: shard_ids.to_vec(),
                vector_count,
                last_sync_block: 0,
                replica_count,
            }),
        ),
    };
    msg.encode_to_vec()
}

/// Decode a gossip message from bytes
pub fn decode_gossip(data: &[u8]) -> Option<GossipMessage> {
    GossipMessage::decode(data).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_heartbeat_roundtrip() {
        let encoded = encode_heartbeat("peer123", 4, 10000, 42);
        let decoded = decode_gossip(&encoded).unwrap();
        match decoded.payload.unwrap() {
            oi_types::proto::opensonarx::gossip_message::Payload::Heartbeat(hb) => {
                assert_eq!(hb.node_id, "peer123");
                assert_eq!(hb.shard_count, 4);
                assert_eq!(hb.vectors_stored, 10000);
                assert_eq!(hb.queries_served, 42);
            }
            _ => panic!("expected heartbeat"),
        }
    }

    #[test]
    fn test_shard_announce_roundtrip() {
        let encoded = encode_shard_announce(
            "peer456",
            &["shard-0".to_string(), "shard-3".to_string()],
            5000,
            0,
        );
        let decoded = decode_gossip(&encoded).unwrap();
        match decoded.payload.unwrap() {
            oi_types::proto::opensonarx::gossip_message::Payload::ShardAnnounce(sa) => {
                assert_eq!(sa.node_id, "peer456");
                assert_eq!(sa.shard_ids, vec!["shard-0", "shard-3"]);
                assert_eq!(sa.vector_count, 5000);
            }
            _ => panic!("expected shard announce"),
        }
    }

    #[test]
    fn test_content_announcement_roundtrip() {
        let encoded = encode_content_announcement(
            "sentinel-1",
            "https://example.com/article",
            "example.com",
            "abc123hash",
            5,
            "tech-v1",
            false,
            "",
        );
        let decoded = decode_gossip(&encoded).unwrap();
        match decoded.payload.unwrap() {
            oi_types::proto::opensonarx::gossip_message::Payload::ContentAnnouncement(ca) => {
                assert_eq!(ca.sentinel_id, "sentinel-1");
                assert_eq!(ca.url, "https://example.com/article");
                assert_eq!(ca.domain, "example.com");
                assert_eq!(ca.content_hash, "abc123hash");
                assert_eq!(ca.chunk_count, 5);
                assert_eq!(ca.shard_id, "tech-v1");
                assert!(!ca.is_update);
                assert!(ca.crawled_at > 0);
            }
            _ => panic!("expected content announcement"),
        }
    }

    #[test]
    fn test_content_update_announcement() {
        let encoded = encode_content_announcement(
            "sentinel-2",
            "https://news.com/breaking",
            "news.com",
            "newhash",
            3,
            "news-v1",
            true,
            "",
        );
        let decoded = decode_gossip(&encoded).unwrap();
        match decoded.payload.unwrap() {
            oi_types::proto::opensonarx::gossip_message::Payload::ContentAnnouncement(ca) => {
                assert!(ca.is_update);
                assert_eq!(ca.sentinel_id, "sentinel-2");
            }
            _ => panic!("expected content announcement"),
        }
    }
}
