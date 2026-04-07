use crate::types::*;
use chrono::Utc;
use tracing::{info, warn, error};
use uuid::Uuid;

/// Execute a RemediationPlan with the given consent.
/// Returns an ExecutionResult regardless of success/failure.
pub async fn execute(plan: &RemediationPlan, consent: ConsentMethod) -> ExecutionResult {
    let mut action_results = Vec::new();
    let mut any_success = false;
    let mut any_failure = false;

    for action in &plan.actions {
        // Verify the action's safety level is compatible with consent
        if !consent_allows(&action.safety_level, &consent) {
            action_results.push(ActionResult {
                action_id: action.action_id,
                status: ActionStatus::Denied,
                started_at: Utc::now(),
                completed_at: Some(Utc::now()),
                pre_check_results: vec![],
                output: None,
                error: Some("Safety level requires explicit consent".into()),
                elapsed_ms: 0,
            });
            any_failure = true;
            continue;
        }

        let result = execute_action(action).await;
        if result.status == ActionStatus::Success {
            any_success = true;
        } else {
            any_failure = true;
        }
        action_results.push(result);
    }

    let overall = match (any_success, any_failure) {
        (true, false)  => OverallStatus::Success,
        (true, true)   => OverallStatus::PartialSuccess,
        (false, true)  => OverallStatus::Failed,
        (false, false) => OverallStatus::Success,  // no actions = trivial success
    };

    ExecutionResult {
        schema_version: "1.0".into(),
        result_id: Uuid::new_v4(),
        plan_id: plan.plan_id,
        executed_at: Utc::now(),
        consent_obtained: true,
        consent_method: consent,
        actions: action_results,
        overall_status: overall,
    }
}

fn consent_allows(safety: &SafetyLevel, consent: &ConsentMethod) -> bool {
    match (safety, consent) {
        (SafetyLevel::AutoSafe, _)                  => true,
        (SafetyLevel::ApprovalRequired, ConsentMethod::UserApproval)  => true,
        (SafetyLevel::ApprovalRequired, ConsentMethod::PolicyOverride) => true,
        (SafetyLevel::ElevationRequired, ConsentMethod::UserApproval) => true,
        (SafetyLevel::NeverAutoFix, _)              => false,
        _                                            => false,
    }
}

async fn execute_action(action: &RemediationAction) -> ActionResult {
    let start = Utc::now();

    // Run pre-checks
    let pre_check_results = run_pre_checks(&action.pre_checks);
    if pre_check_results.iter().any(|c| !c.passed) {
        warn!(action_id = %action.action_id, "Pre-check failed; skipping action");
        return ActionResult {
            action_id: action.action_id,
            status: ActionStatus::SkippedPreCheck,
            started_at: start,
            completed_at: Some(Utc::now()),
            pre_check_results,
            output: None,
            error: Some("One or more pre-checks failed".into()),
            elapsed_ms: (Utc::now() - start).num_milliseconds() as u64,
        };
    }

    info!(action_type = ?action.action_type, "Executing action");
    let result = dispatch_action(&action.action_type).await;

    let elapsed = (Utc::now() - start).num_milliseconds() as u64;

    match result {
        Ok(output) => ActionResult {
            action_id: action.action_id,
            status: ActionStatus::Success,
            started_at: start,
            completed_at: Some(Utc::now()),
            pre_check_results,
            output: Some(output),
            error: None,
            elapsed_ms: elapsed,
        },
        Err(e) => {
            error!(action_id = %action.action_id, error = %e, "Action execution failed");
            ActionResult {
                action_id: action.action_id,
                status: ActionStatus::Failed,
                started_at: start,
                completed_at: Some(Utc::now()),
                pre_check_results,
                output: None,
                error: Some(e.to_string()),
                elapsed_ms: elapsed,
            }
        }
    }
}

fn run_pre_checks(checks: &[String]) -> Vec<PreCheckResult> {
    // In Phase 1, pre-checks are logged but not mechanically verified.
    // Phase 2 adds concrete check implementations per check description.
    checks.iter().map(|c| PreCheckResult {
        check: c.clone(),
        passed: true,  // TODO Phase 2: implement per-check verification
        detail: None,
    }).collect()
}

