use crate::types::*;
use feature_extractor::FeatureSet;
use trigger_engine::IssueClass;

pub fn diagnose(f: &FeatureSet) -> Option<Diagnosis> {
    if f.update_failed {
        return Some(Diagnosis {
            issue_class: IssueClass::UpdateIssue,
            confidence: Confidence::Confirmed,
            severity: Severity::Medium,
            rule_name: "update::failed".into(),
            title: "System update failed".into(),
            explanation: "The last system update attempt failed. Common causes include \
                          insufficient disk space, network failure during download, or \
                          a corrupted update cache. Retrying after ensuring disk space \
                          and network connectivity usually resolves this.".into(),
            evidence: vec![
                Evidence::field("update_failed", f.update_failed),
                Evidence::field("system_disk_free_pct", f.system_disk_free_pct),
                Evidence::field("network_reachable", f.network_reachable),
            ],
            contributing_ml: None,
        });
    }

    if f.pending_reboot_required {
        return Some(Diagnosis {
            issue_class: IssueClass::UpdateIssue,
            confidence: Confidence::Confirmed,
            severity: Severity::Low,
            rule_name: "update::reboot_pending".into(),
            title: "System update pending reboot".into(),
            explanation: "A system update has been downloaded and is waiting for a reboot \
                          to complete installation. Delaying security updates increases \
                          vulnerability exposure. A reboot at your convenience is recommended.".into(),
            evidence: vec![
                Evidence::field("pending_reboot_required", f.pending_reboot_required),
            ],
            contributing_ml: None,
        });
    }

    None
}
