use dagproxy::config::{ForwardPolicy, IpRange, SubnetConfig};
use dagproxy::http_proxy::HttpProxy;
use dagproxy::network_watcher::NetworkWatchTransport;
use netaddr2::Netv4Addr;
use no_proxy::NoProxyItem;
use std::str::FromStr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch::Sender;

// ──────────────────────────────────────────────────────────────────
// Test helpers — mock servers
// ──────────────────────────────────────────────────────────────────

/// A simple TCP echo server that reflects whatever it receives.
/// Returns the port it's listening on.
async fn start_echo_server() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            if let Ok((mut sock, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 8192];
                    loop {
                        match sock.read(&mut buf).await {
                            Ok(0) => break,
                            Ok(n) => {
                                if sock.write_all(&buf[..n]).await.is_err() {
                                    break;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                });
            }
        }
    });
    port
}

/// A mock upstream HTTP proxy that understands CONNECT.
/// It responds `200 Connection established` to CONNECT requests and then
/// relays data bidirectionally to the real target.
///
/// Returns the port it's listening on.
async fn start_mock_proxy_server() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            if let Ok((mut client_sock, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 4096];
                    let n = match client_sock.read(&mut buf).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => n,
                    };

                    let request = String::from_utf8_lossy(&buf[..n]);

                    if request.starts_with("CONNECT") {
                        // Parse the target from "CONNECT host:port HTTP/1.1"
                        let target = request
                            .split_whitespace()
                            .nth(1)
                            .expect("mock proxy: missing target in CONNECT");

                        // Respond 200
                        client_sock
                            .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                            .await
                            .unwrap();

                        // Connect to the actual target and relay
                        match TcpStream::connect(target).await {
                            Ok(mut target_sock) => {
                                relay_bidirectional(&mut client_sock, &mut target_sock).await;
                            }
                            Err(e) => {
                                eprintln!(
                                    "mock proxy: failed to connect to target {}: {}",
                                    target, e
                                );
                            }
                        }
                    } else {
                        // For non-CONNECT requests (plain HTTP proxy), respond with 200 OK
                        // and echo the received request back as the body. This is a simplistic mock.
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}",
                            n, request
                        );
                        client_sock.write_all(response.as_bytes()).await.unwrap();
                    }
                });
            }
        }
    });
    port
}

/// A mock upstream proxy that always responds `407 Proxy Authentication Required`
/// to simulate an auth challenge. Used to verify that DagProxy handles 407 gracefully.
async fn start_407_proxy_server() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            if let Ok((mut sock, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 4096];
                    let _ = sock.read(&mut buf).await;
                    let _ = sock
                        .write_all(
                            b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Negotiate\r\n\r\n",
                        )
                        .await;
                });
            }
        }
    });
    port
}

/// Bidirectional relay between two TCP streams until one side closes.
async fn relay_bidirectional(a: &mut TcpStream, b: &mut TcpStream) {
    let mut buf_a = [0u8; 8192];
    let mut buf_b = [0u8; 8192];

    loop {
        tokio::select! {
            read_a = a.read(&mut buf_a) => {
                match read_a {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if b.write_all(&buf_a[..n]).await.is_err() {
                            break;
                        }
                    }
                }
            }
            read_b = b.read(&mut buf_b) => {
                match read_b {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if a.write_all(&buf_b[..n]).await.is_err() {
                            break;
                        }
                    }
                }
            }
        }
    }
}

/// Starts DagProxy on a random port with the given subnet config.
/// Returns the port DagProxy is listening on.
async fn start_dagproxy(subnet_config: SubnetConfig) -> (u16, Sender<SubnetConfig>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    // Drop the listener — DagProxy will re-bind it. We just needed a free port.
    drop(listener);

    let (transport, sender) = NetworkWatchTransport::new_for_testing(subnet_config);
    let mut proxy = HttpProxy::new(transport);
    tokio::spawn(async move {
        proxy
            .start("127.0.0.1".to_owned(), port as u32)
            .await
            .expect("DagProxy failed to start");
    });

    // Give the proxy a moment to bind the port
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    (port, sender)
}

