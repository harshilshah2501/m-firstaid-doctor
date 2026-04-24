use crate::types::CausalEdge;
use trigger_engine::IssueClass;

// Static causal rule table: (cause, effect, strength, explanation)
// Strength encodes how reliably this cause produces this effect in practice.
// Only edges where both endpoints are *actively detected* in the same cycle
// are surfaced — this avoids speculative chains on healthy metrics.
static CAUSAL_RULES: &[(IssueClass, IssueClass, f32, &str)] = &[
    (
        IssueClass::HighCpu,
        IssueClass::BatteryThermal,
        0.85,
        "Sustained CPU load is generating excess heat, triggering thermal throttling",
    ),
    (
        IssueClass::BatteryThermal,
        IssueClass::HighCpu,
        0.75,
        "Thermal throttling is capping CPU throughput, causing apparent CPU saturation",
    ),
    (
        IssueClass::HighMemory,
        IssueClass::HighCpu,
        0.70,
        "Heavy swap activity (memory pressure) is driving CPU overhead via page faults",
    ),
    (
        IssueClass::HighMemory,
        IssueClass::AppCrash,
        0.80,
        "OOM conditions are causing the OS to kill processes to reclaim memory",
    ),
    (
        IssueClass::LowDisk,
        IssueClass::AppCrash,
        0.65,
        "Applications crash when they cannot write to disk (logs, temp files, swap)",
    ),
    (
        IssueClass::LowDisk,
        IssueClass::UpdateIssue,
        0.90,
        "Insufficient free space prevents the OS from staging or completing updates",
    ),
    (
        IssueClass::NetworkDns,
        IssueClass::AppCrash,
        0.55,
        "Network-dependent applications are timing out and crashing due to DNS failures",
    ),
    (
        IssueClass::NetworkDns,
        IssueClass::ServiceDown,
        0.70,
        "Services that rely on DNS for peer discovery or auth are failing to connect",
    ),
    (
        IssueClass::UpdateIssue,
        IssueClass::ServiceDown,
        0.60,
        "A failed or incomplete update has left a service binary or config in a broken state",
    ),
    (
        IssueClass::DiskIo,
        IssueClass::AppCrash,
        0.65,
        "Slow or failing disk I/O is causing application write timeouts and crashes",
    ),
    (
        IssueClass::DiskIo,
        IssueClass::StartupSlow,
        0.80,
        "Disk I/O bottleneck is delaying executable and library loading at startup",
    ),
    (
        IssueClass::ServiceDown,
        IssueClass::AppCrash,
        0.60,
        "Applications that depend on the downed service are crashing on connection failure",
    ),
    (
        IssueClass::StartupSlow,
        IssueClass::HighCpu,
        0.50,
        "Startup indexing and prefetch activity is competing with foreground CPU demand",
    ),
];

/// Build the set of causal edges that apply to the currently detected issue classes.
/// Only returns edges where *both* cause and effect are present in `active_classes`.
/// Edges are sorted by strength descending.
pub fn build_causal_edges(active_classes: &[IssueClass]) -> Vec<CausalEdge> {
    let mut edges: Vec<CausalEdge> = CAUSAL_RULES
        .iter()
        .filter(|(cause, effect, _, _)| {
            active_classes.contains(cause) && active_classes.contains(effect)
        })
        .map(|(cause, effect, strength, explanation)| CausalEdge {
            cause: cause.clone(),
            effect: effect.clone(),
            strength: *strength,
            explanation: explanation.to_string(),
        })
        .collect();

    edges.sort_by(|a, b| b.strength.partial_cmp(&a.strength).unwrap_or(std::cmp::Ordering::Equal));
    edges
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_edges_for_single_issue() {
        let edges = build_causal_edges(&[IssueClass::HighCpu]);
        assert!(edges.is_empty(), "single issue cannot form a causal pair");
    }

    #[test]
    fn no_edges_for_unrelated_pair() {
        // StartupSlow and UpdateIssue have no direct causal rule between them
        let edges = build_causal_edges(&[IssueClass::StartupSlow, IssueClass::UpdateIssue]);
        assert!(edges.is_empty());
    }

    #[test]
    fn cpu_thermal_bidirectional() {
        let edges = build_causal_edges(&[IssueClass::HighCpu, IssueClass::BatteryThermal]);
        assert_eq!(edges.len(), 2);
        // Sorted by strength: HighCpu→BatteryThermal (0.85) first
        assert_eq!(edges[0].cause, IssueClass::HighCpu);
        assert_eq!(edges[0].effect, IssueClass::BatteryThermal);
    }

    #[test]
    fn memory_oom_chain() {
        let edges = build_causal_edges(&[
            IssueClass::HighMemory,
            IssueClass::HighCpu,
            IssueClass::AppCrash,
        ]);
        // HighMemory→HighCpu (0.70) and HighMemory→AppCrash (0.80) both apply
        assert_eq!(edges.len(), 2);
        let causes: Vec<_> = edges.iter().map(|e| &e.cause).collect();
        assert!(causes.iter().all(|c| **c == IssueClass::HighMemory));
        // Sorted descending: AppCrash (0.80) before HighCpu (0.70)
        assert_eq!(edges[0].effect, IssueClass::AppCrash);
    }

    #[test]
    fn edges_sorted_by_strength_descending() {
        let edges = build_causal_edges(&[
            IssueClass::LowDisk,
            IssueClass::AppCrash,
            IssueClass::UpdateIssue,
        ]);
        for w in edges.windows(2) {
            assert!(w[0].strength >= w[1].strength);
        }
    }
}
