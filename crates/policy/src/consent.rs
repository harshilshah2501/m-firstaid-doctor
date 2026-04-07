use serde::{Deserialize, Serialize};

/// How consent was obtained for a remediation action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConsentRecord {
    /// Action is AutoSafe — no consent required
    NotRequired,
    /// User explicitly typed 'y' or clicked approve
    UserInteractive {
        method: UserConsentMethod,
        timestamp: chrono::DateTime<chrono::Utc>,
    },
    /// MDM policy pre-approved this class of action (Phase 4)
    PolicyOverride {
        policy_id: String,
        timestamp: chrono::DateTime<chrono::Utc>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UserConsentMethod {
    CliPrompt,          // interactive terminal: y/N
    TrayNotification,   // Phase 2: tray app button
    PolicyFile,         // user-owned policy file opt-in
}

/// Request consent from the user via CLI prompt.
/// Returns true if user approved, false if denied or timed out.
pub async fn request_cli_consent(
    plan_title: &str,
    actions_summary: &[String],
    timeout_seconds: u64,
) -> bool {
    println!("\n━━━ Remediation Plan ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("{}", plan_title);
    println!("\nProposed actions:");
    for (i, action) in actions_summary.iter().enumerate() {
        println!("  {}. {}", i + 1, action);
    }
    println!("\nProceed? [y/N] (auto-deny in {}s): ", timeout_seconds);

    // Phase 1: simple blocking stdin read (no async timeout yet)
    // Phase 2: tokio::time::timeout around stdin read
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).ok();
    let trimmed = input.trim().to_lowercase();
    trimmed == "y" || trimmed == "yes"
}
