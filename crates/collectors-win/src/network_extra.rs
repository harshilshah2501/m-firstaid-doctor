//! Windows network extras via ipconfig and netsh.

use collectors::CollectorError;

pub async fn collect_dns() -> Result<Vec<String>, CollectorError> {
    let output = tokio::process::Command::new("powershell")
        .args([
            "-NonInteractive", "-NoProfile", "-Command",
            "Get-DnsClientServerAddress -AddressFamily IPv4 | Select-Object -ExpandProperty ServerAddresses | Sort-Object -Unique | ConvertTo-Json"
        ])
        .output()
        .await
        .map_err(|e| CollectorError::OsApi(e.to_string()))?;

    if output.stdout.is_empty() { return Ok(Vec::new()); }

    let json: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| CollectorError::Parse(e.to_string()))?;

    let servers = match json {
        serde_json::Value::Array(arr) => arr.into_iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .filter(|s| !s.is_empty())
            .collect(),
        serde_json::Value::String(s) => vec![s],
        _ => Vec::new(),
    };

    Ok(servers)
}
