use anyhow::{anyhow, Context, Result};
use bitcoincore_rpc::bitcoin::BlockHash;
use bitcoincore_rpc::{bitcoin::hashes::sha256d::Hash, bitcoin::Txid, Auth, Client, RpcApi};
use bitcoincore_zmq::check::{ClientConfig, NodeChecker};
use bitcoincore_zmq::{MempoolSequence, ZmqSeqListener};
use log::{info, warn, LevelFilter};
use mempool::Mempool;
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use settings::{BitcoindClient, Settings};
use simple_logger::SimpleLogger;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use txdepth::TxDepth;

mod mempool;
mod settings;
mod txdepth;

fn main() -> Result<()> {
    SimpleLogger::new()
        .with_module_level("bitcoincore_rpc", LevelFilter::Info)
        .with_utc_timestamps()
        .with_colors(true)
        .init()
        .unwrap();

    let settings = match Settings::new() {
        Ok(settings) => settings,
        Err(e) => {
            warn!("Error, cannot load all necessary settings from config.toml or environment variables: {}",e);
            return Err(anyhow!("error:{}", e));
        }
    };
    info!("{:#?}", &settings);

    let bcc_settings = &settings.bitcoind_client;
    check_and_wait_till_node_ok(bcc_settings)?;

    let stop_th = Arc::new(AtomicBool::new(false));
    let stop_th2 = stop_th.clone();
    ctrlc::set_handler(move || stop_th2.store(true, Ordering::SeqCst))?;
    let zmqseqlistener = ZmqSeqListener::start(&bcc_settings.zmq_url)?;

    let bcc = get_client(&settings.bitcoind_client)?;
    log_mempool_size(&bcc)?;

    let vec = get_tx_dept_vec(&bcc)?;
    //vec2 is a vector of vectors containing txs with same ancestor_count:
    //(vec2[ancestor_count-1] has a vector with all tx having ancestor_count-1)
    let vec2 = get_mempool_layers(vec);
    log_mempool_layers(&vec2);

    let mempool = Mempool::new();
    mempool.load_mempool_with(vec2);
    info!("Loaded mempool with {} transactions", mempool.len());

    while !stop_th.load(Ordering::SeqCst) {
        let mps = zmqseqlistener.receiver().recv()?;
        info!("{:?}", &mps);
        update_mempool(&mempool, &mps, &bcc)?;
        info!("Mempool size: {}", mempool.len());
        log_mempool_size(&bcc)?;
    }
    Ok(())
}

fn check_and_wait_till_node_ok(bcc_settings: &BitcoindClient) -> Result<(), anyhow::Error> {
    let checker = NodeChecker::new(&ClientConfig {
        cookie_auth_path: bcc_settings.cookie_auth_path.clone(),
        ip_addr: bcc_settings.ip_addr.clone(),
        user: bcc_settings.user.clone().unwrap_or("".to_string()),
        passwd: bcc_settings.passwd.clone().unwrap_or("".to_string()),
    })?;
    let has_index = checker.check_tx_index()?;
    if !has_index {
        return Err(anyhow!(
            "bitcoind must have transactions index enabled, add txindex=1 to bitcoin.conf file"
        ));
    }
    info!("Waiting to node Ok");
    checker.wait_till_node_ok(2, Duration::from_secs(5))?;
    info!("Node Ok");
    Ok(())
}

fn get_client(bcc: &BitcoindClient) -> Result<Client, anyhow::Error> {
    let client = if let Some(path) = &bcc.cookie_auth_path {
        get_client_cookie(&bcc.ip_addr, path.clone())?
    } else {
        get_client_user_passw(
            &bcc.ip_addr,
            bcc.user.as_ref().unwrap().clone(),
            bcc.passwd.as_ref().unwrap().clone(),
        )?
    };
    Ok(client)
}

fn get_client_cookie(ip: &str, path: PathBuf) -> Result<Client> {
    Client::new(ip, Auth::CookieFile(path))
        .with_context(|| format!("Can't connect to bitcoind node: {}", ip))
}

