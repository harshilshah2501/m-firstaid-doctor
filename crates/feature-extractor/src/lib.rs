mod features;
pub use features::FeatureSet;

use signal_normalizer::{NormalizedSignalSnapshot, MemoryPressure, ThermalState};

/// History window for computing rolling averages.
/// The caller passes recent snapshots (newest first).
pub const HISTORY_WINDOW: usize = 10; // 10 × 30s = 5 minutes

/// Number of numeric features in a temporal feature vector.
/// Must stay in sync with `to_feature_vec` and `TEMPORAL_FEATURE_NAMES`.
pub const N_TEMPORAL_FEATURES: usize = 35;

/// Names of temporal features in canonical index order.
/// Must match the training script (`scripts/train_temporal.py`).
pub const TEMPORAL_FEATURE_NAMES: &[&str; N_TEMPORAL_FEATURES] = &[
    "cpu_usage_avg_1m",
    "cpu_usage_avg_5m",
    "cpu_usage_avg_15m",
    "cpu_core_count",
    "top_process_cpu_pct",
    "top_process_is_system",
    "memory_total_mb",
    "memory_available_mb",
    "memory_used_pct",
    "swap_used_mb",
    "swap_used_pct",
    "memory_oom_risk",
    "top_process_rss_mb",
    "system_disk_free_pct",
    "system_disk_free_bytes",
    "smart_failing",
    "network_reachable",
    "wifi_signal_dbm",
    "vpn_active",
    "proxy_configured",
    "dns_resolution_fail_rate",
    "default_gateway_ping_ms",
    "cpu_temp_celsius",
    "fan_speed_rpm",
    "battery_health_pct",
    "battery_cycle_count",
    "battery_charge_pct",
    "startup_items_count",
    "app_crash_count_1h",
    "app_crash_count_24h",
    "service_restart_storm",
    "service_down_expected",
    "pending_reboot_required",
    "recent_update_detected",
    "update_failed",
];

/// Flatten a `FeatureSet` into a fixed-length f32 vector for ML models.
///
/// Layout follows `TEMPORAL_FEATURE_NAMES` index order.
/// Optional fields map to 0.0 when absent (except battery fields → -1.0 to
/// distinguish "not present" from "0% charge").
pub fn to_feature_vec(f: &FeatureSet) -> Vec<f32> {
    vec![
        f.cpu_usage_avg_1m,
        f.cpu_usage_avg_5m,
        f.cpu_usage_avg_15m,
        f.cpu_core_count as f32,
        f.top_process_cpu_pct,
        f.top_process_is_system as u8 as f32,
        f.memory_total_mb as f32,
        f.memory_available_mb as f32,
        f.memory_used_pct,
        f.swap_used_mb as f32,
        f.swap_used_pct,
        f.memory_oom_risk as u8 as f32,
        f.top_process_rss_mb as f32,
        f.system_disk_free_pct,
        f.system_disk_free_bytes as f32,
        f.smart_failing as u8 as f32,
        f.network_reachable as u8 as f32,
        f.wifi_signal_dbm.unwrap_or(0) as f32,
        f.vpn_active as u8 as f32,
        f.proxy_configured as u8 as f32,
        f.dns_resolution_fail_rate,
        f.default_gateway_ping_ms.unwrap_or(0.0),
        f.cpu_temp_celsius.unwrap_or(0.0),
        f.fan_speed_rpm.unwrap_or(0) as f32,
        f.battery_health_pct.unwrap_or(-1.0),
        f.battery_cycle_count.unwrap_or(0) as f32,
        f.battery_charge_pct.unwrap_or(-1.0),
        f.startup_items_count as f32,
        f.app_crash_count_1h as f32,
        f.app_crash_count_24h as f32,
        f.service_restart_storm as u8 as f32,
        f.service_down_expected as u8 as f32,
        f.pending_reboot_required as u8 as f32,
        f.recent_update_detected as u8 as f32,
        f.update_failed as u8 as f32,
    ]
}

