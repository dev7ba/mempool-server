use anyhow::{anyhow, Context, Result};
use bitcoincore_rpc::{bitcoin::Txid, Auth, Client, RpcApi};
use bitcoincore_zmq::check::{ClientConfig, NodeChecker};
use bitcoincore_zmq::ZmqSeqListener;
use log::LevelFilter;
use log::{info, warn};
use settings::{BitcoindClient, Settings};
use simple_logger::SimpleLogger;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

mod mempool;
mod settings;

#[derive(Debug)]
enum ClientType {
    Source,
    Destination,
}

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
    info!("{:#?}", settings);

    let bcc = settings.bitcoind_client;
    let checker = NodeChecker::new(&ClientConfig {
        cookie_auth_path: bcc.cookie_auth_path,
        ip_addr: bcc.ip_addr,
        user: bcc.user.unwrap_or("".to_string()),
        passwd: bcc.passwd.unwrap_or("".to_string()),
    })?;

    info!("Waiting to node Ok");
    checker.wait_till_node_ok(2, false, Duration::from_secs(5))?;
    info!("Node Ok");

    let stop_th = Arc::new(AtomicBool::new(false));
    let stop_th2 = stop_th.clone();
    ctrlc::set_handler(move || stop_th2.store(true, Ordering::SeqCst))?;
    let zmqseqlistener = ZmqSeqListener::start(&bcc.zmq_url)?;
    while !stop_th.load(Ordering::SeqCst) {
        let mps = zmqseqlistener.receiver().recv()?;
        info!("{:?}", mps);
    }
    Ok(())
}

fn get_client(bcc: &BitcoindClient) -> Result<Client, anyhow::Error> {
    let client = if let Some(path) = &bcc.cookie_auth_path {
        get_client_cookie(&bcc.ip_addr, path.clone(), ClientType::Source)?
    } else {
        get_client_user_passw(
            &bcc.ip_addr,
            bcc.user.as_ref().unwrap().clone(),
            bcc.passwd.as_ref().unwrap().clone(),
            ClientType::Source,
        )?
    };
    Ok(client)
}

fn get_client_cookie(ip: &str, path: PathBuf, client_type: ClientType) -> Result<Client> {
    Client::new(ip, Auth::CookieFile(path))
        .with_context(|| format!("Can't connect to {:?} bitcoind node: {}", client_type, ip))
}

fn get_client_user_passw(
    ip: &str,
    user_name: String,
    passwd: String,
    client_type: ClientType,
) -> Result<Client> {
    Client::new(ip, Auth::UserPass(user_name, passwd))
        .with_context(|| format!("Can't connect to {:?} bitcoind node: {}", client_type, ip))
}
