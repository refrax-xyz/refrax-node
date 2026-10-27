//! Cut a file into 16 facets, drop 11 of them, and rebuild it from the other 5.
//!
//!     cargo run -p refrax-core --example roundtrip -- path/to/file

use refrax_core::erasure::{decode_object, encode_object};
use refrax_core::{Manifest, FACETS, REBUILD};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("usage: roundtrip <file>")?;
    let data = std::fs::read(&path)?;

    let encoded = encode_object(&data);
    let manifest = Manifest::from_encoded(&encoded);
    println!("object  {}", manifest.id());
    println!("root    {}", manifest.root);
    println!("facets  {FACETS} × {} bytes", encoded.shard_len);

    // Keep only the last five facets: all parity, no original bytes among them.
    let survivors: Vec<(usize, &[u8])> = (FACETS - REBUILD..FACETS)
        .map(|i| (i, encoded.facets[i].as_slice()))
        .collect();
    let rebuilt = decode_object(&survivors, encoded.size, encoded.shard_len)?;
    assert_eq!(rebuilt, data);
    println!("rebuilt {} bytes from facets 11–15", rebuilt.len());
    Ok(())
}
