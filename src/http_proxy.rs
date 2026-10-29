use crate::config::{ForwardPolicy, SubnetConfig};
use crate::http::{
    NetworkTarget, RequestType, SUCCESS_CONNECT_RESPONSE, connect_to_proxy, connect_with_retry,
    http_407_response, parse_host_from_request,
};
use crate::network_watcher::NetworkWatchTransport;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

pub struct HttpProxy {
    network_watcher: NetworkWatchTransport,
}

impl HttpProxy {
    pub fn new(network_watcher: NetworkWatchTransport) -> Self {
        Self { network_watcher }
    }

    pub async fn start(&mut self, host: String, port: u32) -> Result<(), anyhow::Error> {
        let listener = TcpListener::bind(format!("{}:{}", &host, &port)).await?;

        println!("👂 HTTP Proxy listening on {}:{}", &host, &port);

        loop {
            match listener.accept().await {
                Ok((source_socket, _)) => {
                    let network_watcher = self.network_watcher.clone();

                    let _ = tokio::spawn(async move {
                        let mut proxy_tunnel = ProxyTunnel::new(source_socket, network_watcher);
                        proxy_tunnel.start().await;
                    });
                }
                Err(err) => {
                    eprintln!(
                        "An error has occurred accepting incoming connection: {}",
                        err
                    );
                }
            }
        }
    }
}

struct ProxyTunnel {
    id: u32,
    source_socket: TcpStream,
    network_watcher: NetworkWatchTransport,
    dest_socket: Option<TcpStream>,
    state: ConnectionState
}

impl ProxyTunnel {
    pub fn new(source_socket: TcpStream, network_watcher: NetworkWatchTransport) -> Self {
        let id: u32 = rand::random();
        Self {
            id,
            source_socket,
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
                        eprintln!("💻 ({}) Error receiving network updates: {}", &self.id, e);
                        break;
                    }

                    if let ConnectionState::Forwarding(_target_host) = self.state.clone() {
                        println!("💻 ({}) Closing tunnel", &self.id);
                        break;
                    }
                },
                from_destination = async { self.dest_socket.as_mut().expect("to be here").read(&mut dest_read_buffer).await }, if self.dest_socket.is_some() => {
                    if let Err(err) = from_destination {
                        eprintln!("💻 ({}) Error reading from destination: {}", &self.id, err);
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
                        eprintln!("💻 ({}) Error reading from source: {}", &self.id, err);
                        break;
                    }

                    let bytes_read = from_source.expect("to be here");

                    if bytes_read == 0 {
                        break;
                    }

                    let data = &source_read_buffer[..bytes_read];
                    if let Err(err) = self.on_message_from_source(data).await {
                        eprintln!("💻 ({}) Error processing message from source: {}", &self.id, err);
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
            ConnectionState::Forwarding(_) => {
                self.dest_socket
                    .as_mut()
                    .expect("to be here")
                    .write_all(&data)
                    .await?;
                Ok(())
            }
        }
    }

    async fn initialize(&mut self, data: &[u8]) -> Result<(), anyhow::Error> {
        let (request_type, target_host) = parse_host_from_request(&data)?;
        let network_type = self.network_watcher.subnet_config();
        if let Err(e) = self
            .setup_dest_socket(network_type, &request_type, &target_host, data)
            .await
        {
            self.source_socket.write_all(&http_407_response(&e)).await?;
            return Err(e);
        }
        self.state = ConnectionState::Forwarding(target_host);
        Ok(())
    }

    async fn setup_dest_socket(
        &mut self,
        subnet_config: SubnetConfig,
        request_type: &RequestType,
        target_host: &NetworkTarget,
        request_bytes: &[u8],
    ) -> Result<(), anyhow::Error> {
        let forward_policy = subnet_config
            .get_forward_policy(target_host.host())
            .unwrap_or_default();

        match forward_policy {
            ForwardPolicy::Proxy => {
                let proxy = NetworkTarget::new(&subnet_config.proxy_host, subnet_config.proxy_port);
                println!("💻 ({}) -> {} -> {}", &self.id, &proxy, &target_host);
                self.dest_socket = Some(connect_to_proxy(&proxy, target_host, request_bytes).await?);
                Ok(())
            }
            ForwardPolicy::Direct => {
                println!("💻 ({}) -> {}", &self.id, &target_host);
                let mut dest_socket = connect_with_retry(target_host).await?;
                match request_type {
                    RequestType::Connect => {
                        self.source_socket.write_all(SUCCESS_CONNECT_RESPONSE).await?;
                    }
                    RequestType::Other => {
                        dest_socket.write_all(request_bytes).await?;
                    }
                }
                self.dest_socket = Some(dest_socket);
                Ok(())
            }
            ForwardPolicy::PAC => {
                todo!("PAC resolution is not yet supported")
            }
        }
    }
}

#[derive(Clone, PartialEq)]
enum ConnectionState {
    Initializing,
    Forwarding(NetworkTarget),
}
