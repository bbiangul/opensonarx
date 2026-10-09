use crate::behaviour::OpenSonarXBehaviour;
use crate::config::NetworkConfig;
use crate::fanout::NodeHandle;
use crate::protocol::{QUERY_PROTOCOL, REPLICATION_PROTOCOL};
use crate::routing::RoutingTable;
use libp2p::{
    Multiaddr, PeerId, SwarmBuilder, autonat, dcutr, gossipsub, identify, identity::Keypair, kad,
    mdns, noise, relay, request_response, swarm::behaviour::toggle::Toggle, tcp, upnp, yamux,
};
use oi_index::HybridIndex;
use oi_types::{EarningsTracker, TopicRouter};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::info;

/// An OpenSonarX P2P node.
pub struct Node {
    pub peer_id: PeerId,
    pub config: NetworkConfig,
    pub routing_table: Arc<RwLock<RoutingTable>>,
    keypair: Keypair,
    index: Option<Arc<HybridIndex>>,
    earnings: Arc<EarningsTracker>,
    shard_ids: Vec<String>,
    topic_router: Option<Arc<TopicRouter>>,
}

impl Node {
    pub fn new(config: NetworkConfig) -> Self {
        let keypair = Keypair::generate_ed25519();
        let peer_id = PeerId::from(keypair.public());

        // Load persisted routing table if configured
        let routing_table = if let Some(ref path) = config.routing_table_path {
            let path = std::path::Path::new(path);
            match RoutingTable::load_from_file(path, config.routing_table_max_age_secs) {
                Ok(rt) => {
                    let count = rt.peer_count();
                    if count > 0 {
                        info!("Loaded {} peers from persisted routing table", count);
                    }
                    rt
                }
                Err(e) => {
                    tracing::warn!("Failed to load routing table: {e}");
                    RoutingTable::new()
                }
            }
        } else {
            RoutingTable::new()
        };

        Self {
            peer_id,
            config,
            routing_table: Arc::new(RwLock::new(routing_table)),
            keypair,
            index: None,
            earnings: Arc::new(EarningsTracker::new()),
            shard_ids: Vec::new(),
            topic_router: None,
        }
    }

    /// Attach a search index and shard assignments to this node.
    pub fn with_index(mut self, index: Arc<HybridIndex>, shard_ids: Vec<String>) -> Self {
        self.index = Some(index);
        self.shard_ids = shard_ids;
        self
    }

    /// Attach a topic router for smart query fanout.
    pub fn with_topic_router(mut self, router: Arc<TopicRouter>) -> Self {
        self.topic_router = Some(router);
        self
    }

    /// Get a reference to the earnings tracker.
    pub fn earnings(&self) -> &Arc<EarningsTracker> {
        &self.earnings
    }

    /// Get a clone of the routing table Arc (for test verification).
    pub fn routing_table(&self) -> Arc<RwLock<RoutingTable>> {
        self.routing_table.clone()
    }

