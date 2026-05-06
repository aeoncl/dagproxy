use netaddr2::{Mask, Netv4Addr};
use no_proxy::NoProxyItem;
use std::net::Ipv4Addr;
use std::str::FromStr;

#[derive(Clone, PartialEq, Debug)]
pub enum ForwardPolicy {
    Proxy,
    Direct,
    PAC,
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
    pub pac_url: Option<String>,
    pub rules: Vec<(NoProxyItem, ForwardPolicy)>,
}

#[derive(Clone, PartialEq, Debug)]
pub enum IpRange {
    Subnet(Netv4Addr),
}

impl Default for IpRange {
    fn default() -> Self {
        IpRange::Subnet(Netv4Addr::from_str("127.0.0.1/32").unwrap())
    }
}

impl IpRange {
    pub fn matches(&self, host: &Ipv4Addr) -> bool {
        match self {
            IpRange::Subnet(subnet) => subnet.contains_ipv4(host),
        }
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

    pub fn applies_to(&self, ip: &Ipv4Addr) -> bool {
        self.ip_range.matches(ip)
    }
}

impl Default for SubnetConfig {
    fn default() -> Self {
        Self {
            ip_range: Default::default(),
            proxy_host: String::new(),
            proxy_port: 0,
            pac_url: None,
            rules: vec![(NoProxyItem::Wildcard, Default::default())],
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
            if subnet.applies_to(host) {
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

#[cfg(test)]
mod tests {
    use crate::config::{Config, ForwardPolicy, IpRange, SubnetConfig};
    use netaddr2::Netv4Addr;
    use no_proxy::NoProxyItem;
    use std::net::Ipv4Addr;
    use std::str::FromStr;

    #[test]
    fn subnet_matches_wildcard_test() {
        let config = SubnetConfig {
            ip_range: Default::default(),
            proxy_host: "".to_string(),
            proxy_port: 0,
            pac_url: None,
            rules: vec![(NoProxyItem::Wildcard, ForwardPolicy::Proxy)],
        };

        let fp = config.get_forward_policy("any");

        assert_eq!(Some(ForwardPolicy::Proxy), fp);
    }

    #[test]
    fn subnet_matches_exact_test() {
        let config = SubnetConfig {
            ip_range: Default::default(),
            proxy_host: "".to_string(),
            proxy_port: 0,
            pac_url: None,
            rules: vec![
                (
                    NoProxyItem::Plain("www.test.com".to_string()),
                    ForwardPolicy::Direct,
                ),
                (NoProxyItem::Wildcard, ForwardPolicy::Proxy),
            ],
        };

        let fp_www = config.get_forward_policy("www.test.com");
        assert_eq!(Some(ForwardPolicy::Direct), fp_www);

        let fp_test = config.get_forward_policy("test.com");
        assert_eq!(Some(ForwardPolicy::Proxy), fp_test);

        let fp = config.get_forward_policy("any");
        assert_eq!(Some(ForwardPolicy::Proxy), fp);
    }

    #[test]
    fn subnet_matches_none_test() {
        let config = SubnetConfig {
            ip_range: Default::default(),
            proxy_host: "".to_string(),
            proxy_port: 0,
            pac_url: None,
            rules: vec![(
                NoProxyItem::Plain("www.test.com".to_string()),
                ForwardPolicy::Direct,
            )],
        };

        let fp_www = config.get_forward_policy("www.test.com");
        assert_eq!(Some(ForwardPolicy::Direct), fp_www);

        let fp_test = config.get_forward_policy("test.com");
        assert_eq!(None, fp_test);
    }

    #[test]
    fn subnet_matches_dotted_test() {
        let config = SubnetConfig {
            ip_range: Default::default(),
            proxy_host: "".to_string(),
            proxy_port: 0,
            pac_url: None,
            rules: vec![
                (
                    NoProxyItem::WithDot(".test.".to_string(), true, true),
                    ForwardPolicy::Direct,
                ),
                (
                    NoProxyItem::Plain("test.com".to_string()),
                    ForwardPolicy::PAC,
                ),
            ],
        };

        let fp_www = config.get_forward_policy("www.test.com");
        assert_eq!(Some(ForwardPolicy::Direct), fp_www);

        let fp_www = config.get_forward_policy("www.test.be");
        assert_eq!(Some(ForwardPolicy::Direct), fp_www);

        let fp_test = config.get_forward_policy("test.com");
        assert_eq!(Some(ForwardPolicy::PAC), fp_test);
    }

    #[test]
    fn subnet_applies_to_ip_in_range() {
        let config = SubnetConfig {
            ip_range: IpRange::Subnet(Netv4Addr::from_str("192.168.1.1/24").unwrap()),
            proxy_host: "".to_string(),
            proxy_port: 0,
            pac_url: None,
            rules: vec![
                (
                    NoProxyItem::WithDot(".test.".to_string(), true, true),
                    ForwardPolicy::Direct,
                ),
                (
                    NoProxyItem::Plain("test.com".to_string()),
                    ForwardPolicy::PAC,
                ),
            ],
        };

        let applies = config.applies_to(&Ipv4Addr::from_str("192.168.1.2").unwrap());
        assert_eq!(true, applies);
    }

    #[test]
    fn subnet_doesnt_apply_to_ip_not_in_range() {
        let config = SubnetConfig {
            ip_range: IpRange::Subnet(Netv4Addr::from_str("192.168.1.1/24").unwrap()),
            proxy_host: "".to_string(),
            proxy_port: 0,
            pac_url: None,
            rules: vec![
                (
                    NoProxyItem::WithDot(".test.".to_string(), true, true),
                    ForwardPolicy::Direct,
                ),
                (
                    NoProxyItem::Plain("test.com".to_string()),
                    ForwardPolicy::PAC,
                ),
            ],
        };

        let applies = config.applies_to(&Ipv4Addr::from_str("192.168.2.1").unwrap());
        assert_eq!(false, applies);
    }

    #[test]
    fn config_evaluates_subnets_in_order() {
        let config = Config {
            port: 1234,
            subnets: vec![
                SubnetConfig {
                    ip_range: IpRange::Subnet(Netv4Addr::from_str("192.168.2.1/24").unwrap()),
                    proxy_host: "wildcard_proxy".to_string(),
                    proxy_port: 0,
                    rules: vec![
                        (
                            NoProxyItem::WithDot(".blablabla.".to_string(), true, true),
                            ForwardPolicy::Direct,
                        ),
                        (
                            NoProxyItem::Plain("blablabla.com".to_string()),
                            ForwardPolicy::Proxy,
                        ),
                    ],
                    pac_url: None,
                },
                SubnetConfig {
                    ip_range: IpRange::Subnet(Netv4Addr::from_str("192.168.1.1/24").unwrap()),
                    proxy_host: "non_wildcard_proxy".to_string(),
                    proxy_port: 0,
                    rules: vec![
                        (
                            NoProxyItem::WithDot(".test.".to_string(), true, true),
                            ForwardPolicy::Direct,
                        ),
                        (
                            NoProxyItem::Plain("test.com".to_string()),
                            ForwardPolicy::PAC,
                        ),
                    ],
                    pac_url: None,
                },
            ],
        };

        let config = config.get_matching_config(&Ipv4Addr::from_str("192.168.1.1").unwrap());
        assert!(config.is_some());
        let config = config.unwrap();
        assert_eq!(config.proxy_host, "non_wildcard_proxy");
    }
}
