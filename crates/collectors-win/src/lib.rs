//! Windows-specific collectors using WMI, ETW, Event Log, and Registry.
//!
//! Phase 1: subprocess-based fallbacks using PowerShell and wmic.
//! Phase 2: direct Win32 API calls via the `windows` crate.

#[cfg(target_os = "windows")]
pub mod event_log;
#[cfg(target_os = "windows")]
pub mod wmi_collector;
#[cfg(target_os = "windows")]
pub mod startup;
#[cfg(target_os = "windows")]
pub mod network_extra;

/// Enrich a RawSnapshot with Windows-specific signals.
#[cfg(target_os = "windows")]
pub async fn enrich(snapshot: &mut collectors::RawSnapshot) {
    use tracing::warn;

    // Pending reboot flag (Windows Update)
    match check_pending_reboot().await {
        Ok(pending) => {
            if let Some(w) = &mut snapshot.platform_specific.windows {
                w.pending_reboot_registry = pending;
            } else {
                snapshot.platform_specific.windows = Some(collectors::WindowsSpecificRaw {
                    run_key_items_count: 0,
                    task_scheduler_items_count: None,
                    windows_defender_status: None,
                    pending_reboot_registry: pending,
                    wua_update_count: None,
                    event_log_crash_count_24h: None,
                });
            }
        }
        Err(e) => warn!(error = %e, "Windows pending reboot check failed"),
    }

    // Startup items from Registry Run keys
    match startup::count_run_keys().await {
        Ok(count) => {
            if let Some(w) = &mut snapshot.platform_specific.windows {
                w.run_key_items_count = count;
            }
        }
        Err(e) => warn!(error = %e, "Windows startup items count failed"),
    }

    // DNS servers via ipconfig
    match network_extra::collect_dns().await {
        Ok(dns) => snapshot.network.dns_servers = dns,
        Err(e) => warn!(error = %e, "Windows DNS collection failed"),
    }
}

#[cfg(target_os = "windows")]
async fn check_pending_reboot() -> Result<bool, collectors::CollectorError> {
    // Check HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\WindowsUpdate\Auto Update\RebootRequired
    let output = tokio::process::Command::new("reg")
        .args([
            "query",
            "HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\WindowsUpdate\\Auto Update\\RebootRequired",
        ])
        .output()
        .await
        .map_err(|e| collectors::CollectorError::OsApi(e.to_string()))?;

    // If the key exists, reboot is pending
    Ok(output.status.success())
}
