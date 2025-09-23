mod network;

use netaddr2::{Contains, Netv4Addr};
use std::{env, io};
use std::error::Error;
use std::io::ErrorKind;
use std::net::ToSocketAddrs;
use std::str::{from_utf8, FromStr};
use std::thread::sleep;
use std::time::Duration;
use bytes::{Bytes, BytesMut};
use tokio::net::{TcpListener, TcpSocket, TcpStream};

use http_body_util::BodyExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::runtime;
use tokio::sync::mpsc::Sender;
use crate::network::NetworkType;

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
        println!("Starting DagProxy");

        let network_handle = network::watch_networks(corporate_subnets);

        let listener = TcpListener::bind(format!("127.0.0.1:{}", port))
            .await
            .unwrap();

        loop {
            let (mut source_socket, _) = listener.accept().await.unwrap();

            let mut dest_socket: Option<TcpStream> = Option::None;


            let (source_sender, mut source_receiver) = tokio::sync::mpsc::channel::<Bytes>(200);

            let network_handle_clone = network_handle.clone();
            let upstream_proxy_clone = upstream_proxy.clone().unwrap();

            let mut network_updates_receiver = network_handle_clone.subscribe();

            //drop initial value
            network_updates_receiver.mark_unchanged();
            // state: dest unknown
            //HTTP CONNECT url
            //
            // dest
            //
            let _ = tokio::spawn(async move {
                let mut connection_state = ConnectionState::Initializing;

                //println!("New connection");

                loop {

                    let mut source_read_buffer = [0; 2048];
                    let mut dest_read_buffer = [0; 2048];




                    tokio::select! {

                       network_update = network_updates_receiver.changed() => {
                            if let Ok(_) = network_update {
                               let test = network_updates_receiver.borrow_and_update().clone();
                                println!("Network has changed, closing connection.");
                                match test {
                                    NetworkType::Direct => {
                                        break;
                                    },
                                    NetworkType::Proxied => {
                                        break;
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
                                    break;
                                }

                                let data = &source_read_buffer[..bytes_read];
                                match &mut connection_state {
                                    ConnectionState::Initializing => {
                                            //                            println!("{}", from_utf8(&data).unwrap());

                                        match parse_host(&data)  {
                                            Ok(RequestType::Connect(host)) => {

                                                match network_handle_clone.network_type() {
                                                    NetworkType::Direct => {
                                                        println!("Connecting to {}", &host);
                                                        dest_socket = Some(TcpStream::connect(&host).await.unwrap());
                                                        source_socket.write_all(b"CONNECT").await.unwrap();
                                                    },
                                                    NetworkType::Proxied => {
                                                        println!("Connecting to {} -> {}", &upstream_proxy_clone, &host);
                                                        let mut socket = TcpStream::connect(&upstream_proxy_clone).await.unwrap();
                                                        socket.write_all(data).await.unwrap();
                                                        dest_socket = Some(socket);
                                                    }
                                                }

                                                connection_state = ConnectionState::Forwarding(host);
                                            },
                                            Ok(RequestType::Other(host)) => {

                                              match network_handle_clone.network_type() {
                                                    NetworkType::Direct => {
                                                        println!("Connecting to {}", &host);
                                                        let mut socket = TcpStream::connect(&host).await.unwrap();
                                                        socket.write_all(data).await.unwrap();
                                                        dest_socket = Some(socket);                                                    },
                                                    NetworkType::Proxied => {
                                                        println!("Connecting to {} -> {}", &upstream_proxy_clone, &host);
                                                        let mut socket = TcpStream::connect(&upstream_proxy_clone).await.unwrap();
                                                        socket.write_all(data).await.unwrap();
                                                        dest_socket = Some(socket);
                                                    }
                                                }
                                            }
                                            _ => {
                                                println!(
                                                "Unexpected data received: {}",
                                                String::from_utf8_lossy(&data)
                                            );
                                            }
                                        }
                                    }

                                    ConnectionState::Forwarding(target) => {
                                        dest_socket.as_mut().unwrap().write_all(&data).await.unwrap();
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


fn parse_host(data: &[u8]) -> Result<RequestType, anyhow::Error> {

    if data.starts_with(b"CONNECT ") {
        let connect_body = String::from_utf8_lossy(&data);
        let mut split = connect_body.split_whitespace();
        let host = split.nth(1).unwrap().to_owned();
        Ok(RequestType::Connect(host))
    } else {

        let mut headers = [httparse::EMPTY_HEADER; 16];
        let mut req = httparse::Request::new(&mut headers);
        let res = req.parse(data)?;

        let path = req.path.unwrap();

        let default_port = { if path.starts_with("https") {
                "443".to_owned()
            } else {
                "80".to_owned()
            }

        };


        let mut host = req.headers.iter()
            .find_map(|header| { if header.name == "Host" { Some(String::from_utf8_lossy(header.value).into_owned()) } else {None} })
            .ok_or_else(|| anyhow::anyhow!("No host header found"))?;

        if !host.contains(":") {
            host = format!("{}:{}", host, default_port)
        };

        Ok(RequestType::Other(host))

    }
}

enum RequestType {
    Connect(String),
    Other(String)
}


enum ConnectionState {
    Initializing,
    Forwarding(String),
}
