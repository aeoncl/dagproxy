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
use std::panic::{set_hook, take_hook};
use std::str::FromStr;
use std::{env, fs};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::runtime;

fn main() {
    print_header();

    let env_args: Vec<String> = env::args().collect();
    let config_file = env_args.get(1).expect("Missing config file path");

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

fn print_help() {
    let min_length = 45;

    println!("Usage:");
    println!("\tdagproxy [config_file_path]");
    println!();

    let example_config = r#"
     {
       "port": 3232,
       "subnets": [
         {
           "Proxy": {
             "ip_range": "10.80.0.0/16",
             "proxy_host": "proxygate.onemrva.priv",
             "proxy_port": 8888,
             "no_proxy": [
               "localhost",
               "rvaonem.priv",
               "rvaonem.fgov.be",
               "169.254.169.254",
               "cloud.rvadc.be",
               "onemrva.priv",
               "teams.microsoft.com",
               "google.com"
             ]
           }
         },
         {
           "Proxy": {
             "ip_range": "10.130.0.0/16",
             "proxy_host": "proxygate.onemrva.priv",
             "proxy_port": 8888,
             "no_proxy": [
               "localhost",
               "rvaonem.priv",
               "rvaonem.fgov.be",
               "169.254.169.254",
               "cloud.rvadc.be",
               "onemrva.priv"
             ]
           }
         },
         "Direct"
       ]
     }"#.trim();

    println!("Config example:");
    println!("{}", example_config);

    println!("Example:");
    println!(
        "\tdagproxy /home/user/dagproxy/config.json"
    );
}

fn print_padded(to_pad: &str, other_half: &str, min_length: i32) {
    let spaces_to_add: i32 = min_length - to_pad.len() as i32;
    print!("{}", to_pad);
    if (spaces_to_add > 0) {
        print!("{}", ".".repeat(spaces_to_add as usize));
    }
    print!(" {}", other_half);
    println!();
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

struct DagProxyArgs {
    upstream_proxy_host: String,
    upstream_proxy_port: u32,
    no_proxy: Vec<NoProxyValue>,
    corporate_subnets: Vec<Netv4Addr>,
    listen_port_http: u32,
    listen_port_https: u32,
    transparent_proxy: bool,
}
impl DagProxyArgs {
    fn from_env_args(env_args: Vec<String>) -> Self {
        set_hook(Box::new(|info| {
            if let Some(s) = info.payload().downcast_ref::<String>() {
                println!("{}", s);
            }
        }));

        let (upstream_proxy_host, upstream_proxy_port) = {
            let upstream_proxy = env_args
                .windows(2)
                .find_map(|window| {
                    if window[0] == "--upstream-proxy" {
                        Some(window[1].to_owned())
                    } else {
                        None
                    }
                })
                .expect("Missing required argument: --upstream-proxy <host>:<port>");

            let mut split = upstream_proxy.split(":");
            (
                split
                    .next()
                    .expect("upstream proxy to have host")
                    .to_owned(),
                u32::from_str(split.next().expect("upstream proxy to have port"))
                    .expect("upstream proxy port to be a number"),
            )
        };

        let no_proxy_hosts = env_args
            .windows(2)
            .find_map(|window| {
                if window[0] == "--no-proxy" {
                    let no_proxies = window[1]
                        .split(",")
                        .map(|host| {
                            NoProxyValue::from_str(host)
                                .expect(format!("Invalid no proxy host: {}", &host).as_str())
                        })
                        .collect::<Vec<_>>();

                    Some(no_proxies)
                } else {
                    None
                }
            })
            .unwrap_or_default();

        let corporate_subnets = env_args
            .windows(2)
            .find_map(|window| {
                if window[0] == "--corporate-subnets" {
                    let subnets = window[1]
                        .split(",")
                        .map(|subnet| Netv4Addr::from_str(subnet).unwrap())
                        .collect::<Vec<_>>();
                    Some(subnets)
                } else {
                    None
                }
            })
            .expect("Missing required argument: --corporate-subnets <0.0.0.0/32>,<1.1.1.1/24>");

        let listen_port = env_args
            .windows(2)
            .find_map(|window| {
                if window[0] == "--listen-port" {
                    Some(u32::from_str(&window[1]).expect("port to be a number"))
                } else {
                    None
                }
            })
            .unwrap_or(3232);

        let listen_port_https = env_args
            .windows(2)
            .find_map(|window| {
                if window[0] == "--listen-port-https" {
                    Some(u32::from_str(&window[1]).expect("port to be a number"))
                } else {
                    None
                }
            })
            .unwrap_or(listen_port + 1);

        let transparent_proxy = env_args
            .iter()
            .any(|window| window.as_str() == "--transparent");

        let _ = take_hook();

        Self {
            upstream_proxy_host,
            upstream_proxy_port,
            no_proxy: no_proxy_hosts,
            corporate_subnets,
            listen_port_http: listen_port,
            listen_port_https,
            transparent_proxy,
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