/// Helper: build a subnet config that routes everything through the given proxy.
fn proxy_all_config(proxy_port: u16) -> SubnetConfig {
    SubnetConfig {
        ip_range: IpRange::Subnet(Netv4Addr::from_str("127.0.0.1/32").unwrap()),
        name: "".to_string(),
        proxy_host: "127.0.0.1".to_string(),
        proxy_port: proxy_port as u32,
        rules: vec![(NoProxyItem::Wildcard, ForwardPolicy::Proxy)],
    }
}

/// Helper: build a subnet config that routes everything DIRECT.
fn direct_all_config() -> SubnetConfig {
    SubnetConfig {
        ip_range: IpRange::Subnet(Netv4Addr::from_str("127.0.0.1/32").unwrap()),
        name: "".to_string(),
        proxy_host: String::new(),
        proxy_port: 0,
        rules: vec![(NoProxyItem::Wildcard, ForwardPolicy::Direct)],
    }
}

/// Helper: build a subnet config with mixed rules.
/// `direct_hosts` go DIRECT, everything else goes PROXY.
fn mixed_rules_config(proxy_port: u16, direct_hosts: Vec<&str>) -> SubnetConfig {
    let mut rules: Vec<_> = direct_hosts
        .into_iter()
        .map(|host| (NoProxyItem::Plain(host.to_string()), ForwardPolicy::Direct))
        .collect();
    rules.push((NoProxyItem::Wildcard, ForwardPolicy::Proxy));

    SubnetConfig {
        ip_range: IpRange::Subnet(Netv4Addr::from_str("127.0.0.1/32").unwrap()),
        name: "".to_string(),
        proxy_host: "127.0.0.1".to_string(),
        proxy_port: proxy_port as u32,
        rules,
    }
}

// ──────────────────────────────────────────────────────────────────
// Tests — CONNECT tunnel
// ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn connect_tunnel_echoes_data_through_proxy() {
    let echo_port = start_echo_server().await;
    let mock_proxy_port = start_mock_proxy_server().await;
    let (dagproxy_port, _sender) = start_dagproxy(proxy_all_config(mock_proxy_port)).await;

    let mut client = TcpStream::connect(format!("127.0.0.1:{}", dagproxy_port))
        .await
        .unwrap();

    // Send CONNECT request targeting the echo server
    let connect_request = format!(
        "CONNECT 127.0.0.1:{} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n\r\n",
        echo_port, echo_port
    );
    client.write_all(connect_request.as_bytes()).await.unwrap();

    // Read the 200 response from DagProxy
    let mut buf = [0u8; 512];
    let n = client.read(&mut buf).await.unwrap();
    let response = String::from_utf8_lossy(&buf[..n]);
    assert!(
        response.starts_with("HTTP/1.1 200"),
        "Expected 200 Connection established, got: {}",
        response
    );

    // Now the tunnel should be established — send data and expect echo
    let payload = b"Hello through the tunnel!";
    client.write_all(payload).await.unwrap();

    let mut echo_buf = [0u8; 256];
    let n = client.read(&mut echo_buf).await.unwrap();
    assert_eq!(
        &echo_buf[..n],
        payload,
        "Echoed data should match sent data"
    );
}

