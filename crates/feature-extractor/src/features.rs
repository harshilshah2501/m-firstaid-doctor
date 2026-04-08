use serde::{Deserialize, Serialize};
use signal_normalizer::{DiskType, MemoryPressure, ThermalState};
use uuid::Uuid;

/// Normalized, derived feature set consumed by the trigger engine and rules engine.
/// All fields are cross-platform unless noted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureSet {
    pub snapshot_id: Uuid,

    // ── CPU ─────────────────────────────────────────────────────────────────
    pub cpu_usage_avg_1m: f32,
    pub cpu_usage_avg_5m: f32,
    pub cpu_usage_avg_15m: f32,
    pub cpu_core_count: u32,
    pub top_process_cpu_pct: f32,
    pub top_process_name: String,
    pub top_process_pid: Option<u32>,
    pub top_process_is_system: bool,

    // ── Memory ──────────────────────────────────────────────────────────────
    pub memory_total_mb: u64,
    pub memory_available_mb: u64,
    pub memory_used_pct: f32,
    pub swap_used_mb: u64,
    pub swap_used_pct: f32,
    /// macOS-specific; None on Windows
    pub memory_pressure: Option<MemoryPressure>,
    /// True if OOM kill risk is imminent
    pub memory_oom_risk: bool,
    pub top_process_rss_mb: u64,
    pub top_process_mem_name: String,

    // ── Disk ────────────────────────────────────────────────────────────────
    pub system_disk_free_pct: f32,
    pub system_disk_free_bytes: u64,
    pub system_disk_type: Option<DiskType>,
    pub smart_failing: bool,

    // ── Network ─────────────────────────────────────────────────────────────
    pub network_reachable: bool,
    pub wifi_signal_dbm: Option<i32>,
    pub vpn_active: bool,
    pub proxy_configured: bool,
    /// Updated by active DNS probe on trigger
    pub dns_resolution_fail_rate: f32,
    /// Updated by active ping on trigger
    pub default_gateway_ping_ms: Option<f32>,

    // ── Thermal / Battery ────────────────────────────────────────────────────
    pub thermal_state: Option<ThermalState>,
    pub cpu_temp_celsius: Option<f32>,
    pub fan_speed_rpm: Option<u32>,
    pub battery_health_pct: Option<f32>,
    pub battery_cycle_count: Option<u32>,
    pub battery_charge_pct: Option<f32>,

    // ── Startup ──────────────────────────────────────────────────────────────
    pub startup_items_count: u32,

    // ── Crashes ──────────────────────────────────────────────────────────────
    pub app_crash_count_1h: u32,
    pub app_crash_count_24h: u32,
    pub crash_process_name: Option<String>,
    pub crash_exception_type: Option<String>,
    /// First 512 chars of the most-recent crash report (macOS only for now).
    /// Passed to the CharCNN crash classifier when available.
    pub crash_log_text: Option<String>,

    // ── Services ─────────────────────────────────────────────────────────────
    pub service_restart_storm: bool,
    pub service_down_expected: bool,

    // ── Updates ──────────────────────────────────────────────────────────────
    pub pending_reboot_required: bool,
    pub recent_update_detected: bool,
    pub update_failed: bool,
}
