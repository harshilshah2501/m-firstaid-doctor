use crate::types::*;
use detector::types::{DiagnosisOutput, Severity};
use trigger_engine::IssueClass;
use uuid::Uuid;

/// Translate a DiagnosisOutput into a RemediationPlan.
/// The planner is purely declarative — it does not execute anything.
pub fn plan(diagnosis: &DiagnosisOutput) -> RemediationPlan {
    let mut plan = RemediationPlan::new(diagnosis.diagnosis_id);

    for d in &diagnosis.diagnoses {
        match d.issue_class {
            IssueClass::HighCpu => plan_cpu(&mut plan, d),
            IssueClass::HighMemory => plan_memory(&mut plan, d),
            IssueClass::LowDisk => plan_disk(&mut plan, d),
            IssueClass::NetworkDns => plan_network(&mut plan, d),
            IssueClass::BatteryThermal => plan_battery(&mut plan, d),
            IssueClass::AppCrash => plan_crash(&mut plan, d),
            IssueClass::ServiceDown => plan_service(&mut plan, d),
            IssueClass::UpdateIssue => plan_update(&mut plan, d),
            IssueClass::StartupSlow => plan_startup(&mut plan, d),
            IssueClass::DiskIo => plan_disk_io(&mut plan, d),
        }
    }

    plan
}

fn plan_cpu(plan: &mut RemediationPlan, d: &detector::types::Diagnosis) {
    // Always: report
    plan.actions.push(
        RemediationAction {
            action_id: Uuid::new_v4(),
            issue_class: "HighCpu".into(),
            action_type: ActionType::Report {
                message: format!(
                    "Top CPU consumer: '{}' at {:.0}% CPU",
                    d.evidence.iter().find(|e| e.field == "top_process_name")
                        .and_then(|e| e.value.as_str()).unwrap_or("unknown"),
                    d.evidence.iter().find(|e| e.field == "top_process_cpu_pct")
                        .and_then(|e| e.value.as_f64()).unwrap_or(0.0)
                ),
            },
            title: "Show CPU usage breakdown".into(),
            description: "Display which processes are consuming the most CPU".into(),
            safety_level: SafetyLevel::AutoSafe,
            privilege_required: PrivilegeLevel::User,
            estimated_risk: RiskLevel::Negligible,
            reversible: true,
            rollback_note: "No action taken; report only.".into(),
            pre_checks: vec![],
        }
    );

    // Conditional: offer to kill if user-space process and severity is High+
    let is_system = d.evidence.iter()
        .find(|e| e.field == "top_process_is_system")
        .and_then(|e| e.value.as_bool())
        .unwrap_or(true);  // default safe: assume system

    if !is_system && d.severity >= Severity::High {
        if let Some(pid_val) = d.evidence.iter().find(|e| e.field == "top_process_pid")
            .and_then(|e| e.value.as_u64()) {
            let name = d.evidence.iter().find(|e| e.field == "top_process_name")
                .and_then(|e| e.value.as_str()).unwrap_or("unknown").to_string();
            plan.actions.push(RemediationAction {
                action_id: Uuid::new_v4(),
                issue_class: "HighCpu".into(),
                action_type: ActionType::KillProcess {
                    pid: pid_val as u32,
                    process_name: name.clone(),
                    signal: Signal::Sigterm,
                    fallback: Some(Signal::Sigkill),
                },
                title: format!("Quit '{}' to recover CPU", name),
                description: format!(
                    "Send a quit signal to '{}'. The application will be given 5 seconds \
                     to shut down gracefully before being force-killed.",
                    name
                ),
                safety_level: SafetyLevel::ApprovalRequired,
                privilege_required: PrivilegeLevel::User,
                estimated_risk: RiskLevel::Low,
                reversible: false,
                rollback_note: format!("Reopen '{}' to restore the process.", name),
                pre_checks: vec![
                    format!("Verify PID {} is still running", pid_val),
                    format!("Verify process name still matches '{}'", name),
                    "Verify process is not in D-state (uninterruptible wait)".into(),
                    "Verify process is not on the protected list".into(),
                ],
            });
        }
    }
}

