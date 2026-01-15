mod cert;
mod config;
mod config_dto;
mod http;
pub mod http_proxy;
mod kerberos;
mod network_watcher;

use crate::config::Config;
use crate::config_dto::ConfigDto;
use http_proxy::HttpProxy;
use netaddr2::{Contains, Netv4Addr};
use std::fmt::{Display, Formatter};
use std::net::IpAddr;
use std::str::FromStr;
use std::{env, fs};
use tokio::runtime;

fn main() {
    print_header();

    let env_args: Vec<String> = env::args().collect();
    let config_file = env_args
        .windows(2)
        .find_map(|window| {
            if window[0] == "-c" || window[0] == "--config" {
                Some(window[1].clone())
            } else {
                None
            }
        })
        .expect("Missing config file path. Usage: -c <path> or --config <path>");
    println!("Loading configuration from: {}", config_file);

    let config_json = fs::read_to_string(config_file).unwrap();
    let config_dto: ConfigDto = serde_json::from_str(&config_json).unwrap();
    let config: Config = config_dto.into();

    let rt = runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(async move {
        let network_handle = network_watcher::watch_networks(config.clone());
        let mut http_proxy = HttpProxy::new(network_handle.clone());

        http_proxy
            .start("127.0.0.1".to_owned(), config.port)
            .await
            .unwrap();
    });
}

fn print_header() {
    const HEADER: &str = r#"
  (
  )\ )   ) (  (        (          ) (
 (()/(( /( )\))( `  )  )(   (  ( /( )\ )
  ((_))(_)|(_))\ /(/( (()\  )\ )\()|()/(
  _| ((_)_ (()(_|(_)_\ ((_)((_|(_)\ )(_))
/ _` / _` / _` || '_ \) '_/ _ \ \ /| || |
\__,_\__,_\__, || .__/|_| \___/_\_\ \_, |
          |___/ |_|                 |__/
    "#;
    println!("{}", HEADER);
}

#[derive(PartialEq, Clone, Debug)]
enum NoProxyValue {
    Host(String),
    Subnet(Netv4Addr),
}

impl NoProxyValue {
    pub fn matches_host(&self, other_host: &str) -> bool {
        match self {
            NoProxyValue::Host(host) => other_host.contains(host),
            NoProxyValue::Subnet(range) => {
                if let Ok(ip) = IpAddr::from_str(other_host) {
                    range.contains(&ip)
                } else {
                    false
                }
            }
        }
    }
}
impl FromStr for NoProxyValue {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.contains("/") {
            let test =
                Netv4Addr::from_str(s).map_err(|e| anyhow::anyhow!("Invalid subnet: {}", e))?;
            Ok(Self::Subnet(test))
        } else {
            Ok(Self::Host(s.to_owned()))
        }
    }
}

impl Display for NoProxyValue {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            NoProxyValue::Host(host) => {
                write!(f, "{}", &host)
            }
            NoProxyValue::Subnet(subnet) => {
                write!(f, "{}", &subnet.to_string())
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use crate::NoProxyValue;
    use std::str::FromStr;

    #[test]
    fn test_ip_no_proxy_matches() {
        let no_proxy = NoProxyValue::from_str("127.0.0.0/24").unwrap();
        assert!(no_proxy.matches_host("127.0.0.2"));
    }

    #[test]
    fn test_url_no_proxy_matches() {
        let no_proxy = NoProxyValue::from_str("google.com").unwrap();
        assert!(no_proxy.matches_host("blabla.google.com"));
        assert!(no_proxy.matches_host("google.com"));
    }
}
