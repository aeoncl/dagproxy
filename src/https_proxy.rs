//
// struct TlsProxyTunnel {
//     source_socket: TcpStream,
//     upstream_proxy_host: String,
//     upstream_proxy_port: u32,
//     no_proxy: Vec<String>,
//     network_watcher: NetworkWatchHandle,
//     dest_socket: Option<TcpStream>,
//     state: ConnectionState,
// }
//
// impl TlsProxyTunnel {
//     pub fn new(source_socket: TcpStream, upstream_proxy_host: String, upstream_proxy_port: u32, no_proxy: Vec<String>, network_watcher: NetworkWatchHandle) -> Self {
//         Self {
//             source_socket,
//             upstream_proxy_host,
//             upstream_proxy_port,
//             no_proxy,
//             network_watcher,
//             dest_socket: None,
//             state: ConnectionState::Initializing
//         }
//     }
//
//     pub async fn start(&mut self) -> Result<(), anyhow::Error> {
//
//         println!("Created TLS Proxy Tunnel");
//
//         let mut acceptor_stream = self.source_socket.try_clone();
//         let mut source_socket = self.source_socket.fork();
//         let mut acceptor = Acceptor::default();
//
//         let accepted = loop {
//             acceptor.read_tls(&mut acceptor_stream).unwrap();
//             if let Some(accepted) = acceptor.accept().unwrap() {
//                 break accepted;
//             }
//         };
//
//         let hello = accepted.client_hello();
//         let server_name = hello.server_name();
//         if let None = server_name {
//             println!("Error: received ClientHello without server name, closing tunnel.");
//             return;
//         }
//         let server_name = server_name.expect("to be here");
//         self.initialize(server_name.to_string()).await?;
//
//         let mut network_update_receiver = self.network_watcher.subscribe();
//
//         loop {
//             let mut source_read_buffer = [0; 2048];
//             let mut dest_read_buffer = [0; 2048];
//
//             tokio::select! {
//                 network_update = network_update_receiver.changed() => {
//                     if let Err(e) = network_update {
//                         println!("Error receiving network updates: {}", e);
//                         break;
//                     }
//
//                     if let ConnectionState::Forwarding(target_host) = self.state.clone() {
//                     let network_type = network_update_receiver.borrow_and_update().clone();
//                         if let Err(e) = self.setup_dest_socket(network_type, &target_host).await {
//                             println!("Error switching connection: {}", e);
//                             break;
//                         }
//                     }
//                 },
//                 from_destination = async { self.dest_socket.as_mut().expect("to be here").read(&mut dest_read_buffer).await }, if self.dest_socket.is_some() => {
//                     if let Err(err) = from_destination {
//                         println!("Error reading from destination: {}", err);
//                         break;
//                     }
//
//                     let bytes_read = from_destination.expect("to be here");
//
//                     if bytes_read == 0 {
//                         break;
//                     }
//
//                     let data = &dest_read_buffer[..bytes_read];
//                     self.source_socket.write_all(&data).await.unwrap();
//
//                 },
//                 from_source = source_socket.read(&mut source_read_buffer) => {
//                     if let Err(err) = from_source {
//                         println!("Error reading from source: {}", err);
//                         break;
//                     }
//
//                     let bytes_read = from_source.expect("to be here");
//
//                     if bytes_read == 0 {
//                         break;
//                     }
//
//                     let data = &source_read_buffer[..bytes_read];
//                     self.dest_socket.write_all(&data).await.unwrap();
//                 }
//             }
//
//         }
//     }
//
//     async fn initialize(&mut self, server_name: String) -> Result<(), anyhow::Error> {
//         let network_type = self.network_watcher.network_type();
//         self.setup_dest_socket(network_type, &format!("{}:443", &server_name)).await?;
//         self.state = ConnectionState::Forwarding(server_name);
//         Ok(())
//
//     }
//
//     async fn setup_dest_socket(&mut self, updated_type: NetworkType, target_host: &str) -> Result<(), anyhow::Error> {
//         let no_proxy = self.no_proxy.iter().any(
//             |no_proxy_host| target_host.contains(no_proxy_host)
//         );
//
//         if no_proxy && self.dest_socket.is_none() {
//             println!("Host is no_proxy, setup direct connection to: {}", &target_host);
//             self.dest_socket =  Some(connect_with_retry(&target_host).await?);
//         }
//
//         match updated_type {
//             NetworkType::Direct => {
//                 println!("Setup direct connection to: {}", &target_host);
//                 self.dest_socket =  Some(connect_with_retry(&target_host).await?);
//                 Ok(())
//             },
//             NetworkType::Proxied => {
//                 println!("Setup proxied connection to: {}", &target_host);
//                 self.dest_socket = Some(connect_to_proxy(&format!("{}:{}", &self.upstream_proxy_host, &self.upstream_proxy_port), &target_host).await?);
//                 Ok(())
//             }
//         }
//     }
//
//
//
// }
//