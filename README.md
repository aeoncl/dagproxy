# DagProxy

DagProxy is a kerberos auth proxy. 
It's main purpose is to deal with an annoying corporate proxy with expiring kerberos sessions every 15 minutes.

## Capabilities 
- [x] SPNEGO proxy auth using host kerberos session
- [x] Toggle between direct and proxied network
- [x] Validated on Linux
- [ ] Validated on Windows
- [ ] Transparent proxying using SSL inspection & port redirection

## Usage

```
Usage:
        dagproxy [options]

Options:
        --corporate-subnets <subnet1>,<subnet2>..... Forwards trafic to the upstream proxy when on one of those subnets
        --upstream-proxy <host>:<port>.............. The upstream proxy to forward traffic to
        --listen-port <port>........................ The port to listen on for HTTP traffic. Defaults to 3232
        --listen-port-https <port>.................. The port to listen on for HTTPS traffic. Defaults to 3233. Only used when --transparent is set.
        --no-proxy <host1>,<host2>.................. Hosts to not proxy. Defaults to none.
        --transparent............................... Use transparent proxying. Defaults to false. This will require you to install a certificate on your machine.
        --help...................................... Print this help message
```

## Example

```
dagproxy --corporate-subnets 0.0.0.0/16,10.10.0.0/16 --upstream-proxy 'annoyingproxy.host.internal:7777'
```

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