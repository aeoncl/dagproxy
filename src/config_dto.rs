use crate::config::{Config, ForwardPolicy, IpRange, SubnetConfig};
use indexmap::IndexMap;
use netaddr2::Netv4Addr;
use no_proxy::NoProxyItem;
use std::str::FromStr;

#[derive(serde::Deserialize, serde::Serialize)]
pub struct ConfigDto {
    pub port: u32,
    pub subnets: Vec<SubnetConfigDto>,
}

#[derive(serde::Deserialize, serde::Serialize)]
pub struct SubnetConfigDto {
    pub ip_range: String,
    pub proxy_host: String,
    pub proxy_port: u32,
    pub pac_url: Option<String>,
    pub rules: IndexMap<String, ForwardPolicyDto>,
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ForwardPolicyDto {
    Proxy,
    Direct,
}

impl TryInto<Config> for ConfigDto {
    type Error = anyhow::Error;

    fn try_into(self) -> Result<Config, Self::Error> {
        let subnets = self
            .subnets
            .into_iter()
            .map(|subnet| subnet.try_into())
            .collect::<Result<Vec<SubnetConfig>, anyhow::Error>>()?;

        Ok(Config {
            port: self.port,
            subnets,
        })
    }
}

impl TryInto<SubnetConfig> for SubnetConfigDto {
    type Error = anyhow::Error;

    fn try_into(self) -> Result<SubnetConfig, Self::Error> {
        let rules = self
            .rules
            .into_iter()
            .map(|(key, forward_policy)| {
                let key_parsed = NoProxyItem::try_from(&key)?;
                let forward_policy = forward_policy.into();
                Ok::<(NoProxyItem, ForwardPolicy), anyhow::Error>((key_parsed, forward_policy))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let ip_range = {
            match self.ip_range.as_str() {
                subnet => IpRange::Subnet(
                    Netv4Addr::from_str(subnet)
                        .map_err(|e| anyhow::anyhow!("Invalid subnet: {}", e))?,
                ),
            }
        };

        Ok(SubnetConfig {
            ip_range,
            proxy_host: self.proxy_host,
            proxy_port: self.proxy_port,
            pac_url: self.pac_url,
            rules,
        })
    }
}

impl Into<ForwardPolicy> for ForwardPolicyDto {
    fn into(self) -> ForwardPolicy {
        match self {
            ForwardPolicyDto::Proxy => ForwardPolicy::Proxy,
            ForwardPolicyDto::Direct => ForwardPolicy::Direct,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::config_dto::ConfigDto;

    #[test]
    fn deser_test() {
        let config_json = r#"
{
  "port": 3232,
  "subnets": [
    {
        "ip_range": "10.0.0.0/24",
        "proxy_host": "upstream.proxy",
        "proxy_port": 8080,
        "rules": {
          "subdomain.example.com": "DIRECT",
          "*": "PROXY"
        }
    },
    {
        "ip_range": "11.0.0.0/24",
        "proxy_host": "upstream.proxy",
        "proxy_port": 8080,
        "rules": {
          "subdomain.example.com": "DIRECT",
          "*": "PROXY"
        }
    }
  ]
}
    "#;

        let config_dto: ConfigDto = serde_json::from_str(&config_json).unwrap();

        assert_eq!(config_dto.port, 3232);
        assert_eq!(config_dto.subnets.len(), 2);

        let first_subnet = &config_dto.subnets[0];
        assert_eq!(first_subnet.ip_range, "10.0.0.0/24");

        let second_subnet = &config_dto.subnets[1];
        assert_eq!(second_subnet.ip_range, "11.0.0.0/24");
    }
}
