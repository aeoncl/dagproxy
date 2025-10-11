pub mod kerberos {
    use base64::engine::general_purpose;
    use base64::Engine;
    use bytes::Bytes;
    use cross_krb5::{ClientCtx, InitiateFlags};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use crate::http::connect_with_retry;

    pub async fn negotiate_with_krb5(proxy_host: &str) -> Result<(), anyhow::Error> {

        let proxy_without_port = {
            let parts = proxy_host.split(":").collect::<Vec<&str>>();
            parts.first().map(|e| e.to_owned()).ok_or(anyhow::anyhow!("Could not split proxy on :"))
        }?;

        let proxy_spn = format!("HTTP/{}", proxy_without_port);

        let (kerberos_client, token) = ClientCtx::new(InitiateFlags::empty(), None, &proxy_spn, None)?;
        let token = Bytes::copy_from_slice(&*token);
        let token_b64 = general_purpose::STANDARD.encode(token);

        let mut proxy_stream = connect_with_retry(proxy_host).await?;
        proxy_stream.write_all(format!("GET http://{} HTTP/1.1\r\nHost: {}\r\nProxy-Authorization: Negotiate {}\r\n\r\n", "google.com", "google.com", &token_b64).as_bytes()).await?;
        proxy_stream.flush().await?;

        let mut read_buffer = [0; 2048];
        let bytes_read = proxy_stream.read(&mut read_buffer).await?;
        let data = &read_buffer[..bytes_read];

        if data.starts_with(b"HTTP/1.1 4") || data.starts_with(b"HTTP/1.1 5") {
            Err(anyhow::anyhow!("Proxy negotiate failed: {}", String::from_utf8_lossy(data)))
        } else if data.starts_with(b"HTTP/1.1") {
            println!("Proxy negotiate successfull.");
            Ok(())
        } else {
            Err(anyhow::anyhow!("Error: Unexpected proxy negotiate response: {}", String::from_utf8_lossy(data)))
        }
    }
}
