# DagProxy

DagProxy is a local kerberos auth forwarding proxy built in Rust.

It's main purpose is to make the system proxy configuration hotswappable, allowing you to switch between multiple proxy configurations without restarting your apps.

It also handle proxy auth refresh using kerberos SP-NEGO protocol.

## Capabilities 
- [x] SPNEGO proxy auth using host kerberos session
- [x] Toggle between direct and proxied network

## Usage

`dagproxy [path_to_config-file.json]`

## Config File

Dag proxy will detect the ip range you are in and use the appropriate subnet configuration.

```json
{
  "port": 3232,
  "subnets": [
    {
      "ip_range": "10.0.0.0/24",
      "proxy_host": "upstream.proxy",
      "proxy_port": 8080,
      "name": "Corporate VPN",
      "rules": {
        "subdomain.example.com": "DIRECT",
        "*": "PROXY"
      }
    },
    {
      "ip_range": "11.0.0.0/24",
      "proxy_host": "upstream.proxy",
      "proxy_port": 8080,
      "name": "Corporate Office",
      "rules": {
        "subdomain.example.com": "DIRECT",
        "*": "PROXY"
      }
    }
  ]
}

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
ExecStart=/home/user/.local/bin/dagproxy -c /home/user/.config/dagproxy/config.json
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

## Package a .deb 

1. Build the linux binary
2. Create the .deb package using cargo-deb

```
cargo install cargo-deb
# cd in the project's root directory
cargo deb
```

## Testing the .deb

```
sudo dpkg -i {{path/to/deb}}
```

## Uninstalling the .deb

```
sudo dpkg -P dagproxy
```

## Build on Windows

```
# Windows exe
cargo build --release

```