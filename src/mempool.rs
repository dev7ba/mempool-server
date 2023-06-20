use std::ops::RangeFrom;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use super::TxDepth;
use crossbeam_skiplist::SkipMap;
use dashmap::iter::Iter;
use dashmap::DashMap;

/// This is an special mempool that keeps track of the order of arrival for incoming transactions.
/// Each time a tx is added, counter is incremented.
/// Then we can send to a bitcoin node the ordered list of transactions without problems caused by
/// dependencies between those txs.
pub struct Mempool {
    counter: AtomicU64,
    id_tx_map: SkipMap<u64, Vec<u8>>,
    txid_id_map: DashMap<String, u64>,
}

impl Mempool {
    pub fn new() -> Self {
        Mempool {
            counter: AtomicU64::new(0),
            id_tx_map: SkipMap::new(),
            txid_id_map: DashMap::with_capacity(100000),
        }
    }

    pub fn add_tx(&self, tx_id: String, bytes: Vec<u8>) {
        let previous_value = self.counter.fetch_add(1, Ordering::SeqCst);
        self.txid_id_map.insert(tx_id, previous_value);
        self.id_tx_map.insert(previous_value, bytes);
    }

    pub fn remove_tx(&self, tx_id: &String) {
        let kk = self.txid_id_map.remove(tx_id);
        match kk {
            Some((_, id)) => {
                self.id_tx_map.remove(&id);
            }
            None => {}
        };
    }

    pub fn len(&self) -> u32 {
        self.txid_id_map.len() as u32
    }

    pub fn counter(&self) -> u64 {
        self.counter.load(Ordering::SeqCst)
    }

    pub fn load_mempool_with(&self, vec2: Vec<Vec<TxDepth>>) {
        vec2.into_iter().for_each(|vec| {
            vec.into_iter()
                .for_each(|tx_depth| self.add_tx(tx_depth.tx_id.to_string(), tx_depth.bytes))
        });
    }

    pub fn txid_pos_iterator(&self) -> Iter<String, u64> {
        self.txid_id_map.iter()
    }

    pub fn pos_data_iterator(&self) -> crossbeam_skiplist::map::Iter<u64, Vec<u8>> {
        self.id_tx_map.iter()
    }

    pub fn pos_data_iterator_from(
        &self,
        from: u64,
    ) -> crossbeam_skiplist::map::Range<'_, u64, RangeFrom<u64>, u64, Vec<u8>> {
        self.id_tx_map.range(from..)
    }
}
