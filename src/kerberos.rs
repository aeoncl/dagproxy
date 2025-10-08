pub mod kerberos {
    use base64::engine::general_purpose;
    use base64::Engine;
    use bytes::Bytes;
    use cross_krb5::{ClientCtx, InitiateFlags};

    pub fn get_negociate_token(proxy_url: &str) -> Result<String, anyhow::Error> {
        let proxy_spn = format!("HTTP/{}", proxy_url);

        let (_, token) = ClientCtx::new(InitiateFlags::empty(), None, &proxy_spn, None)?;
        let token = Bytes::copy_from_slice(&*token);

        Ok(general_purpose::STANDARD.encode(token))
    }
}