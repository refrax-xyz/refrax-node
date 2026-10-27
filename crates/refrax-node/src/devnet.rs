//! A local cluster: the registry plus a full committee of nodes, in one process.
//! Node keys persist in `<dir>/node-NN/keypair.json`; facets and registry state start fresh.

use crate::node::{self, NodeConfig, NodeHandle};
use crate::registry::{self, Registry};
use refrax_core::Keypair;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

/// Devnet settings.
#[derive(Clone, Debug)]
pub struct DevnetConfig {
    /// Number of nodes (16 fills one committee).
    pub nodes: usize,
    /// Registry port; node `i` listens on `port + i`. 0 picks free ports.
    pub port: u16,
    /// Address to bind.
    pub host: IpAddr,
    /// Working directory for keys and facets.
    pub dir: PathBuf,
    /// Stake each node bonds.
    pub stake: u64,
    /// Pause between audit rounds.
    pub audit_every: Duration,
}

impl Default for DevnetConfig {
    fn default() -> Self {
        DevnetConfig {
            nodes: refrax_core::FACETS,
            port: 7700,
            host: IpAddr::from([127, 0, 0, 1]),
            dir: PathBuf::from(".refrax/devnet"),
            stake: 2_500,
            audit_every: Duration::from_secs(2),
        }
    }
}

/// A running devnet.
pub struct Devnet {
    /// The registry, for direct inspection and event subscriptions.
    pub registry: Arc<Registry>,
    /// Registry URL.
    pub registry_url: String,
    /// Nodes in join order.
    pub nodes: Vec<NodeHandle>,
    tasks: Vec<JoinHandle<()>>,
}

impl Devnet {
    /// Stop everything.
    pub fn shutdown(self) {
        for n in &self.nodes {
            n.abort();
        }
        for t in &self.tasks {
            t.abort();
        }
    }
}

/// Start the registry and the nodes.
pub async fn start(cfg: DevnetConfig) -> anyhow::Result<Devnet> {
    let registry = Registry::new("devnet-local");
    let clock = registry.spawn_clock();
    let listener = TcpListener::bind(SocketAddr::new(cfg.host, cfg.port))
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "cannot listen on {}:{} ({e}). Is another devnet running?",
                cfg.host,
                cfg.port
            )
        })?;
    let addr = listener.local_addr()?;
    let host = if addr.ip().is_unspecified() {
        "127.0.0.1".to_string()
    } else {
        addr.ip().to_string()
    };
    let registry_url = format!("http://{host}:{}", addr.port());
    let app = registry::router(Arc::clone(&registry));
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let mut nodes = Vec::with_capacity(cfg.nodes);
    for i in 0..cfg.nodes {
        let dir = cfg.dir.join(format!("node-{:02}", i + 1));
        let key_path = dir.join("keypair.json");
        let keypair = match Keypair::read(&key_path) {
            Ok(k) => k,
            Err(_) => {
                let k = Keypair::generate();
                k.write(&key_path)?;
                k
            }
        };
        let facets = dir.join("facets");
        if facets.exists() {
            tokio::fs::remove_dir_all(&facets).await?;
        }
        let port = if cfg.port == 0 {
            0
        } else {
            cfg.port + 1 + i as u16
        };
        let handle = node::spawn(NodeConfig {
            listen: SocketAddr::new(cfg.host, port),
            public_url: None,
            registry: registry_url.clone(),
            data_dir: dir,
            keypair: Arc::new(keypair),
            stake: cfg.stake,
            allow_wipe: true,
            audit_every: cfg.audit_every,
        })
        .await?;
        nodes.push(handle);
    }
    Ok(Devnet {
        registry,
        registry_url,
        nodes,
        tasks: vec![clock, server],
    })
}
