//! `refrax` — run a storage node, start a local devnet, put and get objects.

mod ui;

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use refrax_core::erasure::{decode_object, encode_object};
use refrax_core::{Keypair, Manifest, ObjectId, Pubkey, FACETS, QUORUM, REBUILD};
use refrax_node::client::Network;
use refrax_node::devnet::{self, DevnetConfig};
use refrax_node::node::{self, NodeConfig};
use refrax_node::registry::{SLOTS_PER_EPOCH, SLOT_MS};
use refrax_node::types::{Event, EventRecord, ObjectStatus};
use refrax_node::TOKEN;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Parser)]
#[command(
    name = "refrax",
    version,
    about = "Refrax storage node and client. One file in, sixteen facets out, any five bring it back.",
    long_about = None
)]
struct Cli {
    /// Registry to talk to.
    #[arg(
        long,
        global = true,
        env = "REFRAX_REGISTRY",
        default_value = "http://127.0.0.1:7700"
    )]
    registry: String,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Store a file: encode, register, scatter to the committee, certify.
    Put {
        /// File to store.
        file: PathBuf,
    },
    /// Fetch an object from any five facets and write it out.
    Get {
        /// Object id.
        object: ObjectId,
        /// Output path (default: ./<object-id>.bin).
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
    /// Challenge every facet holder of an object once.
    Audit {
        /// Object id.
        object: ObjectId,
    },
    /// List objects on the registry.
    Ls,
    /// Cluster, committee and stake.
    Status,
    /// Encode a file locally and show its facets, without any network.
    Inspect {
        /// File to inspect.
        file: PathBuf,
    },
    /// Node identity keys (solana-keygen JSON format).
    #[command(subcommand)]
    Keys(KeysCmd),
    /// Run a storage node.
    #[command(subcommand)]
    Node(NodeCmd),
    /// A local cluster: registry + 16 nodes.
    #[command(subcommand)]
    Devnet(DevnetCmd),
}

#[derive(Subcommand)]
enum KeysCmd {
    /// Create a new node keypair.
    New {
        /// Where to write it.
        #[arg(long, default_value_os_t = default_key())]
        out: PathBuf,
        /// Overwrite an existing file.
        #[arg(long)]
        force: bool,
    },
    /// Print the public key of a keypair.
    Show {
        /// Keypair file.
        #[arg(long, default_value_os_t = default_key())]
        key: PathBuf,
    },
}

#[derive(Subcommand)]
enum NodeCmd {
    /// Join the registry and serve facets until Ctrl+C.
    Start(NodeArgs),
}

#[derive(Args)]
struct NodeArgs {
    /// Keypair file.
    #[arg(long, env = "REFRAX_KEY", default_value_os_t = default_key())]
    key: PathBuf,
    /// Where facets live.
    #[arg(long, env = "REFRAX_DATA_DIR", default_value = "./refrax-data")]
    data_dir: PathBuf,
    /// Address to listen on.
    #[arg(long, env = "REFRAX_LISTEN", default_value = "0.0.0.0:7701")]
    listen: SocketAddr,
    /// URL others reach this node at (default: derived from --listen).
    #[arg(long, env = "REFRAX_PUBLIC_URL")]
    public_url: Option<String>,
    /// Stake to bond, in REFRAX.
    #[arg(long, env = "REFRAX_STAKE", default_value_t = 2_500)]
    stake: u64,
    /// Seconds between audit rounds.
    #[arg(long, default_value_t = 2)]
    audit_every: u64,
    /// Enable DELETE /v1/facets. Devnet demos only.
    #[arg(long)]
    allow_wipe: bool,
}

