//! Test whether a stored compact filter matches given scriptPubKeys.
//!
//! Usage: cargo run -p dash-spv --example filter_probe -- \
//!            <data-dir> <height> <script-hex> [script-hex ...]

use dash_spv::storage::{
    BlockHeaderStorage, FilterStorage, PersistentBlockHeaderStorage, PersistentFilterStorage,
    PersistentStorage,
};
use dashcore::bip158::BlockFilter;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let data_dir = &args[1];
    let height: u32 = args[2].parse().expect("height");
    let scripts: Vec<Vec<u8>> =
        args[3..].iter().map(|h| hex::decode(h).expect("script hex")).collect();

    let headers = PersistentBlockHeaderStorage::open(data_dir).await.expect("open headers");
    let filters = PersistentFilterStorage::open(data_dir).await.expect("open filters");

    let header = headers
        .load_headers(height..height + 1)
        .await
        .expect("load header")
        .pop()
        .expect("header present");
    let filter_bytes =
        filters.load_filters(height..height + 1).await.expect("load filter").pop().expect("filter");
    let filter = BlockFilter::new(&filter_bytes);

    println!("height {height} block {} filter {} bytes", header.hash(), filter_bytes.len());
    for script in &scripts {
        let query: Vec<&[u8]> = vec![script.as_slice()];
        let matched = filter
            .match_any(header.hash(), &query.into_iter().collect())
            .expect("match");
        println!("script {}... matched: {matched}", hex::encode(&script[..8.min(script.len())]));
    }
}
