mod network_watcher;
mod kerberos;
mod http;
mod cert;
pub mod http_proxy;
mod https_proxy;
mod transparent_proxy;

use netaddr2::Netv4Addr;
use std::str::FromStr;
use std::env;
use std::panic::{set_hook, take_hook};
use http_proxy::HttpProxy;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::runtime;
fn main() {
    print_header();


    let env_args: Vec<String> = env::args().collect();

    if env_args.get(1) == Some(&"--help".to_owned()) {
        print_help();
        return;
    }

    let args = DagProxyArgs::from_env_args(env_args);

    if args.transparent_proxy {
        println!("Transparent mode coming soon ;)");
        return;
    }

    let rt = runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Cannot start thread pool");

    rt.block_on(async move {
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
    println!("\tdagproxy [options]");
    println!();
    println!("Options:");
    print_padded("\t--corporate-subnets <subnet1>,<subnet2>" ,"Forwards trafic to the upstream proxy when on one of those subnets", min_length);
    print_padded("\t--upstream-proxy <host>:<port>", "The upstream proxy to forward traffic to", min_length);
    print_padded("\t--listen-port <port>", "The port to listen on for HTTP traffic. Defaults to 3232", min_length);
    print_padded("\t--listen-port-https <port>", "The port to listen on for HTTPS traffic. Defaults to 3233. Only used when --transparent is set.", min_length);
    print_padded("\t--no-proxy <host1>,<host2>", "Hosts to not proxy. Defaults to none.", min_length);
    print_padded("\t--transparent", "Use transparent proxying. Defaults to false. This will require you to install a certificate on your machine.", min_length);
    print_padded("\t--help", "Print this help message", min_length);
    println!();
    println!("Example:");
    println!("\tdagproxy --upstream-proxy 192.168.1.1:3128 --corporate-subnets 192.168.1.0/24 --listen-port 3232");
}

fn print_padded(to_pad: &str, other_half: &str, min_length: i32) {
    let spaces_to_add: i32 = min_length - to_pad.len() as i32;
    print!("{}", to_pad);
    if(spaces_to_add > 0) {
        print!("{}", ".".repeat(spaces_to_add as usize));
    }
    print!(" {}", other_half);
    println!();
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
            }).expect("Missing required argument: --upstream-proxy <host>:<port>");

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
        }).expect("Missing required argument: --corporate-subnets <0.0.0.0/32>,<1.1.1.1/24>");

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

        let transparent_proxy = env_args.iter().any(|window| {
            window.as_str() == "--transparent"
        });

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