#[derive(Subcommand)]
enum DevnetCmd {
    /// Start a registry and a full committee on this machine.
    Up {
        /// Nodes to start.
        #[arg(long, default_value_t = FACETS)]
        nodes: usize,
        /// Registry port; nodes use the next ports.
        #[arg(long, default_value_t = 7700)]
        port: u16,
        /// Address to bind.
        #[arg(long, default_value = "127.0.0.1")]
        host: IpAddr,
        /// Working directory for node keys and facets.
        #[arg(long, default_value = ".refrax/devnet")]
        dir: PathBuf,
    },
    /// Delete every facet on one devnet node and watch the committee slash it.
    Wipe {
        /// Node number, as shown by `refrax status` (1–16).
        node: usize,
    },
}

fn default_key() -> PathBuf {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .unwrap_or_else(|| ".".into());
    PathBuf::from(home).join(".config/refrax/node.json")
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(cli).await {
        eprintln!("\n  {} {e:#}\n", ui::red("error"));
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> Result<()> {
    let net = Network::new(&cli.registry);
    match cli.cmd {
        Cmd::Put { file } => put(&net, file).await,
        Cmd::Get { object, out } => get(&net, object, out).await,
        Cmd::Audit { object } => audit(&net, object).await,
        Cmd::Ls => ls(&net).await,
        Cmd::Status => status(&net).await,
        Cmd::Inspect { file } => inspect(file),
        Cmd::Keys(KeysCmd::New { out, force }) => keys_new(out, force),
        Cmd::Keys(KeysCmd::Show { key }) => {
            println!(
                "{}",
                Keypair::read(&key)
                    .with_context(|| format!("reading {}", key.display()))?
                    .pubkey()
            );
            Ok(())
        }
        Cmd::Node(NodeCmd::Start(a)) => node_start(&cli.registry, a).await,
        Cmd::Devnet(DevnetCmd::Up {
            nodes,
            port,
            host,
            dir,
        }) => devnet_up(nodes, port, host, dir).await,
        Cmd::Devnet(DevnetCmd::Wipe { node }) => devnet_wipe(&net, node).await,
    }
}

// ───────────────────────────── object commands ─────────────────────────────

async fn put(net: &Network, file: PathBuf) -> Result<()> {
    let data = tokio::fs::read(&file)
        .await
        .with_context(|| format!("reading {}", file.display()))?;
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    ui::header("put", &format!("{name} · {}", ui::bytes(data.len() as u64)));

    let t = Instant::now();
    let enc = tokio::task::spawn_blocking(move || encode_object(&data)).await?;
    let manifest = Manifest::from_encoded(&enc);
    ui::step(
        "encode",
        &format!(
            "{FACETS} facets × {} · any {REBUILD} rebuild it",
            ui::bytes(enc.shard_len as u64)
        ),
        Some(t.elapsed()),
    );

    let t = Instant::now();
    let rec = net.registry.register_object(&manifest).await?;
    let info = net.registry.info().await?;
    ui::step(
        "register",
        &format!(
            "object {} · root {} · slot {}",
            ui::violet(&rec.id.short()),
            manifest.root.short(),
            ui::num(info.slot)
        ),
        Some(t.elapsed()),
    );

    let t = Instant::now();
    let sc = net.scatter(&rec, &enc).await?;
    let mark = if sc.failed.is_empty() {
        ui::mint("✓")
    } else {
        ui::amber("!")
    };
    ui::step(
        "scatter",
        &format!(
            "{}/{FACETS} nodes accepted their facet {mark}",
            sc.acks.len()
        ),
        Some(t.elapsed()),
    );
    for (f, why) in &sc.failed {
        println!(
            "  {}  {}",
            " ".repeat(9),
            ui::amber(&format!("facet {f:02}: {why}"))
        );
    }

    let t = Instant::now();
    let n = sc.acks.len();
    let cert = net
        .registry
        .certify(&rec.id, sc.acks)
        .await
        .context("certification failed")?;
    ui::step(
        "certify",
        &format!(
            "{n}/{FACETS} signatures · quorum {QUORUM} · {}",
            ui::mint("certified ✓")
        ),
        Some(t.elapsed()),
    );

    println!();
    ui::kv("object", &ui::violet(&cert.id.to_string()));
    ui::kv("root", &cert.manifest.root.to_hex());
    ui::kv("fetch", &ui::dim(&format!("refrax get {}", cert.id)));
    println!();
    Ok(())
}

async fn get(net: &Network, object: ObjectId, out: Option<PathBuf>) -> Result<()> {
    let rec = net.registry.object(&object).await?;
    ui::header(
        "get",
        &format!("{} · {}", object.short(), ui::bytes(rec.manifest.size)),
    );
    if rec.status != ObjectStatus::Certified {
        println!(
            "  {}",
            ui::amber("object is registered but not certified yet")
        );
    }
    let t = Instant::now();
    let got = net.fetch(&rec).await?;
    let used: Vec<String> = got.used.iter().map(|f| format!("{f:02}")).collect();
    ui::step(
        "fetch",
        &format!(
            "facets {} · {} skipped",
            ui::violet(&used.join(" ")),
            got.failed.len()
        ),
        Some(t.elapsed()),
    );
    for (f, why) in &got.failed {
        println!(
            "  {}  {}",
            " ".repeat(9),
            ui::dim(&format!("facet {f:02}: {why}"))
        );
    }
    let decoded = if got.used.iter().all(|f| (*f as usize) < REBUILD) {
        "data facets, no decoding"
    } else {
        "decoded"
    };
    ui::step(
        "rebuild",
        &format!(
            "{} · {decoded} · roots match {}",
            ui::bytes(got.data.len() as u64),
            ui::mint("✓")
        ),
        None,
    );
    let path = out.unwrap_or_else(|| PathBuf::from(format!("{object}.bin")));
    tokio::fs::write(&path, &got.data)
        .await
        .with_context(|| format!("writing {}", path.display()))?;
    ui::step("write", &path.display().to_string(), None);
    println!();
    Ok(())
}

async fn audit(net: &Network, object: ObjectId) -> Result<()> {
    let rec = net.registry.object(&object).await?;
    let info = net.registry.info().await?;
    ui::header(
        "audit",
        &format!(
            "{} · beacon {} · slot {}",
            object.short(),
            info.beacon.short(),
            ui::num(info.slot)
        ),
    );
    let labels = labels(net).await?;
    let rows = net.audit(&rec).await?;
    let ok = rows.iter().filter(|r| r.result.is_ok()).count();
    let table: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                format!("{:02}", r.facet),
                labels
                    .get(&r.holder)
                    .cloned()
                    .unwrap_or_else(|| r.holder.short()),
                ui::dim(&r.holder.short()),
                match &r.result {
                    Ok(()) => ui::mint("proof ok"),
                    Err(e) => ui::red(e),
                },
                ui::dim(&ui::ms(r.elapsed)),
            ]
        })
        .collect();
    ui::table(&["facet", "node", "pubkey", "result", "time"], &table);
    println!();
    let verdict = if ok >= REBUILD {
        ui::mint("recoverable")
    } else {
        ui::red("NOT recoverable")
    };
    println!("  {}/{FACETS} facets proved · {verdict}\n", ok);
    Ok(())
}

