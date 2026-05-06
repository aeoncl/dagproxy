use crate::config::{Config, SubnetConfig};
use std::sync::{Arc, Mutex};
use netwatcher::WatchHandle;
use tokio::sync::watch::Receiver;

#[derive(Clone)]
pub(crate) struct NetworkWatchHandle {
    notification_receiver: Receiver<SubnetConfig>,
    #[allow(dead_code)]
    handle: Arc<Mutex<WatchHandle>>,
}

impl NetworkWatchHandle {
    pub fn subnet_config(&self) -> SubnetConfig {
        self.notification_receiver
            .clone()
            .borrow_and_update()
            .clone()
    }

    pub fn subscribe(&self) -> Receiver<SubnetConfig> {
        self.notification_receiver.clone()
    }
}

pub(crate) fn watch_networks(config: Config) -> NetworkWatchHandle {
    let current_subnet_config: Arc<Mutex<SubnetConfig>> = Arc::new(Mutex::new(Default::default()));
    let (notification_sender, notification_receiver) =
        tokio::sync::watch::channel::<SubnetConfig>(Default::default());

    let cloned_current_subnet_config = current_subnet_config.clone();
    let handle = netwatcher::watch_interfaces_with_callback(move |update| {
        // This callback will fire once immediately with the existing state

        let found_subnet = update.interfaces.iter().find_map(|(_, interface)| {
            interface
                .ipv4_ips()
                .find_map(|ipv4_ip| config.get_matching_config(&ipv4_ip))
        }).unwrap_or_default();


        if found_subnet.eq(&*cloned_current_subnet_config.lock().unwrap()) {
            return;
        }

        println!("📡 Network configuration: {:?}", found_subnet.ip_range);

        {
            let mut network = cloned_current_subnet_config.lock().unwrap();
            *network = found_subnet.clone();
        }

        notification_sender.send(found_subnet.clone()).unwrap();
    }).unwrap();

    NetworkWatchHandle {
        notification_receiver,
        handle: Arc::new(Mutex::new(handle)),
    }
}