#[tokio::test]
async fn connect_tunnel_relays_multiple_round_trips() {
    let echo_port = start_echo_server().await;
    let mock_proxy_port = start_mock_proxy_server().await;
    let (dagproxy_port, _sender) = start_dagproxy(proxy_all_config(mock_proxy_port)).await;

    let mut client = TcpStream::connect(format!("127.0.0.1:{}", dagproxy_port))
        .await
        .unwrap();

    let connect_request = format!(
        "CONNECT 127.0.0.1:{} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n\r\n",
        echo_port, echo_port
    );
    client.write_all(connect_request.as_bytes()).await.unwrap();

    // Consume the 200 response
    let mut buf = [0u8; 512];
    let _ = client.read(&mut buf).await.unwrap();

    // Send multiple payloads and verify each is echoed back
    for i in 0..5 {
        let payload = format!("round-trip-{}", i);
        client.write_all(payload.as_bytes()).await.unwrap();

        let mut echo_buf = [0u8; 256];
        let n = client.read(&mut echo_buf).await.unwrap();
        assert_eq!(
            &echo_buf[..n],
            payload.as_bytes(),
            "Round trip {} failed",
            i
        );
    }
}

// ──────────────────────────────────────────────────────────────────
// Tests — Direct connection
// ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn connect_direct_echoes_data() {
    let echo_port = start_echo_server().await;
    let (dagproxy_port, _sender) = start_dagproxy(direct_all_config()).await;

    let mut client = TcpStream::connect(format!("127.0.0.1:{}", dagproxy_port))
        .await
        .unwrap();

    // CONNECT request to the echo server — should go DIRECT
    let connect_request = format!(
        "CONNECT 127.0.0.1:{} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n\r\n",
        echo_port, echo_port
    );
    client.write_all(connect_request.as_bytes()).await.unwrap();

    let mut buf = [0u8; 512];
    let n = client.read(&mut buf).await.unwrap();
    let response = String::from_utf8_lossy(&buf[..n]);
    assert!(
        response.starts_with("HTTP/1.1 200"),
        "Expected 200, got: {}",
        response
    );

    let payload = b"Direct connection works!";
    client.write_all(payload).await.unwrap();

    let mut echo_buf = [0u8; 256];
    let n = client.read(&mut echo_buf).await.unwrap();
    assert_eq!(&echo_buf[..n], payload);
}

// ──────────────────────────────────────────────────────────────────
// Tests — Mixed routing rules
// ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn mixed_rules_direct_host_goes_direct() {
    let echo_port = start_echo_server().await;
    let _mock_proxy_port = start_mock_proxy_server().await;
    let (dagproxy_port, _sender) = start_dagproxy(mixed_rules_config(0, vec!["127.0.0.1"])).await;

    // Since "127.0.0.1" is in the DIRECT list, the traffic should NOT go through
    // the mock proxy. The echo server should be reachable directly.
    let mut client = TcpStream::connect(format!("127.0.0.1:{}", dagproxy_port))
        .await
        .unwrap();

    let connect_request = format!(
        "CONNECT 127.0.0.1:{} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n\r\n",
        echo_port, echo_port
    );
    client.write_all(connect_request.as_bytes()).await.unwrap();

    let mut buf = [0u8; 512];
    let n = client.read(&mut buf).await.unwrap();
    let response = String::from_utf8_lossy(&buf[..n]);
    assert!(
        response.starts_with("HTTP/1.1 200"),
        "Expected 200 for DIRECT host, got: {}",
        response
    );

    let payload = b"Direct via mixed rules";
    client.write_all(payload).await.unwrap();

    let mut echo_buf = [0u8; 256];
    let n = client.read(&mut echo_buf).await.unwrap();
    assert_eq!(&echo_buf[..n], payload);
}

