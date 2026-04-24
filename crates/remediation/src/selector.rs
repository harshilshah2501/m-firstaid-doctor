//! ML-assisted remediation action selector.
//!
//! Scores and reorders `RemediationAction` items by predicted effectiveness
//! for the current system state. Two operating modes:
//!
//! ## Heuristic (always active, zero ML deps)
//! Three deterministic signals:
//!   1. Safety alignment — prefer least-risky action that still addresses the issue
//!   2. Issue-class specificity — boost actions whose `issue_class` matches an active class
//!   3. Action type weight — concrete state-changing actions rank above pure `Report`
//!
//! ## ONNX (future, `--features onnx` + model present in inference crate)
//! The orchestrator can pass pre-computed ONNX scores as `Option<Vec<f32>>` and
//! this function will apply them directly, bypassing the heuristic.

use trigger_engine::IssueClass;
use crate::types::{ActionType, RemediationAction, SafetyLevel};

/// Score and reorder `actions` by predicted effectiveness.
///
/// `active_classes` — issue classes that fired in this diagnostic cycle.
/// `onnx_scores`    — if provided, pre-computed per-action scores from an ONNX
///                    model (same order as `actions`); heuristic is skipped.
///
/// Returns the same actions with `ml_score` and `ml_rank` populated, sorted
/// by descending score. Safety gates are never modified.
pub fn score_remediation_actions(
    actions: &[RemediationAction],
    active_classes: &[IssueClass],
    onnx_scores: Option<&[f32]>,
) -> Vec<RemediationAction> {
    if actions.is_empty() {
        return Vec::new();
    }

    let raw_scores: Vec<f32> = if let Some(scores) = onnx_scores {
        // ONNX path: use externally computed scores directly
        scores.iter().copied()
            .chain(std::iter::repeat(0.0))
            .take(actions.len())
            .collect()
    } else {
        // Heuristic path
        heuristic_scores(actions, active_classes)
    };

    let mut indexed: Vec<(f32, &RemediationAction)> = raw_scores
        .iter()
        .zip(actions.iter())
        .map(|(&s, a)| (s, a))
        .collect();

    indexed.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    indexed
        .into_iter()
        .enumerate()
        .map(|(i, (score, action))| {
            let mut a = action.clone();
            a.ml_score = Some(score);
            a.ml_rank  = Some(i + 1);
            a
        })
        .collect()
}

fn heuristic_scores(actions: &[RemediationAction], active_classes: &[IssueClass]) -> Vec<f32> {
    let active_labels: Vec<&str> = active_classes.iter().map(issue_class_label).collect();

    actions.iter().map(|action| {
        let mut score: f32 = 0.0;

        // Safety alignment
        score += match action.safety_level {
            SafetyLevel::AutoSafe          => 1.00,
            SafetyLevel::ApprovalRequired  => 0.65,
            SafetyLevel::ElevationRequired => 0.40,
            SafetyLevel::NeverAutoFix      => 0.10,
        };

        // Issue-class alignment
        if active_labels.contains(&action.issue_class.as_str()) {
            score += 0.30;
        }

        // Action type specificity
        score += match &action.action_type {
            ActionType::FlushDnsCache              => 0.20,
            ActionType::KillProcess { .. }         => 0.15,
            ActionType::ClearCache { .. }          => 0.15,
            ActionType::RestartService { .. }      => 0.15,
            ActionType::StopService { .. }         => 0.10,
            ActionType::DisableStartupItem { .. }  => 0.10,
            ActionType::ReprioritizeProcess { .. } => 0.10,
            ActionType::RunConnectivityProbe { .. }=> 0.10,
            ActionType::EnableStartupItem { .. }   => 0.05,
            ActionType::Report { .. }              => 0.00,
        };

        // Reversibility bonus
        if action.reversible {
            score += 0.05;
        }

        score
    }).collect()
}

