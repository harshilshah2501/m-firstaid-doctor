//! Normalized signal snapshot — the lingua franca of the system.
//! All downstream components consume this type, never RawSnapshot directly.

use chrono::{DateTime, Utc};
use collectors::Platform;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedSignalSnapshot {
    pub schema_version: String,
    pub snapshot_id: Uuid,
    pub collected_at: DateTime<Utc>,
    pub normalized_at: DateTime<Utc>,
    pub platform: Platform,
    pub cpu: NormalizedCpu,
    pub memory: NormalizedMemory,
    pub disks: Vec<NormalizedDisk>,
    pub network: NormalizedNetwork,
    pub thermal: Option<NormalizedThermal>,
    pub battery: Option<NormalizedBattery>,
    pub startup: Option<NormalizedStartup>,
    pub updates: NormalizedUpdates,
    pub crashes: NormalizedCrashes,
    pub services: Vec<NormalizedService>,
}

// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedCpu {
    /// Current total CPU usage 0–100.0
    pub usage_pct: f32,
    pub core_count: u32,
    pub user_pct: f32,
    pub system_pct: f32,
    pub idle_pct: f32,
    pub top_process: Option<NormalizedProcessRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedProcessRef {
    pub pid: u32,
    pub name: String,
    pub cpu_pct: f32,
    pub rss_mb: u64,
    pub is_system: bool,
}

// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedMemory {
    pub total_mb: u64,
    pub available_mb: u64,
    /// 0.0–100.0
    pub used_pct: f32,
    pub swap_total_mb: u64,
    pub swap_used_mb: u64,
    /// 0.0–100.0
    pub swap_used_pct: f32,
    /// macOS only
    pub pressure: Option<MemoryPressure>,
    pub top_process: Option<NormalizedProcessRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum MemoryPressure {
    Normal,
    Warning,
    Critical,
    Emergency,
}

// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedDisk {
    pub mount_point: String,
    pub is_system: bool,
    /// 0.0–100.0
    pub free_pct: f32,
    pub free_bytes: u64,
    pub total_bytes: u64,
    pub disk_type: Option<DiskType>,
    pub smart_status: Option<SmartStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum DiskType { SSD, HDD, NVMe, Fusion, Unknown }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SmartStatus { Ok, Warning, Failing, Unknown }

// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedNetwork {
    pub reachable: bool,
    pub ipv4: Option<String>,
    pub ipv6: Option<String>,
    pub wifi_ssid: Option<String>,
    pub wifi_signal_dbm: Option<i32>,
    pub wifi_noise_dbm: Option<i32>,
    pub dns_servers: Vec<String>,
    pub vpn_active: bool,
    pub proxy_configured: bool,
}

// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedThermal {
    pub state: Option<ThermalState>,
    pub cpu_temp_celsius: Option<f32>,
    pub gpu_temp_celsius: Option<f32>,
    pub fan_speed_rpm: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum ThermalState { Normal, Fair, Serious, Critical, Throttling }

// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedBattery {
    pub present: bool,
    pub charge_pct: f32,
    pub charging: bool,
    pub health_pct: Option<f32>,
    pub cycle_count: Option<u32>,
    pub current_ma: Option<i32>,
    pub time_remaining_min: Option<i32>,
}

// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedStartup {
    pub items_count: u32,
    pub launch_agents_count: Option<u32>,
    pub launch_daemons_count: Option<u32>,
    pub run_key_items_count: Option<u32>,
    pub items: Vec<StartupItemRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartupItemRef {
    pub name: String,
    pub path: String,
    pub enabled: bool,
    pub is_system: bool,
    pub source: StartupSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StartupSource { LaunchAgent, LaunchDaemon, LoginItem, RunKey, TaskScheduler }

// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedUpdates {
    pub pending_reboot_required: bool,
    pub recent_update_detected: bool,
    pub update_failed: bool,
}

// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedCrashes {
    pub count_1h: u32,
    pub count_24h: u32,
    pub count_7d: u32,
    pub last_process_name: Option<String>,
    pub last_exception_type: Option<String>,
    /// First 512 chars of the most-recent crash report, for ML classification.
    pub last_crash_log_text: Option<String>,
}

// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedService {
    pub name: String,
    pub running: bool,
    pub restart_count_1h: u32,
    pub exit_code: Option<i32>,
    pub uptime_seconds: Option<u64>,
    pub expected_running: bool,
}
