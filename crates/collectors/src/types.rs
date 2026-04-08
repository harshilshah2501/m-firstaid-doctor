//! Raw collector output types.
//! These are uninterpreted, platform-biased types as close to OS APIs as practical.
//! Normalization and unit standardization happens in `signal-normalizer`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ──────────────────────────────────────────────────────────────────────────────
// Top-level snapshot
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawSnapshot {
    pub schema_version: String,
    pub collected_at: DateTime<Utc>,
    pub platform: Platform,
    pub collector_errors: Vec<CollectorErrorEntry>,
    pub cpu: CpuRaw,
    pub memory: MemoryRaw,
    pub disks: Vec<DiskRaw>,
    pub network: NetworkRaw,
    pub processes: Vec<ProcessRaw>,
    pub battery: Option<BatteryRaw>,
    pub thermal: Option<ThermalRaw>,
    pub platform_specific: PlatformSpecificRaw,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectorErrorEntry {
    pub collector: String,
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Macos,
    Windows,
    Linux,
    Unknown,
}

impl Platform {
    pub fn current() -> Self {
        #[cfg(target_os = "macos")]
        return Platform::Macos;
        #[cfg(target_os = "windows")]
        return Platform::Windows;
        #[cfg(target_os = "linux")]
        return Platform::Linux;
        #[allow(unreachable_code)]
        Platform::Unknown
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// CPU
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CpuRaw {
    /// Overall CPU usage 0–100
    pub usage_total_pct: f32,
    /// Per-core usage, 0–100 each
    pub usage_per_core_pct: Vec<f32>,
    pub user_pct: f32,
    pub system_pct: f32,
    pub idle_pct: f32,
    /// Only available on Linux
    pub iowait_pct: Option<f32>,
    pub core_count: u32,
    pub brand: String,
    pub frequency_mhz: Option<u64>,
}

// ──────────────────────────────────────────────────────────────────────────────
// Memory
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemoryRaw {
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub used_bytes: u64,
    pub swap_total_bytes: u64,
    pub swap_used_bytes: u64,
    /// macOS-specific: 0=Normal, 1=Warning, 2=Critical, 3=Emergency
    pub macos_pressure_level: Option<u8>,
    /// Windows-specific: commit charge as bytes
    pub windows_commit_charge_bytes: Option<u64>,
    pub windows_commit_limit_bytes: Option<u64>,
}

// ──────────────────────────────────────────────────────────────────────────────
// Disk
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskRaw {
    pub mount_point: String,
    pub device: String,
    pub fs_type: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
    /// Cumulative read bytes since boot (used for delta calculation)
    pub read_bytes_total: u64,
    /// Cumulative write bytes since boot
    pub write_bytes_total: u64,
    pub is_removable: bool,
    pub disk_type: Option<RawDiskType>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RawDiskType {
    Ssd,
    Hdd,
    Nvme,
    Fusion,
    Unknown,
}

// ──────────────────────────────────────────────────────────────────────────────
// Network
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NetworkRaw {
    pub interfaces: Vec<NetworkInterfaceRaw>,
    pub dns_servers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkInterfaceRaw {
    pub name: String,
    pub ipv4: Option<String>,
    pub ipv6: Option<String>,
    pub mac: Option<String>,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub wifi_ssid: Option<String>,
    /// dBm, typically -30 (excellent) to -90 (poor)
    pub wifi_signal_dbm: Option<i32>,
    pub wifi_noise_dbm: Option<i32>,
    pub is_loopback: bool,
    pub is_up: bool,
}

// ──────────────────────────────────────────────────────────────────────────────
// Processes
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessRaw {
    pub pid: u32,
    pub name: String,
    /// CPU usage 0–100
    pub cpu_pct: f32,
    /// Resident set size in bytes
    pub rss_bytes: u64,
    /// Virtual memory size in bytes
    pub vms_bytes: u64,
    pub status: RawProcessStatus,
    pub parent_pid: Option<u32>,
    pub start_time: Option<DateTime<Utc>>,
    pub exe_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RawProcessStatus {
    Running,
    Sleeping,
    Idle,
    Stopped,
    Zombie,
    Unknown,
}

// ──────────────────────────────────────────────────────────────────────────────
// Battery
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatteryRaw {
    pub charge_pct: f32,
    pub is_charging: bool,
    pub is_plugged_in: bool,
    /// mAh
    pub current_capacity_mah: Option<u32>,
    pub max_capacity_mah: Option<u32>,
    pub design_capacity_mah: Option<u32>,
    pub cycle_count: Option<u32>,
    /// milliamps; negative = discharging
    pub current_ma: Option<i32>,
    pub voltage_mv: Option<u32>,
    pub temperature_celsius: Option<f32>,
    pub time_remaining_minutes: Option<i32>,
}

// ──────────────────────────────────────────────────────────────────────────────
// Thermal
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThermalRaw {
    /// macOS: 0=Normal, 1=Fair, 2=Serious, 3=Critical
    pub macos_thermal_state: Option<u8>,
    pub cpu_temp_celsius: Option<f32>,
    pub gpu_temp_celsius: Option<f32>,
    pub battery_temp_celsius: Option<f32>,
    pub fan_speeds_rpm: Vec<u32>,
}

// ──────────────────────────────────────────────────────────────────────────────
// Platform-specific extras
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PlatformSpecificRaw {
    pub macos: Option<MacosSpecificRaw>,
    pub windows: Option<WindowsSpecificRaw>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MacosSpecificRaw {
    pub launch_agents_count: u32,
    pub launch_daemons_count: u32,
    pub login_items_count: u32,
    /// Whether mds_stores (Spotlight) is actively indexing
    pub spotlight_indexing: bool,
    /// Whether TimeMachine backup is in progress
    pub timemachine_active: bool,
    /// Raw text of the most recent crash report (first 512 chars), for ML classification.
    pub last_crash_log_text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowsSpecificRaw {
    pub run_key_items_count: u32,
    pub task_scheduler_items_count: Option<u32>,
    pub windows_defender_status: Option<String>,
    pub pending_reboot_registry: bool,
    pub wua_update_count: Option<u32>,
    pub event_log_crash_count_24h: Option<u32>,
}
