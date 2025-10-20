use crate::network_watcher::{NetworkType, NetworkWatchHandle};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use crate::http::{connect_to_proxy, connect_with_retry, parse_host_from_request, RequestType, SUCCESS_CONNECT_RESPONSE};

pub struct HttpProxy {
    upstream_proxy_host: String,
    upstream_proxy_port: u32,
    no_proxy: Vec<String>,
    network_watcher: NetworkWatchHandle,
}

impl HttpProxy {

    pub fn new(upstream_proxy_host: String, upstream_proxy_port:u32, no_proxy: Vec<String>, network_watcher: NetworkWatchHandle) -> Self {
        Self {
            upstream_proxy_host,
            upstream_proxy_port,
            no_proxy,
            network_watcher
        }
    }

    pub async fn start(&mut self, host: String, port: u32) -> Result<(), anyhow::Error> {

        let listener = TcpListener::bind(format!("{}:{}", &host, &port))
            .await?;

        println!("ℹ️ HTTP Proxy listening on {}:{}", &host, &port);
        println!("ℹ️ Upstream Proxy: {}", format!("{}:{}", &self.upstream_proxy_host, &self.upstream_proxy_port));
        if !self.no_proxy.is_empty() {
            println!("ℹ️ No Proxy Hosts: {}", &self.no_proxy.join(", "));
        }

        loop {

            match listener.accept().await {
                Ok((source_socket, _)) => {

                    let upstream_proxy_host = self.upstream_proxy_host.clone();
                    let upstream_proxy_port = self.upstream_proxy_port.clone();
                    let network_watcher = self.network_watcher.clone();
                    let no_proxy = self.no_proxy.clone();

                    let _ = tokio::spawn(async move {
                        let mut proxy_tunnel = ProxyTunnel::new(source_socket, upstream_proxy_host, upstream_proxy_port, no_proxy, network_watcher);
                        proxy_tunnel.start().await;
                    });
                }
                Err(err) => {
                    println!("An error has occured accepting incoming connection: {}", err);
                }
            }

        };

    }


}

struct ProxyTunnel {
    source_socket: TcpStream,
    upstream_proxy_host: String,
    upstream_proxy_port: u32,
    no_proxy: Vec<String>,
    network_watcher: NetworkWatchHandle,
    dest_socket: Option<TcpStream>,
    state: ConnectionState,
}

impl ProxyTunnel {
    pub fn new(source_socket: TcpStream, upstream_proxy_host: String, upstream_proxy_port: u32, no_proxy: Vec<String>, network_watcher: NetworkWatchHandle) -> Self {
        Self {
            source_socket,
            upstream_proxy_host,
            upstream_proxy_port,
            no_proxy,
            network_watcher,
            dest_socket: None,
            state: ConnectionState::Initializing
        }
    }

    pub async fn start(&mut self) {

        let mut network_update_receiver = self.network_watcher.subscribe();

        loop {

            let mut source_read_buffer = [0; 2048];
            let mut dest_read_buffer = [0; 2048];

            tokio::select! {
                network_update = network_update_receiver.changed() => {
                    if let Err(e) = network_update {
                        println!("Error receiving network updates: {}", e);
                        break;
                    }

                    if let ConnectionState::Forwarding(target_host) = self.state.clone() {
                    let network_type = network_update_receiver.borrow_and_update().clone();
                        if let Err(e) = self.setup_dest_socket(network_type, &target_host).await {
                            println!("Error switching connection: {}", e);
                            break;
                        }
                    }
                },
                from_destination = async { self.dest_socket.as_mut().expect("to be here").read(&mut dest_read_buffer).await }, if self.dest_socket.is_some() => {
                    if let Err(err) = from_destination {
                        println!("Error reading from destination: {}", err);
                        break;
                    }

                    let bytes_read = from_destination.expect("to be here");

                    if bytes_read == 0 {
                        break;
                    }

                    let data = &dest_read_buffer[..bytes_read];
                    self.source_socket.write_all(&data).await.unwrap();

                },
                from_source = self.source_socket.read(&mut source_read_buffer) => {
                    if let Err(err) = from_source {
                        println!("Error reading from source: {}", err);
                        break;
                    }

                    let bytes_read = from_source.expect("to be here");

                    if bytes_read == 0 {
                        break;
                    }

                    let data = &source_read_buffer[..bytes_read];
                    if let Err(err) = self.on_message_from_source(data).await {
                        println!("Error processing message from source: {}", err);
                        break;
                    }

                }
            }
        }
    }

    async fn on_message_from_source(&mut self, data: &[u8]) -> Result<(), anyhow::Error> {
        match &self.state {
            ConnectionState::Initializing => {
                self.initialize(data).await?;
                Ok(())
            }
            ConnectionState::Forwarding(target_host) => {
                self.dest_socket.as_mut().expect("to be here").write_all(&data).await?;
                Ok(())
            }
        }
    }

    async fn initialize(&mut self, data: &[u8]) -> Result<(), anyhow::Error> {
        let (request_type, target_host) = parse_host_from_request(&data)?;
        let network_type = self.network_watcher.network_type();
        self.setup_dest_socket(network_type, &target_host).await?;
        self.state = ConnectionState::Forwarding(target_host);

        match request_type {
            RequestType::Connect => {
                self.source_socket.write_all(SUCCESS_CONNECT_RESPONSE).await?;
            }
            RequestType::Other => {
                self.dest_socket.as_mut().expect("to be here").write_all(data).await?;
            }
        }

        Ok(())
    }

    async fn setup_dest_socket(&mut self, updated_type: NetworkType, target_host: &str) -> Result<(), anyhow::Error> {
        let no_proxy = self.no_proxy.iter().any(
            |no_proxy_host| target_host.contains(no_proxy_host)
        );

        if no_proxy {
            if self.dest_socket.is_none() {
                println!("💻 -> {} [NO_PROXY]", &target_host);
                self.dest_socket =  Some(connect_with_retry(&target_host).await?);
            }
            return Ok(());
        }

        match updated_type {
            NetworkType::Direct => {
                println!("💻 -> {}", &target_host);
                self.dest_socket =  Some(connect_with_retry(&target_host).await?);
                Ok(())
            },
            NetworkType::Proxied => {
                let proxy_uri = &format!("{}:{}", &self.upstream_proxy_host, &self.upstream_proxy_port);
                println!("💻 -> {} -> {}", &proxy_uri ,&target_host);
                self.dest_socket = Some(connect_to_proxy(proxy_uri, &target_host).await?);
                Ok(())
            }
        }
    }
}

#[derive(Clone)]
#[derive(PartialEq)]
enum ConnectionState {
    Initializing,
    Forwarding(String)
}