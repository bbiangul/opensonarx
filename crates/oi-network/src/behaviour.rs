use crate::protocol::{QueryCodec, ReplicationCodec};
use libp2p::{
    autonat, dcutr, gossipsub, identify, kad, mdns, relay, request_response,
    swarm::NetworkBehaviour, upnp,
};

/// Combined network behaviour for OpenSonarX nodes.
#[derive(NetworkBehaviour)]
pub struct OpenSonarXBehaviour {
    pub kademlia: kad::Behaviour<kad::store::MemoryStore>,
    pub gossipsub: gossipsub::Behaviour,
    pub identify: identify::Behaviour,
    pub request_response: request_response::Behaviour<QueryCodec>,
    pub replication: request_response::Behaviour<ReplicationCodec>,
    pub mdns: libp2p::swarm::behaviour::toggle::Toggle<mdns::tokio::Behaviour>,
    pub autonat: libp2p::swarm::behaviour::toggle::Toggle<autonat::Behaviour>,
    pub relay_client: libp2p::swarm::behaviour::toggle::Toggle<relay::client::Behaviour>,
    pub dcutr: libp2p::swarm::behaviour::toggle::Toggle<dcutr::Behaviour>,
    pub upnp: libp2p::swarm::behaviour::toggle::Toggle<upnp::tokio::Behaviour>,
}
