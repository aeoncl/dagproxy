use crate::config::{Config, ProxyConfig, SubNetKey};
use netaddr2::Netv4Addr;
use std::str::FromStr;
use crate::NoProxyValue;

#[derive(serde::Deserialize, serde::Serialize)]
pub struct ConfigDto {
    pub port: u32,
    pub subnets: Vec<ProxyConfigDto>,
}
impl ConfigDto {
    pub fn to_config(self) -> Config {
        let subnets = self.subnets.iter().map(|subnet| match subnet {
            ProxyConfigDto::Direct => (SubNetKey::Default, ProxyConfig::Direct),
            ProxyConfigDto::Proxy(subnet_dto) => (
                SubNetKey::Subnet(Netv4Addr::from_str(subnet_dto.ip_range.as_str()).unwrap()),
                ProxyConfig::Proxy {
                    host: subnet_dto.proxy_host.clone(),
                    port: subnet_dto.proxy_port,
                    no_proxy: subnet_dto.no_proxy.iter()
                        .map(|no_proxy| NoProxyValue::from_str(no_proxy.as_str()).unwrap())
                        .collect::<Vec<_>>(),
                },
            ),
        }).collect::<Vec<_>>();

        Config {
            port: self.port,
            subnets,
        }
    }
}

#[derive(serde::Deserialize, serde::Serialize)]
pub enum ProxyConfigDto {
    Direct,
    Proxy(ProxySubnet),
}

#[derive(serde::Deserialize, serde::Serialize)]
pub struct ProxySubnet {
    pub ip_range: String,
    pub proxy_host: String,
    pub proxy_port: u32,
    pub no_proxy: Vec<String>,
}
