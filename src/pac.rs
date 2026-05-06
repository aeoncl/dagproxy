use std::{
    fmt::format,
    mem,
    net::{Ipv4Addr, UdpSocket},
    sync::{Arc, Mutex, RwLock},
    time::Duration,
};

use std::net::ToSocketAddrs;

use boa_engine::{
    Context, JsNativeError, JsResult, JsString, JsValue, NativeFunction, Source, js_string,
};
use chrono::{Datelike, Utc, Weekday};
use regex::Regex;
use reqwest::blocking::Client;
use tokio::{sync::watch::Receiver, time};

use crate::{config::ForwardPolicy, http::NetworkTarget, network_watcher::NetworkWatchTransport};

#[derive(Clone)]
pub struct PacMatcher {
    pac_file_content: String,
}

impl PacMatcher {
    pub fn new(content: String) -> Self {
        Self {
            pac_file_content: content,
        }
    }

    pub fn resolve_forward_policy(
        &self,
        target: &NetworkTarget,
    ) -> Result<ForwardPolicy, anyhow::Error> {
        let url = format!("http://{}", target);
        let host = target.host();
        let result = execute_pac_script(&self.pac_file_content, &url, host).map_err(|e| {
            anyhow::anyhow!("An error has occured while evaluating PAC script: {}", e)
        })?;

        let forward_policy = if result.starts_with("DIRECT") {
            ForwardPolicy::Direct
        } else {
            ForwardPolicy::Proxy
        };

        Ok(forward_policy)
    }
}

pub fn watch_pac(network_watcher: NetworkWatchTransport) -> Receiver<Option<PacMatcher>> {
    let (pac_matcher_snd, pac_matcher_recv) =
        tokio::sync::watch::channel::<Option<PacMatcher>>(Default::default());

    tokio::spawn(async move {
        let mut network_update_receiver = network_watcher.subscribe();
        let mut timer = time::interval(Duration::from_mins(5));

        let mut pac_url: Option<String> = None;
        loop {
            tokio::select! {
                network_update = network_update_receiver.changed() => {
                    if let Err(e) = network_update {
                        eprintln!("Error receiving network updates: {}", e);
                        break;
                    }

                    let network_type = network_update_receiver.borrow_and_update().clone();
                    pac_url = network_type.pac_url;
                    if let Some(pac_url) = &pac_url {
                        match download_pac(&pac_url) {
                            Ok(pac_content) => {
                                let matcher = PacMatcher::new(pac_content);
                                pac_matcher_snd.send(Some(matcher)).unwrap();
                            },
                            Err(err) => println!("Error: could not download PAC at url: {}. {:?}", &pac_url, err),
                        }
                    }

                }
                interval = timer.tick() => {
                    if let Some(pac_url) = &pac_url {
                        match download_pac(&pac_url) {
                            Ok(pac_content) => {
                                let matcher = PacMatcher::new(pac_content);
                                pac_matcher_snd.send(Some(matcher)).unwrap();
                            },
                            Err(err) => println!("Error: could not download refresh PAC at url: {}. {:?}", &pac_url, err),
                        }
                    }
                }
            }
        }
    });

    pac_matcher_recv
}

// Source: https://github.com/zongyangbigpolo/rustproxyparser

pub fn download_pac(pac_url: &str) -> Result<String, anyhow::Error> {
    println!("Downloading PAC script from: {}", pac_url);

    let client = Client::builder().timeout(Duration::from_secs(15)).build()?;

    let response = client.get(pac_url).send()?;

    if !response.status().is_success() {
        println!("PAC download failed: HTTP {}", response.status());
        return Err(anyhow::anyhow!("HTTP {}", response.status()));
    }

    let text = response.text()?;
    println!("PAC script downloaded successfully ({} bytes)", text.len());
    Ok(text)
}

fn execute_pac_script(script: &str, url: &str, host: &str) -> JsResult<String> {
    let mut context = Context::default();

    register_pac_functions(&mut context)?;

    context.eval(Source::from_bytes(script.as_bytes()))?;

    let global = context.global_object();
    let func_val = global.get(js_string!("FindProxyForURL"), &mut context)?;

    let func = func_val
        .as_callable()
        .ok_or(JsNativeError::typ().with_message("FindProxyForURL is not a function"))?;

    let args = [
        JsValue::from(js_string!(url)),
        JsValue::from(js_string!(host)),
    ];

    let result = func.call(&JsValue::undefined(), &args, &mut context)?;
    let js_str: JsString = result.to_string(&mut context)?;

    Ok(js_str.to_std_string().unwrap_or_default())
}