async fn ls(net: &Network) -> Result<()> {
    let objects = net.registry.objects().await?;
    ui::header("ls", &format!("{} objects", objects.len()));
    if objects.is_empty() {
        println!(
            "  {}\n",
            ui::dim("nothing stored yet · try `refrax put <file>`")
        );
        return Ok(());
    }
    let rows: Vec<Vec<String>> = objects
        .iter()
        .map(|o| {
            vec![
                ui::violet(&o.id.to_string()),
                ui::bytes(o.manifest.size),
                match o.status {
                    ObjectStatus::Certified => ui::mint("certified"),
                    ObjectStatus::Registered => ui::amber("registered"),
                },
                format!("{}/{FACETS}", o.acks.len()),
                ui::num(o.certified_slot.unwrap_or(o.registered_slot)),
            ]
        })
        .collect();
    ui::table(&["object", "size", "status", "sigs", "slot"], &rows);
    println!();
    Ok(())
}

async fn labels(net: &Network) -> Result<HashMap<Pubkey, String>> {
    Ok(net
        .registry
        .nodes()
        .await?
        .iter()
        .enumerate()
        .map(|(i, n)| (n.pubkey, ui::node_label(i + 1)))
        .collect())
}

async fn status(net: &Network) -> Result<()> {
    let info = net.registry.info().await?;
    let nodes = net.registry.nodes().await?;
    ui::header(
        "status",
        &format!(
            "{} · slot {} · epoch {} · v{}",
            info.cluster,
            ui::num(info.slot),
            info.epoch,
            info.version
        ),
    );
    ui::kv("registry", &ui::cyan(net.registry.url()));
    ui::kv(
        "objects",
        &format!("{} registered · {} certified", info.objects, info.certified),
    );
    ui::kv(
        "committee",
        &format!(
            "{} of {FACETS} seats · quorum {QUORUM} · any {REBUILD} facets rebuild",
            info.committee.len()
        ),
    );
    println!();
    let mut set = tokio::task::JoinSet::new();
    for (i, n) in nodes.iter().enumerate() {
        let (net, url) = (net.clone(), n.url.clone());
        set.spawn(async move { (i, net.health(&url).await.ok()) });
    }
    let mut health = HashMap::new();
    while let Some(Ok((i, h))) = set.join_next().await {
        health.insert(i, h);
    }
    let rows: Vec<Vec<String>> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let h = health.get(&i).cloned().flatten();
            let seat = info.committee.iter().position(|p| *p == n.pubkey);
            vec![
                ui::node_label(i + 1),
                ui::dim(&n.pubkey.short()),
                ui::cyan(&n.url),
                format!("{} {TOKEN}", ui::num(n.stake)),
                if n.slashed > 0 {
                    ui::red(&ui::num(n.slashed))
                } else {
                    ui::dim("0")
                },
                seat.map(|s| format!("{s:02}"))
                    .unwrap_or_else(|| ui::dim("–")),
                match h {
                    Some(h) => format!("{} · {}", h.facets, ui::bytes(h.bytes)),
                    None => ui::red("offline"),
                },
            ]
        })
        .collect();
    ui::table(
        &[
            "node", "pubkey", "url", "stake", "slashed", "seat", "stored",
        ],
        &rows,
    );
    println!();
    Ok(())
}

