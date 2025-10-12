
#[cfg(target_os = "windows")]
pub(crate) mod windows {
    use windivert::prelude::{WinDivertEvent, WinDivertFlags, WinDivertLayer};
    use windivert::WinDivert;

    fn setup_transparent_proxy_redirect(http_proxy_ip: String, http_proxy_port: u32, https_proxy_port: u32) -> Result<(), anyhow::Error>{

        let filter = format!(
            "outbound and tcp and (tcp.DstPort == 80 or tcp.DstPort == 443) and ip.SrcAddr != {}",
            http_proxy_ip
        );

        let mut divert = WinDivert::network(
            &filter,
            0,
            WinDivertFlags::default()
        )?;

        let mut packet_buffer = [0u8; 2048]; // Standard buffer size

        loop {
            // Read a socket event
            let result = divert.recv(Some(&mut packet_buffer));
            if let Err(e) = result {
                eprintln!("Error receiving socket event: {}", e);
                continue;
            }

            let packet = result.unwrap();



        }

        Ok(())
    }


    fn stop_transparent_proxy_redirect() {

    }

    #[cfg(test)]
    mod tests {
        use crate::transparent_proxy::windows::setup_transparent_proxy_redirect;

        #[test]
        pub fn test() {
            setup_transparent_proxy_redirect(3232,3233);
        }


    }

}