fn register_pac_functions(context: &mut Context) -> JsResult<()> {
    // isPlainHostName(host)
    context.register_global_callable(
        "isPlainHostName".into(),
        1,
        NativeFunction::from_fn_ptr(|_this, args, _ctx| {
            let host = args
                .get(0)
                .and_then(|v| v.as_string())
                .and_then(|s| s.to_std_string().ok())
                .unwrap_or_default();
            Ok((!host.contains('.')).into())
        }),
    )?;

    // dnsDomainIs(host, domain)
    context.register_global_callable(
        "dnsDomainIs".into(),
        2,
        NativeFunction::from_fn_ptr(|_this, args, _ctx| {
            let host = args
                .get(0)
                .and_then(|v| v.as_string())
                .and_then(|s| s.to_std_string().ok())
                .unwrap_or_default();
            let domain = args
                .get(1)
                .and_then(|v| v.as_string())
                .and_then(|s| s.to_std_string().ok())
                .unwrap_or_default();
            let is_match = host.ends_with(&domain)
                && (host.len() == domain.len()
                    || host.as_bytes()[host.len() - domain.len() - 1] == b'.');
            Ok(is_match.into())
        }),
    )?;

    // localHostOrDomainIs(host, domain)
    context.register_global_callable(
        "localHostOrDomainIs".into(),
        2,
        NativeFunction::from_fn_ptr(|_this, args, _ctx| {
            let host = args
                .get(0)
                .and_then(|v| v.as_string())
                .and_then(|s| s.to_std_string().ok())
                .unwrap_or_default();
            let domain = args
                .get(1)
                .and_then(|v| v.as_string())
                .and_then(|s| s.to_std_string().ok())
                .unwrap_or_default();
            Ok((host == domain || host.ends_with(&format!(".{}", domain))).into())
        }),
    )?;

    // isResolvable(host)
    context.register_global_callable(
        "isResolvable".into(),
        1,
        NativeFunction::from_fn_ptr(|_this, args, _ctx| {
            let host = args
                .get(0)
                .and_then(|v| v.as_string())
                .and_then(|s| s.to_std_string().ok())
                .unwrap_or_default();
            let resolved = (host.as_str(), 0).to_socket_addrs().is_ok();
            Ok(resolved.into())
        }),
    )?;

    // dnsResolve(host)
    context.register_global_callable(
        "dnsResolve".into(),
        1,
        NativeFunction::from_fn_ptr(|_this, args, _ctx| {
            let host = args
                .get(0)
                .and_then(|v| v.as_string())
                .and_then(|s| s.to_std_string().ok())
                .unwrap_or_default();
            let ip = (host.as_str(), 0)
                .to_socket_addrs()
                .ok()
                .and_then(|mut addrs| addrs.next())
                .map(|addr| addr.ip().to_string());
            if let Some(ip_str) = ip {
                Ok(js_string!(ip_str).into())
            } else {
                Ok(JsValue::null())
            }
        }),
    )?;

    // isInNet(ip, net, mask)
    context.register_global_callable(
        "isInNet".into(),
        3,
        NativeFunction::from_fn_ptr(|_this, args, _ctx| {
            let ip_str = args
                .get(0)
                .and_then(|v| v.as_string())
                .and_then(|s| s.to_std_string().ok())
                .unwrap_or_default();
            let net_str = args
                .get(1)
                .and_then(|v| v.as_string())
                .and_then(|s| s.to_std_string().ok())
                .unwrap_or_default();
            let mask_str = args
                .get(2)
                .and_then(|v| v.as_string())
                .and_then(|s| s.to_std_string().ok())
                .unwrap_or_default();

            let ip: Option<Ipv4Addr> = ip_str.parse().ok();
            let net: Option<Ipv4Addr> = net_str.parse().ok();
            let mask: Option<Ipv4Addr> = mask_str.parse().ok();

            if let (Some(ip), Some(net), Some(mask)) = (ip, net, mask) {
                let ip_u32 = u32::from_be_bytes(ip.octets());
                let net_u32 = u32::from_be_bytes(net.octets());
                let mask_u32 = u32::from_be_bytes(mask.octets());
                Ok(((ip_u32 & mask_u32) == (net_u32 & mask_u32)).into())
            } else {
                Ok(false.into())
            }
        }),
    )?;

    // dnsDomainLevels(host)
    context.register_global_callable(
        "dnsDomainLevels".into(),
        1,
        NativeFunction::from_fn_ptr(|_this, args, _ctx| {
            let host = args
                .get(0)
                .and_then(|v| v.as_string())
                .and_then(|s| s.to_std_string().ok())
                .unwrap_or_default();
            let levels = host.matches('.').count() as i32;
            Ok(JsValue::from(levels))
        }),
    )?;

    // myIpAddress()
    context.register_global_callable(
        "myIpAddress".into(),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _ctx| {
            let socket = UdpSocket::bind("0.0.0.0:0").ok();
            let ip = socket
                .as_ref()
                .and_then(|s| s.connect("8.8.8.8:53").ok())
                .and_then(|_| socket.as_ref().and_then(|s| s.local_addr().ok()))
                .map(|addr| addr.ip().to_string())
                .unwrap_or_else(|| "127.0.0.1".to_string());
            Ok(js_string!(ip).into())
        }),
    )?;

    // shExpMatch(str, glob)
    context.register_global_callable(
        "shExpMatch".into(),
        2,
        NativeFunction::from_fn_ptr(|_this, args, _ctx| {
            let str_val = args
                .get(0)
                .and_then(|v| v.as_string())
                .and_then(|s| s.to_std_string().ok())
                .unwrap_or_default();
            let pattern = args
                .get(1)
                .and_then(|v| v.as_string())
                .and_then(|s| s.to_std_string().ok())
                .unwrap_or_default();

            let regex_pattern = pattern
                .replace('*', ".*")
                .replace('?', ".")
                .replace('|', "\\|");

            let re = Regex::new(&format!("^{}$", regex_pattern))
                .unwrap_or_else(|_| Regex::new("^$").unwrap());

            Ok(re.is_match(&str_val).into())
        }),
    )?;

    // weekdayRange(wd1, [wd2], [gmt])
    context.register_global_callable(
        "weekdayRange".into(),
        3, // 最多 3 个参数
        NativeFunction::from_fn_ptr(|_this, args, _ctx| {
            let now = Utc::now();
            let current_wd = match now.weekday() {
                Weekday::Mon => "MON",
                Weekday::Tue => "TUE",
                Weekday::Wed => "WED",
                Weekday::Thu => "THU",
                Weekday::Fri => "FRI",
                Weekday::Sat => "SAT",
                Weekday::Sun => "SUN",
            };

            let wd1 = args
                .get(0)
                .and_then(|v| v.as_string())
                .and_then(|s| s.to_std_string().ok())
                .unwrap_or_default()
                .to_uppercase();

            let wd2 = args
                .get(1)
                .and_then(|v| v.as_string())
                .and_then(|s| s.to_std_string().ok());

            // GMT 参数忽略（大多数 PAC 不依赖）
            if wd2.is_none() {
                return Ok((current_wd == wd1).into());
            }

            let wd2_str = wd2.unwrap().to_uppercase();

            if wd1 == wd2_str {
                Ok((current_wd == wd1).into())
            } else {
                let days = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"];
                let start = days.iter().position(|&d| d == wd1).unwrap_or(0);
                let end = days.iter().position(|&d| d == wd2_str).unwrap_or(0);
                let curr = days.iter().position(|&d| d == current_wd).unwrap_or(0);

                // 如果跨周（例如 FRI 到 MON），简单处理为 false 或 true，根据常见行为
                if start <= end {
                    Ok((curr >= start && curr <= end).into())
                } else {
                    Ok((curr >= start || curr <= end).into())
                }
            }
        }),
    )?;

    context.register_global_callable(
        "dateRange".into(),
        8,
        NativeFunction::from_fn_ptr(|_, _, _| Ok(true.into())),
    )?;
    context.register_global_callable(
        "timeRange".into(),
        6,
        NativeFunction::from_fn_ptr(|_, _, _| Ok(true.into())),
    )?;

    Ok(())
}