fn inspect(file: PathBuf) -> Result<()> {
    let data = std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    ui::header(
        "inspect",
        &format!("{name} · {} · no network", ui::bytes(data.len() as u64)),
    );
    let t = Instant::now();
    let enc = encode_object(&data);
    let m = Manifest::from_encoded(&enc);
    let took = t.elapsed();
    let rows: Vec<Vec<String>> = (0..FACETS)
        .map(|f| {
            vec![
                format!("{f:02}"),
                if f < REBUILD {
                    "data".into()
                } else {
                    ui::dim("parity")
                },
                ui::bytes(enc.shard_len as u64),
                ui::dim(&m.facet_roots[f].short()),
            ]
        })
        .collect();
    ui::table(&["facet", "kind", "size", "merkle root"], &rows);
    println!();
    ui::kv("root", &m.root.to_hex());
    ui::kv("object", &ui::violet(&m.id().to_string()));
    ui::kv(
        "encode",
        &format!(
            "{} · {:.1}× overhead",
            ui::ms(took),
            FACETS as f64 / REBUILD as f64
        ),
    );

    // Rebuild from 32 random five-facet subsets to show the code at work.
    let t = Instant::now();
    let mut ok = 0;
    for round in 0u64..32 {
        let mut idx: Vec<usize> = (0..FACETS).collect();
        let mut x = round.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        for i in (1..FACETS).rev() {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            idx.swap(i, (x % (i as u64 + 1)) as usize);
        }
        let pick: Vec<(usize, &[u8])> = idx[..REBUILD]
            .iter()
            .map(|&i| (i, enc.facets[i].as_slice()))
            .collect();
        if decode_object(&pick, enc.size, enc.shard_len)? == data {
            ok += 1;
        }
    }
    let mark = if ok == 32 {
        ui::mint("✓")
    } else {
        ui::red("✗")
    };
    ui::kv(
        "rebuild",
        &format!(
            "{ok}/32 random sets of {REBUILD} facets restore the file {mark} · {}",
            ui::ms(t.elapsed())
        ),
    );
    println!();
    Ok(())
}

