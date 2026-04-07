//! macOS memory pressure collection.
//!
//! The memory pressure level is exposed through IOKit's kIOResourceMemoryPressure.
//! For Phase 1, we use the `memory_pressure` command-line tool as a fallback.
//! Direct IOKit binding (via core-foundation-sys) is planned for Phase 2.

use collectors::CollectorError;

/// Returns macOS memory pressure level: 0=Normal, 1=Warning, 2=Critical, 3=Emergency
pub async fn collect() -> Result<u8, CollectorError> {
    // Phase 1: parse `memory_pressure` command output
    let output = tokio::process::Command::new("memory_pressure")
        .output()
        .await
        .map_err(|e| CollectorError::OsApi(format!("memory_pressure command failed: {}", e)))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_memory_pressure_output(&stdout)
        .ok_or_else(|| CollectorError::Parse("Could not parse memory_pressure output".into()))
}

fn parse_memory_pressure_output(output: &str) -> Option<u8> {
    // Example output:
    // The system memory pressure is currently WARN: 48% of memory in use
    for line in output.lines() {
        let lower = line.to_lowercase();
        if lower.contains("currently") {
            if lower.contains("normal")    { return Some(0); }
            if lower.contains("warn")      { return Some(1); }
            if lower.contains("critical")  { return Some(2); }
            if lower.contains("emergency") { return Some(3); }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_normal() {
        let output = "The system memory pressure is currently NORMAL: 30% of memory in use\n";
        assert_eq!(parse_memory_pressure_output(output), Some(0));
    }

    #[test]
    fn test_parse_warn() {
        let output = "The system memory pressure is currently WARN: 82% of memory in use\n";
        assert_eq!(parse_memory_pressure_output(output), Some(1));
    }

    #[test]
    fn test_parse_critical() {
        let output = "The system memory pressure is currently CRITICAL: 95% of memory in use\n";
        assert_eq!(parse_memory_pressure_output(output), Some(2));
    }
}
