mod network;
mod kerberos;

use anyhow::anyhow;
use bytes::{Bytes, BytesMut};
use netaddr2::{Contains, Netv4Addr};
use std::error::Error;
use std::io::ErrorKind;
use std::net::ToSocketAddrs;
use std::str::{from_utf8, FromStr};
use std::thread::sleep;
use std::time::Duration;
use std::{env, io, mem};
use tokio::net::{TcpListener, TcpSocket, TcpStream};

use crate::kerberos::kerberos::get_negotiate_token;
use crate::network::NetworkType;
use backon::Retryable;
use backon::{ExponentialBackoff, ExponentialBuilder};
use http_body_util::BodyExt;
use retry_strategy::{retry, ToDuration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::runtime;
use tokio::sync::mpsc::Sender;

const SUCCESS_CONNECT_RESPONSE: &[u8] = b"HTTP/1.1 200 Connection established\r\n\r\n";

fn main() {
    let args: Vec<String> = env::args().collect();

    let upstream_proxy = args.windows(2).find_map(|window| {
        if window[0] == "--upstream-proxy" {
            Some(window[1].to_owned())
        } else {
            None
        }
    });

    let port = args
        .windows(2)
        .find_map(|window| {
            if window[0] == "--port" {
                Some(window[1].to_owned())
            } else {
                None
            }
        })
        .unwrap_or("3232".into());

    let corporate_subnets = args.windows(2).find_map(|window| {
        if window[0] == "--corporate-subnets" {
            let subnets = window[1]
                .split(",")
                .map(|subnet| Netv4Addr::from_str(subnet).unwrap())
                .collect::<Vec<_>>();
            Some(subnets)
        } else {
            None
        }
    });

    let corporate_subnets = corporate_subnets.unwrap();

    let rt = runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(async move {
        println!("Starting DagProxy on port: {}", &port);

        let network_handle = network::watch_networks(corporate_subnets);

        let listener = TcpListener::bind(format!("127.0.0.1:{}", port))
            .await
            .unwrap();

        loop {
            let (mut source_socket, _) = listener.accept().await.unwrap();
            let mut dest_socket: Option<TcpStream> = Option::None;

            let network_handle_clone = network_handle.clone();
            let upstream_proxy_clone = upstream_proxy.clone().unwrap();

            let mut network_updates_receiver = network_handle_clone.subscribe();
            network_updates_receiver.mark_unchanged();

            let mut connection_state = ConnectionState::Initializing;

            let _ = tokio::spawn(async move {

                loop {

                    let mut source_read_buffer = [0; 2048];
                    let mut dest_read_buffer = [0; 2048];


                    tokio::select! {
                       network_update = network_updates_receiver.changed() => {
                            if let Ok(_) = network_update {
                               let network_type = network_updates_receiver.borrow_and_update().clone();
                                println!("Network has changed, switching connection.");

                                if let ConnectionState::Forwarding(host)  = connection_state.clone() {
                                    match network_type {
                                        NetworkType::Direct => {
                                            drop(dest_socket.expect("to be here"));
                                            match connect_with_retry(&host).await {
                                                Ok(socket) => {
                                                    println!("Switched to Direct");
                                                    dest_socket = Some(socket);
                                                }
                                                Err(e) => {
                                                    println!("Failed to connect to {}: {}", host, e);
                                                    break;
                                                }
                                            }
                                        },
                                        NetworkType::Proxied => {
                                            drop(dest_socket.expect("to be here"));
                                            match connect_to_proxy(&upstream_proxy_clone, &host).await {
                                                Ok(socket) => {
                                                    println!("Switched to Proxied");
                                                    dest_socket = Some(socket);
                                                }
                                                Err(e) => {
                                                    println!("Failed to connect to {}: {}", host, e);
                                                    break;
                                                }
                                            }

                                        }
                                    }
                                }
                            }
                       }
                       from_destination = async { dest_socket.as_mut().unwrap().read(&mut dest_read_buffer).await }, if dest_socket.is_some() => {
                            if let Ok(bytes_read) = from_destination {
                                if bytes_read == 0 {
                                    break;
                                }
                                let data = &dest_read_buffer[..bytes_read];
                                source_socket.write_all(&data).await.unwrap();
                            }

                        }
                        from_source = source_socket.read(&mut source_read_buffer) => {
                            if let Ok(bytes_read) = from_source {

                                if bytes_read == 0 {
                                    //Socket closed
                                    break;
                                }

                                let data = &source_read_buffer[..bytes_read];

                                match &mut connection_state {
                                    ConnectionState::Initializing => {
                                        match parse_host_from_request(&data) {
                                            Ok((req_type, host)) => {
                                                match network_handle_clone.network_type() {
                                                    NetworkType::Direct => {
                                                        println!("Connecting to {}", &host);
                                                        let connect_result = connect_with_retry(&host).await;
                                                        match connect_result {
                                                            Ok(socket) => {
                                                                dest_socket = Some(socket);
                                                                connection_state = ConnectionState::Forwarding(host);
                                                                     match req_type {
                                                                         RequestType::Connect => {
                                                                             let _ = source_socket.write_all(SUCCESS_CONNECT_RESPONSE).await;
                                                                         },
                                                                         RequestType::Other => {
                                                                             let _ = dest_socket.as_mut().expect("to be here").write_all(data).await;
                                                                         }
                                                                     }
                                                            },
                                                            Err(e) => {
                                                                println!("Failed to connect to {}: {}", upstream_proxy_clone, e);
                                                                break;
                                                            }
                                                        }

                                                    },
                                                    NetworkType::Proxied => {
                                                        println!("Connecting to {} -> {}", &upstream_proxy_clone, &host);
                                                        let connect_result = connect_to_proxy(&upstream_proxy_clone, &host).await;
                                                        match connect_result {
                                                            Ok(socket) => {
                                                                dest_socket = Some(socket);
                                                                connection_state = ConnectionState::Forwarding(host.clone());

                                                                match req_type {
                                                                    RequestType::Connect => {
                                                                        let _ = source_socket.write_all(SUCCESS_CONNECT_RESPONSE).await;
                                                                    },
                                                                    RequestType::Other => {
                                                                        let _ = dest_socket.as_mut().expect("to be here").write_all(data).await;
                                                                    }
                                                                }
                                                            }
                                                            Err(e) => {
                                                                println!("Failed to connect to {}: {}", upstream_proxy_clone, e);
                                                                break;
                                                            }
                                                        }

                                                    }
                                                }
                                            }
                                            Err(e) => {
                                                println!(
                                                "Unexpected data received: {}",
                                                String::from_utf8_lossy(&data)
                                            );
                                            }
                                        }
                                    }

                                    ConnectionState::Forwarding(target) => {
                                        let result = dest_socket.as_mut().unwrap().write_all(&data).await;
                                        if let Err(e) = result {
                                            println!("Warn: failed to write to destination ({}): {}", &target, e);
                                        }
                                    }
                                }


                            }

                        }
                    }
                }
            });
        }
    });
}


async fn connect_to_proxy(proxy_host: &str, target_host: &str) -> Result<TcpStream, anyhow::Error> {

    let proxy_without_port = {
        let parts = proxy_host.split(":").collect::<Vec<&str>>();
        parts.first().map(|e| e.to_owned()).ok_or(anyhow::anyhow!("Could not split proxy on :"))
    }?;

    let mut proxy_stream = connect_with_retry(proxy_host).await?;
        proxy_stream.write_all(format!("CONNECT {} HTTP/1.1\r\nHost: {}\r\n\r\n", &target_host, &target_host).as_bytes()).await?;
        proxy_stream.flush().await?;

        let mut read_buffer = [0; 2048];

        let bytes_read = proxy_stream.read(&mut read_buffer).await?;
        let data = &read_buffer[..bytes_read];


        let result = if data.starts_with(b"HTTP/1.1 407") {
            println!("Received proxy 407, negotiating Kerberos");
            let kerberos_token = get_negotiate_token(&proxy_without_port)?;
            proxy_stream.write_all(format!("CONNECT {} HTTP/1.1\r\nHost: {}\r\nAuthorization: Negotiate {}\r\n\r\n", &target_host, &target_host, &kerberos_token).as_bytes()).await?;
            proxy_stream.flush().await?;

            //let bytes_read = proxy_stream.read(&mut read_buffer).await?;
            //let data = &read_buffer[..bytes_read];

            //if data.starts_with(b"HTTP/1.1 2") {
            Ok(proxy_stream)
            //} else {
            //    Err(anyhow!("Proxy Negotiation failed: {}", String::from_utf8_lossy(&data)  ))
            //}
        } else if data.starts_with(b"HTTP/1.1 200") {
            Ok(proxy_stream)
        } else {
            Err(anyhow!("Received Error from proxy"))
        };

        result
    }

fn parse_host_from_request(data: &[u8]) -> Result<(RequestType, String), anyhow::Error> {
    if data.starts_with(b"CONNECT ") {
        let connect_body = String::from_utf8_lossy(&data);
        let mut split = connect_body.split_whitespace();
        let host = split.nth(1).unwrap().to_owned();
        Ok((RequestType::Connect, host))
    } else {
        let mut headers = [httparse::EMPTY_HEADER; 16];
        let mut req = httparse::Request::new(&mut headers);
        let res = req.parse(data)?;

        let path = req.path.unwrap();

        let default_port = {
            if path.starts_with("https") {
                "443".to_owned()
            } else {
                "80".to_owned()
            }
        };

        let mut host = req
            .headers
            .iter()
            .find_map(|header| {
                if header.name == "Host" {
                    Some(String::from_utf8_lossy(header.value).into_owned())
                } else {
                    None
                }
            })
            .ok_or_else(|| anyhow::anyhow!("No host header found"))?;

        if !host.contains(":") {
            host = format!("{}:{}", host, default_port)
        };

        Ok((RequestType::Other, host))
    }
}

async fn connect_with_retry(host: &str) -> Result<TcpStream, io::Error> {
    (|| async { TcpStream::connect(&host).await }).retry(&ExponentialBuilder::default()
        .with_min_delay(Duration::from_millis(500))
        .with_max_delay(Duration::from_secs(5))
        .with_max_times(5)).await
}

enum RequestType {
    Connect,
    Other,
}

#[derive(Clone)]
enum ConnectionState {
    Initializing,
    Forwarding(String)
}
