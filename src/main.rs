use dagproxy::config::Config;
use dagproxy::config_dto::ConfigDto;
use dagproxy::http_proxy::HttpProxy;
use dagproxy::network_watcher;
use std::{env, fs};
use tokio::runtime;

fn main() {
    print_header();

    let env_args: Vec<String> = env::args().collect();
    let config_file = env_args
        .windows(2)
        .find_map(|window| {
            if window[0] == "-c" || window[0] == "--config" {
                Some(window[1].clone())
            } else {
                None
            }
        })
        .expect("Missing config file path. Usage: -c <path> or --config <path>");
    println!("Loading configuration from: {}", config_file);

    let config_json = fs::read_to_string(config_file).unwrap();
    let config_dto: ConfigDto = serde_json::from_str(&config_json).unwrap();
    let config: Config = config_dto.try_into().unwrap();

    let rt = runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(async move {
        let (_drop_handle, transport) = network_watcher::watch_networks(config.clone());
        let mut http_proxy = HttpProxy::new(transport);

        http_proxy
            .start("127.0.0.1".to_owned(), config.port)
            .await
            .unwrap();
    });
}

fn print_header() {
    const HEADER: &str = r#"
    ___              ___
   /   \__ _  __ _  / _ \_ __ _____  ___   _
  / /\ / _` |/ _` |/ /_)/ '__/ _ \ \/ / | | |
 / /_// (_| | (_| / ___/| | | (_) >  <| |_| |
/___,' \__,_|\__, \/    |_|  \___/_/\_\\__, |
             |___/                     |___/
    "#;
    println!("{}", HEADER);
}
