//! CLI rendering helpers — format DiagnosisOutput and RemediationPlan for human consumption.

use detector::types::{DiagnosisOutput, Severity};
use remediation::{RemediationPlan, SafetyLevel};

const BOLD: &str = "\x1b[1m";
const RED: &str = "\x1b[31m";
const YELLOW: &str = "\x1b[33m";
const GREEN: &str = "\x1b[32m";
const CYAN: &str = "\x1b[36m";
const RESET: &str = "\x1b[0m";

pub fn render_diagnosis(d: &DiagnosisOutput) {
    if d.diagnoses.is_empty() {
        println!("{GREEN}✓ No issues detected.{RESET}");
        return;
    }

    println!("\n{BOLD}━━━ Endpoint Health Diagnosis ━━━━━━━━━━━━━━━━━━━━━━━━━━━{RESET}\n");
    println!("  Diagnosed at: {}", d.diagnosed_at.format("%Y-%m-%d %H:%M:%S UTC"));
    println!("  Issues found: {}", d.diagnoses.len());
    println!();

    for (i, diag) in d.diagnoses.iter().enumerate() {
        let color = match diag.severity {
            Severity::Critical => RED,
            Severity::High => RED,
            Severity::Medium => YELLOW,
            Severity::Low | Severity::Info => GREEN,
        };

        println!("{color}{BOLD}[{}/{}] {:?} — {:?}{RESET}",
            i + 1, d.diagnoses.len(), diag.issue_class, diag.severity);
        println!("  {BOLD}{}{RESET}", diag.title);
        println!();
        // Word-wrap the explanation at 72 chars
        for line in wrap_text(&diag.explanation, 70) {
            println!("  {}", line);
        }
        println!();
        println!("  {CYAN}Evidence:{RESET}");
        for ev in &diag.evidence {
            let threshold_str = ev.threshold.as_ref()
                .map(|t| format!(" (threshold: {})", t))
                .unwrap_or_default();
            println!("    • {}: {}{}", ev.field, ev.value, threshold_str);
        }
        println!();
    }

    println!("{BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━{RESET}");
    println!("Run {CYAN}firstaid plan{RESET} to see available remediation actions.\n");
}

pub fn render_plan(plan: &RemediationPlan) {
    println!("\n{BOLD}━━━ Remediation Plan ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━{RESET}\n");

    if !plan.actions.is_empty() {
        println!("{BOLD}Executable actions:{RESET}");
        for (i, action) in plan.actions.iter().enumerate() {
            let badge = match action.safety_level {
                SafetyLevel::AutoSafe          => format!("{GREEN}[AUTO]{RESET}"),
                SafetyLevel::ApprovalRequired  => format!("{YELLOW}[APPROVAL REQUIRED]{RESET}"),
                SafetyLevel::ElevationRequired => format!("{RED}[ELEVATION REQUIRED]{RESET}"),
                SafetyLevel::NeverAutoFix      => format!("{RED}[NEVER AUTO]{RESET}"),
            };
            println!("  {}. {} {}", i + 1, badge, action.title);
            println!("     {}", action.description);
            if !action.rollback_note.is_empty() {
                println!("     {CYAN}Rollback: {}{RESET}", action.rollback_note);
            }
            println!();
        }
    }

    if !plan.recommended_only.is_empty() {
        println!("{BOLD}Recommendations (manual action required):{RESET}");
        for r in &plan.recommended_only {
            println!("  • {}", r.title);
            println!("    {}", r.description);
            println!();
        }
    }

    println!("{BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━{RESET}");
    println!("Run {CYAN}firstaid fix --diagnosis-id <id>{RESET} to execute with consent.\n");
}

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if !current.is_empty() && current.len() + word.len() + 1 > width {
            lines.push(current.clone());
            current.clear();
        }
        if !current.is_empty() { current.push(' '); }
        current.push_str(word);
    }
    if !current.is_empty() { lines.push(current); }
    lines
}
