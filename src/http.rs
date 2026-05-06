use std::fmt::{Display, Formatter};
use std::io;
use std::str::FromStr;
use std::time::Duration;
use anyhow::anyhow;
use backon::ExponentialBuilder;
use tokio::net::TcpStream;
use backon::Retryable;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use crate::kerberos::kerberos::negotiate_with_krb5;

pub(crate) const SUCCESS_CONNECT_RESPONSE: &[u8] = b"HTTP/1.1 200 Connection established\r\n\r\n";


pub(crate) enum RequestType {
    Connect,
    Other,
}

pub(crate) fn parse_host_from_request(data: &[u8]) -> Result<(RequestType, NetworkTarget), anyhow::Error> {
    if data.starts_with(b"CONNECT ") {
        let connect_body = String::from_utf8_lossy(&data);
        let mut split = connect_body.split_whitespace();
        let host = split.nth(1).unwrap().to_owned();
        Ok((RequestType::Connect, NetworkTarget::from_str(&host)?))
    } else {
        let mut headers = [httparse::EMPTY_HEADER; 16];
        let mut req = httparse::Request::new(&mut headers);
        req.parse(data)?;

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

        Ok((RequestType::Other, NetworkTarget::from_str(&host)?))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NetworkTarget {
    host: String,
    port: u32,
}

impl NetworkTarget {
    pub fn new(host: &str, port: u32) -> Self {
        Self {
            host: host.to_string(),
            port,
        }
    }

    pub fn host(&self) -> &str {
        self.host.as_str()
    }

    pub fn port(&self) -> u32 {
        self.port
    }
}

impl FromStr for NetworkTarget {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (host, port) = s.split_once(":").ok_or(anyhow::anyhow!("Could not split {{domain:port}} on ':' for : {}", s))?;
        let port_casted = u32::from_str(port)?;
        Ok(Self::new(host, port_casted))
    }
}

impl Display for NetworkTarget {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.host, self.port)
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
        println!("🤝 Received proxy 407, negotiating Kerberos");
        drop(proxy_stream);
        negotiate_with_krb5(&proxy_host).await?;
        
        println!("🤝 Proxy negotiate successfull");

        let mut proxy_stream = connect_with_retry(proxy_host).await?;
        proxy_stream.write_all(format!("CONNECT {} HTTP/1.1\r\nHost: {}\r\n\r\n", &target_host, &target_host).as_bytes()).await?;
        proxy_stream.flush().await?;

        Ok(proxy_stream)

    } else if data.starts_with(b"HTTP/1.1 2") {
        Ok(proxy_stream)
    } else {
        if bytes_read != 0 {
            Err(anyhow!("Received Error from proxy: {}", String::from_utf8_lossy(&data) ))
        } else {
            Err(anyhow!("Proxy closed connection for target_host: {}", &target_host))
        }
    };

    result
}