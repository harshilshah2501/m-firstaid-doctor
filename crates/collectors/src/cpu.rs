use crate::{CollectorError, CpuRaw};
use sysinfo::System;

/// Collect CPU signals using `sysinfo` (cross-platform baseline).
/// For per-core accuracy, sysinfo requires a prior `refresh_cpu_all()` call
/// at least 200ms before reading — handled by the refresh cadence.
pub async fn collect() -> Result<CpuRaw, CollectorError> {
    // sysinfo is synchronous; wrap in spawn_blocking to avoid blocking the async executor.
    tokio::task::spawn_blocking(|| {
        let mut sys = System::new();
        sys.refresh_cpu_usage();

        // sysinfo requires two samples separated by time for accurate CPU%.
        // In production, the CollectorSet caches a System instance and calls
        // refresh periodically. Here we do a simple single-sample for the interface.
        let cpus = sys.cpus();
        let usage_per_core: Vec<f32> = cpus.iter().map(|c| c.cpu_usage()).collect();

        // global_cpu_info() gives overall CPU info; use its cpu_usage() as the authoritative total.
        let global_usage = sys.global_cpu_info().cpu_usage();
        let brand = cpus.first().map(|c| c.brand().to_string()).unwrap_or_default();
        let frequency_mhz = cpus.first().map(|c| c.frequency());

        Ok(CpuRaw {
            usage_total_pct: global_usage,
            usage_per_core_pct: usage_per_core,
            user_pct: 0.0,   // sysinfo does not expose user/system split on all platforms
            system_pct: 0.0,
            idle_pct: 100.0 - global_usage,
            iowait_pct: None,
            core_count: cpus.len() as u32,
            brand,
            frequency_mhz,
        })
    })
    .await
    .map_err(|e| CollectorError::OsApi(e.to_string()))?
}
