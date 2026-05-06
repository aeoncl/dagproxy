use crate::config::{ForwardPolicy, SubnetConfig};
use crate::http::{
    NetworkTarget, RequestType, SUCCESS_CONNECT_RESPONSE, connect_to_proxy, connect_with_retry,
    parse_host_from_request,
};
use crate::network_watcher::NetworkWatchTransport;
use crate::pac::PacMatcher;
use anyhow::anyhow;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch::Receiver;

pub struct HttpProxy {
    network_watcher: NetworkWatchTransport,
    pac_update_recv: Receiver<Option<PacMatcher>>,
}

impl HttpProxy {
    pub fn new(
        network_watcher: NetworkWatchTransport,
        pac_update_recv: Receiver<Option<PacMatcher>>,
    ) -> Self {
        Self {
            network_watcher,
            pac_update_recv,
        }
    }

    pub async fn start(&mut self, host: String, port: u32) -> Result<(), anyhow::Error> {
        let listener = TcpListener::bind(format!("{}:{}", &host, &port)).await?;
        let mut pac_matcher = self.pac_update_recv.borrow().clone();
        println!("👂 HTTP Proxy listening on {}:{}", &host, &port);

        loop {
            tokio::select! {
               accepted = listener.accept() => {
                   match accepted {
                       Ok((source_socket, _)) => {
                           let network_watcher = self.network_watcher.clone();

                           let pac_matcher_clone = pac_matcher.clone();
                           let _ = tokio::spawn(async move {
                               let mut proxy_tunnel = ProxyTunnel::new(source_socket, network_watcher, pac_matcher_clone);
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
               },
               pac_update = self.pac_update_recv.changed() => {
                   if let Err(e) = pac_update {
                       eprintln!("Error receiving PAC updates: {}", e);
                       break;
                   }

                   let new_pac_matcher = self.pac_update_recv.borrow_and_update().clone();
                   pac_matcher = new_pac_matcher;
               }
            }
        }
        Ok(())
    }
}

struct ProxyTunnel {
    source_socket: TcpStream,
    network_watcher: NetworkWatchTransport,
    dest_socket: Option<TcpStream>,
    state: ConnectionState,
    pac_matcher: Option<PacMatcher>,
}

impl ProxyTunnel {
    pub fn new(
        source_socket: TcpStream,
        network_watcher: NetworkWatchTransport,
        pac_matcher: Option<PacMatcher>,
    ) -> Self {
        Self {
            source_socket,
            network_watcher,
            dest_socket: None,
            state: ConnectionState::Initializing,
            pac_matcher,
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
                        eprintln!("Error receiving network updates: {}", e);
                        break;
                    }

                    if let ConnectionState::Forwarding(target_host) = self.state.clone() {
                    let network_type = network_update_receiver.borrow_and_update().clone();
                        if let Err(e) = self.setup_dest_socket(network_type, &target_host).await {
                            eprintln!("Error switching connection: {}", e);
                            break;
                        }
                    }
                },
                from_destination = async { self.dest_socket.as_mut().expect("to be here").read(&mut dest_read_buffer).await }, if self.dest_socket.is_some() => {
                    if let Err(err) = from_destination {
                        eprintln!("Error reading from destination: {}", err);
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
                        eprintln!("Error reading from source: {}", err);
                        break;
                    }

                    let bytes_read = from_source.expect("to be here");

                    if bytes_read == 0 {
                        break;
                    }

                    let data = &source_read_buffer[..bytes_read];
                    if let Err(err) = self.on_message_from_source(data).await {
                        eprintln!("Error processing message from source: {}", err);
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
        self.setup_dest_socket(network_type, &target_host).await?;
        self.state = ConnectionState::Forwarding(target_host);

        match request_type {
            RequestType::Connect => {
                self.source_socket
                    .write_all(SUCCESS_CONNECT_RESPONSE)
                    .await?;
            }
            RequestType::Other => {
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
        target_host: &NetworkTarget,
    ) -> Result<(), anyhow::Error> {
        let forward_policy = subnet_config
            .get_forward_policy(target_host.host())
            .unwrap_or_default();

        let effective_policy = match forward_policy {
            ForwardPolicy::PAC => match self.pac_matcher.as_ref() {
                Some(matcher) => matcher.resolve_forward_policy(target_host)?,
                None => {
                    return Err(anyhow::anyhow!(
                        "Could not resolve PAC policy for: {:?} because PAC file was not resolved",
                        &target_host
                    ));
                }
            },
            other => other,
        };

        match effective_policy {
            ForwardPolicy::Proxy => {
                let proxy = NetworkTarget::new(&subnet_config.proxy_host, subnet_config.proxy_port);
                println!("💻 -> {} -> {}", &proxy, &target_host);
                self.dest_socket = Some(connect_to_proxy(&proxy, target_host).await?);
                Ok(())
            }
            ForwardPolicy::Direct => {
                println!("💻 -> {}", &target_host);
                self.dest_socket = Some(connect_with_retry(target_host).await?);
                Ok(())
            }
            ForwardPolicy::PAC => Err(anyhow!("Should never happen")),
        }
    }
}

#[derive(Clone, PartialEq)]
enum ConnectionState {
    Initializing,
    Forwarding(NetworkTarget),
}
