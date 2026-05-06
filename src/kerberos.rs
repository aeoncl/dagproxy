pub mod kerberos {
    use crate::http::{NetworkTarget, connect_with_retry};
    use base64::Engine;
    use base64::engine::general_purpose;
    use bytes::Bytes;
    use cross_krb5::{ClientCtx, InitiateFlags};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    pub fn validate_proxy_response(data: &[u8]) -> Result<(), anyhow::Error> {
        if data.starts_with(b"HTTP/1.1 4") || data.starts_with(b"HTTP/1.1 5") {
            Err(anyhow::anyhow!(
                "Proxy negotiate failed: {}",
                String::from_utf8_lossy(data)
            ))
        } else if data.starts_with(b"HTTP/1.1") {
            Ok(())
        } else {
            Err(anyhow::anyhow!(
                "Error: Unexpected proxy negotiate response: {}",
                String::from_utf8_lossy(data)
            ))
        }
    }

    pub async fn negotiate_with_krb5(proxy: &NetworkTarget) -> Result<(), anyhow::Error> {
        let proxy_without_port = proxy.host();

        let proxy_spn = format!("HTTP/{}", proxy_without_port);

        #[cfg(target_os = "windows")]
        let flags = InitiateFlags::NEGOTIATE_TOKEN;
        #[cfg(not(target_os = "windows"))]
        let flags = InitiateFlags::empty();

        let (_, token) = ClientCtx::new(flags, None, &proxy_spn, None)?;
        let token = Bytes::copy_from_slice(&*token);
        let token_b64 = general_purpose::STANDARD.encode(token);

        let mut proxy_stream = connect_with_retry(proxy).await?;
        proxy_stream.write_all(format!("GET http://{} HTTP/1.1\r\nHost: {}\r\nProxy-Authorization: Negotiate {}\r\n\r\n", "google.com", "google.com", &token_b64).as_bytes()).await?;
        proxy_stream.flush().await?;

        let mut read_buffer = [0; 2048];
        let bytes_read = proxy_stream.read(&mut read_buffer).await?;
        let data = &read_buffer[..bytes_read];

        validate_proxy_response(data)
    }
}

#[cfg(test)]
mod tests {
    use crate::kerberos::kerberos::validate_proxy_response;

    // ─── Success cases ───

    #[test]
    fn accepts_200_ok_response() {
        let data = b"HTTP/1.1 200 OK\r\n\r\n";
        assert!(validate_proxy_response(data).is_ok());
    }

    #[test]
    fn accepts_200_connection_established() {
        let data = b"HTTP/1.1 200 Connection established\r\n\r\n";
        assert!(validate_proxy_response(data).is_ok());
    }

    #[test]
    fn accepts_204_no_content() {
        let data = b"HTTP/1.1 204 No Content\r\n\r\n";
        assert!(validate_proxy_response(data).is_ok());
    }

    #[test]
    fn accepts_301_redirect() {
        let data = b"HTTP/1.1 301 Moved Permanently\r\nLocation: https://example.com\r\n\r\n";
        assert!(validate_proxy_response(data).is_ok());
    }

    #[test]
    fn accepts_302_found() {
        let data = b"HTTP/1.1 302 Found\r\nLocation: https://example.com\r\n\r\n";
        assert!(validate_proxy_response(data).is_ok());
    }

    // ─── 4xx error cases ───

    #[test]
    fn rejects_400_bad_request() {
        let data = b"HTTP/1.1 400 Bad Request\r\n\r\n";
        let result = validate_proxy_response(data);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Proxy negotiate failed"),
            "Expected 'Proxy negotiate failed' in error, got: {}",
            err
        );
    }

    #[test]
    fn rejects_401_unauthorized() {
        let data = b"HTTP/1.1 401 Unauthorized\r\n\r\n";
        let result = validate_proxy_response(data);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Proxy negotiate failed")
        );
    }

    #[test]
    fn rejects_403_forbidden() {
        let data = b"HTTP/1.1 403 Forbidden\r\n\r\n";
        assert!(validate_proxy_response(data).is_err());
    }

    #[test]
    fn rejects_407_proxy_auth_required() {
        let data =
            b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Negotiate\r\n\r\n";
        let result = validate_proxy_response(data);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Proxy negotiate failed")
        );
    }

    // ─── 5xx error cases ───

    #[test]
    fn rejects_500_internal_server_error() {
        let data = b"HTTP/1.1 500 Internal Server Error\r\n\r\n";
        let result = validate_proxy_response(data);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Proxy negotiate failed")
        );
    }

    #[test]
    fn rejects_502_bad_gateway() {
        let data = b"HTTP/1.1 502 Bad Gateway\r\n\r\n";
        assert!(validate_proxy_response(data).is_err());
    }

    #[test]
    fn rejects_503_service_unavailable() {
        let data = b"HTTP/1.1 503 Service Unavailable\r\n\r\n";
        assert!(validate_proxy_response(data).is_err());
    }

    // ─── Unexpected / garbage responses ───

    #[test]
    fn rejects_empty_response() {
        let data = b"";
        let result = validate_proxy_response(data);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Unexpected proxy negotiate response")
        );
    }

    #[test]
    fn rejects_raw_text() {
        let data = b"FOOBAR";
        let result = validate_proxy_response(data);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Unexpected proxy negotiate response")
        );
    }

    #[test]
    fn rejects_http_1_0_response() {
        let data = b"HTTP/1.0 200 OK\r\n\r\n";
        let result = validate_proxy_response(data);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Unexpected proxy negotiate response")
        );
    }

    #[test]
    fn rejects_binary_garbage() {
        let data = &[0x00, 0x01, 0x02, 0x03];
        let result = validate_proxy_response(data);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Unexpected proxy negotiate response")
        );
    }

    // ─── Error message includes the response body ───

    #[test]
    fn error_message_contains_response_body_on_4xx() {
        let data = b"HTTP/1.1 407 Proxy Auth Required\r\nProxy-Authenticate: Negotiate\r\n\r\n";
        let err_msg = validate_proxy_response(data).unwrap_err().to_string();
        assert!(
            err_msg.contains("HTTP/1.1 407"),
            "Error message should contain the raw response, got: {}",
            err_msg
        );
    }

    #[test]
    fn error_message_contains_response_body_on_garbage() {
        let data = b"FOOBAR";
        let err_msg = validate_proxy_response(data).unwrap_err().to_string();
        assert!(
            err_msg.contains("FOOBAR"),
            "Error message should contain the raw response, got: {}",
            err_msg
        );
    }
}
