//! End to end on a real local cluster: 16 nodes over HTTP, a registry, the auditors.

use refrax_core::erasure::encode_object;
use refrax_core::{Manifest, FACETS, QUORUM, REBUILD};
use refrax_node::client::Network;
use refrax_node::devnet::{self, DevnetConfig};
use refrax_node::types::{Event, ObjectStatus};
use std::time::Duration;

async fn cluster(audit_ms: u64) -> (devnet::Devnet, Network, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let net = devnet::start(DevnetConfig {
        port: 0,
        dir: dir.path().to_path_buf(),
        audit_every: Duration::from_millis(audit_ms),
        ..DevnetConfig::default()
    })
    .await
    .unwrap();
    let client = Network::new(&net.registry_url);
    (net, client, dir)
}

fn sample(len: usize) -> Vec<u8> {
    (0..len as u32)
        .map(|i| (i.wrapping_mul(2654435761) >> 13) as u8)
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn put_get_with_eleven_nodes_down() {
    let (net, client, _dir) = cluster(60_000).await;
    let data = sample(300_000);
    let enc = encode_object(&data);
    let manifest = Manifest::from_encoded(&enc);

    let rec = client.registry.register_object(&manifest).await.unwrap();
    assert_eq!(rec.committee.len(), FACETS);
    let scatter = client.scatter(&rec, &enc).await.unwrap();
    assert_eq!(scatter.acks.len(), FACETS, "failed: {:?}", scatter.failed);
    let cert = client
        .registry
        .certify(&rec.id, scatter.acks)
        .await
        .unwrap();
    assert_eq!(cert.status, ObjectStatus::Certified);

    let got = client.fetch(&cert).await.unwrap();
    assert_eq!(got.data, data);

    // Take 11 of 16 nodes offline, all five data facets among them: the file still comes back
    // from the five parity facets. A fresh client so no pooled connection outlives the nodes.
    for n in net.nodes.iter().take(FACETS - REBUILD) {
        n.abort();
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    let fresh = Network::new(&net.registry_url);
    let got = fresh.fetch(&cert).await.unwrap();
    assert_eq!(got.data, data);
    assert_eq!(got.used, vec![11, 12, 13, 14, 15]);

    net.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_wiped_node_gets_slashed() {
    let (net, client, _dir) = cluster(250).await;
    let mut events = net.registry.subscribe();
    let enc = encode_object(&sample(40_000));
    let manifest = Manifest::from_encoded(&enc);
    let rec = client.registry.register_object(&manifest).await.unwrap();
    let scatter = client.scatter(&rec, &enc).await.unwrap();
    client
        .registry
        .certify(&rec.id, scatter.acks)
        .await
        .unwrap();

    // Healthy committee: every seat proves its facet.
    let rows = client.audit(&rec).await.unwrap();
    assert!(rows.iter().all(|r| r.result.is_ok()), "{rows:?}");

    // Wipe one node and let the auditors notice.
    let victim = &net.nodes[6];
    assert_eq!(client.wipe(&victim.url).await.unwrap(), 1);
    let slashed = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if let Ok(rec) = events.recv().await {
                if let Event::Slashed {
                    node, stake, votes, ..
                } = rec.event
                {
                    if node == victim.pubkey {
                        return (stake, votes);
                    }
                }
            }
        }
    })
    .await
    .expect("victim should be slashed within 30 s");
    assert_eq!(slashed.0, 2_000);
    assert!(slashed.1 >= QUORUM);

    // The data is still there.
    let got = client
        .fetch(&client.registry.object(&rec.id).await.unwrap())
        .await
        .unwrap();
    assert_eq!(got.data.len(), 40_000);
    net.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nodes_refuse_facets_that_are_not_theirs() {
    let (net, client, _dir) = cluster(60_000).await;
    let enc = encode_object(b"seat check");
    let manifest = Manifest::from_encoded(&enc);
    let rec = client.registry.register_object(&manifest).await.unwrap();
    let urls = client.urls().await.unwrap();
    // Send facet 0 to the node seated at facet 1.
    let wrong = &urls[&rec.committee[1]];
    let resp = reqwest::Client::new()
        .put(format!("{wrong}/v1/facets/{}/0", rec.id))
        .body(enc.facets[0].clone())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 403);
    // Send garbage to the right node.
    let right = &urls[&rec.committee[0]];
    let resp = reqwest::Client::new()
        .put(format!("{right}/v1/facets/{}/0", rec.id))
        .body(vec![0u8; enc.shard_len])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 422);
    net.shutdown();
}
