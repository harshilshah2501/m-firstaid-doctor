use crate::{CollectorError, MemoryRaw};
use sysinfo::System;

pub async fn collect() -> Result<MemoryRaw, CollectorError> {
    tokio::task::spawn_blocking(|| {
        let mut sys = System::new();
        sys.refresh_memory();

        Ok(MemoryRaw {
            total_bytes: sys.total_memory(),
            available_bytes: sys.available_memory(),
            used_bytes: sys.used_memory(),
            swap_total_bytes: sys.total_swap(),
            swap_used_bytes: sys.used_swap(),
            // Platform-specific pressure fields are populated by platform adapters.
            macos_pressure_level: None,
            windows_commit_charge_bytes: None,
            windows_commit_limit_bytes: None,
        })
    })
    .await
    .map_err(|e| CollectorError::OsApi(e.to_string()))?
}
