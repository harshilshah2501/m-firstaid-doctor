use chrono::{DateTime, Utc};
use feature_extractor::FeatureSet;
use serde::{Deserialize, Serialize};
use signal_normalizer::MemoryPressure;
use std::collections::HashMap;
use tracing::debug;

// ──────────────────────────────────────────────────────────────────────────────
// Public types
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum IssueClass {
    HighCpu,
    HighMemory,
    LowDisk,
    DiskIo,
    StartupSlow,
    AppCrash,
    NetworkDns,
    ServiceDown,
    UpdateIssue,
    BatteryThermal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerEvent {
    pub triggered_at: DateTime<Utc>,
    pub issue_classes: Vec<IssueClass>,
    pub reasons: Vec<TriggerReason>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerReason {
    pub issue_class: IssueClass,
    pub field: String,
    pub value: String,
    pub threshold: String,
}

// ──────────────────────────────────────────────────────────────────────────────
// Trigger state (hysteresis tracking)
// ──────────────────────────────────────────────────────────────────────────────

/// Per-issue-class trigger state for hysteresis.
#[derive(Debug, Default)]
struct TriggerState {
    /// True if this class is currently in "triggered" (above threshold) state.
    active: bool,
    /// Consecutive samples above threshold
    samples_above: u32,
    /// Consecutive samples below recovery threshold (for re-arming)
    samples_below_recovery: u32,
    /// Last time this class triggered
    last_triggered: Option<DateTime<Utc>>,
}

/// Stateful trigger engine. Maintained across poll cycles.
pub struct TriggerEngine {
    state: HashMap<IssueClass, TriggerState>,
    /// Minimum samples above threshold before firing (anti-flap)
    samples_required: u32,
    /// Samples below recovery threshold before re-arming
    recovery_samples: u32,
    /// Minimum seconds between triggers for the same class (cooldown)
    cooldown_seconds: u64,
}

impl TriggerEngine {
    pub fn new() -> Self {
        Self {
            state: HashMap::new(),
            samples_required: 3,    // 3 × 30s = 90 seconds of sustained signal
            recovery_samples: 2,    // need 2 good samples to re-arm
            cooldown_seconds: 300,  // 5-minute cooldown per class
        }
    }

    /// Evaluate a new feature set. Returns Some(TriggerEvent) if one or more
    /// issue classes should be diagnosed, None otherwise.
    pub fn evaluate(&mut self, features: &FeatureSet) -> Option<TriggerEvent> {
        let now = Utc::now();
        let mut fired_classes = Vec::new();
        let mut reasons = Vec::new();

        self.evaluate_class(
            IssueClass::HighCpu,
            features.cpu_usage_avg_5m > 85.0,
            features.cpu_usage_avg_5m < 70.0,
            &now,
            &mut fired_classes,
            &mut reasons,
            TriggerReason {
                issue_class: IssueClass::HighCpu,
                field: "cpu_usage_avg_5m".into(),
                value: format!("{:.1}%", features.cpu_usage_avg_5m),
                threshold: ">85%".into(),
            },
        );

        self.evaluate_class(
            IssueClass::HighMemory,
            features.memory_used_pct > 90.0
                || features.memory_oom_risk
                || features.memory_pressure.as_ref()
                    .map(|p| p >= &MemoryPressure::Critical)
                    .unwrap_or(false),
            features.memory_used_pct < 75.0,
            &now,
            &mut fired_classes,
            &mut reasons,
            TriggerReason {
                issue_class: IssueClass::HighMemory,
                field: "memory_used_pct".into(),
                value: format!("{:.1}%", features.memory_used_pct),
                threshold: ">90% or Critical pressure".into(),
            },
        );

        self.evaluate_class(
            IssueClass::LowDisk,
            features.system_disk_free_pct < 10.0,
            features.system_disk_free_pct > 15.0,
            &now,
            &mut fired_classes,
            &mut reasons,
            TriggerReason {
                issue_class: IssueClass::LowDisk,
                field: "system_disk_free_pct".into(),
                value: format!("{:.1}%", features.system_disk_free_pct),
                threshold: "<10%".into(),
            },
        );

        self.evaluate_class(
            IssueClass::AppCrash,
            features.app_crash_count_24h >= 2,
            features.app_crash_count_24h == 0,
            &now,
            &mut fired_classes,
            &mut reasons,
            TriggerReason {
                issue_class: IssueClass::AppCrash,
                field: "app_crash_count_24h".into(),
                value: features.app_crash_count_24h.to_string(),
                threshold: ">=2".into(),
            },
        );

        self.evaluate_class(
            IssueClass::NetworkDns,
            !features.network_reachable || features.dns_resolution_fail_rate > 0.2,
            features.network_reachable && features.dns_resolution_fail_rate < 0.05,
            &now,
            &mut fired_classes,
            &mut reasons,
            TriggerReason {
                issue_class: IssueClass::NetworkDns,
                field: "network_reachable".into(),
                value: features.network_reachable.to_string(),
                threshold: "reachable=false or dns_fail_rate>20%".into(),
            },
        );

        self.evaluate_class(
            IssueClass::BatteryThermal,
            features.cpu_temp_celsius.map(|t| t > 90.0).unwrap_or(false)
                || features.battery_health_pct.map(|h| h < 80.0).unwrap_or(false)
                || features.thermal_state.as_ref()
                    .map(|s| *s >= signal_normalizer::ThermalState::Serious)
                    .unwrap_or(false),
            features.cpu_temp_celsius.map(|t| t < 80.0).unwrap_or(true),
            &now,
            &mut fired_classes,
            &mut reasons,
            TriggerReason {
                issue_class: IssueClass::BatteryThermal,
                field: "cpu_temp_celsius".into(),
                value: format!("{:?}", features.cpu_temp_celsius),
                threshold: ">90°C or battery_health<80%".into(),
            },
        );

        self.evaluate_class(
            IssueClass::ServiceDown,
            features.service_down_expected || features.service_restart_storm,
            !features.service_down_expected && !features.service_restart_storm,
            &now,
            &mut fired_classes,
            &mut reasons,
            TriggerReason {
                issue_class: IssueClass::ServiceDown,
                field: "service_down_expected".into(),
                value: features.service_down_expected.to_string(),
                threshold: "true".into(),
            },
        );

        self.evaluate_class(
            IssueClass::UpdateIssue,
            features.update_failed || features.pending_reboot_required,
            !features.update_failed && !features.pending_reboot_required,
            &now,
            &mut fired_classes,
            &mut reasons,
            TriggerReason {
                issue_class: IssueClass::UpdateIssue,
                field: "update_failed".into(),
                value: features.update_failed.to_string(),
                threshold: "true".into(),
            },
        );

        if fired_classes.is_empty() {
            debug!("No triggers fired");
            return None;
        }

        debug!(classes = ?fired_classes, "Trigger fired");
        Some(TriggerEvent {
            triggered_at: now,
            issue_classes: fired_classes,
            reasons,
        })
    }

    fn evaluate_class(
        &mut self,
        class: IssueClass,
        above_threshold: bool,
        below_recovery: bool,
        now: &DateTime<Utc>,
        fired: &mut Vec<IssueClass>,
        reasons: &mut Vec<TriggerReason>,
        reason: TriggerReason,
    ) {
        let state = self.state.entry(class.clone()).or_default();

        if above_threshold {
            state.samples_above += 1;
            state.samples_below_recovery = 0;
        } else if below_recovery {
            state.samples_below_recovery += 1;
            if state.samples_below_recovery >= self.recovery_samples {
                state.active = false;
                state.samples_above = 0;
            }
        } else {
            // Between threshold and recovery — don't reset samples_above
        }

        let in_cooldown = state.last_triggered.map(|t| {
            now.signed_duration_since(t).num_seconds() < self.cooldown_seconds as i64
        }).unwrap_or(false);

        if state.samples_above >= self.samples_required && !state.active && !in_cooldown {
            state.active = true;
            state.last_triggered = Some(*now);
            fired.push(class);
            reasons.push(reason);
        }
    }
}

impl Default for TriggerEngine {
    fn default() -> Self {
        Self::new()
    }
}
