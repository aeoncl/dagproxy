use crate::kerberos::kerberos::negotiate_with_krb5;
use anyhow::anyhow;
use backon::ExponentialBuilder;
use backon::Retryable;
use std::fmt::{Display, Formatter};
use std::io;
use std::str::FromStr;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
pub(crate) enum RequestType {
    Connect,
    Other,
}

pub(crate) fn http_407_response(error: &anyhow::Error) -> Vec<u8> {
    let body = format!("{error:#}\n");
    format!(
        "HTTP/1.1 407 Proxy Authentication Required\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {body}",
        body.len()
    )
    .into_bytes()
}

pub(crate) fn parse_host_from_request(
    data: &[u8],
) -> Result<(RequestType, NetworkTarget), anyhow::Error> {
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
        let (host, port) = s.rsplit_once(":").ok_or(anyhow::anyhow!(
            "Could not split {{domain:port}} on ':' for : {}",
            s
        ))?;
        let port_casted = u32::from_str(port)?;
        Ok(Self::new(host, port_casted))
    }
}

impl Display for NetworkTarget {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.host, self.port)
    }
}

pub(crate) async fn connect_with_retry(target: &NetworkTarget) -> Result<TcpStream, io::Error> {
    (|| async { TcpStream::connect(target.to_string()).await })
        .retry(
            &ExponentialBuilder::default()
                .with_min_delay(Duration::from_millis(500))
                .with_max_delay(Duration::from_secs(5))
                .with_max_times(5),
        )
        .await
}

