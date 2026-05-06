use std::net::Ipv4Addr;
use std::ops::Sub;
use netaddr2::{Mask, Netv4Addr};
use no_proxy::NoProxyItem;

#[derive(Clone, PartialEq, Debug)]
pub enum ForwardPolicy {
    Proxy,
    Direct,
}

impl Default for ForwardPolicy {
    fn default() -> Self {
        Self::Direct
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct SubnetConfig {
    pub ip_range: IpRange,
    pub proxy_host: String,
    pub proxy_port: u32,
    pub rules: Vec<(NoProxyItem, ForwardPolicy)>
}

#[derive(Clone, PartialEq, Debug)]
pub enum IpRange {
    Subnet(Netv4Addr),
    Wildcard,
}

impl IpRange {
    pub fn matches(&self, host: &Ipv4Addr) -> bool {
        match self {
            IpRange::Subnet(subnet) => subnet.contains_ipv4(host),
            IpRange::Wildcard => true,
        }
    }
}

impl Default for IpRange {
    fn default() -> Self {
        Self::Wildcard
    }
}

impl SubnetConfig {
    pub fn get_forward_policy(&self, host: &str) -> Option<ForwardPolicy> {
        for (no_proxy_item, forward_policy) in &self.rules {
            if no_proxy_item.matches(host) {
                return Some(forward_policy.clone());
            }
        }
        None
    }
}

impl Default for SubnetConfig {
    fn default() -> Self {
        Self {
            ip_range: Default::default(),
            proxy_host: String::new(),
            proxy_port: 0,
            rules: vec![
                (NoProxyItem::Wildcard, Default::default())
            ],
        }
    }
}

#[derive(Clone)]
pub struct Config {
    pub port: u32,
    pub subnets: Vec<SubnetConfig>,
}

impl Config {
    pub fn get_matching_config(&self, host: &Ipv4Addr) -> Option<SubnetConfig> {
        for subnet in &self.subnets {
            if subnet.ip_range.matches(host) {
                return Some(subnet.clone());
            }
        }
        None
    }
}

impl Default for Config {
    fn default() -> Self {
        todo!()
    }
}

trait ContainsIpV4 {
    fn contains_ipv4(&self, ip: &Ipv4Addr) -> bool;
}
impl ContainsIpV4 for Netv4Addr {
    fn contains_ipv4(&self, ip: &Ipv4Addr) -> bool {
        let other: Self = Self::from(*ip);
        other.addr().mask(&self.mask()) == self.addr()
    }
}