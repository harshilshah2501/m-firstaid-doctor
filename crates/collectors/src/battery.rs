use crate::{BatteryRaw, CollectorError};

/// Cross-platform battery collection stub.
/// Real implementations live in `collectors-mac` and `collectors-win`.
/// This returns None on unsupported platforms.
pub async fn collect() -> Result<Option<BatteryRaw>, CollectorError> {
    #[cfg(target_os = "macos")]
    {
        // Delegated to collectors-mac via platform adapter
        return Ok(None);
    }
    #[cfg(target_os = "windows")]
    {
        // Delegated to collectors-win via WMI Win32_Battery
        return Ok(None);
    }
    #[allow(unreachable_code)]
    Ok(None)
}