fn plan_memory(plan: &mut RemediationPlan, d: &detector::types::Diagnosis) {
    plan.actions.push(RemediationAction {
        action_id: Uuid::new_v4(),
        issue_class: "HighMemory".into(),
        action_type: ActionType::Report {
            message: "Memory usage breakdown: showing top consumers".into(),
        },
        title: "Show memory usage breakdown".into(),
        description: "Display which processes are consuming the most RAM".into(),
        safety_level: SafetyLevel::AutoSafe,
        privilege_required: PrivilegeLevel::User,
        estimated_risk: RiskLevel::Negligible,
        reversible: true,
        rollback_note: "No action taken.".into(),
        pre_checks: vec![],
    });

    plan.recommended_only.push(Recommendation {
        issue_class: "HighMemory".into(),
        title: "Close unused applications".into(),
        description: "Closing browser tabs and unused applications will free memory \
                      without risking data loss.".into(),
        safety_level: SafetyLevel::NeverAutoFix,
    });
}

fn plan_disk(plan: &mut RemediationPlan, _d: &detector::types::Diagnosis) {
    // AutoSafe: clear temp files
    plan.actions.push(RemediationAction {
        action_id: Uuid::new_v4(),
        issue_class: "LowDisk".into(),
        action_type: ActionType::ClearCache {
            target_paths: vec![
                "/private/tmp".into(),
                "~/Library/Caches".into(),
            ],
            min_age_days: 7,
            dry_run: false,
        },
        title: "Clear temporary files and user caches (>7 days old)".into(),
        description: "Delete cached files older than 7 days from standard cache locations. \
                      These files are recreated automatically by applications.".into(),
        safety_level: SafetyLevel::ApprovalRequired,
        privilege_required: PrivilegeLevel::User,
        estimated_risk: RiskLevel::Low,
        reversible: false,
        rollback_note: "Deleted cache files cannot be restored but are regenerated on next app use.".into(),
        pre_checks: vec![
            "Verify target paths are in the approved list".into(),
            "Dry-run: enumerate files to be deleted before proceeding".into(),
            "Verify no target files are currently open by running processes".into(),
        ],
    });

    plan.recommended_only.push(Recommendation {
        issue_class: "LowDisk".into(),
        title: "Review large files and applications".into(),
        description: "Use Storage Management (macOS) or Storage Sense (Windows) to identify \
                      large files and applications that can be removed.".into(),
        safety_level: SafetyLevel::NeverAutoFix,
    });
}

fn plan_network(plan: &mut RemediationPlan, d: &detector::types::Diagnosis) {
    // Always: run connectivity probe (AutoSafe)
    plan.actions.push(RemediationAction {
        action_id: Uuid::new_v4(),
        issue_class: "NetworkDns".into(),
        action_type: ActionType::RunConnectivityProbe {
            probe_type: ProbeType::DnsLookup { domain: "apple.com".into() },
        },
        title: "Run layered connectivity diagnostic".into(),
        description: "Test DNS, gateway reachability, and internet connectivity layer by layer.".into(),
        safety_level: SafetyLevel::AutoSafe,
        privilege_required: PrivilegeLevel::User,
        estimated_risk: RiskLevel::Negligible,
        reversible: true,
        rollback_note: "Read-only probe; no changes made.".into(),
        pre_checks: vec![],
    });

    // DNS flush (AutoSafe)
    plan.actions.push(RemediationAction {
        action_id: Uuid::new_v4(),
        issue_class: "NetworkDns".into(),
        action_type: ActionType::FlushDnsCache,
        title: "Flush DNS resolver cache".into(),
        description: "Clear the OS DNS cache to remove stale or corrupted DNS entries. \
                      This is safe and takes effect immediately.".into(),
        safety_level: SafetyLevel::AutoSafe,
        privilege_required: PrivilegeLevel::User,
        estimated_risk: RiskLevel::Negligible,
        reversible: true,
        rollback_note: "DNS cache is rebuilt automatically. No permanent change.".into(),
        pre_checks: vec![],
    });
}