#[tokio::test]
async fn mixed_rules_unknown_host_goes_through_proxy() {
    let echo_port = start_echo_server().await;
    let mock_proxy_port = start_mock_proxy_server().await;
    // "localhost" is NOT in the DIRECT list, so it should go PROXY.
    // The mock proxy will relay to the echo server.
    let (dagproxy_port, _sender) = start_dagproxy(mixed_rules_config(
        mock_proxy_port,
        vec!["direct.example.com"],
    ))
    .await;

    let mut client = TcpStream::connect(format!("127.0.0.1:{}", dagproxy_port))
        .await
        .unwrap();

    // CONNECT to echo server via a host that is NOT in the direct list
    let connect_request = format!(
        "CONNECT 127.0.0.1:{} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n\r\n",
        echo_port, echo_port
    );
    client.write_all(connect_request.as_bytes()).await.unwrap();

    let mut buf = [0u8; 512];
    let n = client.read(&mut buf).await.unwrap();
    let response = String::from_utf8_lossy(&buf[..n]);
    assert!(
        response.starts_with("HTTP/1.1 200"),
        "Expected 200 via upstream proxy, got: {}",
        response
    );

    let payload = b"Proxied via mixed rules";
    client.write_all(payload).await.unwrap();

    let mut echo_buf = [0u8; 256];
    let n = client.read(&mut echo_buf).await.unwrap();
    assert_eq!(&echo_buf[..n], payload);
}

// ──────────────────────────────────────────────────────────────────
// Tests — 407 from upstream proxy
// ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn proxy_407_causes_connect_failure_when_no_krb() {
    let _echo_port = start_echo_server().await;
    let reject_proxy_port = start_407_proxy_server().await;
    let (dagproxy_port, _sender) = start_dagproxy(proxy_all_config(reject_proxy_port)).await;

    let mut client = TcpStream::connect(format!("127.0.0.1:{}", dagproxy_port))
        .await
        .unwrap();

    // The CONNECT should fail because the upstream proxy returns 407 and
    // Kerberos negotiation will likely fail without a KDC.
    // DagProxy should close the connection (or we'll get an error).
    let connect_request = format!("CONNECT 127.0.0.1:80 HTTP/1.1\r\nHost: 127.0.0.1:80\r\n\r\n");
    client.write_all(connect_request.as_bytes()).await.unwrap();

    // We expect the connection to be closed or return an error,
    // since Kerberos negotiation will fail in a test environment.
    let mut buf = [0u8; 512];
    let result =
        tokio::time::timeout(std::time::Duration::from_secs(5), client.read(&mut buf)).await;

    // Either we get 0 bytes (closed), an error, or a timeout.
    // All are acceptable — the key is we don't get a 200 Connection established.
    match result {
        Ok(Ok(0)) | Ok(Err(_)) | Err(_) => {
            // Connection closed, error, or timeout — all expected
        }
        Ok(Ok(n)) => {
            let response = String::from_utf8_lossy(&buf[..n]);
            assert!(
                !response.starts_with("HTTP/1.1 200"),
                "Should not get 200 when upstream proxy returns 407, got: {}",
                response
            );
        }
    }
}

// ──────────────────────────────────────────────────────────────────
// Tests — Plain HTTP (non-CONNECT) forwarding
// ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn plain_http_get_forwarded_through_proxy() {
    // Start an echo server that also captures what it receives
    let echo_port = start_echo_server().await;
    let mock_proxy_port = start_mock_proxy_server().await;
    let (dagproxy_port, _sender) = start_dagproxy(proxy_all_config(mock_proxy_port)).await;

    let mut client = TcpStream::connect(format!("127.0.0.1:{}", dagproxy_port))
        .await
        .unwrap();

    // Send a plain HTTP GET request (not CONNECT)
    let http_request = format!(
        "GET http://127.0.0.1:{}/test-path HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n\r\n",
        echo_port, echo_port
    );
    client.write_all(http_request.as_bytes()).await.unwrap();

    // DagProxy sends the request to the upstream proxy as-is (no CONNECT tunnel).
    // The mock proxy answers 200 and echoes the request it received as the body.
    let mut buf = [0u8; 2048];
    let n = client.read(&mut buf).await.unwrap();
    let response = String::from_utf8_lossy(&buf[..n]);
    assert!(
        response.starts_with("HTTP/1.1 200 OK"),
        "Expected the proxy's own response, got: {}",
        response
    );
    assert!(
        response.contains(&format!("GET http://127.0.0.1:{}/test-path HTTP/1.1", echo_port)),
        "Expected the proxy to receive our absolute-URI request, got: {}",
        response
    );
    assert!(
        !response.contains("CONNECT"),
        "Plain HTTP must not be tunnelled with CONNECT, got: {}",
        response
    );
}

