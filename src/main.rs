mod network_watcher;
mod kerberos;
mod http;
pub mod http_proxy;

use netaddr2::Netv4Addr;
use std::str::FromStr;
use std::env;
use std::panic::{set_hook, take_hook};
use http_proxy::HttpProxy;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::runtime;
fn main() {



    let env_args: Vec<String> = env::args().collect();

    if env_args.get(1) == Some(&"--help".to_owned()) {
        println!("Usage: dagproxy --upstream-proxy <host>:<port> --corporate-subnets <subnet1>,<subnet2> --listen-port <port> --listen-port-https <port> [--transparent]");
        return;
    }

    let args = DagProxyArgs::from_env_args(env_args);

    let rt = runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(async move {
        println!("Starting DagProxy");
        let network_handle = network_watcher::watch_networks(args.corporate_subnets);
        let mut http_proxy = HttpProxy::new(
            args.upstream_proxy_host,
            args.upstream_proxy_port,
            args.no_proxy,
            network_handle.clone()
        );

        http_proxy.start("127.0.0.1".to_owned(), args.listen_port_http).await.unwrap();



    });

}

struct DagProxyArgs {
    upstream_proxy_host: String,
    upstream_proxy_port: u32,
    no_proxy: Vec<String>,
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

            let upstream_proxy = env_args.windows(2).find_map(|window| {
                if window[0] == "--upstream-proxy" {
                    Some(window[1].to_owned())
                } else {
                    None
                }
            }).expect("Missing required argument: --upstream-proxy");

            let mut split = upstream_proxy.split(":");
            (split.next().expect("upstream proxy to have host").to_owned(), u32::from_str(split.next().expect("upstream proxy to have port")).expect("upstream proxy port to be a number"))
        };

        let no_proxy_hosts = env_args.windows(2).find_map(|window| {
            if window[0] == "--no-proxy" {
                Some(window[1].split(",").map(|host| host.to_owned()).collect::<Vec<_>>())
            } else {
                None
            }
        }).unwrap_or_default();

        let corporate_subnets = env_args.windows(2).find_map(|window| {
            if window[0] == "--corporate-subnets" {
                let subnets = window[1]
                    .split(",")
                    .map(|subnet| Netv4Addr::from_str(subnet).unwrap())
                    .collect::<Vec<_>>();
                Some(subnets)
            } else {
                None
            }
        }).expect("Missing required argument: --corporate-subnets");

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
            .unwrap_or(listen_port+1);

        let transparent_proxy = env_args.iter().find_map(|window| {
            if window == "--transparent" {
                Some(true)
            } else {
                None
            }
        }).unwrap_or(false);

        let _ = take_hook();

        Self {
            upstream_proxy_host,
            upstream_proxy_port,
            no_proxy: no_proxy_hosts,
            corporate_subnets,
            listen_port_http: listen_port,
            listen_port_https: listen_port_https,
            transparent_proxy,
        }

    }
}