fn plan_battery(plan: &mut RemediationPlan, _d: &detector::types::Diagnosis) {
    plan.actions.push(RemediationAction {
        action_id: Uuid::new_v4(),
        issue_class: "BatteryThermal".into(),
        action_type: ActionType::Report {
            message: "Battery health and thermal state report".into(),
        },
        title: "Show battery and thermal diagnostics".into(),
        description: "Display current battery health, temperature readings, and thermal state.".into(),
        safety_level: SafetyLevel::AutoSafe,
        privilege_required: PrivilegeLevel::User,
        estimated_risk: RiskLevel::Negligible,
        reversible: true,
        rollback_note: "No action taken.".into(),
        pre_checks: vec![],
    });

    plan.recommended_only.push(Recommendation {
        issue_class: "BatteryThermal".into(),
        title: "Ensure ventilation — remove device from enclosed space".into(),
        description: "Keep the device on a hard flat surface. Soft surfaces (beds, cushions) \
                      block ventilation and increase thermal stress.".into(),
        safety_level: SafetyLevel::NeverAutoFix,
    });
}

fn plan_crash(plan: &mut RemediationPlan, _d: &detector::types::Diagnosis) {
    plan.recommended_only.push(Recommendation {
        issue_class: "AppCrash".into(),
        title: "Clear application cache and preferences".into(),
        description: "In Finder: ~/Library/Caches/<AppName>. This can resolve crashes \
                      caused by corrupted cache data.".into(),
        safety_level: SafetyLevel::NeverAutoFix,
    });
}

fn plan_service(plan: &mut RemediationPlan, _d: &detector::types::Diagnosis) {
    plan.recommended_only.push(Recommendation {
        issue_class: "ServiceDown".into(),
        title: "Review service logs and restart manually".into(),
        description: "Check the service log for error details before restarting. \
                      Automatic restart may mask the underlying error.".into(),
        safety_level: SafetyLevel::NeverAutoFix,
    });
}

fn plan_update(plan: &mut RemediationPlan, _d: &detector::types::Diagnosis) {
    plan.recommended_only.push(Recommendation {
        issue_class: "UpdateIssue".into(),
        title: "Schedule a system reboot to complete pending updates".into(),
        description: "Reboot at a convenient time to apply pending updates. \
                      Keeping updates current is important for security.".into(),
        safety_level: SafetyLevel::NeverAutoFix,
    });
}

fn plan_startup(plan: &mut RemediationPlan, _d: &detector::types::Diagnosis) {
    plan.recommended_only.push(Recommendation {
        issue_class: "StartupSlow".into(),
        title: "Review and disable non-essential startup items".into(),
        description: "Use System Preferences > Users & Groups > Login Items (macOS) or \
                      Task Manager > Startup (Windows) to disable items you don't need at login.".into(),
        safety_level: SafetyLevel::NeverAutoFix,
    });
}

fn plan_disk_io(plan: &mut RemediationPlan, d: &detector::types::Diagnosis) {
    // SMART failure is always recommendation-only
    if d.rule_name.contains("smart_failure") {
        plan.recommended_only.push(Recommendation {
            issue_class: "DiskIo".into(),
            title: "URGENT: Back up all data immediately — disk is failing".into(),
            description: "SMART reports a disk failure condition. Back up to an external drive \
                          or cloud storage NOW. Do not shut down the machine until backup completes. \
                          Seek hardware replacement.".into(),
            safety_level: SafetyLevel::NeverAutoFix,
        });
    }
}
