//! WMI-based collectors for Windows.
//! Phase 1: PowerShell subprocess fallback.
//! Phase 2: Direct WMI via the `wmi` crate.

use collectors::CollectorError;

/// Query battery info via WMI Win32_Battery.
pub async fn query_battery() -> Result<Option<collectors::BatteryRaw>, CollectorError> {
    // PowerShell: Get-WmiObject -Class Win32_Battery | Select-Object EstimatedChargeRemaining
    let output = tokio::process::Command::new("powershell")
        .args([
            "-NonInteractive", "-NoProfile", "-Command",
            "Get-WmiObject -Class Win32_Battery | Select-Object -Property EstimatedChargeRemaining,BatteryStatus | ConvertTo-Json"
        ])
        .output()
        .await
        .map_err(|e| CollectorError::OsApi(e.to_string()))?;

    if !output.status.success() || output.stdout.is_empty() {
        return Ok(None);  // No battery (desktop)
    }

    let json: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| CollectorError::Parse(e.to_string()))?;

    let charge_pct = json["EstimatedChargeRemaining"].as_f64().unwrap_or(0.0) as f32;
    // BatteryStatus: 2 = Plugged in and charging, 1 = Discharging
    let battery_status = json["BatteryStatus"].as_u64().unwrap_or(0);
    let is_charging = battery_status == 2;
    let is_plugged = battery_status != 1;

    Ok(Some(collectors::BatteryRaw {
        charge_pct,
        is_charging,
        is_plugged_in: is_plugged,
        current_capacity_mah: None,
        max_capacity_mah: None,
        design_capacity_mah: None,
        cycle_count: None,
        current_ma: None,
        voltage_mv: None,
        temperature_celsius: None,
        time_remaining_minutes: None,
    }))
}