/// Extract a `FeatureSet` from a current snapshot and a history window.
/// `history` is ordered newest-to-oldest, not including `current`.
pub fn extract(
    current: &NormalizedSignalSnapshot,
    history: &[NormalizedSignalSnapshot],
) -> FeatureSet {
    let cpu_history: Vec<f32> = std::iter::once(current.cpu.usage_pct)
        .chain(history.iter().map(|s| s.cpu.usage_pct))
        .collect();

    let cpu_avg_1m = rolling_avg(&cpu_history, 2);   // 2 × 30s samples = 1m
    let cpu_avg_5m = rolling_avg(&cpu_history, 10);  // 10 × 30s = 5m
    let cpu_avg_15m = rolling_avg(&cpu_history, 30); // 30 × 30s = 15m (use all available)

    let system_disk = current.disks.iter().find(|d| d.is_system);

    let top_process = current.cpu.top_process.as_ref();

    FeatureSet {
        snapshot_id: current.snapshot_id,

        // CPU
        cpu_usage_avg_1m: cpu_avg_1m,
        cpu_usage_avg_5m: cpu_avg_5m,
        cpu_usage_avg_15m: cpu_avg_15m,
        cpu_core_count: current.cpu.core_count,
        top_process_cpu_pct: top_process.map(|p| p.cpu_pct).unwrap_or(0.0),
        top_process_name: top_process.map(|p| p.name.clone()).unwrap_or_default(),
        top_process_pid: top_process.map(|p| p.pid),
        top_process_is_system: top_process.map(|p| p.is_system).unwrap_or(false),

        // Memory
        memory_total_mb: current.memory.total_mb,
        memory_available_mb: current.memory.available_mb,
        memory_used_pct: current.memory.used_pct,
        swap_used_mb: current.memory.swap_used_mb,
        swap_used_pct: current.memory.swap_used_pct,
        memory_pressure: current.memory.pressure.clone(),
        memory_oom_risk: is_oom_risk(current),
        top_process_rss_mb: current.memory.top_process.as_ref().map(|p| p.rss_mb).unwrap_or(0),
        top_process_mem_name: current.memory.top_process.as_ref().map(|p| p.name.clone()).unwrap_or_default(),

        // Disk
        system_disk_free_pct: system_disk.map(|d| d.free_pct).unwrap_or(100.0),
        system_disk_free_bytes: system_disk.map(|d| d.free_bytes).unwrap_or(u64::MAX),
        system_disk_type: system_disk.and_then(|d| d.disk_type.clone()),
        smart_failing: current.disks.iter().any(|d| {
            d.smart_status.as_ref().map(|s| *s == signal_normalizer::SmartStatus::Failing).unwrap_or(false)
        }),

        // Network
        network_reachable: current.network.reachable,
        wifi_signal_dbm: current.network.wifi_signal_dbm,
        vpn_active: current.network.vpn_active,
        proxy_configured: current.network.proxy_configured,
        dns_resolution_fail_rate: 0.0,     // populated by active probe in trigger engine
        default_gateway_ping_ms: None,     // populated by active probe

        // Thermal / Battery
        thermal_state: current.thermal.as_ref().and_then(|t| t.state.clone()),
        cpu_temp_celsius: current.thermal.as_ref().and_then(|t| t.cpu_temp_celsius),
        fan_speed_rpm: current.thermal.as_ref().and_then(|t| t.fan_speed_rpm),
        battery_health_pct: current.battery.as_ref().and_then(|b| b.health_pct),
        battery_cycle_count: current.battery.as_ref().and_then(|b| b.cycle_count),
        battery_charge_pct: current.battery.as_ref().map(|b| b.charge_pct),

        // Startup
        startup_items_count: current.startup.as_ref().map(|s| s.items_count).unwrap_or(0),

        // Crashes
        app_crash_count_1h: current.crashes.count_1h,
        app_crash_count_24h: current.crashes.count_24h,
        crash_process_name: current.crashes.last_process_name.clone(),
        crash_exception_type: current.crashes.last_exception_type.clone(),
        crash_log_text: current.crashes.last_crash_log_text.clone(),

        // Services
        service_restart_storm: current.services.iter().any(|s| s.restart_count_1h > 10),
        service_down_expected: current.services.iter().any(|s| s.expected_running && !s.running),

        // Updates
        pending_reboot_required: current.updates.pending_reboot_required,
        recent_update_detected: current.updates.recent_update_detected,
        update_failed: current.updates.update_failed,
    }
}

fn rolling_avg(values: &[f32], window: usize) -> f32 {
    let n = values.len().min(window);
    if n == 0 { return 0.0; }
    values[..n].iter().sum::<f32>() / n as f32
}

fn is_oom_risk(s: &NormalizedSignalSnapshot) -> bool {
    s.memory.available_mb < 150
        || s.memory.pressure.as_ref().map(|p| *p >= MemoryPressure::Critical).unwrap_or(false)
}
