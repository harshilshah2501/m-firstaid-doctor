use crate::types::*;
use feature_extractor::FeatureSet;
use signal_normalizer::MemoryPressure;
use trigger_engine::IssueClass;

pub fn diagnose(f: &FeatureSet) -> Option<Diagnosis> {
    // Rule 1: Emergency pressure or OOM imminent
    if f.memory_oom_risk || f.memory_pressure.as_ref() == Some(&MemoryPressure::Emergency) {
        return Some(Diagnosis {
            issue_class: IssueClass::HighMemory,
            confidence: Confidence::Confirmed,
            severity: Severity::Critical,
            rule_name: "memory::oom_imminent".into(),
            title: "Critical: OOM risk — system may kill processes imminently".into(),
            explanation: format!(
                "Available memory has dropped to {} MB (below 150 MB threshold). \
                 The OS may begin forcibly terminating processes to prevent a crash. \
                 Immediate action is required.",
                f.memory_available_mb
            ),
            evidence: vec![
                Evidence::field("memory_available_mb", f.memory_available_mb).with_threshold(150u64),
                Evidence::field("memory_used_pct", f.memory_used_pct),
                Evidence::field("swap_used_mb", f.swap_used_mb),
                Evidence::field("memory_pressure", format!("{:?}", f.memory_pressure)),
            ],
            contributing_ml: None,
        });
    }

    // Rule 2: Critical pressure with active swap
    if f.memory_pressure.as_ref().map(|p| *p >= MemoryPressure::Critical).unwrap_or(false)
        && f.swap_used_mb > 1024
    {
        return Some(Diagnosis {
            issue_class: IssueClass::HighMemory,
            confidence: Confidence::Confirmed,
            severity: Severity::Critical,
            rule_name: "memory::critical_pressure_with_swap".into(),
            title: format!(
                "Critical memory pressure — {:.0}% used, {:.1} GB swap active",
                f.memory_used_pct, f.swap_used_mb as f32 / 1024.0
            ),
            explanation: format!(
                "System is at {:.0}% memory utilization with {:.1} GB of swap in use. \
                 Memory pressure is at Critical level. The top memory consumer is '{}' \
                 using {} MB. Performance is severely degraded and stability is at risk.",
                f.memory_used_pct,
                f.swap_used_mb as f32 / 1024.0,
                f.top_process_mem_name,
                f.top_process_rss_mb
            ),
            evidence: vec![
                Evidence::field("memory_pressure", format!("{:?}", f.memory_pressure)),
                Evidence::field("memory_used_pct", f.memory_used_pct).with_threshold(90.0),
                Evidence::field("swap_used_mb", f.swap_used_mb).with_threshold(1024u64),
                Evidence::field("top_process_rss_mb", f.top_process_rss_mb)
                    .with_context(&f.top_process_mem_name),
            ],
            contributing_ml: None,
        });
    }

    // Rule 3: Single process using >40% of total RAM
    let process_ram_pct = if f.memory_total_mb > 0 {
        (f.top_process_rss_mb as f32 / f.memory_total_mb as f32) * 100.0
    } else { 0.0 };

    if process_ram_pct > 40.0 {
        return Some(Diagnosis {
            issue_class: IssueClass::HighMemory,
            confidence: Confidence::Confirmed,
            severity: Severity::High,
            rule_name: "memory::single_process_dominant".into(),
            title: format!(
                "'{}' consuming {:.0}% of total RAM ({} MB)",
                f.top_process_mem_name, process_ram_pct, f.top_process_rss_mb
            ),
            explanation: format!(
                "The process '{}' is using {} MB, which is {:.0}% of your total {} MB RAM. \
                 This is the primary driver of memory pressure.",
                f.top_process_mem_name, f.top_process_rss_mb,
                process_ram_pct, f.memory_total_mb
            ),
            evidence: vec![
                Evidence::field("top_process_rss_mb", f.top_process_rss_mb)
                    .with_context(&format!("{:.0}% of {} MB total", process_ram_pct, f.memory_total_mb)),
                Evidence::field("memory_used_pct", f.memory_used_pct),
                Evidence::field("swap_used_mb", f.swap_used_mb),
            ],
            contributing_ml: None,
        });
    }

    // Rule 4: High usage, warning pressure
    if f.memory_used_pct > 90.0 {
        return Some(Diagnosis {
            issue_class: IssueClass::HighMemory,
            confidence: Confidence::Probable,
            severity: Severity::High,
            rule_name: "memory::high_usage_no_single_cause".into(),
            title: format!("High memory usage: {:.0}% utilized", f.memory_used_pct),
            explanation: format!(
                "System memory is {:.0}% utilized with {} MB available. \
                 No single dominant process was found — multiple applications are \
                 collectively exhausting memory. Closing unused applications is recommended.",
                f.memory_used_pct, f.memory_available_mb
            ),
            evidence: vec![
                Evidence::field("memory_used_pct", f.memory_used_pct).with_threshold(90.0),
                Evidence::field("memory_available_mb", f.memory_available_mb),
                Evidence::field("swap_used_mb", f.swap_used_mb),
            ],
            contributing_ml: None,
        });
    }

    None
}