fn get_client_user_passw(ip: &str, user_name: String, passwd: String) -> Result<Client> {
    Client::new(ip, Auth::UserPass(user_name, passwd))
        .with_context(|| format!("Can't connect to bitcoind node: {}", ip))
}

fn log_mempool_size(bcc: &Client) -> Result<(), anyhow::Error> {
    let size = bcc
        .get_mempool_info()
        .with_context(|| "Can't connect to bitcoind node")?
        .size;
    info!("# {} Transactions in bitcoin node mempool)", size);
    Ok(())
}

fn get_tx_dept_vec(source_client: &Client) -> Result<Vec<TxDepth>> {
    let vec: Vec<TxDepth> = source_client
        .get_raw_mempool_verbose()?
        .par_iter()
        .filter_map(|(tx_ide, mempool_entry)| {
            match source_client.get_raw_transaction_hex(tx_ide, None) {
                Ok(raw) => Some(TxDepth {
                    ancestor_count: mempool_entry.ancestor_count as usize,
                    tx_id: tx_ide.clone(),
                    bytes: hex::decode(raw).unwrap(),
                }),
                Err(_) => None, //If tx_id do not exist we don't care
            }
        })
        .collect();
    return Ok(vec);
}

fn get_mempool_layers(vec: Vec<TxDepth>) -> Vec<Vec<TxDepth>> {
    let mut vec2: Vec<Vec<TxDepth>> = vec![];
    for tx_depth in vec {
        let ancestor_index = tx_depth.ancestor_count - 1;
        while vec2.len() <= ancestor_index {
            vec2.push(vec![]);
        }
        vec2[ancestor_index].push(tx_depth);
    }
    vec2
}

fn log_mempool_layers(vec2: &Vec<Vec<TxDepth>>) {
    info!("Transactions dependencies:");
    for (i, txid_vec) in vec2.iter().enumerate() {
        info!("#Txs depending of {} parents: {}", i, txid_vec.len());
    }
}

fn get_raw_transaction_hex(bcc: &Client, tx_id: &Txid) -> Option<Vec<u8>> {
    // thread::sleep(Duration::from_millis(1000));
    match bcc.get_raw_transaction_hex(tx_id, None) {
        Ok(tx) => Some(hex::decode(tx).unwrap()),
        Err(e) => {
            //Don't care if not found
            info!("tx_id: {} not found, err{}", tx_id, e);
            None
        }
    }
}

fn update_mempool(mempool: &Mempool, mps: &MempoolSequence, bcc: &Client) -> Result<()> {
    match mps {
        MempoolSequence::SeqStart {
            bitcoind_already_working,
        } => {
            if !bitcoind_already_working {
                Err(anyhow!("Bitcoind node was not already working:"))
            } else {
                Ok(())
            }
        }
        MempoolSequence::SeqError { error } => Err(anyhow!("Error: {}", error)),
        MempoolSequence::TxAdded { txid, .. } => {
            let tx_id = &Txid::from(Hash::from_str(txid.as_str())?);
            if let Some(bytes) = get_raw_transaction_hex(bcc, tx_id) {
                mempool.add_tx(txid.clone(), bytes);
            }
            Ok(())
        }
        MempoolSequence::TxRemoved { txid, .. } => {
            mempool.remove_tx(txid);
            Ok(())
        }
        MempoolSequence::BlockConnection { block_hash, .. } => {
            let block = bcc.get_block_info(&BlockHash::from_str(&block_hash)?)?;
            block.tx.iter().for_each(|tx_id| {
                mempool.remove_tx(&tx_id.to_string());
            });
            Ok(())
        }
        MempoolSequence::BlockDisconnection { block_hash, .. } => {
            let block = bcc.get_block_info(&BlockHash::from_str(&block_hash)?)?;
            block.tx.iter().for_each(|tx_id| {
                if let Some(bytes) = get_raw_transaction_hex(bcc, tx_id) {
                    mempool.add_tx(tx_id.to_string(), bytes);
                }
            });
            Ok(())
        }
    }
}