fn issue_class_label(cls: &IssueClass) -> &'static str {
    match cls {
        IssueClass::HighCpu        => "HighCpu",
        IssueClass::HighMemory     => "HighMemory",
        IssueClass::LowDisk        => "LowDisk",
        IssueClass::DiskIo         => "DiskIo",
        IssueClass::StartupSlow    => "StartupSlow",
        IssueClass::AppCrash       => "AppCrash",
        IssueClass::NetworkDns     => "NetworkDns",
        IssueClass::ServiceDown    => "ServiceDown",
        IssueClass::UpdateIssue    => "UpdateIssue",
        IssueClass::BatteryThermal => "BatteryThermal",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{PrivilegeLevel, RiskLevel, Signal};
    use uuid::Uuid;

    fn make_action(class: &str, action_type: ActionType, safety: SafetyLevel) -> RemediationAction {
        RemediationAction {
            action_id: Uuid::new_v4(),
            issue_class: class.into(),
            action_type,
            title: "test".into(),
            description: String::new(),
            safety_level: safety,
            privilege_required: PrivilegeLevel::User,
            estimated_risk: RiskLevel::Low,
            reversible: false,
            rollback_note: String::new(),
            pre_checks: vec![],
            ml_score: None,
            ml_rank: None,
        }
    }

    #[test]
    fn empty_input_returns_empty() {
        assert!(score_remediation_actions(&[], &[], None).is_empty());
    }

    #[test]
    fn all_actions_get_score_and_rank() {
        let actions = vec![
            make_action("NetworkDns", ActionType::FlushDnsCache, SafetyLevel::AutoSafe),
            make_action("NetworkDns", ActionType::Report { message: "x".into() }, SafetyLevel::AutoSafe),
        ];
        let result = score_remediation_actions(&actions, &[IssueClass::NetworkDns], None);
        assert!(result.iter().all(|a| a.ml_score.is_some() && a.ml_rank.is_some()));
    }

    #[test]
    fn flush_dns_ranks_above_report() {
        let actions = vec![
            make_action("NetworkDns", ActionType::Report { message: "x".into() }, SafetyLevel::AutoSafe),
            make_action("NetworkDns", ActionType::FlushDnsCache, SafetyLevel::AutoSafe),
        ];
        let result = score_remediation_actions(&actions, &[IssueClass::NetworkDns], None);
        let flush  = result.iter().find(|a| matches!(a.action_type, ActionType::FlushDnsCache)).unwrap();
        let report = result.iter().find(|a| matches!(a.action_type, ActionType::Report { .. })).unwrap();
        assert!(flush.ml_rank < report.ml_rank);
    }

    #[test]
    fn autosafe_report_beats_approval_required_kill_on_score() {
        let actions = vec![
            make_action("HighCpu", ActionType::KillProcess {
                pid: 1, process_name: "foo".into(),
                signal: Signal::Sigterm, fallback: None,
            }, SafetyLevel::ApprovalRequired),
            make_action("HighCpu", ActionType::Report { message: "y".into() }, SafetyLevel::AutoSafe),
        ];
        let result = score_remediation_actions(&actions, &[IssueClass::HighCpu], None);
        let report = result.iter().find(|a| matches!(a.action_type, ActionType::Report { .. })).unwrap();
        let kill   = result.iter().find(|a| matches!(a.action_type, ActionType::KillProcess { .. })).unwrap();
        assert!(report.ml_score.unwrap() > kill.ml_score.unwrap());
    }

    #[test]
    fn ranks_are_1_indexed_contiguous() {
        let actions = vec![
            make_action("LowDisk", ActionType::ClearCache {
                target_paths: vec![], min_age_days: 7, dry_run: false,
            }, SafetyLevel::ApprovalRequired),
            make_action("LowDisk", ActionType::Report { message: "z".into() }, SafetyLevel::AutoSafe),
            make_action("LowDisk", ActionType::RunConnectivityProbe {
                probe_type: crate::types::ProbeType::PingGateway,
            }, SafetyLevel::AutoSafe),
        ];
        let result = score_remediation_actions(&actions, &[IssueClass::LowDisk], None);
        let mut ranks: Vec<usize> = result.iter().map(|a| a.ml_rank.unwrap()).collect();
        ranks.sort();
        assert_eq!(ranks, vec![1, 2, 3]);
    }

    #[test]
    fn onnx_scores_override_heuristic() {
        let actions = vec![
            make_action("HighCpu", ActionType::Report { message: "a".into() }, SafetyLevel::AutoSafe),
            make_action("HighCpu", ActionType::FlushDnsCache, SafetyLevel::AutoSafe),
        ];
        // Force the Report to win by giving it a higher ONNX score
        let onnx = vec![0.99f32, 0.10];
        let result = score_remediation_actions(&actions, &[IssueClass::HighCpu], Some(&onnx));
        assert_eq!(result[0].ml_rank, Some(1));
        assert!(matches!(result[0].action_type, ActionType::Report { .. }));
    }
}
