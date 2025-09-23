mod network;

use netaddr2::{Contains, Netv4Addr};
use std::env;
use std::error::Error;
use std::str::{from_utf8, FromStr};
use std::thread::sleep;
use std::time::Duration;
use bytes::{Bytes, BytesMut};
use tokio::net::{TcpListener, TcpSocket, TcpStream};

use http_body_util::BodyExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::runtime;
use tokio::sync::mpsc::Sender;

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
            let (mut tcp_stream, _) = listener.accept().await.unwrap();

            let (source_sender, mut source_receiver) = tokio::sync::mpsc::channel::<Bytes>(200);

            let network_handle_clone = network_handle.clone();
            let upstream_proxy_clone = upstream_proxy.clone().unwrap();

            // state: dest unknown
            //HTTP CONNECT url
            //
            // dest
            //


            let _ = tokio::spawn(async move {
                let mut connection_state = ConnectionState::Initializing;

                //println!("New connection");

                loop {

                    let mut source_read_buffer = [0; 1024];

                    tokio::select! {
                        from_destination = source_receiver.recv() => {
                            if let Some(data) = from_destination {
                                //println!("Sending data to source");
                                tcp_stream.write_all(&data).await.unwrap();

                            }
                        }
                        from_source = tcp_stream.read(&mut source_read_buffer) => {
                            if let Ok(bytes_read) = from_source {

                                if bytes_read == 0 {
                                    break;
                                }

                                let data = &source_read_buffer[..bytes_read];
                                match &mut connection_state {
                                    ConnectionState::Initializing => {
                                            //                            println!("{}", from_utf8(&data).unwrap());

                                        //HTTP 1.1 CONNECT HEADER
                                        if data.starts_with(b"CONNECT ") {
                                            let connect_body = String::from_utf8_lossy(&data[0..bytes_read]);

                                            let mut split = connect_body.split_whitespace();
                                            let url = split.nth(1).unwrap().to_owned();
                                            let http_version = split.nth(0).unwrap();

                                            println!("Connecting to {} with {}", url, http_version);

                                            let source_sender_clone = source_sender.clone();
                                            let (destination_sender, mut destination_receiver) = tokio::sync::mpsc::channel::<Bytes>(200);
                                            let url_clone = url.clone();

                                            tokio::spawn(async move {
                                                let mut forward_socket = TcpStream::connect(url_clone).await.unwrap();


                                                let mut dest_read_buffer = [0; 1024];
                                                loop {

                                                    tokio::select! {
                                                        from_destination = forward_socket.read(&mut dest_read_buffer) => {
                                                             if let Ok(bytes_read) = from_destination {

                                                                if bytes_read == 0 {
                                                                    break;
                                                                }

                                                                let data = &dest_read_buffer[..bytes_read];
                                                                match source_sender_clone.send(Bytes::copy_from_slice(&data)).await {
                                                                    Ok(_) => {},
                                                                    Err(e)  => {
                                                                        println!("Error sending data to source: {}", e);
                                                                    }
                                                                }
                                                            }
                                                        }
                                                        to_destination = destination_receiver.recv() => {
                                                            if let Some(data) = to_destination {
                                                                forward_socket.write_all(&data).await.unwrap();
                                                            }
                                                        }

                                                    }



                                                }
                                            });



                                            source_sender.send(Bytes::from_static(b"CONNECT")).await.unwrap();
                                            connection_state =
                                                ConnectionState::Forwarding(url, destination_sender);
                                        } else {
                                            //Unexpected data received. //TODO close socket.
                                            println!(
                                                "Unexpected data received: {}",
                                                String::from_utf8_lossy(&data)
                                            );
                                        }
                                    }
                                    ConnectionState::Forwarding(target, dest_sender) => {
                                        match dest_sender.send(Bytes::copy_from_slice(data)).await {
                                            Ok(_) => {}
                                            Err(e) => {
                                                println!("Error sending data to destination: {}", e);
                                            }
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

enum ConnectionState {
    Initializing,
    Forwarding(String, Sender<Bytes>),
}
