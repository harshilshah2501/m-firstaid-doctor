//! macOS thermal state and temperature collection.
//!
//! Implementation strategy:
//! - Thermal state: read from IOKit IOPMCopySystemPowerStatus or parse `pmset -g therm`
//! - CPU temperature: read from SMC keys via IOKit (requires no elevated privileges)
//!
//! For Phase 1, we use the subprocess fallback (`powermetrics --samplers smc -n 1`)
//! which is available without private APIs. Phase 2 adds direct SMC IOKit access.

use collectors::{CollectorError, ThermalRaw};

pub async fn collect() -> Result<ThermalRaw, CollectorError> {
    // Phase 1: use `pmset -g therm` to get thermal state
    let output = tokio::process::Command::new("pmset")
        .args(["-g", "therm"])
        .output()
        .await
        .map_err(|e| CollectorError::OsApi(e.to_string()))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let thermal_state = parse_pmset_thermal(&stdout);

    Ok(ThermalRaw {
        macos_thermal_state: thermal_state,
        cpu_temp_celsius: None,   // Phase 2: direct SMC read
        gpu_temp_celsius: None,
        battery_temp_celsius: None,
        fan_speeds_rpm: Vec::new(),
    })
}

/// Parse thermal level from `pmset -g therm` output.
/// Returns: 0=Normal, 1=Fair, 2=Serious, 3=Critical
fn parse_pmset_thermal(output: &str) -> Option<u8> {
    for line in output.lines() {
        if line.contains("CPU_Speed_Limit") {
            // CPU_Speed_Limit = 100 → Normal
            // CPU_Speed_Limit = 80  → Fair/Throttling
            if let Some(val) = line.split('=').nth(1).and_then(|s| s.trim().parse::<u32>().ok()) {
                return Some(match val {
                    91..=100 => 0,  // Normal
                    71..=90  => 1,  // Fair
                    51..=70  => 2,  // Serious
                    _        => 3,  // Critical
                });
            }
        }
    }
    Some(0)  // default: Normal if we can't parse
}
