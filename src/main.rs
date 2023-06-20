use anyhow::{anyhow, Context, Result};
use bitcoincore_rpc::bitcoin::BlockHash;
use bitcoincore_rpc::{bitcoin::hashes::sha256d::Hash, bitcoin::Txid, Auth, Client, RpcApi};
use bitcoincore_zmq::check::{ClientConfig, NodeChecker};
use bitcoincore_zmq::{MempoolSequence, ZmqSeqListener};
use log::{info, warn, LevelFilter};
use mempool::Mempool;
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use rocket::response::stream::{ByteStream, TextStream};
use rocket::State;
use settings::{BitcoindClient, Settings};
use simple_logger::SimpleLogger;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;
use txdepth::TxDepth;

mod mempool;
mod settings;
mod txdepth;

#[macro_use]
extern crate rocket;

struct App {
    pub mempool: Arc<Mempool>,
    pub zmqseqlistener_stop: Arc<AtomicBool>,
    pub zmqseqlistener_thread: JoinHandle<()>,
    pub mp_filler_stop: Arc<AtomicBool>,
    pub mp_filler_thread: JoinHandle<()>,
}

#[rocket::main]
async fn main() -> Result<(), rocket::Error> {
    match main_app() {
        Ok(app) => {
            info!("Mempool data loaded, launching REST Server...");
            rocket::build()
                .manage(app.mempool)
                .mount("/mempool", routes![size, txsids, txsdata, txsdatafrom])
                .launch()
                .await?;

            info!("ZMQ listener is stopping...");
            app.zmqseqlistener_stop.store(true, Ordering::SeqCst);
            app.zmqseqlistener_thread.join().unwrap();
            info!("ZMQ listener stopped.");

            info!("Mempool filler thread is stopping...");
            app.mp_filler_stop.store(true, Ordering::SeqCst);
            app.mp_filler_thread.join().unwrap();
            info!("Mempool filler thread is stopped.");
        }
        Err(e) => {
            error!("{}", e);
        }
    }

    Ok(())
}

fn main_app() -> Result<App> {
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
    let zmqseqlistener = ZmqSeqListener::start(&bcc_settings.zmq_url)?;
    let bcc = get_client(&settings.bitcoind_client)?;
    let size = log_mempool_size(&bcc)?;

    let vec = get_tx_dept_vec(&bcc, size)?;
    //vec2 is a vector of vectors containing txs with same ancestor_count:
    //(vec2[ancestor_count-1] has a vector with all tx having ancestor_count-1)
    let vec2 = get_mempool_layers(vec);
    log_mempool_layers(&vec2);

    let mempool = Arc::new(Mempool::new());
    let mempool2 = mempool.clone();
    mempool.load_mempool_with(vec2);
    info!("Loaded mempool with {} transactions", mempool.len());

    let thread = thread::spawn(move || {
        while !stop_th2.load(Ordering::SeqCst) {
            let mps = zmqseqlistener.rx.recv().unwrap();
            info!("{:?}", &mps);
            update_mempool(&mempool, &mps, &bcc).unwrap();
            info!(
                "Mempool size: {}, mempool counter: {}",
                mempool.len(),
                mempool.counter()
            );
            log_mempool_size(&bcc).unwrap();
        }
    });
    Ok(App {
        mempool: mempool2,
        zmqseqlistener_stop: zmqseqlistener.stop,
        zmqseqlistener_thread: zmqseqlistener.thread,
        mp_filler_stop: stop_th,
        mp_filler_thread: thread,
    })
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

fn log_mempool_size(bcc: &Client) -> Result<usize, anyhow::Error> {
    let size = bcc
        .get_mempool_info()
        .with_context(|| "Can't connect to bitcoind node")?
        .size;
    info!("# {} Transactions in bitcoin node mempool", size);
    Ok(size)
}

fn get_tx_dept_vec(source_client: &Client, size: usize) -> Result<Vec<TxDepth>> {
    info!("Loading mempool txids and hierarchy...");
    let i = AtomicU32::new(0);
    let last_per = AtomicU32::new(0);
    let vec: Vec<TxDepth> = source_client
        .get_raw_mempool_verbose()?
        .par_iter()
        .map(|(txid, mpe)| {
            percent(&i, &last_per, size as u32);
            (txid, mpe)
        })
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

//This funcion is incorrect, but the worst can happen (very unlikely) is a % been skipped.
fn percent(ai: &AtomicU32, alast_per: &AtomicU32, size: u32) {
    let i = ai.fetch_add(1, Ordering::SeqCst);
    if i == 0 || size == 0 {
        info!("Mempool txids and hierarchy loaded, now asking full txs binary data...");
        info!("Loading: 0%");
    } else {
        if i == size {
            info!("Done: 100%");
        } else {
            let per = ((i as f32 / size as f32) * 100f32).trunc() as u32;
            if alast_per.fetch_max(per, Ordering::SeqCst) != per {
                info!("Loading: {}%", per);
            }
        }
    }
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

#[get("/size")]
fn size(mempool: &State<Arc<Mempool>>) -> String {
    format!("{}", mempool.len())
}

#[get("/txsids")]
fn txsids(mempool: &State<Arc<Mempool>>) -> TextStream![String + '_] {
    TextStream! {
        for entry in mempool.txid_pos_iterator(){
            yield format!("{}\n",entry.key());
        }
    }
}

#[get("/txsdata")]
fn txsdata(mempool: &State<Arc<Mempool>>) -> ByteStream![Vec<u8> + '_] {
    let mut first = true;
    ByteStream! {
    info!("Empieza stream");
        for entry in mempool.pos_data_iterator(){
            let data = entry.value().clone();
            let size = data.len() as u32;
            if first {
                first=false;
                yield u64::MAX.to_be_bytes().to_vec();//Magic number to start a correct stream
                yield mempool.len().to_be_bytes().to_vec();//u32 as a hint of its size
                yield mempool.counter().to_be_bytes().to_vec();//u64 mempool counter
            }
            yield size.to_be_bytes().to_vec();
            yield data;
        }
    info!("Fin del stream");
    }
}

#[get("/txsdatafrom/<from>")]
fn txsdatafrom(from: u64, mempool: &State<Arc<Mempool>>) -> ByteStream![Vec<u8> + '_] {
    let mut first = true;
    ByteStream! {
    info!("Empieza stream");
    let range = mempool.pos_data_iterator_from(from);
        for entry in range{
            let data = entry.value().clone();
            let size = data.len() as u32;
            if first {
                first=false;
                yield u64::MAX.to_be_bytes().to_vec();//Magic number to start a correct stream
                //No hint of its size since calculate it consumes the iterator
                yield mempool.counter().to_be_bytes().to_vec();//u64 mempool counter
            }
            yield size.to_be_bytes().to_vec();
            yield data;
        }
    info!("Fin del stream");
    }
}