// ───────────────────────────── keys and node ─────────────────────────────

fn keys_new(out: PathBuf, force: bool) -> Result<()> {
    if out.exists() && !force {
        bail!(
            "{} already exists (use --force to overwrite)",
            out.display()
        );
    }
    let kp = Keypair::generate();
    kp.write(&out)?;
    ui::header("keys new", "");
    ui::kv("pubkey", &ui::violet(&kp.pubkey().to_string()));
    ui::kv("saved", &out.display().to_string());
    println!(
        "\n  {}\n",
        ui::dim(
            "Same format as solana-keygen. Back it up: it is the node's identity and stake owner."
        )
    );
    Ok(())
}

async fn node_start(registry: &str, a: NodeArgs) -> Result<()> {
    init_logs("info");
    let keypair = Keypair::read(&a.key).with_context(|| {
        format!(
            "reading {} (create one with `refrax keys new`)",
            a.key.display()
        )
    })?;
    let pubkey = keypair.pubkey();
    let h = node::spawn(NodeConfig {
        listen: a.listen,
        public_url: a.public_url,
        registry: registry.to_string(),
        data_dir: a.data_dir.clone(),
        keypair: Arc::new(keypair),
        stake: a.stake,
        allow_wipe: a.allow_wipe,
        audit_every: Duration::from_secs(a.audit_every.max(1)),
    })
    .await?;
    ui::header("node", &format!("v{}", refrax_node::VERSION));
    ui::kv("pubkey", &ui::violet(&pubkey.to_string()));
    ui::kv("url", &ui::cyan(&h.url));
    ui::kv("registry", &ui::cyan(registry));
    ui::kv("data", &a.data_dir.display().to_string());
    ui::kv("stake", &format!("{} {TOKEN}", ui::num(a.stake)));
    ui::kv("metrics", &ui::dim(&format!("{}/metrics", h.url)));
    println!(
        "\n  {}\n",
        ui::dim("serving facets and auditing peers · Ctrl+C to stop")
    );
    tokio::signal::ctrl_c().await?;
    h.abort();
    println!("\n  {}\n", ui::dim("stopped"));
    Ok(())
}

fn init_logs(default: &str) {
    let filter = tracing_subscriber::EnvFilter::try_from_env("REFRAX_LOG").unwrap_or_else(|_| {
        tracing_subscriber::EnvFilter::new(format!("refrax_node={default},refrax={default}"))
    });
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .try_init();
}

// ───────────────────────────── devnet ─────────────────────────────

