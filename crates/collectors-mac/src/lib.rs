//! macOS-specific collectors using IOKit, OSLog, and launchctl.
//!
//! These supplement the cross-platform `collectors` crate with signals
//! only available through macOS-native APIs.

#[cfg(target_os = "macos")]
pub mod battery;
#[cfg(target_os = "macos")]
pub mod crash_logs;
#[cfg(target_os = "macos")]
pub mod memory_pressure;
#[cfg(target_os = "macos")]
pub mod network_extra;
#[cfg(target_os = "macos")]
pub mod startup;
#[cfg(target_os = "macos")]
pub mod thermal;

/// Enrich a RawSnapshot with macOS-specific signals.
/// Called by CollectorSet on macOS after the cross-platform collection completes.
#[cfg(target_os = "macos")]
pub async fn enrich(snapshot: &mut collectors::RawSnapshot) {
    use tracing::warn;

    // Memory pressure (IOKit kIOResourceMemoryPressure)
    match memory_pressure::collect().await {
        Ok(level) => {
            snapshot.memory.macos_pressure_level = Some(level);
        }
        Err(e) => warn!(error = %e, "macOS memory pressure collection failed"),
    }

    // Thermal state and temperatures (IOKit SMC)
    match thermal::collect().await {
        Ok(t) => snapshot.thermal = Some(t),
        Err(e) => warn!(error = %e, "macOS thermal collection failed"),
    }

    // Battery (IOKit IOPSCopyPowerSourcesInfo)
    match battery::collect().await {
        Ok(b) => snapshot.battery = Some(b),
        Err(e) => warn!(error = %e, "macOS battery collection failed"),
    }

    // Launch agents/daemons count
    match startup::count_launch_agents().await {
        Ok((agents, daemons, login_items)) => {
            let last_crash_log_text = crash_logs::collect_latest().await
                .unwrap_or(None);
            snapshot.platform_specific.macos = Some(collectors::MacosSpecificRaw {
                launch_agents_count: agents,
                launch_daemons_count: daemons,
                login_items_count: login_items,
                spotlight_indexing: false,  // TODO
                timemachine_active: false,  // TODO
                last_crash_log_text,
            });
        }
        Err(e) => warn!(error = %e, "macOS launch agent count failed"),
    }

    // Network extras (WiFi signal, DNS servers via SystemConfiguration)
    if let Ok(extra) = network_extra::collect().await {
        for iface in &mut snapshot.network.interfaces {
            if iface.name == extra.wifi_interface {
                iface.wifi_ssid = extra.ssid.clone();
                iface.wifi_signal_dbm = extra.signal_dbm;
                iface.wifi_noise_dbm = extra.noise_dbm;
                iface.ipv4 = extra.ipv4.clone();
            }
        }
        snapshot.network.dns_servers = extra.dns_servers;
    }
}
