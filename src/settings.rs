use config::{Config, ConfigError, Environment, File};
use serde::Deserialize;
use std::fmt;
use std::{env, path::PathBuf};
use url::Url;

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
    #[serde(rename = "zmqurl")]
    pub zmq_url: Url,
}

#[derive(Debug, Deserialize)]
#[allow(unused)]
pub struct Settings {
    #[serde(rename = "bitcoindclient")]
    pub bitcoind_client: BitcoindClient,
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
/// ```
/// If you are using environment variables, you must use MPS as prefix. All section/variables
/// cannot have '_' in its name since '_' is used as delimiter, therefore:
/// ```
/// export MPS_BITCOINDCLIENT_COOKIEAUTHPATH=/whatever
/// export MPS_BITCOINDCLIENT_IPADDR=localhost
/// export MPS_BITCOINDCLIENT_USER=my_user
/// export MPS_BITCOINDCLIENT_PASSWD=my_passwd
/// ```
impl Settings {
    pub fn new() -> Result<Self, ConfigError> {
        let mut path = env::current_exe().unwrap();
        path.pop();
        path.push("config.toml");
        let s = Config::builder()
            .add_source(File::with_name(path.to_str().unwrap()).required(false))
            .add_source(
                Environment::with_prefix("mps")
                    .prefix_separator("_")
                    .separator("_"),
            )
            .build()?;
        s.try_deserialize()
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
            .field("zmqurl", &self.zmq_url)
            .finish()
    }
}
