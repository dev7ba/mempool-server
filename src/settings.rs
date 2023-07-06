use config::{Config, ConfigError, Environment, File};
use dirs;
use serde::Deserialize;
use std::fmt;
use std::{env, path::PathBuf};

#[derive(Deserialize)]
#[allow(unused)]
pub struct BitcoindClient {
    ///cookie_auth_path takes precedence over user/passwd authentication.
    #[serde(rename = "cookieauthpath")]
    pub cookie_auth_path: Option<PathBuf>,
    #[serde(rename = "ipaddr")]
    pub ip_addr: String,
    pub user: Option<String>,
    pub passwd: Option<String>,
    #[serde(rename = "zmqport")]
    pub zmq_port: u16,
    #[serde(rename = "waittimeoutsec")]
    pub wait_timeout_sec: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[allow(unused)]
pub struct Settings {
    #[serde(rename = "bitcoindclient")]
    pub bitcoind_client: BitcoindClient,
}

impl Default for Settings {
    fn default() -> Self {
        let mut path = dirs::home_dir().unwrap();
        path.push(".bitcoin/.cookie");
        Settings {
            bitcoind_client: BitcoindClient {
                cookie_auth_path: Some(path),
                ip_addr: String::from("127.0.0.1"),
                user: None,
                passwd: None,
                zmq_port: 29000,
                wait_timeout_sec: Some(60),
            },
        }
    }
}
///Settings can be loaded from config.toml file located in the executable directory or from env
///variables. environment variables takes precedence.
/// Note that toml must have have all variable names in lowercase without '_' separators
/// ```
/// [bitcoindclient]
/// 	cookieauthpath = "/home/ba/.bitcoin/.cookie"
///   ipaddr = "localhost"
///   user = "anon"
///   passwd = "anon"
///   zmqport = 29000
///   waittimeoutsec = 60
/// ```
/// If you are using environment variables, you must use MPS as prefix. All section/variables
/// cannot have '_' in its name since '_' is used as delimiter, therefore:
/// ```
/// export MPS_BITCOINDCLIENT_COOKIEAUTHPATH=/whatever
/// export MPS_BITCOINDCLIENT_IPADDR=localhost
/// export MPS_BITCOINDCLIENT_USER=my_user
/// export MPS_BITCOINDCLIENT_PASSWD=my_passwd
/// export MPS_BITCOINDCLIENT_ZMQPORT=my_zmqport
/// export MPS_BITCOINDCLIENT_WAITTIMEOUTSEC=my_wait_timeout_sec
/// ```
/// Note: use always export (not set or "varible=value")
impl Settings {
    pub fn new() -> Result<Self, ConfigError> {
        let mut path = env::current_exe().unwrap();
        path.pop();
        path.push("config.toml");
        if path.exists() {
            let s = Config::builder()
                .set_default("bitcoindclient.waittimeout", "5")?
                .add_source(File::with_name(path.to_str().unwrap()).required(false))
                .add_source(
                    Environment::with_prefix("MPS")
                        .try_parsing(true)
                        .prefix_separator("_")
                        .separator("_"),
                )
                .build()?;
            s.try_deserialize() //.map_or_else(|e| Err(e), |set| Ok(set))
        } else {
            Ok(Settings::default())
        }
    }
}

//Manually implemented Debug to avoid password leak
impl fmt::Debug for BitcoindClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BitcoindClient")
            .field("cookie_auth_path", &self.cookie_auth_path)
            .field("ip_addr", &self.ip_addr)
            .field("user", &"****")
            .field("passwd", &"****")
            .field("zmqport", &self.zmq_port.to_string())
            .field("wait_timeout_sec", &self.wait_timeout_sec)
            .finish()
    }
}
