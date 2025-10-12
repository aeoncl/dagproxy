use anyhow::anyhow;
use backon::ExponentialBuilder;
use backon::Retryable;
use bytes::Buf;
use std::io;
use std::time::Duration;
use tokio::net::TcpStream;

use crate::kerberos::kerberos::negotiate_with_krb5;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub(crate) const SUCCESS_CONNECT_RESPONSE: &[u8] = b"HTTP/1.1 200 Connection established\r\n\r\n";
const TLS_HANDSHAKE_RECORD_TYPE: &[u8; 1] = &[0x16];

pub(crate) enum RequestType {
    Connect,
    Other,
}


pub(crate) fn parse_host_from_tls_client_hello(data: &[u8]) -> Result<(RequestType, String), anyhow::Error> {
    let mut parser = tls_client_hello_parser::Parser::new();
    let owned = data.to_vec();
    let mut reader = Box::new(owned.reader());
    let client_hello_payload = parser.parse(&mut reader)?
        .ok_or(anyhow!("Could not parse TLS Client Hello"))?;
    let client_hello = client_hello_payload.client_hello()?;
    let host_name = client_hello
        .server_name()
        .ok_or(anyhow!("Could not find Server Name in TLS Client Hello"))?;

    Ok((RequestType::Other, format!("{}:{}", host_name, 443)))
}

fn parse_host_from_http_request(data: &[u8]) -> Result<(RequestType, String), anyhow::Error> {
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

pub(crate) fn parse_host_from_request(data: &[u8]) -> Result<(RequestType, String), anyhow::Error> {
    if data.starts_with(b"CONNECT ") {
        let connect_body = String::from_utf8_lossy(&data);
        let mut split = connect_body.split_whitespace();
        let host = split.nth(1).unwrap().to_owned();
        Ok((RequestType::Connect, host))
    } else if data.starts_with(TLS_HANDSHAKE_RECORD_TYPE) {
        parse_host_from_tls_client_hello(&data)
    } else {
        parse_host_from_http_request(&data)
    }
}


pub(crate) async fn connect_with_retry(host: &str) -> Result<TcpStream, io::Error> {
    (|| async { TcpStream::connect(&host).await }).retry(&ExponentialBuilder::default()
        .with_min_delay(Duration::from_millis(500))
        .with_max_delay(Duration::from_secs(5))
        .with_max_times(5)).await
}


pub(crate) async fn connect_to_proxy(proxy_host: &str, target_host: &str) -> Result<TcpStream, anyhow::Error> {

    let mut proxy_stream = connect_with_retry(proxy_host).await?;
    proxy_stream.write_all(format!("CONNECT {} HTTP/1.1\r\nHost: {}\r\n\r\n", &target_host, &target_host).as_bytes()).await?;
    proxy_stream.flush().await?;

    let mut read_buffer = [0; 2048];

    let bytes_read = proxy_stream.read(&mut read_buffer).await?;
    let data = &read_buffer[..bytes_read];


    let result = if data.starts_with(b"HTTP/1.1 407") {
        println!("Received proxy 407, negotiating Kerberos");
        drop(proxy_stream);
        negotiate_with_krb5(&proxy_host).await?;

        let mut proxy_stream = connect_with_retry(proxy_host).await?;
        proxy_stream.write_all(format!("CONNECT {} HTTP/1.1\r\nHost: {}\r\n\r\n", &target_host, &target_host).as_bytes()).await?;
        proxy_stream.flush().await?;

        Ok(proxy_stream)

    } else if data.starts_with(b"HTTP/1.1 2") {
        Ok(proxy_stream)
    } else {
        if (bytes_read != 0) {
            Err(anyhow!("Received Error from proxy: {}", String::from_utf8_lossy(&data) ))
        } else {
            Err(anyhow!("Proxy closed connection for target_host: {}", &target_host))
        }
    };

    result
}

#[cfg(test)]
mod tests {
    use base64::Engine;
    use base64::engine::general_purpose;
    use crate::http::{parse_host_from_request, RequestType};

    const sample_hello: &[u8; 692] = b"FgMBAgABAAH8AwN3t6WJKcsKcWo+roqQX7Nuc8SYCUAKTIkINuDoJm4ooiDRiC2236q0JY/NewWV9KcViEzk7S03gwwUSioSOKbOcAAkEwETAxMCwCvAL8ypzKjALMAwwArACcATwBQAMwA5AC8ANQAKAQABjwAAAA4ADAAACWxvY2FsaG9zdAAXAAD/AQABAAAKAA4ADAAdABcAGAAZAQABAQALAAIBAAAjAAAAEAAOAAwCaDIIaHR0cC8xLjEABQAFAQAAAAAAMwBrAGkAHQAgcqzbr+1AYblh6qcR+qvjokWhIpbChkaqpXuDY9uHhVoAFwBBBAq/uAsPt0n3lc9MGArs6RqLoQE+1eWkstNR0zPjxlQcqGSD+1mKyvSCGEwU0DCZAEFEvhnj5YxSyqcAFODwnp4AKwAJCAMEAwMDAgMBAA0AGAAWBAMFAwYDCAQIBQgGBAEFAQYBAgMCAQAtAAIBAQAcAAJAAQAVAJUAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA==";

    #[test]
    pub fn test_tls_client_hello_parser() {

        let client_hello = general_purpose::STANDARD.decode(&sample_hello).unwrap();

        let (req_type, hostname) = parse_host_from_request(&client_hello).unwrap();

        assert!(matches!(req_type, RequestType::Other));
        assert_eq!(hostname, "localhost:443");
    }

}