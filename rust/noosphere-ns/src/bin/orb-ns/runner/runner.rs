use crate::runner::config::RunnerNodeConfig;
use anyhow::Result;
use noosphere_ipfs::{IpfsStore, KuboClient};
use noosphere_ns::{Multiaddr, NameSystem, NameSystemClient, PeerId};
use noosphere_storage::{BlockStoreRetry, MemoryStore, UcanStore};
use serde::Serialize;
use std::{
    future::Future,
    net::{SocketAddr, TcpListener},
    pin::Pin,
    sync::Arc,
    task,
    time::Duration,
};
use tokio::sync::Mutex;
use url::Url;

#[cfg(feature = "api-server")]
use noosphere_ns::server::ApiServer;

#[cfg(not(feature = "api-server"))]
struct ApiServer;
#[cfg(not(feature = "api-server"))]
impl ApiServer {
    pub fn serve(_ns: Arc<Mutex<NameSystem>>, _listener: TcpListener) -> Self {
        ApiServer {}
    }
}

/// [NameSystemRunner] wraps and runs a [NameSystem], and optionally,
/// an APIServer, and useful static info to log to the user, configured
/// from a [CLICommand].

#[derive(Serialize)]
pub struct NameSystemRunner {
    #[serde(skip_serializing)]
    #[allow(dead_code)]
    name_system: Arc<Mutex<NameSystem>>,
    #[serde(skip_serializing)]
    #[allow(dead_code)]
    api_thread: Option<ApiServer>,
    peer_id: PeerId,
    listening_address: Option<Multiaddr>,
    api_address: Option<Url>,
}

impl NameSystemRunner {
    pub(crate) async fn try_from_config(mut config: RunnerNodeConfig) -> Result<Self> {
        let node = if let Some(ipfs_api_url) = config.ipfs_api_url {
            let store = MemoryStore::default();
            let store = IpfsStore::new(store, Some(KuboClient::new(&ipfs_api_url).unwrap()));
            let store = BlockStoreRetry::new(store, 5u32, Duration::new(1, 0));
            let store = UcanStore(store);
            NameSystem::new(&config.key_material, config.dht_config.to_owned(), store)?
        } else {
            let store = MemoryStore::default();
            let store = UcanStore(store);
            NameSystem::new(&config.key_material, config.dht_config.to_owned(), store)?
        };
        let peer_id = node.peer_id().to_owned();

        let listening_address = if let Some(requested_addr) = config.listening_address.take() {
            // Request address from DHT to resolve default port (0) to
            // selected port.
            let resolved_addr = node.listen(requested_addr.to_owned()).await?;
            Some(resolved_addr)
        } else {
            None
        };

        node.add_peers(config.peers.to_owned()).await?;
        node.bootstrap().await?;

        let wrapped_node = Arc::new(Mutex::new(node));

        let (api_address, api_thread) = if cfg!(feature = "api-server") {
            if let Some(requested_addr) = config.api_address.take() {
                let api_listener = TcpListener::bind(requested_addr)?;
                let api_address = socket_addr_to_url(api_listener.local_addr()?)?;
                (
                    Some(api_address),
                    Some(ApiServer::serve(wrapped_node.clone(), api_listener)),
                )
            } else {
                (None, None)
            }
        } else {
            (None, None)
        };

        Ok(NameSystemRunner {
            name_system: wrapped_node,
            peer_id,
            listening_address,
            api_address,
            api_thread,
        })
    }
}

/// Future implementation for [NameSystemRunner] so we can
/// keep alive on a pending future for the necessary resources.
impl Future for NameSystemRunner {
    type Output = Result<()>;

    fn poll(self: Pin<&mut Self>, _cx: &mut task::Context<'_>) -> task::Poll<Self::Output> {
        task::Poll::Pending
    }
}

fn socket_addr_to_url(socket_addr: SocketAddr) -> Result<Url> {
    Url::parse(&format!(
        "http://{}:{}",
        socket_addr.ip(),
        socket_addr.port()
    ))
    .map_err(|e| e.into())
}