    /// Build the libp2p swarm (does not start it — caller must run the event loop)
    pub fn build_swarm(
        &self,
    ) -> Result<libp2p::Swarm<OpenSonarXBehaviour>, Box<dyn std::error::Error>> {
        let peer_id = PeerId::from(self.keypair.public());

        // Gossipsub config
        let gossipsub_config = gossipsub::ConfigBuilder::default()
            .heartbeat_interval(Duration::from_secs(self.config.heartbeat_interval_secs))
            .build()
            .map_err(|e| format!("gossipsub config error: {e}"))?;

        let gossipsub = gossipsub::Behaviour::new(
            gossipsub::MessageAuthenticity::Signed(self.keypair.clone()),
            gossipsub_config,
        )
        .map_err(|e| format!("gossipsub error: {e}"))?;

        // Kademlia
        let store = kad::store::MemoryStore::new(peer_id);
        let kademlia = kad::Behaviour::new(peer_id, store);

        // Identify
        let identify = identify::Behaviour::new(identify::Config::new(
            "/opensonarx/0.1.0".to_string(),
            self.keypair.public(),
        ));

        // Request-Response for queries
        let request_response = request_response::Behaviour::new(
            [(
                QUERY_PROTOCOL.clone(),
                request_response::ProtocolSupport::Full,
            )],
            request_response::Config::default()
                .with_request_timeout(Duration::from_secs(self.config.query_timeout_secs)),
        );

        // Request-Response for replication
        let replication = request_response::Behaviour::new(
            [(
                REPLICATION_PROTOCOL.clone(),
                request_response::ProtocolSupport::Full,
            )],
            request_response::Config::default().with_request_timeout(Duration::from_secs(30)),
        );

        // mDNS (optional, wrapped in Toggle)
        let mdns_behaviour = if self.config.enable_mdns {
            match mdns::tokio::Behaviour::new(mdns::Config::default(), peer_id) {
                Ok(b) => Toggle::from(Some(b)),
                Err(e) => {
                    tracing::warn!("mDNS init failed: {e}");
                    Toggle::from(None)
                }
            }
        } else {
            Toggle::from(None)
        };

        // NAT traversal behaviours (optional, wrapped in Toggle)
        let (autonat_behaviour, relay_client_behaviour, dcutr_behaviour, upnp_behaviour) =
            if self.config.enable_nat_traversal {
                let autonat = autonat::Behaviour::new(peer_id, autonat::Config::default());
                let dcutr = dcutr::Behaviour::new(peer_id);
                let upnp = Toggle::from(Some(upnp::tokio::Behaviour::default()));
                (
                    Toggle::from(Some(autonat)),
                    Toggle::from(None::<relay::client::Behaviour>), // relay client requires transport integration
                    Toggle::from(Some(dcutr)),
                    upnp,
                )
            } else {
                (
                    Toggle::from(None),
                    Toggle::from(None),
                    Toggle::from(None),
                    Toggle::from(None),
                )
            };

        let behaviour = OpenSonarXBehaviour {
            kademlia,
            gossipsub,
            identify,
            request_response,
            replication,
            mdns: mdns_behaviour,
            autonat: autonat_behaviour,
            relay_client: relay_client_behaviour,
            dcutr: dcutr_behaviour,
            upnp: upnp_behaviour,
        };

        let swarm = if self.config.enable_quic {
            SwarmBuilder::with_existing_identity(self.keypair.clone())
                .with_tokio()
                .with_tcp(
                    tcp::Config::default(),
                    noise::Config::new,
                    yamux::Config::default,
                )?
                .with_quic()
                .with_behaviour(|_| behaviour)?
                .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(60)))
                .build()
        } else {
            SwarmBuilder::with_existing_identity(self.keypair.clone())
                .with_tokio()
                .with_tcp(
                    tcp::Config::default(),
                    noise::Config::new,
                    yamux::Config::default,
                )?
                .with_behaviour(|_| behaviour)?
                .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(60)))
                .build()
        };

        info!("Node built with peer ID: {peer_id}");
        Ok(swarm)
    }

    pub fn listen_addr(&self) -> Multiaddr {
        format!("/ip4/0.0.0.0/tcp/{}", self.config.listen_port)
            .parse()
            .unwrap()
    }

    pub fn quic_listen_addr(&self) -> Multiaddr {
        format!("/ip4/0.0.0.0/udp/{}/quic-v1", self.config.listen_port)
            .parse()
            .unwrap()
    }

    /// Start the node: listen on the configured address, dial bootstrap peers,
    /// and spawn the P2P event loop as a background task.
    ///
    /// Returns a `NodeHandle` that can be used to submit queries to the running node.
    pub async fn start(self) -> Result<NodeHandle, Box<dyn std::error::Error>> {
        let mut swarm = self.build_swarm()?;
        let addr = self.listen_addr();
        swarm.listen_on(addr)?;

        // Also listen on QUIC if enabled
        if self.config.enable_quic {
            let quic_addr = self.quic_listen_addr();
            if let Err(e) = swarm.listen_on(quic_addr.clone()) {
                tracing::warn!("QUIC listen failed on {quic_addr}: {e}");
            }
        }

        // Dial bootstrap peers
        for peer_addr in &self.config.bootstrap_peers {
            if let Ok(addr) = peer_addr.parse::<Multiaddr>()
                && let Err(e) = swarm.dial(addr.clone())
            {
                tracing::warn!("Failed to dial bootstrap peer {addr}: {e}");
            }
        }

        // DNS bootstrap resolution with retry + exponential backoff
        if let Some(ref host) = self.config.dns_bootstrap {
            let mut resolved = false;
            let mut delay = Duration::from_secs(1);
            for attempt in 1..=3 {
                match tokio::net::lookup_host(format!("{host}:4001")).await {
                    Ok(addrs) => {
                        for addr in addrs {
                            let ma: Multiaddr =
                                format!("/ip4/{}/tcp/4001", addr.ip()).parse().unwrap();
                            if let Err(e) = swarm.dial(ma.clone()) {
                                tracing::warn!("DNS bootstrap dial failed {ma}: {e}");
                            }
                        }
                        resolved = true;
                        break;
                    }
                    Err(e) => {
                        tracing::warn!(
                            "DNS bootstrap resolution failed for {host} (attempt {attempt}/3): {e}"
                        );
                        if attempt < 3 {
                            tokio::time::sleep(delay).await;
                            delay *= 2;
                        }
                    }
                }
            }
            if !resolved {
                tracing::warn!("DNS bootstrap exhausted retries for {host}");
            }
        }

        info!("Node started with peer ID: {}", self.peer_id);

        let (query_tx, query_rx) = tokio::sync::mpsc::channel(64);
        let (gossip_tx, gossip_rx) = tokio::sync::mpsc::channel(64);
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        let query_timeout = Duration::from_secs(self.config.query_timeout_secs);
        let max_query_fanout = self.config.max_query_fanout;
        let target_coverage = self.config.target_coverage;
        let peer_id = self.peer_id;

        // Wrap shard_ids in Arc<RwLock> for dynamic updates via ShardAnnounce
        let shard_ids = Arc::new(tokio::sync::RwLock::new(self.shard_ids));

        // Spawn event loop as background task
        let topic_router = self.topic_router;
        let min_replicas = self.config.min_replicas_per_shard;
        let repl_check_secs = self.config.replication_check_interval_secs;
        let enable_tit_for_tat = self.config.enable_tit_for_tat;
        let min_contribution_ratio = self.config.min_contribution_ratio;
        let routing_table_path = self.config.routing_table_path.clone();
        tokio::spawn(async move {
            crate::event_loop::run_event_loop(
                swarm,
                self.routing_table,
                self.peer_id.to_string(),
                self.index,
                self.earnings,
                shard_ids,
                query_rx,
                gossip_rx,
                max_query_fanout,
                query_timeout,
                target_coverage,
                topic_router,
                min_replicas,
                repl_check_secs,
                enable_tit_for_tat,
                min_contribution_ratio,
                routing_table_path,
                shutdown_rx,
            )
            .await;
        });

        Ok(NodeHandle::new(peer_id, query_tx, gossip_tx, shutdown_tx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_creation() {
        let node = Node::new(NetworkConfig::default());
        assert!(!node.peer_id.to_string().is_empty());
    }

    #[test]
    fn test_listen_addr() {
        let node = Node::new(NetworkConfig {
            listen_port: 5000,
            ..Default::default()
        });
        let addr = node.listen_addr();
        assert!(addr.to_string().contains("5000"));
    }

    #[tokio::test]
    async fn test_build_swarm() {
        let node = Node::new(NetworkConfig::default());
        let swarm = node.build_swarm();
        assert!(swarm.is_ok());
    }

    #[test]
    fn test_node_with_index() {
        let index = Arc::new(HybridIndex::new(4));
        let node = Node::new(NetworkConfig::default())
            .with_index(index, vec!["shard-0".into(), "shard-1".into()]);
        assert!(node.index.is_some());
        assert_eq!(node.shard_ids.len(), 2);
        assert_eq!(node.earnings().total_earned(), 0);
    }

    #[tokio::test]
    async fn test_build_swarm_with_mdns() {
        let node = Node::new(NetworkConfig {
            enable_mdns: true,
            ..Default::default()
        });
        let swarm = node.build_swarm();
        assert!(swarm.is_ok());
    }

    #[tokio::test]
    async fn test_build_swarm_without_mdns() {
        let node = Node::new(NetworkConfig {
            enable_mdns: false,
            ..Default::default()
        });
        let swarm = node.build_swarm();
        assert!(swarm.is_ok());
    }

    #[test]
    fn test_config_default_has_mdns() {
        let config = NetworkConfig::default();
        assert!(config.enable_mdns);
    }

    #[test]
    fn test_default_config_has_no_bootstrap_peers() {
        let config = NetworkConfig::default();
        assert!(
            config.bootstrap_peers.is_empty(),
            "default config should start with no bootstrap peers (local-only mode)"
        );
    }

    #[test]
    fn test_default_config_has_no_dns_bootstrap() {
        let config = NetworkConfig::default();
        assert!(
            config.dns_bootstrap.is_none(),
            "default config should have no DNS bootstrap"
        );
    }

    #[test]
    fn test_explicit_bootstrap_overrides_defaults() {
        let config = NetworkConfig {
            bootstrap_peers: vec!["/ip4/1.2.3.4/tcp/4001".to_string()],
            dns_bootstrap: None,
            ..Default::default()
        };
        assert_eq!(config.bootstrap_peers.len(), 1);
        assert!(config.bootstrap_peers[0].contains("1.2.3.4"));
        assert!(config.dns_bootstrap.is_none());
    }
}
