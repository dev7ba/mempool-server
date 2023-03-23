use std::sync::atomic::AtomicU64;

use crossbeam_skiplist::SkipMap;
use dashmap::DashMap;

struct Mempool {
    counter: AtomicU64,
    id_tx_map: SkipMap<u64, Vec<u8>>,
    txid_id_map: DashMap<String, u64>,
}

impl Mempool {
    pub fn new() -> Self {
        Mempool {
            counter: AtomicU64::new(0),
            id_tx_map: SkipMap::new(),
            txid_id_map: DashMap::new(),
        }
    }
}
