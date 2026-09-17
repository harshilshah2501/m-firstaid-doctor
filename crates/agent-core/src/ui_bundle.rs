//! Stable JSON envelope for desktop / web UIs (e.g. EndpointDoctor).
//! Contract: `ui_bundle_schema` + `agent_version` identify the payload shape.

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::orchestrator::{DiagnosticCycleDetail, DiagnosticCycleResult};
use detector::{DiagnosisOutput, Severity};
use signal_normalizer::NormalizedSignalSnapshot;

/// Increment when adding/removing/renaming fields in [`UiBundle`].
pub const UI_BUNDLE_SCHEMA: &str = "1.0";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UiOverallHealth {
    Healthy,
    Degraded,
    Critical,
}

#[derive(Debug, Clone, Serialize)]
pub struct UiCycleSummary {
    pub snapshot_id: uuid::Uuid,
    pub triggered: bool,
    pub diagnoses_found: Vec<String>,
    pub plan_id: Option<uuid::Uuid>,
    pub executed: bool,
}

impl From<&DiagnosticCycleResult> for UiCycleSummary {
    fn from(r: &DiagnosticCycleResult) -> Self {
        Self {
            snapshot_id: r.snapshot_id,
            triggered: r.triggered,
            diagnoses_found: r.diagnoses_found.clone(),
            plan_id: r.plan_id,
            executed: r.executed,
        }
    }
}

/// Flattened snapshot fields for dashboards (aligned loosely with EndpointDoctor `EndpointSnapshot`).
#[derive(Debug, Clone, Serialize)]
pub struct UiSnapshotSummary {
    pub collected_at: DateTime<Utc>,
    pub platform: String,
    pub cpu_usage_pct: f32,
    pub cpu_core_count: u32,
    pub memory_used_pct: f32,
    pub memory_available_mb: u64,
    pub memory_total_mb: u64,
    pub memory_pressure: Option<String>,
    pub system_disk_free_pct: Option<f32>,
    pub system_disk_free_gb: Option<f32>,
    pub network_reachable: bool,
    pub wifi_ssid: Option<String>,
    pub crashes_24h: u32,
    pub pending_reboot: bool,
    pub top_process_name: Option<String>,
    pub top_process_cpu_pct: Option<f32>,
}

impl UiSnapshotSummary {
    pub fn from_normalized(n: &NormalizedSignalSnapshot) -> Self {
        let system_disk = n
            .disks
            .iter()
            .find(|d| d.is_system)
            .or_else(|| n.disks.first());

        let (free_pct, free_gb) = system_disk
            .map(|d| {
                let gb = d.free_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
                (Some(d.free_pct), Some(gb as f32))
            })
            .unwrap_or((None, None));

        let mem_pressure = n
            .memory
            .pressure
            .as_ref()
            .map(|p| format!("{p:?}"))
            .map(|s| s.to_lowercase());

        let top = n.cpu.top_process.as_ref().or(n.memory.top_process.as_ref());

        Self {
            collected_at: n.collected_at,
            platform: format!("{:?}", n.platform).to_lowercase(),
            cpu_usage_pct: n.cpu.usage_pct,
            cpu_core_count: n.cpu.core_count,
            memory_used_pct: n.memory.used_pct,
            memory_available_mb: n.memory.available_mb,
            memory_total_mb: n.memory.total_mb,
            memory_pressure: mem_pressure,
            system_disk_free_pct: free_pct,
            system_disk_free_gb: free_gb,
            network_reachable: n.network.reachable,
            wifi_ssid: n.network.wifi_ssid.clone(),
            crashes_24h: n.crashes.count_24h,
            pending_reboot: n.updates.pending_reboot_required,
            top_process_name: top.map(|p| p.name.clone()),
            top_process_cpu_pct: top.map(|p| p.cpu_pct),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct UiBundle {
    pub ui_bundle_schema: &'static str,
    pub generated_at: DateTime<Utc>,
    pub agent_version: &'static str,
    pub source: &'static str,
    pub overall_health: UiOverallHealth,
    pub cycle: UiCycleSummary,
    pub snapshot_summary: UiSnapshotSummary,
    pub diagnosis: DiagnosisOutput,
}

fn overall_health(diagnosis: &DiagnosisOutput) -> UiOverallHealth {
    if diagnosis.diagnoses.is_empty() {
        return UiOverallHealth::Healthy;
    }
    match diagnosis.max_severity() {
        None => UiOverallHealth::Healthy,
        Some(s) if matches!(s, Severity::Critical | Severity::High) => UiOverallHealth::Critical,
        Some(s) if matches!(s, Severity::Medium | Severity::Low) => UiOverallHealth::Degraded,
        Some(_) => UiOverallHealth::Healthy,
    }
}

impl UiBundle {
    pub fn from_live_cycle(detail: DiagnosticCycleDetail) -> Self {
        let cycle = UiCycleSummary::from(&detail.inner);
        let snapshot_summary = UiSnapshotSummary::from_normalized(&detail.normalized);
        let overall_health = overall_health(&detail.diagnosis);
        Self {
            ui_bundle_schema: UI_BUNDLE_SCHEMA,
            generated_at: Utc::now(),
            agent_version: env!("CARGO_PKG_VERSION"),
            source: "live",
            overall_health,
            cycle,
            snapshot_summary,
            diagnosis: detail.diagnosis,
        }
    }

    /// Fixture / offline runs (`firstaid --fixture … --ui-bundle`).
    pub fn from_fixture(
        normalized: NormalizedSignalSnapshot,
        diagnosis: DiagnosisOutput,
        cycle: DiagnosticCycleResult,
    ) -> Self {
        let snapshot_summary = UiSnapshotSummary::from_normalized(&normalized);
        let overall_health = overall_health(&diagnosis);
        Self {
            ui_bundle_schema: UI_BUNDLE_SCHEMA,
            generated_at: Utc::now(),
            agent_version: env!("CARGO_PKG_VERSION"),
            source: "fixture",
            overall_health,
            cycle: UiCycleSummary::from(&cycle),
            snapshot_summary,
            diagnosis,
        }
    }
}
