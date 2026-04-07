use crate::{CollectorError, NetworkInterfaceRaw, NetworkRaw};
use sysinfo::Networks;

pub async fn collect() -> Result<NetworkRaw, CollectorError> {
    tokio::task::spawn_blocking(|| {
        let networks = Networks::new_with_refreshed_list();
        let interfaces = networks
            .iter()
            .filter(|(_, data)| !data.mac_address().is_unspecified())
            .map(|(name, data)| NetworkInterfaceRaw {
                name: name.clone(),
                ipv4: None,         // sysinfo doesn't expose IPs; platform adapters fill this
                ipv6: None,
                mac: Some(data.mac_address().to_string()),
                rx_bytes: data.total_received(),
                tx_bytes: data.total_transmitted(),
                wifi_ssid: None,        // platform adapter
                wifi_signal_dbm: None,  // platform adapter
                wifi_noise_dbm: None,   // platform adapter
                is_loopback: name == "lo" || name == "lo0" || name == "Loopback Pseudo-Interface 1",
                is_up: true,            // sysinfo only lists active interfaces
            })
            .collect();

        Ok(NetworkRaw {
            interfaces,
            dns_servers: Vec::new(), // platform adapter fills this
        })
    })
    .await
    .map_err(|e| CollectorError::OsApi(e.to_string()))?
}