#[tokio::test]
async fn plain_http_407_passed_to_client_when_no_krb() {
    let reject_proxy_port = start_407_proxy_server().await;
    let (dagproxy_port, _sender) = start_dagproxy(proxy_all_config(reject_proxy_port)).await;

    let mut client = TcpStream::connect(format!("127.0.0.1:{}", dagproxy_port))
        .await
        .unwrap();

    let http_request = "GET http://example.com/ HTTP/1.1\r\nHost: example.com\r\n\r\n";
    client.write_all(http_request.as_bytes()).await.unwrap();

    // Kerberos negotiation fails in a test environment, so the proxy's 407 is
    // passed on to the client instead of closing the connection without a reply.
    let mut buf = [0u8; 512];
    let n = tokio::time::timeout(std::time::Duration::from_secs(5), client.read(&mut buf))
        .await
        .expect("no response within 5s")
        .unwrap();
    let response = String::from_utf8_lossy(&buf[..n]);
    assert!(
        response.starts_with("HTTP/1.1 407"),
        "Expected the upstream 407 to be passed through, got: {}",
        response
    );
}

#[tokio::test]
async fn plain_http_get_forwarded_directly() {
    let echo_port = start_echo_server().await;
    let (dagproxy_port, _sender) = start_dagproxy(direct_all_config()).await;

    let mut client = TcpStream::connect(format!("127.0.0.1:{}", dagproxy_port))
        .await
        .unwrap();

    // For direct mode, DagProxy connects to the echo server and forwards
    // the raw request. The echo server reflects the bytes back.
    let http_request = format!(
        "GET http://127.0.0.1:{}/test HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n\r\n",
        echo_port, echo_port
    );
    client.write_all(http_request.as_bytes()).await.unwrap();

    let mut buf = [0u8; 2048];
    let n = client.read(&mut buf).await.unwrap();
    let response = String::from_utf8_lossy(&buf[..n]);
    // The echo server reflects the raw request bytes back to us
    assert!(
        response.contains("GET http://127.0.0.1"),
        "Expected the echo server to reflect our raw request, got: {}",
        response
    );
}

// ──────────────────────────────────────────────────────────────────
// Tests — Multiple concurrent connections
// ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn handles_multiple_concurrent_connect_tunnels() {
    let echo_port = start_echo_server().await;
    let mock_proxy_port = start_mock_proxy_server().await;
    let (dagproxy_port, _sender) = start_dagproxy(proxy_all_config(mock_proxy_port)).await;

    let mut handles = Vec::new();

    for i in 0..5 {
        let dagproxy_addr = format!("127.0.0.1:{}", dagproxy_port);
        let echo_addr = format!("127.0.0.1:{}", echo_port);

        handles.push(tokio::spawn(async move {
            let mut client = TcpStream::connect(&dagproxy_addr).await.unwrap();

            let connect_request = format!(
                "CONNECT {} HTTP/1.1\r\nHost: {}\r\n\r\n",
                echo_addr, echo_addr
            );
            client.write_all(connect_request.as_bytes()).await.unwrap();

            let mut buf = [0u8; 512];
            let n = client.read(&mut buf).await.unwrap();
            let response = String::from_utf8_lossy(&buf[..n]);
            assert!(response.starts_with("HTTP/1.1 200"));

            let payload = format!("concurrent-{}", i);
            client.write_all(payload.as_bytes()).await.unwrap();

            let mut echo_buf = [0u8; 256];
            let n = client.read(&mut echo_buf).await.unwrap();
            assert_eq!(&echo_buf[..n], payload.as_bytes());
        }));
    }

    for handle in handles {
        handle.await.unwrap();
    }
}
