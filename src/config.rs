use crate::NoProxyValue;
use netaddr2::Netv4Addr;
use std::str::FromStr;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum SubNetKey {
    Default,
    Subnet(Netv4Addr)
}

#[derive(Clone, PartialEq, Debug)]
pub enum ProxyConfig {
    Direct,
    Proxy { host: String, port: u32, no_proxy: Vec<NoProxyValue> }
}
impl Default for ProxyConfig {
    fn default() -> Self {
        ProxyConfig::Direct
    }
}

#[derive(Clone, PartialEq)]
pub struct Config {
    pub port: u32,
    pub subnets: Vec<(SubNetKey, ProxyConfig)>,
}
impl Default for Config {
    fn default() -> Self {
        let mut subnets = Vec::new();

        subnets.push(
            (SubNetKey::Subnet(Netv4Addr::from_str("10.80.0.0/16").unwrap()),
            ProxyConfig::Proxy {
                host: "proxygate.onemrva.priv".to_owned(),
                port: 8888,
                no_proxy: "localhost,rvaonem.priv,rvaonem.fgov.be,169.254.169.254,cloud.rvadc.be,onemrva.priv,teams.microsoft.com,google.com".split(",").map(|host| {
                    NoProxyValue::from_str(host).expect(format!("Invalid no proxy host: {}", &host).as_str())
                }).collect::<Vec<_>>()
            })
        );

        subnets.push(
            (SubNetKey::Subnet(Netv4Addr::from_str("10.130.0.0/16").unwrap()),
            ProxyConfig::Proxy {
                host: "proxygate.onemrva.priv".to_owned(),
                port: 8888,
                no_proxy: "localhost,rvaonem.priv,rvaonem.fgov.be,169.254.169.254,cloud.rvadc.be,onemrva.priv".split(",").map(|host| {
                    NoProxyValue::from_str(host).expect(format!("Invalid no proxy host: {}", &host).as_str())
                }).collect::<Vec<_>>()
            })
        );

        subnets.push((SubNetKey::Default, ProxyConfig::Direct));

        Self {
            port: 3333,
            subnets
        }
    }
}