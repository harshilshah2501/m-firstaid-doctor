use serde::{Deserialize, Serialize};
use tracing::warn;

/// Protected process names — actions targeting these are ALWAYS denied.
/// This list is hardcoded and cannot be overridden by configuration.
pub const PROTECTED_PROCESS_NAMES: &[&str] = &[
    "kernel_task",
    "launchd",
    "WindowServer",
    "loginwindow",
    "securityd",
    "trustd",
    "lsass.exe",
    "csrss.exe",
    "smss.exe",
    "wininit.exe",
    "services.exe",
    "System",
    "svchost.exe",
];

/// Approved paths for cache/temp clearing.
/// Actions targeting paths outside this list are ALWAYS denied.
pub const APPROVED_CLEAR_PATHS: &[&str] = &[
    "/private/tmp",
    "/tmp",
    // ~/Library/Caches is added dynamically with HOME expansion
    "/var/folders",
];

/// Actions that can NEVER be auto-executed.
/// This is a compile-time constant — not configurable.
pub const NEVER_AUTO_FIX_ACTION_TYPES: &[&str] = &[
    "Reboot",
    "Shutdown",
    "InstallUpdate",
    "UninstallApp",
    "DeleteFile",     // generic file deletion outside approved paths
    "ModifyKext",
    "DisableSip",
    "ModifyFirewall",
    "ResetNetworkStack",
    "ModifyUserAccount",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolicyDecision {
    /// Action may proceed
    Allowed,
    /// Action requires user approval before proceeding
    PendingConsent,
    /// Action requires elevation (admin/sudo) AND user approval
    RequiresElevation,
    /// Action is unconditionally denied
    Denied { reason: String },
    /// Action can never be automated regardless of consent
    NeverAllowed { reason: String },
}

impl PolicyDecision {
    pub fn is_blocked(&self) -> bool {
        matches!(self, PolicyDecision::Denied { .. } | PolicyDecision::NeverAllowed { .. })
    }
}

/// Evaluate whether an action is allowed to proceed.
/// This is the single gating point before any remediation execution.
pub fn evaluate_action(
    action_type_name: &str,
    target_process_name: Option<&str>,
    target_path: Option<&str>,
    safety_level: u8,  // 0=AutoSafe, 1=ApprovalRequired, 2=ElevationRequired, 3=NeverAutoFix
    consent_given: bool,
) -> PolicyDecision {
    // Gate 1: NeverAutoFix — absolutely unconditional
    if safety_level >= 3 || NEVER_AUTO_FIX_ACTION_TYPES.contains(&action_type_name) {
        return PolicyDecision::NeverAllowed {
            reason: format!("'{}' is classified as NeverAutoFix and cannot be automated", action_type_name),
        };
    }

    // Gate 2: Protected process check
    if let Some(proc_name) = target_process_name {
        if PROTECTED_PROCESS_NAMES.iter().any(|&n| proc_name.eq_ignore_ascii_case(n)) {
            warn!(process = proc_name, "Denied: target is a protected process");
            return PolicyDecision::Denied {
                reason: format!("'{}' is a protected system process and cannot be targeted", proc_name),
            };
        }
    }

    // Gate 3: Protected path check
    if let Some(path) = target_path {
        if is_protected_path(path) {
            warn!(path, "Denied: target path is protected");
            return PolicyDecision::Denied {
                reason: format!("'{}' is a protected system path", path),
            };
        }
    }

    // Gate 4: Safety level gating
    match safety_level {
        0 => PolicyDecision::Allowed,                               // AutoSafe
        1 if consent_given  => PolicyDecision::Allowed,             // ApprovalRequired + consented
        1 if !consent_given => PolicyDecision::PendingConsent,      // ApprovalRequired, no consent yet
        2 if consent_given  => PolicyDecision::RequiresElevation,   // ElevationRequired + consented
        2 if !consent_given => PolicyDecision::PendingConsent,
        _ => PolicyDecision::Denied { reason: "Unknown safety level".into() },
    }
}

fn is_protected_path(path: &str) -> bool {
    const PROTECTED_PATH_PREFIXES: &[&str] = &[
        "/System",
        "/Library/System",
        "/usr/lib",
        "/bin",
        "/sbin",
        "/usr/bin",
        "/usr/sbin",
        "/private/var/db",
        "C:\\Windows",
        "C:\\Windows\\System32",
    ];

    PROTECTED_PATH_PREFIXES.iter().any(|prefix| {
        path.starts_with(prefix)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_protected_process_always_denied() {
        for proc in PROTECTED_PROCESS_NAMES {
            let decision = evaluate_action("KillProcess", Some(proc), None, 1, true);
            assert!(
                matches!(decision, PolicyDecision::Denied { .. }),
                "Protected process {} should always be denied", proc
            );
        }
    }

    #[test]
    fn test_never_autofix_always_blocked() {
        let decision = evaluate_action("Reboot", None, None, 3, true);
        assert!(matches!(decision, PolicyDecision::NeverAllowed { .. }));
    }

    #[test]
    fn test_autosafe_allowed_without_consent() {
        let decision = evaluate_action("FlushDnsCache", None, None, 0, false);
        assert_eq!(decision, PolicyDecision::Allowed);
    }

    #[test]
    fn test_approval_required_blocked_without_consent() {
        let decision = evaluate_action("KillProcess", Some("Safari"), None, 1, false);
        assert_eq!(decision, PolicyDecision::PendingConsent);
    }

    #[test]
    fn test_approval_required_allowed_with_consent() {
        let decision = evaluate_action("KillProcess", Some("Safari"), None, 1, true);
        assert_eq!(decision, PolicyDecision::Allowed);
    }

    #[test]
    fn test_protected_path_denied() {
        let decision = evaluate_action("ClearCache", None, Some("/System/Library"), 0, true);
        assert!(matches!(decision, PolicyDecision::Denied { .. }));
    }
}
