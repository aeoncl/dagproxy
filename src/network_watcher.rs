use crate::config::{Config, ProxyConfig};
use crate::config::SubNetKey;
use crate::config::SubNetKey::Subnet;
use netaddr2::{Mask, Netv4Addr};
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use netwatcher::WatchHandle;
use tokio::sync::watch::Receiver;
use crate::config::ProxyConfig::Direct;


#[derive(Clone)]
pub(crate) struct NetworkWatchHandle {
    notification_receiver: Receiver<ProxyConfig>,
    #[allow(dead_code)]
    handle: Arc<Mutex<WatchHandle>>,
}

impl NetworkWatchHandle {
    pub fn network_type(&self) -> ProxyConfig {
        self.notification_receiver
            .clone()
            .borrow_and_update()
            .clone()
    }

    pub fn subscribe(&self) -> Receiver<ProxyConfig> {
        self.notification_receiver.clone()
    }
}

pub(crate) fn watch_networks(config: Config) -> NetworkWatchHandle {
    let network_type = Arc::new(Mutex::new(ProxyConfig::Direct));
    let (notification_sender, notification_receiver) =
        tokio::sync::watch::channel::<ProxyConfig>(ProxyConfig::Direct);

    let cloned_network_type = network_type.clone();
    let handle = netwatcher::watch_interfaces(move |update| {
        // This callback will fire once immediately with the existing state

        let current_subnet = config.subnets.iter().find(|(key, _)| match key {
            SubNetKey::Default => true,
            Subnet(subnet) => {
                 update.interfaces.iter().any(|(_, interface)| {
                    interface
                        .ipv4_ips()
                        .any(|ipv4_ip| subnet.contains_ipv4(&ipv4_ip))
                })
            },
        })
            .map(|(_, value)| value)
            .unwrap_or(&Direct);

        if current_subnet.eq(&*cloned_network_type.lock().unwrap()) {
            return;
        }


        if current_subnet.eq(&Direct) {
            println!("📡 Network configuration: Direct");
        } else {
            println!("📡 Network configuration: Proxied");
        }

        {
            let mut network = cloned_network_type.lock().unwrap();
            *network = current_subnet.clone();
        }

        notification_sender.send(current_subnet.clone()).unwrap();
    })
    .unwrap();

    NetworkWatchHandle {
        notification_receiver,
        handle: Arc::new(Mutex::new(handle)),
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