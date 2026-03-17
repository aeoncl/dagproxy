# DagProxy

DagProxy is a local kerberos auth forwarding proxy build in Rust. 
It's main purpose is to make the system proxy configuration hotswappable, allowing you to switch between multiple proxy configurations without restarting your apps.
It also handle proxy auth refresh using kerberos SP-NEGO protocol.

## Capabilities 
- [x] SPNEGO proxy auth using host kerberos session
- [x] Toggle between direct and proxied network

## Usage

`dagproxy [path_to_config-file.json]`


## Config File

```
TODO
```

## Usage as a systemd user service

It's important to run it as a user service as it needs to access the `KRB5CCNAME` environment variable.

### User service file 

`/home/user/.config/systemd/user/dagproxy.service`
```
[Unit]
Description=DagProxy
After=network.target

[Service]
Type=simple
ExecStart=/home/user/.local/bin/dagproxy /home/user/.config/dagproxy/config.json
Restart=on-failure
RestartSec=5

[Install]
```

### Kerberos Config
Set the `default_ccache_name` in `/etc/krb5.conf` to avoid aving to restart the service when the KRB token changes location.

```
[libdefaults]
...
default_ccache_name = FILE:/tmp/krb5cc
...
```

## Build on Ubuntu

```
# Linux exe
sudo apt-get install -y libkrb5-dev
`sudo apt-get install -y clang`

cargo build --release

# Windows exe
sudo apt install mingw-w64
rustup target add x86_64-pc-windows-gnu

cargo build --release --target x86_64-pc-windows-gnu
```

## Build on Windows

```
cargo build --release
```
