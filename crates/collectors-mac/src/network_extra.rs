//! macOS network extras: WiFi signal, DNS servers via SystemConfiguration.
//! Phase 1: subprocess-based using `networksetup` and `airport`.

use collectors::CollectorError;

pub struct MacNetworkExtra {
    pub wifi_interface: String,
    pub ssid: Option<String>,
    pub signal_dbm: Option<i32>,
    pub noise_dbm: Option<i32>,
    pub ipv4: Option<String>,
    pub dns_servers: Vec<String>,
}

pub async fn collect() -> Result<MacNetworkExtra, CollectorError> {
    let (wifi, dns) = tokio::join!(collect_wifi(), collect_dns());

    let (ssid, signal_dbm, noise_dbm) = wifi.unwrap_or((None, None, None));
    let dns_servers = dns.unwrap_or_default();

    Ok(MacNetworkExtra {
        wifi_interface: "en0".into(),
        ssid,
        signal_dbm,
        noise_dbm,
        ipv4: collect_ipv4("en0").await.ok(),
        dns_servers,
    })
}

async fn collect_wifi() -> Result<(Option<String>, Option<i32>, Option<i32>), CollectorError> {
    let output = tokio::process::Command::new("/System/Library/PrivateFrameworks/Apple80211.framework/Versions/Current/Resources/airport")
        .arg("-I")
        .output()
        .await
        .map_err(|e| CollectorError::OsApi(e.to_string()))?;

    let text = String::from_utf8_lossy(&output.stdout);
    let mut ssid = None;
    let mut signal = None;
    let mut noise = None;

    for line in text.lines() {
        let parts: Vec<&str> = line.splitn(2, ':').collect();
        if parts.len() != 2 { continue; }
        let key = parts[0].trim();
        let val = parts[1].trim();
        match key {
            "SSID"   => ssid = Some(val.to_string()),
            "agrCtlRSSI" => signal = val.parse::<i32>().ok(),
            "agrCtlNoise" => noise = val.parse::<i32>().ok(),
            _ => {}
        }
    }

    Ok((ssid, signal, noise))
}

async fn collect_dns() -> Result<Vec<String>, CollectorError> {
    let output = tokio::process::Command::new("networksetup")
        .args(["-getdnsservers", "Wi-Fi"])
        .output()
        .await
        .map_err(|e| CollectorError::OsApi(e.to_string()))?;

    let text = String::from_utf8_lossy(&output.stdout);
    let servers: Vec<String> = text.lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty() && !l.contains("There aren"))
        .collect();

    Ok(servers)
}

async fn collect_ipv4(interface: &str) -> Result<String, CollectorError> {
    let output = tokio::process::Command::new("ipconfig")
        .args(["getifaddr", interface])
        .output()
        .await
        .map_err(|e| CollectorError::OsApi(e.to_string()))?;

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
