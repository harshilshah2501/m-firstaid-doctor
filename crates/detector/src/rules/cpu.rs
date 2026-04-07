use crate::types::*;
use feature_extractor::FeatureSet;
use trigger_engine::IssueClass;

/// Diagnose high CPU usage from features.
/// Returns None if no diagnosis can be made (should not happen if trigger fired correctly).
pub fn diagnose(f: &FeatureSet) -> Option<Diagnosis> {
    // Rule 1: Single process monopolizing CPU (user-space)
    if f.top_process_cpu_pct > 75.0 && !f.top_process_is_system {
        return Some(Diagnosis {
            issue_class: IssueClass::HighCpu,
            confidence: Confidence::Confirmed,
            severity: Severity::High,
            rule_name: "cpu::single_process_user_space".into(),
            title: format!(
                "High CPU: '{}' consuming {:.0}%",
                f.top_process_name, f.top_process_cpu_pct
            ),
            explanation: format!(
                "The process '{}' (PID {:?}) has been consuming {:.0}% CPU for an extended period. \
                 On a {}-core machine, this represents {:.0}% of total compute. This is likely a \
                 runaway loop, intensive task, or software defect.",
                f.top_process_name,
                f.top_process_pid,
                f.top_process_cpu_pct,
                f.cpu_core_count,
                f.top_process_cpu_pct / f.cpu_core_count as f32
            ),
            evidence: vec![
                Evidence::field("top_process_cpu_pct", f.top_process_cpu_pct)
                    .with_threshold(75.0),
                Evidence::field("top_process_name", &f.top_process_name),
                Evidence::field("cpu_usage_avg_5m", f.cpu_usage_avg_5m),
                Evidence::field("top_process_is_system", f.top_process_is_system),
            ],
            contributing_ml: None,
        });
    }

    // Rule 2: Single process monopolizing CPU (system process — more care needed)
    if f.top_process_cpu_pct > 75.0 && f.top_process_is_system {
        let (title, explanation) = explain_system_process_cpu(&f.top_process_name, f.top_process_cpu_pct);
        return Some(Diagnosis {
            issue_class: IssueClass::HighCpu,
            confidence: Confidence::Probable,
            severity: Severity::Medium,
            rule_name: "cpu::single_process_system".into(),
            title,
            explanation,
            evidence: vec![
                Evidence::field("top_process_cpu_pct", f.top_process_cpu_pct)
                    .with_threshold(75.0),
                Evidence::field("top_process_name", &f.top_process_name)
                    .with_context("system process — remediation requires more caution"),
                Evidence::field("cpu_usage_avg_5m", f.cpu_usage_avg_5m),
            ],
            contributing_ml: None,
        });
    }

    // Rule 3: Distributed CPU load — no single dominant process
    if f.cpu_usage_avg_5m > 85.0 && f.top_process_cpu_pct < 40.0 {
        return Some(Diagnosis {
            issue_class: IssueClass::HighCpu,
            confidence: if f.cpu_usage_avg_5m > 92.0 { Confidence::Confirmed } else { Confidence::Probable },
            severity: Severity::High,
            rule_name: "cpu::distributed_load".into(),
            title: format!("High CPU: distributed load across processes ({:.0}% avg)", f.cpu_usage_avg_5m),
            explanation: format!(
                "CPU utilization has averaged {:.0}% over 5 minutes, but no single process is \
                 dominant (top consumer at {:.0}%). This suggests many concurrent tasks are competing \
                 for CPU — possible causes include background indexing, update operations, or many \
                 active applications.",
                f.cpu_usage_avg_5m, f.top_process_cpu_pct
            ),
            evidence: vec![
                Evidence::field("cpu_usage_avg_5m", f.cpu_usage_avg_5m).with_threshold(85.0),
                Evidence::field("top_process_cpu_pct", f.top_process_cpu_pct)
                    .with_context("no single dominant consumer"),
                Evidence::field("recent_update_detected", f.recent_update_detected),
            ],
            contributing_ml: None,
        });
    }

    // Rule 4: Post-update high CPU (likely indexing / compile)
    if f.cpu_usage_avg_5m > 70.0 && f.recent_update_detected {
        return Some(Diagnosis {
            issue_class: IssueClass::HighCpu,
            confidence: Confidence::Probable,
            severity: Severity::Low,
            rule_name: "cpu::post_update_indexing".into(),
            title: "Elevated CPU likely due to recent update processing".into(),
            explanation: "A system update was recently applied and CPU is elevated. This is \
                          commonly caused by Spotlight re-indexing, update compilation, or \
                          software update agents completing background work. This usually \
                          resolves within 30–60 minutes.".into(),
            evidence: vec![
                Evidence::field("cpu_usage_avg_5m", f.cpu_usage_avg_5m),
                Evidence::field("recent_update_detected", f.recent_update_detected),
            ],
            contributing_ml: None,
        });
    }

    // Fallback — trigger fired but rules are inconclusive (unlikely but possible)
    Some(Diagnosis {
        issue_class: IssueClass::HighCpu,
        confidence: Confidence::Possible,
        severity: Severity::Medium,
        rule_name: "cpu::generic_high".into(),
        title: format!("High CPU usage detected ({:.0}% avg over 5m)", f.cpu_usage_avg_5m),
        explanation: "CPU utilization is elevated but the root cause could not be \
                      specifically identified from current signals.".into(),
        evidence: vec![
            Evidence::field("cpu_usage_avg_5m", f.cpu_usage_avg_5m).with_threshold(85.0),
        ],
        contributing_ml: None,
    })
}

fn explain_system_process_cpu(name: &str, pct: f32) -> (String, String) {
    match name {
        "mds_stores" | "mds" | "mdworker" => (
            format!("Spotlight indexing at {:.0}% CPU", pct),
            "Spotlight (mds_stores) is re-indexing the disk. This is usually triggered after \
             a large file change, software update, or first boot after update. It typically \
             completes within 30–60 minutes and should not be force-terminated.".into(),
        ),
        "kernel_task" => (
            format!("kernel_task at {:.0}% — likely thermal throttling", pct),
            "macOS intentionally inflates kernel_task CPU to throttle applications when the \
             system is thermally stressed. This is a protective mechanism, not a bug. \
             Addressing the thermal issue (see Battery/Thermal diagnosis) will resolve this.".into(),
        ),
        "backupd" | "com.apple.TimeMachineHelper" => (
            format!("Time Machine backup at {:.0}% CPU", pct),
            "A Time Machine backup is in progress. This is expected and will complete \
             automatically. You can pause it via System Preferences > Time Machine if \
             the CPU impact is interfering with your work.".into(),
        ),
        _ => (
            format!("System process '{}' at {:.0}% CPU", name, pct),
            format!("The system process '{}' is consuming significant CPU. System processes \
                     often represent legitimate OS work (updates, maintenance, security scans). \
                     Investigate in Activity Monitor before taking action.", name),
        ),
    }
}