async fn dispatch_action(action: &ActionType) -> anyhow::Result<serde_json::Value> {
    match action {
        ActionType::Report { message } => {
            info!("REPORT: {}", message);
            Ok(serde_json::json!({"reported": message}))
        }

        ActionType::FlushDnsCache => {
            flush_dns_cache().await
        }

        ActionType::KillProcess { pid, process_name, signal, fallback } => {
            kill_process(*pid, process_name, signal, fallback.as_ref()).await
        }

        ActionType::ClearCache { target_paths, min_age_days, dry_run } => {
            clear_cache(target_paths, *min_age_days, *dry_run).await
        }

        ActionType::RunConnectivityProbe { probe_type } => {
            run_probe(probe_type).await
        }

        ActionType::ReprioritizeProcess { pid, nice_value } => {
            renice(*pid, *nice_value).await
        }

        ActionType::DisableStartupItem { name, source } => {
            Err(anyhow::anyhow!("DisableStartupItem not yet implemented in Phase 1"))
        }

        ActionType::EnableStartupItem { name, source } => {
            Err(anyhow::anyhow!("EnableStartupItem not yet implemented in Phase 1"))
        }

        ActionType::RestartService { service_name } => {
            Err(anyhow::anyhow!("RestartService not yet implemented in Phase 1"))
        }

        ActionType::StopService { service_name } => {
            Err(anyhow::anyhow!("StopService not yet implemented in Phase 1"))
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Platform-branched action implementations
// ──────────────────────────────────────────────────────────────────────────────

async fn flush_dns_cache() -> anyhow::Result<serde_json::Value> {
    #[cfg(target_os = "macos")]
    {
        let status = tokio::process::Command::new("dscacheutil")
            .arg("-flushcache")
            .status()
            .await?;
        if status.success() {
            // macOS also requires killing mDNSResponder
            let _ = tokio::process::Command::new("killall")
                .args(["-HUP", "mDNSResponder"])
                .status()
                .await;
            return Ok(serde_json::json!({"flushed": true, "platform": "macos"}));
        }
        return Err(anyhow::anyhow!("dscacheutil -flushcache failed"));
    }

    #[cfg(target_os = "windows")]
    {
        let status = tokio::process::Command::new("ipconfig")
            .arg("/flushdns")
            .status()
            .await?;
        if status.success() {
            return Ok(serde_json::json!({"flushed": true, "platform": "windows"}));
        }
        return Err(anyhow::anyhow!("ipconfig /flushdns failed"));
    }

    #[allow(unreachable_code)]
    Err(anyhow::anyhow!("DNS flush not supported on this platform"))
}

async fn kill_process(
    pid: u32,
    name: &str,
    signal: &Signal,
    fallback: Option<&Signal>,
) -> anyhow::Result<serde_json::Value> {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        let sig = match signal {
            Signal::Sigterm => nix::sys::signal::Signal::SIGTERM,
            Signal::Sigkill => nix::sys::signal::Signal::SIGKILL,
            Signal::Sigstop => nix::sys::signal::Signal::SIGSTOP,
        };
        // Safety: kill() only sends a signal; it does not grant any new capabilities.
        // The process must be owned by the current user or we will get EPERM.
        nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(pid as i32),
            sig,
        ).map_err(|e| anyhow::anyhow!("kill({pid}, {sig}): {e}"))?;
        return Ok(serde_json::json!({"pid": pid, "signal": format!("{:?}", signal), "sent": true}));
    }

    #[cfg(target_os = "windows")]
    {
        let status = tokio::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .status()
            .await?;
        if status.success() {
            return Ok(serde_json::json!({"pid": pid, "killed": true}));
        }
        return Err(anyhow::anyhow!("taskkill failed for PID {}", pid));
    }

    #[allow(unreachable_code)]
    Err(anyhow::anyhow!("Process kill not supported on this platform"))
}

async fn clear_cache(
    target_paths: &[String],
    min_age_days: u32,
    dry_run: bool,
) -> anyhow::Result<serde_json::Value> {
    // Validated list of approved paths — never delete outside these
    const APPROVED_PREFIXES: &[&str] = &[
        "/private/tmp",
        "/tmp",
        "/Library/Caches",
    ];

    let home = std::env::var("HOME").unwrap_or_default();
    let approved: Vec<String> = APPROVED_PREFIXES.iter()
        .map(|p| p.to_string())
        .chain(std::iter::once(format!("{}/Library/Caches", home)))
        .collect();

    let mut total_bytes: u64 = 0;
    let mut files_deleted: u64 = 0;

    for raw_path in target_paths {
        let expanded = raw_path.replace("~", &home);
        // Safety gate: reject any path not in the approved list
        let is_approved = approved.iter().any(|a| expanded.starts_with(a.as_str()));
        if !is_approved {
            warn!(path = %expanded, "Rejected non-approved cache path");
            continue;
        }

        // TODO Phase 2: walk directory, filter by mtime > min_age_days, collect candidates,
        // verify no open file handles, then delete if !dry_run
        info!(path = %expanded, dry_run, "Would clear cache (Phase 1: dry-run only)");
    }

    Ok(serde_json::json!({
        "files_deleted": files_deleted,
        "bytes_freed": total_bytes,
        "dry_run": dry_run,
    }))
}

async fn run_probe(probe_type: &ProbeType) -> anyhow::Result<serde_json::Value> {
    match probe_type {
        ProbeType::DnsLookup { domain } => {
            let start = std::time::Instant::now();
            let result = tokio::net::lookup_host(format!("{}:80", domain)).await;
            let elapsed = start.elapsed().as_millis();
            Ok(serde_json::json!({
                "domain": domain,
                "resolved": result.is_ok(),
                "latency_ms": elapsed,
            }))
        }
        ProbeType::PingGateway | ProbeType::PingPublicDns => {
            // Phase 2: implement ICMP ping via raw socket
            Ok(serde_json::json!({"probe": "not yet implemented", "result": null}))
        }
    }
}

async fn renice(pid: u32, nice_value: i8) -> anyhow::Result<serde_json::Value> {
    #[cfg(unix)]
    {
        let result = unsafe {
            libc::setpriority(libc::PRIO_PROCESS, pid, nice_value as i32)
        };
        if result == 0 {
            return Ok(serde_json::json!({"pid": pid, "nice": nice_value}));
        }
        return Err(anyhow::anyhow!("setpriority failed: errno {}", std::io::Error::last_os_error()));
    }
    #[allow(unreachable_code)]
    Err(anyhow::anyhow!("renice not supported on this platform"))
}
