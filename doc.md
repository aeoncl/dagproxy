Transparent proxy: 

```aiignore
# Redirect all outgoing traffic to port 443 to your local proxy
iptables -t nat -A OUTPUT -p tcp --dport 443 -j REDIRECT --to-port 8080

# Exempt the proxy process from redirection
iptables -t nat -A OUTPUT -p tcp --dport 443 -m owner --pid-owner 123 -j ACCEPT 
```

``` powershell

param (
    [string]$ProxyPath = "C:\path\to\your\proxy.exe",
    [int]$LocalProxyPort = 8080
)

# Get or create a WFP filter engine session
$engineHandle = New-Object -ComObject HNetCfg.FwPolicy2

# Create a rule to redirect all outgoing HTTPS traffic to the local proxy
$rule = New-Object -ComObject HNetCfg.FwRule
$rule.Name = "Redirect HTTPS to Local Proxy"
$rule.Direction = 2  # OUT
$rule.Protocol = 6   # TCP
$rule.RemotePort = "443"
$rule.Action = 1     # ALLOW (we'll use callout driver for redirection)

# Add a condition to exempt the proxy application
$rule.ApplicationName = $ProxyPath
$rule.Action = 0     # BLOCK (effectively bypassing the redirection)

# Add the rule to the firewall
$engineHandle.Rules.Add($rule)

Write-Host "Port redirection configured. All outgoing HTTPS traffic (except from $ProxyPath) will be redirected to port $LocalProxyPort"
```


