use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Polling interval for cheap collectors (seconds)
    pub poll_interval_seconds: u64,
    /// SQLite database path
    pub db_path: PathBuf,
    /// Log file path
    pub log_path: PathBuf,
    /// Snapshot retention in days
    pub snapshot_retention_days: u32,
    /// Whether to enable ML inference (Phase 3)
    pub ml_enabled: bool,
    /// Model directory path
    pub models_dir: Option<PathBuf>,
    /// Services to monitor for unexpected stops
    pub monitored_services: Vec<MonitoredService>,
    /// Thresholds (overrideable by MDM policy in Phase 4)
    pub thresholds: Thresholds,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitoredService {
    pub name: String,
    pub display_name: String,
    pub expected_running: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Thresholds {
    pub cpu_trigger_pct: f32,
    pub cpu_recovery_pct: f32,
    pub memory_trigger_pct: f32,
    pub disk_trigger_pct: f32,
    pub disk_recovery_pct: f32,
    pub dns_fail_rate_trigger: f32,
    pub thermal_trigger_celsius: f32,
}

impl Default for AgentConfig {
    fn default() -> Self {
        let data_dir = dirs_data_dir();
        Self {
            poll_interval_seconds: 30,
            db_path: data_dir.join("firstaid.db"),
            log_path: data_dir.join("firstaid.log"),
            snapshot_retention_days: 7,
            ml_enabled: false,
            models_dir: None,
            monitored_services: Vec::new(),
            thresholds: Thresholds::default(),
        }
    }
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            cpu_trigger_pct: 85.0,
            cpu_recovery_pct: 70.0,
            memory_trigger_pct: 90.0,
            disk_trigger_pct: 10.0,
            disk_recovery_pct: 15.0,
            dns_fail_rate_trigger: 0.2,
            thermal_trigger_celsius: 90.0,
        }
    }
}

fn dirs_data_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        return PathBuf::from(home).join("Library/Application Support/m-firstaid-doctor");
    }
    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var("APPDATA").unwrap_or_else(|_| "C:\\Temp".into());
        return PathBuf::from(appdata).join("m-firstaid-doctor");
    }
    #[allow(unreachable_code)]
    PathBuf::from("/tmp/firstaid")
}
