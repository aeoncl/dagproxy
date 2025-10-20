use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use netaddr2::{Mask, Netv4Addr};
use netwatcher::WatchHandle;
use tokio::sync::watch::Receiver;

#[derive(Clone)]
pub(crate) struct NetworkWatchHandle {
    network_type: Arc<Mutex<NetworkType>>,
    notification_receiver: Receiver<NetworkType>,
    handle: Arc<Mutex<WatchHandle>>,
}

impl NetworkWatchHandle {

    pub fn network_type(&self) -> NetworkType {
        self.notification_receiver.clone().borrow_and_update().clone()
    }

    pub fn subscribe(&self) -> Receiver<NetworkType> {
        self.notification_receiver.clone()
    }

}

pub(crate) fn watch_networks(subnets: Vec<Netv4Addr>) -> NetworkWatchHandle {

    let network_type = Arc::new(Mutex::new(NetworkType::Direct));
    let (notification_sender, notification_receiver) = tokio::sync::watch::channel::<NetworkType>(NetworkType::Direct);


    let cloned_network_type = network_type.clone();
    let handle = netwatcher::watch_interfaces(move |update| {
        // This callback will fire once immediately with the existing state

        let is_in_subnet = update.interfaces.iter().any(|(_, interface)| {
            interface.ipv4_ips().any(|ipv4_ip| {
                subnets.iter().any(|subnet| {
                    subnet.contains_ipv4(&ipv4_ip)
                })
            })
        });

        
        
        let needs_updates = {
            let mut network = cloned_network_type.lock().unwrap();
            
            if is_in_subnet && !matches!(*network, NetworkType::Proxied) {
                *network = NetworkType::Proxied;
                true
            } else if !is_in_subnet && !matches!(*network, NetworkType::Direct){
                *network = NetworkType::Direct;
                true
            } else {
                false 
            }
        };

        if needs_updates {
            if is_in_subnet {
                println!("📡 Network configuration: Proxied");
                notification_sender.send(NetworkType::Proxied).unwrap();
            } else {
                println!("📡 Network configuration: Direct");
                notification_sender.send(NetworkType::Direct).unwrap();
            }
        }
        
      
    }).unwrap();

    NetworkWatchHandle {
        network_type,
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

#[derive(Clone)]
pub(crate) enum NetworkType {
    Direct,
    Proxied
}