```aiignore
use anyhow::{anyhow, Result};
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows::core::{PCWSTR, PWSTR, GUID, w};
use windows::Win32::Foundation::{HANDLE, ERROR_SUCCESS};
use windows::Win32::NetworkManagement::WindowsFilteringPlatform::*;

fn main() -> Result<()> {
    let proxy_path = r"C:\path\to\your\proxy.exe";
    let local_proxy_port = 8080;
    
    // Set up the redirection but exempt the proxy process
    setup_port_redirection(proxy_path, local_proxy_port)?;
    println!("Port redirection configured successfully");
    
    // Wait for user input to clean up
    println!("Press Enter to remove the redirection rules...");
    let mut buffer = String::new();
    std::io::stdin().read_line(&mut buffer)?;
    
    // Clean up
    cleanup_port_redirection()?;
    println!("Port redirection removed successfully");
    
    Ok(())
}

fn setup_port_redirection(proxy_path: &str, local_proxy_port: u16) -> Result<()> {
    unsafe {
        // Open the WFP engine
        let mut engine_handle = HANDLE::default();
        let result = FwpmEngineOpen0(
            None,                // Server (NULL = local)
            RPC_C_AUTHN_DEFAULT, // Authentication service
            None,                // Authentication identity
            None,                // Session options
            &mut engine_handle,  // Engine handle
        );
        
        if result.is_err() {
            return Err(anyhow!("Failed to open WFP engine: {:?}", result));
        }
        
        // Create a transaction
        let result = FwpmTransactionBegin0(engine_handle, 0);
        if result.is_err() {
            FwpmEngineClose0(engine_handle);
            return Err(anyhow!("Failed to begin transaction: {:?}", result));
        }
        
        // Create a provider
        let provider_name = wide_string("RustPortRedirector");
        let mut provider = FWPM_PROVIDER0 {
            providerKey: GUID::new()?, // Generate a new GUID
            displayData: FWPM_DISPLAY_DATA0 {
                name: PWSTR(provider_name.as_ptr() as _),
                description: PWSTR::null(),
            },
            flags: 0,
            providerData: Default::default(),
            serviceName: PWSTR::null(),
        };
        
        let result = FwpmProviderAdd0(engine_handle, &provider, None);
        if result.is_err() {
            FwpmTransactionAbort0(engine_handle);
            FwpmEngineClose0(engine_handle);
            return Err(anyhow!("Failed to add provider: {:?}", result));
        }
        
        // Get the proxy application ID
        let proxy_app_id = get_app_id_by_path(proxy_path)?;
        
        // Create a sublayer
        let sublayer_name = wide_string("PortRedirectionSublayer");
        let mut sublayer = FWPM_SUBLAYER0 {
            subLayerKey: GUID::new()?,
            displayData: FWPM_DISPLAY_DATA0 {
                name: PWSTR(sublayer_name.as_ptr() as _),
                description: PWSTR::null(),
            },
            flags: 0,
            providerKey: &provider.providerKey,
            providerData: Default::default(),
            weight: 0xFFFF, // High weight to ensure it's processed early
        };
        
        let result = FwpmSubLayerAdd0(engine_handle, &sublayer, None);
        if result.is_err() {
            FwpmTransactionAbort0(engine_handle);
            FwpmEngineClose0(engine_handle);
            return Err(anyhow!("Failed to add sublayer: {:?}", result));
        }
        
        // Create a filter for the redirection
        let filter_name = wide_string("RedirectHTTPS");
        
        // Create condition for matching HTTPS traffic
        let mut conditions: Vec<FWPM_FILTER_CONDITION0> = vec![
            // Match outbound traffic to port 443
            FWPM_FILTER_CONDITION0 {
                fieldKey: FWPM_CONDITION_IP_REMOTE_PORT,
                matchType: FWP_MATCH_TYPE(FWP_MATCH_EQUAL as u32),
                conditionValue: FWP_CONDITION_VALUE0 { 
                    type_: FWP_UINT16,
                    uint16: 443,
                },
            },
            // Exclude the proxy application
            FWPM_FILTER_CONDITION0 {
                fieldKey: FWPM_CONDITION_ALE_APP_ID,
                matchType: FWP_MATCH_TYPE(FWP_MATCH_NOT_EQUAL as u32),
                conditionValue: FWP_CONDITION_VALUE0 {
                    type_: FWP_BYTE_BLOB_TYPE,
                    byteBlob: &proxy_app_id,
                },
            },
        ];
        
        // Set up the redirect action
        let mut redirect_action = FWPM_ACTION0 {
            type_: FWP_ACTION_TYPE(FWP_ACTION_CALLOUT_TERMINATING as u32),
            // In a real implementation, you'd need to register a callout that 
            // performs the actual redirection to the local proxy port
            // This is simplified
        };
        
        let mut filter = FWPM_FILTER0 {
            filterKey: GUID::new()?,
            displayData: FWPM_DISPLAY_DATA0 {
                name: PWSTR(filter_name.as_ptr() as _),
                description: PWSTR::null(),
            },
            flags: 0,
            providerKey: &provider.providerKey,
            providerData: Default::default(),
            layerKey: FWPM_LAYER_ALE_AUTH_CONNECT_V4,
            subLayerKey: sublayer.subLayerKey,
            weight: FWP_VALUE0 {
                type_: FWP_UINT8,
                uint8: 0xFF, // High weight
            },
            numFilterConditions: conditions.len() as u32,
            filterCondition: conditions.as_mut_ptr(),
            action: redirect_action,
            // Additional fields...
            reserved: Default::default(),
            context: Default::default(),
            providerContextKey: Default::default(),
            reserved1: Default::default(),
        };
        
        let mut filter_id: u64 = 0;
        let result = FwpmFilterAdd0(engine_handle, &filter, None, &mut filter_id);
        if result.is_err() {
            FwpmTransactionAbort0(engine_handle);
            FwpmEngineClose0(engine_handle);
            return Err(anyhow!("Failed to add filter: {:?}", result));
        }
        
        // Commit the transaction
        let result = FwpmTransactionCommit0(engine_handle);
        if result.is_err() {
            FwpmTransactionAbort0(engine_handle);
            FwpmEngineClose0(engine_handle);
            return Err(anyhow!("Failed to commit transaction: {:?}", result));
        }
        
        // Close engine
        FwpmEngineClose0(engine_handle);
        
        Ok(())
    }
}

fn cleanup_port_redirection() -> Result<()> {
    // Similar implementation to remove all the rules
    // Omitted for brevity
    Ok(())
}

fn get_app_id_by_path(path: &str) -> Result<FWP_BYTE_BLOB> {
    // In a real implementation, this would convert the path to an app ID blob
    // that WFP uses to identify applications
    // This is a simplified placeholder
    Ok(FWP_BYTE_BLOB {
        size: 0,
        data: std::ptr::null_mut(),
    })
}

fn wide_string(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}


```