async fn devnet_up(nodes: usize, port: u16, host: IpAddr, dir: PathBuf) -> Result<()> {
    init_logs("error");
    ui::header("devnet", &format!("{nodes} nodes · local registry"));
    let t = Instant::now();
    let net = devnet::start(DevnetConfig {
        nodes,
        port,
        host,
        dir: dir.clone(),
        ..DevnetConfig::default()
    })
    .await?;
    ui::kv(
        "registry",
        &format!(
            "{}  {}",
            ui::cyan(&net.registry_url),
            ui::dim("mirrors the Solana program accounts")
        ),
    );
    ui::kv(
        "slots",
        &format!("{SLOT_MS} ms · epoch = {SLOTS_PER_EPOCH} slots"),
    );
    ui::kv("keys", &dir.display().to_string());
    println!();
    let rows: Vec<Vec<String>> = net
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            vec![
                ui::node_label(i + 1),
                ui::cyan(&n.url),
                ui::dim(&n.pubkey.to_string()),
            ]
        })
        .collect();
    ui::table(&["node", "url", "pubkey"], &rows);
    println!();
    if nodes < FACETS {
        println!(
            "  {}",
            ui::amber(&format!(
                "{nodes} nodes cannot fill a {FACETS}-seat committee; puts will be refused"
            ))
        );
    }
    println!(
        "  {} in {}. In another terminal:",
        ui::mint("ready"),
        ui::ms(t.elapsed())
    );
    println!();
    println!("    {}", ui::violet("refrax put ./README.md"));
    println!(
        "    {}   {}",
        ui::violet("refrax devnet wipe 7"),
        ui::dim("then watch the committee slash node-07")
    );
    println!();
    println!("  {}", ui::dim("slot      event"));

    let labels: HashMap<Pubkey, String> = net
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.pubkey, ui::node_label(i + 1)))
        .collect();
    let mut rx = net.registry.subscribe();
    let label = |p: &Pubkey| labels.get(p).cloned().unwrap_or_else(|| p.short());
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            ev = rx.recv() => {
                if let Ok(rec) = ev { print_event(&rec, &label) }
            }
        }
    }
    println!(
        "\n  {}\n",
        ui::dim("devnet stopped · facets and registry state are discarded, keys are kept")
    );
    net.shutdown();
    Ok(())
}

fn print_event(rec: &EventRecord, label: &impl Fn(&Pubkey) -> String) {
    let slot = ui::dim(&format!("{:<8}", ui::num(rec.slot)));
    let line = match &rec.event {
        Event::NodeJoined { .. } => return,
        Event::ObjectRegistered { object, size } => {
            format!(
                "object {} registered · {}",
                ui::violet(&object.short()),
                ui::bytes(*size)
            )
        }
        Event::ObjectCertified { object, signatures } => format!(
            "object {} {} · {signatures}/{FACETS} signatures",
            ui::violet(&object.short()),
            ui::mint("certified")
        ),
        Event::VoteOpened {
            accused,
            voter,
            object,
            facet,
        } => format!(
            "{} missed a proof · {} facet {facet:02} · vote opened by {}",
            ui::amber(&label(accused)),
            ui::violet(&object.short()),
            label(voter)
        ),
        Event::Slashed {
            node,
            object,
            amount,
            stake,
            votes,
        } => format!(
            "{} {} · {} · {votes} votes · stake {} {TOKEN}",
            ui::red(&label(node)),
            ui::red(&format!("slashed {} {TOKEN}", ui::num(*amount))),
            ui::violet(&object.short()),
            ui::num(*stake)
        ),
    };
    println!("  {slot}  {line}");
}

async fn devnet_wipe(net: &Network, n: usize) -> Result<()> {
    let nodes = net.registry.nodes().await?;
    let node = nodes
        .get(n.wrapping_sub(1))
        .with_context(|| format!("no node-{n:02}; the devnet has {}", nodes.len()))?;
    let wiped = net.wipe(&node.url).await?;
    ui::header(&format!("devnet wipe {n}"), &ui::node_label(n));
    ui::kv(
        "deleted",
        &format!(
            "{wiped} facet{} from {}",
            if wiped == 1 { "" } else { "s" },
            ui::cyan(&node.url)
        ),
    );
    println!(
        "\n  {}\n",
        ui::dim(&format!(
            "Peers miss two proofs in a row, then vote. At {QUORUM} votes the registry slashes the node. Watch the devnet log."
        ))
    );
    Ok(())
}
