use crate::types::*;
use feature_extractor::FeatureSet;
use trigger_engine::IssueClass;

pub fn diagnose(f: &FeatureSet) -> Option<Diagnosis> {
    // Rule 1: Critical disk + pending update = double risk
    if f.system_disk_free_pct < 10.0 && f.pending_reboot_required {
        return Some(Diagnosis {
            issue_class: IssueClass::LowDisk,
            confidence: Confidence::Confirmed,
            severity: Severity::Critical,
            rule_name: "disk::critical_with_pending_update".into(),
            title: format!(
                "Critical: disk at {:.1}% free with pending OS update",
                f.system_disk_free_pct
            ),
            explanation: "Disk space is critically low AND a pending system update requires a \
                          reboot. The update may fail to complete if there is insufficient space \
                          for temporary files. This requires immediate attention.".into(),
            evidence: vec![
                Evidence::field("system_disk_free_pct", f.system_disk_free_pct).with_threshold(10.0),
                Evidence::field("pending_reboot_required", f.pending_reboot_required),
                Evidence::field("system_disk_free_bytes", f.system_disk_free_bytes),
            ],
            contributing_ml: None,
        });
    }

    // Rule 2: Critical free space
    if f.system_disk_free_pct < 5.0 {
        return Some(Diagnosis {
            issue_class: IssueClass::LowDisk,
            confidence: Confidence::Confirmed,
            severity: Severity::Critical,
            rule_name: "disk::system_volume_critical".into(),
            title: format!(
                "Critical disk space: {:.1}% free ({:.1} GB)",
                f.system_disk_free_pct,
                f.system_disk_free_bytes as f64 / 1_073_741_824.0
            ),
            explanation: format!(
                "The system volume has only {:.1}% free space ({:.1} GB). \
                 Below 5%, the OS may fail to create temporary files, crash logs, \
                 and swap space, leading to application failures and potential data loss.",
                f.system_disk_free_pct,
                f.system_disk_free_bytes as f64 / 1_073_741_824.0
            ),
            evidence: vec![
                Evidence::field("system_disk_free_pct", f.system_disk_free_pct).with_threshold(5.0),
                Evidence::field("system_disk_free_bytes", f.system_disk_free_bytes),
            ],
            contributing_ml: None,
        });
    }

    // Rule 3: Low free space (5–10%)
    if f.system_disk_free_pct < 10.0 {
        return Some(Diagnosis {
            issue_class: IssueClass::LowDisk,
            confidence: Confidence::Confirmed,
            severity: Severity::High,
            rule_name: "disk::system_volume_low".into(),
            title: format!("Low disk space: {:.1}% free", f.system_disk_free_pct),
            explanation: format!(
                "The system volume is below 10% free space. Performance may degrade \
                 and certain operations may fail. {:.1} GB available.",
                f.system_disk_free_bytes as f64 / 1_073_741_824.0
            ),
            evidence: vec![
                Evidence::field("system_disk_free_pct", f.system_disk_free_pct).with_threshold(10.0),
                Evidence::field("system_disk_free_bytes", f.system_disk_free_bytes),
            ],
            contributing_ml: None,
        });
    }

    None
}
