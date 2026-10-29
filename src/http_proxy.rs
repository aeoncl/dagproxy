use crate::config::{ForwardPolicy, SubnetConfig};
use crate::http::{
    NetworkTarget, RequestType, SUCCESS_CONNECT_RESPONSE, connect_to_proxy, connect_with_retry,
    parse_host_from_request,
};
use crate::kerberos::kerberos::negotiate_with_krb5;
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
    state: ConnectionState,
    pending_replay: Option<PendingReplay>,
}

/// A plain-HTTP request sent to the upstream proxy, kept until the proxy's first response so it
/// can be resent after Kerberos negotiation if the proxy answers 407.
struct PendingReplay {
    proxy: NetworkTarget,
    request: Vec<u8>,
}

impl ProxyTunnel {
    pub fn new(source_socket: TcpStream, network_watcher: NetworkWatchTransport) -> Self {
        let id: u32 = rand::random();
        Self {
            id,
            source_socket,
            network_watcher,
            dest_socket: None,
            state: ConnectionState::Initializing,
            pending_replay: None,
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

                    if let Some(pending) = self.pending_replay.take() {
                        if data.starts_with(b"HTTP/1.1 407") {
                            match self.replay_after_negotiate(pending).await {
                                Ok(()) => continue,
                                Err(err) => {
                                    eprintln!("💻 ({}) Proxy negotiate failed, passing 407 to client: {}", &self.id, err);
                                }
                            }
                        }
                    }

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
                if let Some(pending) = self.pending_replay.as_mut() {
                    pending.request.extend_from_slice(data);
                }
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
        self.setup_dest_socket(network_type, &request_type, &target_host)
            .await?;
        self.state = ConnectionState::Forwarding(target_host);

        match request_type {
            RequestType::Connect => {
                self.source_socket
                    .write_all(SUCCESS_CONNECT_RESPONSE)
                    .await?;
            }
            RequestType::Other => {
                if let Some(pending) = self.pending_replay.as_mut() {
                    pending.request.extend_from_slice(data);
                }
                self.dest_socket
                    .as_mut()
                    .expect("to be here")
                    .write_all(data)
                    .await?;
            }
        }

        Ok(())
    }

    async fn setup_dest_socket(
        &mut self,
        subnet_config: SubnetConfig,
        request_type: &RequestType,
        target_host: &NetworkTarget,
    ) -> Result<(), anyhow::Error> {
        let forward_policy = subnet_config
            .get_forward_policy(target_host.host())
            .unwrap_or_default();

        match forward_policy {
            ForwardPolicy::Proxy => {
                let proxy = NetworkTarget::new(&subnet_config.proxy_host, subnet_config.proxy_port);
                println!("💻 ({}) -> {} -> {}", &self.id, &proxy, &target_host);
                match request_type {
                    RequestType::Connect => {
                        self.dest_socket = Some(connect_to_proxy(&proxy, target_host).await?);
                    }
                    // The client's request is already in absolute-URI form, so send it to the proxy
                    // as-is. Tunnelling it with CONNECT host:80 is rejected by some proxies
                    // (e.g. Cisco Umbrella answers 400 Bad Request).
                    RequestType::Other => {
                        self.dest_socket = Some(connect_with_retry(&proxy).await?);
                        self.pending_replay = Some(PendingReplay {
                            proxy,
                            request: Vec::new(),
                        });
                    }
                }
                Ok(())
            }
            ForwardPolicy::Direct => {
                println!("💻 ({}) -> {}", &self.id, &target_host);
                self.dest_socket = Some(connect_with_retry(target_host).await?);
                Ok(())
            }
            ForwardPolicy::PAC => {
                todo!("PAC resolution is not yet supported")
            }
        }
    }

    async fn replay_after_negotiate(&mut self, pending: PendingReplay) -> Result<(), anyhow::Error> {
        println!("🤝 ({}) Received proxy 407, negotiating Kerberos", &self.id);
        negotiate_with_krb5(&pending.proxy).await?;

        println!("🤝 ({}) Proxy negotiate successfull, resending request", &self.id);
        let mut proxy_stream = connect_with_retry(&pending.proxy).await?;
        proxy_stream.write_all(&pending.request).await?;
        self.dest_socket = Some(proxy_stream);
        Ok(())
    }
}

#[derive(Clone, PartialEq)]
enum ConnectionState {
    Initializing,
    Forwarding(NetworkTarget),
}
