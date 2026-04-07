//! macOS battery collection via `pmset -g batt` and `system_profiler SPPowerDataType`.

use collectors::{BatteryRaw, CollectorError};

pub async fn collect() -> Result<BatteryRaw, CollectorError> {
    let output = tokio::process::Command::new("pmset")
        .args(["-g", "batt"])
        .output()
        .await
        .map_err(|e| CollectorError::OsApi(e.to_string()))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_pmset_batt(&stdout)
        .ok_or_else(|| CollectorError::Parse("Could not parse pmset -g batt output".into()))
}

fn parse_pmset_batt(output: &str) -> Option<BatteryRaw> {
    // Example output line:
    // -InternalBattery-0 (id=xxx)    61%; discharging; 1:27 remaining
    let mut charge_pct = 0.0f32;
    let mut is_charging = false;
    let mut is_plugged_in = false;
    let mut time_remaining = None;

    for line in output.lines() {
        let lower = line.to_lowercase();
        if lower.contains("battery") || lower.contains("internalbattery") {
            if lower.contains("charging") { is_charging = true; }
            if lower.contains("ac power") || lower.contains("plugged") { is_plugged_in = true; }

            // Parse percentage: "61%"
            if let Some(pct_str) = line.split(';').next() {
                if let Some(pct) = pct_str.split('%').next()
                    .and_then(|s| s.split_whitespace().last())
                    .and_then(|s| s.parse::<f32>().ok()) {
                    charge_pct = pct;
                }
            }

            // Parse time remaining: "1:27 remaining"
            for part in line.split(';') {
                if part.contains("remaining") {
                    if let Some(time_str) = part.trim().split_whitespace().next() {
                        let parts: Vec<&str> = time_str.split(':').collect();
                        if parts.len() == 2 {
                            if let (Ok(h), Ok(m)) = (parts[0].parse::<i32>(), parts[1].parse::<i32>()) {
                                time_remaining = Some(h * 60 + m);
                            }
                        }
                    }
                }
            }
        }

        if lower.contains("ac power") { is_plugged_in = true; }
    }

    Some(BatteryRaw {
        charge_pct,
        is_charging,
        is_plugged_in,
        current_capacity_mah: None,  // Requires IOKit in Phase 2
        max_capacity_mah: None,
        design_capacity_mah: None,
        cycle_count: None,           // Requires IOKit or system_profiler
        current_ma: None,
        voltage_mv: None,
        temperature_celsius: None,
        time_remaining_minutes: time_remaining,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_discharging() {
        let output = "Now drawing from 'Battery Power'\n -InternalBattery-0 (id=4522019)   61%; discharging; 1:27 remaining present: true\n";
        let batt = parse_pmset_batt(output).unwrap();
        assert!((batt.charge_pct - 61.0).abs() < 0.1);
        assert!(!batt.is_charging);
        assert_eq!(batt.time_remaining_minutes, Some(87));
    }
}
