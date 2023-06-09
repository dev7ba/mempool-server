use bitcoincore_rpc::bitcoin::Txid;

#[derive(Debug)]
pub struct TxDepth {
    pub ancestor_count: usize,
    pub tx_id: Txid,
    pub bytes: Vec<u8>,
}
