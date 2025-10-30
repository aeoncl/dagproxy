
#[cfg(target_os = "windows")]
pub(crate) mod windows {
    use std::collections::HashMap;
    use std::net::Ipv4Addr;
    use std::str::FromStr;
    use etherparse::err::ip::HeadersSliceError;
    use etherparse::err::packet::SliceError;
    use etherparse::{IpHeaders, IpPayloadSlice, NetHeaders, PacketHeaders, TransportHeader};
    use windivert::address::WinDivertAddress;
    use windivert::layer::NetworkLayer;
    use windivert::packet::WinDivertPacket;
    use windivert::prelude::{WinDivertEvent, WinDivertFlags, WinDivertLayer};
    use windivert::WinDivert;
    use windivert_sys::ChecksumFlags;

    pub(crate) fn setup_transparent_proxy_redirect(http_proxy_ip: &str, http_proxy_port: u16, https_proxy_port: u32) -> Result<(), anyhow::Error>{

        let proxy_ip = Ipv4Addr::from_str(http_proxy_ip)?;

        let filter = format!(
            "outbound and tcp and ip and ip.SrcAddr != {}",
            http_proxy_ip
        );

        let mut divert = WinDivert::network(
            &filter,
            0,
            WinDivertFlags::default()
        )?;


        let mut packet_buffer = [0u8; 2048]; // Standard buffer size

        // map of packet hash to original destination
        let mut nat_map: HashMap<String, Ipv4Addr> = HashMap::new();

        loop {
            // Read a socket event
            let result = divert.recv(Some(&mut packet_buffer));
            if let Err(e) = result {
                eprintln!("Error receiving socket event: {}", e);
                continue;
            }

            let mut packet = result.unwrap();
            let data = packet.data.to_mut();

            let (original_dest_ip, original_dest_port) = match etherparse::PacketHeaders::from_ip_slice(data) {
                Ok(headers) => {
                    let dest_ip = if let Some(NetHeaders::Ipv4(mut ipv4_headers, _)) = headers.net {
                        let original_dest = Ipv4Addr::new(ipv4_headers.destination[0], ipv4_headers.destination[1], ipv4_headers.destination[2], ipv4_headers.destination[3]);
                        ipv4_headers.destination = proxy_ip.octets();
                        Some(original_dest)
                    } else {
                        None
                    };

                    let dest_port = if let Some(TransportHeader::Tcp(mut tcp)) = headers.transport {
                        let dest_port = tcp.destination_port;
                        tcp.destination_port = http_proxy_port;
                        Some(tcp.destination_port)
                    } else {
                        None
                    };

                    (dest_ip, dest_port)
                }
                Err(e) => {
                    println!("Error parsing winderive packet: {}", e);
                    (None, None)
                }
            };

            if original_dest_ip.is_none() || original_dest_port.is_none() {
                divert.send(&packet).unwrap();
                continue;
            }

            let dest_ip = original_dest_ip.unwrap();
            let dest_port = original_dest_port.unwrap();



            packet.recalculate_checksums(ChecksumFlags::default()).unwrap();

            //Todo save new packet checksum & original destination in nat_map

            divert.send(&packet).unwrap();

        }



        Ok(())
    }


    fn stop_transparent_proxy_redirect() {

    }

    #[cfg(test)]
    mod tests {
        use std::thread::sleep;
        use std::time::Duration;
        use crate::transparent_proxy::windows::setup_transparent_proxy_redirect;

        #[test]
        pub fn test() {
            setup_transparent_proxy_redirect("127.0.0.2", 3232, 3233).unwrap();

            sleep(Duration::from_secs(60))

        }


    }

}

