mod network;

use std::arch::x86_64::_MM_FROUND_RAISE_EXC;
use std::convert::Infallible;
use crate::network::{NetworkType, NetworkWatchHandle};
use hyper_util::server::conn::auto::Builder;
use netaddr2::{Contains, Netv4Addr};
use std::env;
use std::error::Error;
use std::str::FromStr;
use hyper::body::{Bytes, Incoming};
use hyper::{body, Request, Response, Version};
use hyper::client::conn::http1;
use hyper::http::uri::Scheme;
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use tokio::net::{TcpListener, TcpStream};

use tokio::runtime;
use tokio::runtime::Handle;
use http_body_util::{BodyExt, Empty, Full};
use http_body_util::combinators::BoxBody;
use hyper_rustls::{HttpsConnector, HttpsConnectorBuilder};
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;

fn main() {
    let args: Vec<String> = env::args().collect();

    let upstream_proxy = args
        .windows(2)
        .find_map(|window| if window[0] == "--upstream-proxy" { Some(window[1].to_owned()) } else { None } );

    let port = args
        .windows(2)
        .find_map(|window| if window[0] == "--port" { Some(window[1].to_owned()) } else { None } )
        .unwrap_or("3232".into());

    let corporate_subnets = args
        .windows(2)
        .find_map(|window| if window[0] == "--corporate-subnets" {
            let subnets = window[1].split(",")
                    .map(|subnet| Netv4Addr::from_str(subnet).unwrap())
                    .collect::<Vec<_>>();
            Some(subnets)
        } else { None } );

    let corporate_subnets = corporate_subnets.unwrap();


    let rt = runtime::Builder::new_current_thread()
        .enable_all()
        .build().unwrap();

    rt.block_on(async move {
        println!("Starting DagProxy");

        let network_handle = network::watch_networks(corporate_subnets);


        let listener = TcpListener::bind(format!("127.0.0.1:{}", port)).await.unwrap();

        let https_connector = HttpsConnectorBuilder::new()
            .with_native_roots().unwrap()
            .https_or_http()
            .enable_http1()
            .build();


        let client: Client<_, Incoming> = Client::builder(TokioExecutor::new()).build(https_connector);

        loop {
            
            let (socket, _) = listener.accept().await.unwrap();
            let network_handle_clone = network_handle.clone();
            let upstream_proxy_clone = upstream_proxy.clone().unwrap();
            let client_clone = client.clone();

            // state: dest unknown
            //HTTP CONNECT url
            //
            // dest
            // 
            
            tokio::spawn(async move {
                handle_client(socket, &upstream_proxy_clone, network_handle_clone, client_clone).await;
            });

        }

    });




}

async fn handle_client(mut socket: TcpStream, upstream_proxy: &str, network_watch_handle: NetworkWatchHandle, client_clone: Client<HttpsConnector<HttpConnector>, Incoming>) {
    let socket = TokioIo::new(socket);

    let service = service_fn(move |request: Request<body::Incoming>|{
        let watch_handle = network_watch_handle.clone();
        let client_clone = client_clone.clone();
        async move {
        // Here you can access the request


        let response: Result<_, anyhow::Error> = match watch_handle.network_type() {
            NetworkType::Direct => {


                match send_request(request, "", client_clone).await {

                    Ok(response) => Ok(response),
                    Err(e) => {
                        println!("{}", e);
                        let error_response = Response::builder()
                            .status(500)
                            .body(Full::new(Bytes::from("Internal Server Error")) .map_err(|never: Infallible| -> hyper::Error { match never {} })
                                .boxed()
                            )
                            .unwrap();
                        Ok(error_response)

                    }
                }
            }
            NetworkType::Proxied => {
                //TODO kerberos

                //let proxy_stream = TcpStream::connect(upstream_proxy).await.unwrap();
                Ok(todo!())
            }
        };

        response
    }


    }

    );




    if let Err(err) = Builder::new(TokioExecutor::new())
        .serve_connection_with_upgrades(socket, service)
        .await
    {
        eprintln!("Error serving connection: {:?}", err);
    }








}

async fn send_request(request: Request<Incoming>, dest: &str, client_clone: Client<HttpsConnector<HttpConnector>, Incoming>) -> Result<Response<BoxBody<Bytes, hyper::Error>>, anyhow::Error> {
    let (mut parts, body) = request.into_parts();

    // Remove hop-by-hop headers from the request. These are meant for the current
    // connection only and should not be forwarded.
    parts.headers.remove(hyper::header::CONNECTION);
    parts.headers.remove(hyper::header::PROXY_AUTHENTICATE);
    parts.headers.remove(hyper::header::PROXY_AUTHORIZATION);
    parts.headers.remove(hyper::header::TE);
    parts.headers.remove(hyper::header::TRANSFER_ENCODING);
    parts.headers.remove(hyper::header::UPGRADE);
    // This is a non-standard but common header.
    parts.headers.remove("proxy-connection");

    // Re-assemble the request with the sanitized headers.
    let request = Request::from_parts(parts, body);

    let response = client_clone.request(request).await.expect("TODO: panic message");


    let (parts, body) = response.into_parts();


    // Return response with collected body
    Ok(Response::from_parts(parts, body.boxed()))
}





