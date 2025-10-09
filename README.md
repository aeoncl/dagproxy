# DagProxy

DagProxy is a kerberos auth proxy. 
It's main purpose is to deal with an annoying corporate proxy with expiring kerberos sessions every 15 minutes.

It can:
- SPNEGO proxy auth using host kerberos session
- Toggle between direct and proxied network

## Usage

```
dagproxy --corporate-subnets 0.0.0.0/16,10.10.0.0/16 --upstream-proxy 'annoyingproxy.host.internal:7777'
```

| params              | description                                              | example                           |
|---------------------|----------------------------------------------------------|-----------------------------------|
| --corporate-subnets | Comma separated list of subnets to detect network change | 0.0.0.0/16,10.10.0.0/16           |
| --upstream-proxy    | Proxy to redirect traffic to when on a corporate subnet  | annoyingproxy.host.internal:7777  |


## Build on Ubuntu

```
sudo apt-get install -y libkrb5-dev
sudo apt-get install -y clang

cargo build --release
```

## Build on Windows

```
cargo build --release
```