pub(crate) async fn connect_to_proxy(
    proxy: &NetworkTarget,
    target: &NetworkTarget,
    request_bytes: &[u8]
) -> Result<TcpStream, anyhow::Error> {
    let mut proxy_stream = connect_with_retry(proxy).await?;
    proxy_stream
        .write_all(request_bytes)
        .await?;
    proxy_stream.flush().await?;

    let mut read_buffer = [0; 2048];
    let mut bytes_read = proxy_stream.peek(&mut read_buffer).await?;

    let data = &read_buffer[..bytes_read];

    let result = if data.starts_with(b"HTTP/1.1 407") {
        println!("🤝 Received proxy 407, negotiating Kerberos");
        drop(proxy_stream);
        negotiate_with_krb5(proxy).await?;

        println!("🤝 Proxy negotiate successfull");

        let mut proxy_stream = connect_with_retry(proxy).await?;
        proxy_stream
            .write_all(request_bytes)
            .await?;
        proxy_stream.flush().await?;

        Ok(proxy_stream)
    } else {
        Ok(proxy_stream)
    };

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── NetworkTarget::from_str ─────────────────────────────────

    #[test]
    fn network_target_parses_host_and_port() {
        let target = NetworkTarget::from_str("example.com:443").unwrap();
        assert_eq!(target.host(), "example.com");
        assert_eq!(target.port(), 443);
    }

    #[test]
    fn network_target_parses_ipv4_address() {
        let target = NetworkTarget::from_str("10.0.0.1:8080").unwrap();
        assert_eq!(target.host(), "10.0.0.1");
        assert_eq!(target.port(), 8080);
    }

    #[test]
    fn network_target_parses_port_zero() {
        let target = NetworkTarget::from_str("host:0").unwrap();
        assert_eq!(target.host(), "host");
        assert_eq!(target.port(), 0);
    }

    #[test]
    fn network_target_rejects_no_port() {
        let result = NetworkTarget::from_str("example.com");
        assert!(result.is_err());
    }

    #[test]
    fn network_target_rejects_invalid_port() {
        let result = NetworkTarget::from_str("example.com:abc");
        assert!(result.is_err());
    }

    #[test]
    fn network_target_rejects_empty_port() {
        let result = NetworkTarget::from_str("example.com:");
        assert!(result.is_err());
    }

    #[test]
    fn network_target_rejects_empty_string() {
        let result = NetworkTarget::from_str("");
        assert!(result.is_err());
    }

    // ─── NetworkTarget Display roundtrip ─────────────────────────

    #[test]
    fn network_target_display_roundtrip() {
        let target = NetworkTarget::from_str("example.com:8080").unwrap();
        assert_eq!(target.to_string(), "example.com:8080");
    }

    #[test]
    fn network_target_new_equals_from_str() {
        let a = NetworkTarget::new("example.com", 443);
        let b = NetworkTarget::from_str("example.com:443").unwrap();
        assert_eq!(a, b);
    }

    // ─── parse_host_from_request — CONNECT ───────────────────────

    #[test]
    fn parse_connect_hostname() {
        let data = b"CONNECT example.com:443 HTTP/1.1\r\nHost: example.com:443\r\n\r\n";
        let (req_type, target) = parse_host_from_request(data).unwrap();
        assert!(matches!(req_type, RequestType::Connect));
        assert_eq!(target.host(), "example.com");
        assert_eq!(target.port(), 443);
    }

    #[test]
    fn parse_connect_ipv4() {
        let data = b"CONNECT 10.0.0.1:8443 HTTP/1.1\r\n\r\n";
        let (req_type, target) = parse_host_from_request(data).unwrap();
        assert!(matches!(req_type, RequestType::Connect));
        assert_eq!(target.host(), "10.0.0.1");
        assert_eq!(target.port(), 8443);
    }

    #[test]
    fn parse_connect_non_standard_port() {
        let data = b"CONNECT internal.corp:9999 HTTP/1.1\r\n\r\n";
        let (req_type, target) = parse_host_from_request(data).unwrap();
        assert!(matches!(req_type, RequestType::Connect));
        assert_eq!(target.host(), "internal.corp");
        assert_eq!(target.port(), 9999);
    }

    // ─── parse_host_from_request — plain HTTP GET ────────────────

    #[test]
    fn parse_http_get_with_host_header() {
        let data = b"GET http://example.com/path HTTP/1.1\r\nHost: example.com\r\n\r\n";
        let (req_type, target) = parse_host_from_request(data).unwrap();
        assert!(matches!(req_type, RequestType::Other));
        assert_eq!(target.host(), "example.com");
        assert_eq!(target.port(), 80);
    }

    #[test]
    fn parse_https_get_defaults_to_port_443() {
        let data = b"GET https://example.com/path HTTP/1.1\r\nHost: example.com\r\n\r\n";
        let (req_type, target) = parse_host_from_request(data).unwrap();
        assert!(matches!(req_type, RequestType::Other));
        assert_eq!(target.host(), "example.com");
        assert_eq!(target.port(), 443);
    }

    #[test]
    fn parse_http_get_with_explicit_port_in_host_header() {
        let data = b"GET http://example.com:8080/path HTTP/1.1\r\nHost: example.com:8080\r\n\r\n";
        let (req_type, target) = parse_host_from_request(data).unwrap();
        assert!(matches!(req_type, RequestType::Other));
        assert_eq!(target.host(), "example.com");
        assert_eq!(target.port(), 8080);
    }

    #[test]
    fn parse_http_get_ignores_url_port_when_host_has_port() {
        // The Host header takes precedence; port comes from there, not the URL
        let data = b"GET http://example.com:9090/path HTTP/1.1\r\nHost: example.com:7070\r\n\r\n";
        let (_, target) = parse_host_from_request(data).unwrap();
        assert_eq!(target.port(), 7070);
    }

    // ─── parse_host_from_request — plain HTTP POST ───────────────

    #[test]
    fn parse_http_post_request() {
        let data = b"POST http://api.example.com/submit HTTP/1.1\r\nHost: api.example.com\r\nContent-Length: 4\r\n\r\nbody";
        let (req_type, target) = parse_host_from_request(data).unwrap();
        assert!(matches!(req_type, RequestType::Other));
        assert_eq!(target.host(), "api.example.com");
        assert_eq!(target.port(), 80);
    }

    // ─── parse_host_from_request — error cases ───────────────────

    #[test]
    fn parse_missing_host_header_returns_error() {
        let data = b"GET /path HTTP/1.1\r\n\r\n";
        let result = parse_host_from_request(data);
        assert!(result.is_err());
    }

    #[test]
    #[should_panic]
    fn parse_empty_data_panics() {
        // Empty input causes a panic due to unwrap() on None path.
        // This test documents the current behavior — ideally the function
        // should return Err instead of panicking.
        let data = b"";
        let _ = parse_host_from_request(data);
    }

    // ─── http_407_response ────────────────────────────────────────

    #[test]
    fn http_407_response_is_a_complete_http_response() {
        let err = anyhow!("outer").context("Kerberos negotiation failed");
        let bytes = http_407_response(&err);
        let text = String::from_utf8(bytes).unwrap();

        let (head, body) = text.split_once("\r\n\r\n").unwrap();
        let mut lines = head.split("\r\n");
        assert_eq!(lines.next(), Some("HTTP/1.1 407 Proxy Authentication Required"));
        let headers: Vec<&str> = lines.collect();
        assert!(headers.contains(&"Content-Type: text/plain; charset=utf-8"));
        assert!(headers.contains(&"Connection: close"));
        assert!(headers.contains(&format!("Content-Length: {}", body.len()).as_str()));
        assert_eq!(body, "Kerberos negotiation failed: outer\n");
    }

    #[test]
    fn http_407_response_parses_with_httparse() {
        let bytes = http_407_response(&anyhow!("boom"));
        let mut headers = [httparse::EMPTY_HEADER; 8];
        let mut resp = httparse::Response::new(&mut headers);
        let status = resp.parse(&bytes).unwrap();
        assert!(status.is_complete());
        assert_eq!(resp.code, Some(407));
    }
}
