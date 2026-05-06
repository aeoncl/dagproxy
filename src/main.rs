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
    let config: Config = config_dto.try_into().unwrap();

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