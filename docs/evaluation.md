# Evaluation Strategy and Test Plan

Testing an endpoint health agent without waiting for real-world incidents requires a synthetic scenario harness, fixture-driven unit tests, and cross-platform parity checks.

---

## Test Layers

### Layer 1: Unit Tests (per-crate)

Every crate has `#[test]` tests in `src/` or a `tests/` module. These never touch the OS, never spawn processes, never write files outside of tempdir.

**collectors:**
- Test that RawSnapshot serializes/deserializes round-trip without data loss
- Test that each collector fails gracefully when its data source returns an error
- Test platform-specific field is None on the wrong platform (use cfg gates)

**signal-normalizer:**
- Test that each raw snapshot field maps to the expected normalized field with correct unit conversion
- Test that partially missing raw data (collector error) produces valid normalized snapshot with Option::None fields
- Test all enum conversions (thermal state, memory pressure, disk type)

**feature-extractor:**
- Test that each feature is computed correctly from a known normalized snapshot
- Test derived features (used_pct = used / total, etc.) with edge cases (zero total, u64 overflow)
- Test that optional features are None when their source fields are None

**trigger-engine:**
- Test each trigger type fires correctly at threshold boundary (at, above, below)
- Test hysteresis: trigger fires, then does not re-fire until recovery threshold is crossed
- Test cooldown: rapid-fire events within cooldown window produce only one trigger
- Test storm prevention: >10 triggers in 60s produces a single batched trigger
- Test multiple simultaneous triggers are batched into one diagnostic run

**detector (rules engine):**
- Test each rule produces the expected DiagnosisOutput for a synthetic FeatureSet
- Test confidence levels (Confirmed/Probable/Possible/Inconclusive) map to correct conditions
- Test that inconclusive returns empty diagnoses list, not an error
- Test that multiple simultaneous issue classes produce multiple DiagnosisOutput entries

**remediation (planner):**
- Test that each DiagnosisOutput maps to the correct RemediationPlan
- Test that NeverAutoFix issues produce only recommended_only entries
- Test that pre_checks are populated correctly per action type
- Test that protected process names are not included as kill targets

**policy (safety gate):**
- Test AutoSafe actions pass gate without waiting for consent
- Test ApprovalRequired actions are blocked pending consent
- Test ElevationRequired actions require both consent and elevation
- Test NeverAutoFix actions are always blocked
- Test protected process list blocks kill actions unconditionally

**storage:**
- Test that every data contract type serializes to SQLite and deserializes correctly
- Test WAL mode is enabled after DB creation
- Test 7-day retention correctly expires old snapshots
- Test audit log is append-only (UPDATE/DELETE operations on audit_log table should fail or panic)

**audit:**
- Test that every event type produces a valid AuditEvent
- Test event chain: prev_event_id of event N+1 equals event_id of event N
- Test sequence numbers are monotonically increasing

---

### Layer 2: Collector Validation Tests

These verify that collectors return plausible values on the test machine. They run on both macOS and Windows in CI.

```
test_cpu_usage_is_valid_range:
  collected = collect_cpu()
  assert 0.0 <= collected.usage_total_pct <= 100.0
  assert collected.core_count >= 1

test_memory_values_are_consistent:
  collected = collect_memory()
  assert collected.available_bytes <= collected.total_bytes
  assert collected.used_bytes <= collected.total_bytes
  assert collected.used_bytes + collected.available_bytes <= collected.total_bytes

test_disk_mount_is_nonempty:
  collected = collect_disks()
  assert len(collected.mounts) >= 1
  let system = find_system_mount(collected)
  assert system.free_bytes < system.total_bytes
  assert system.free_bytes > 0   // if this fails on a CI runner, something is wrong with CI

test_network_interface_exists:
  collected = collect_network()
  assert len(collected.interfaces) >= 1

test_process_list_is_nonempty:
  collected = collect_processes()
  assert len(collected) > 5    // any sane system has at least 5 processes
  assert_exists(process where name == "kernel_task" OR name == "System")
```

These tests are marked `#[ignore]` by default and run explicitly via `cargo test -- --include-ignored` in CI, since they require a real OS.

---

### Layer 3: Cross-Platform Parity Tests

Run the same fixture snapshot through the normalizer and feature extractor on macOS CI and Windows CI. Assert that cross-platform fields produce identical output.

**Test design:**
1. Load a pre-built `RawSnapshot` fixture (JSON file from `fixtures/snapshots/`)
2. Run through `signal-normalizer` → `feature-extractor`
3. Assert that all cross-platform fields in the `FeatureSet` match expected values
4. Assert that platform-specific fields are Some() on the target platform and None on the other

**Key parity tests:**
- `cpu_usage_avg_1m` — same value from same snapshot
- `memory_used_pct` — same value from same snapshot
- `system_disk_free_pct` — same value from same snapshot
- `memory_pressure` — Some(Critical) on macOS fixture, None on Windows fixture (different field)
- `commit_charge_pct` — None on macOS fixture, Some(...) on Windows fixture

This catches regressions where platform-specific code leaks into cross-platform fields.

---

### Layer 4: Synthetic Issue Scenario Tests

These are the most valuable tests. Each test:
1. Loads or constructs a `NormalizedSignalSnapshot` that represents a specific issue
2. Runs it through the full pipeline: feature-extractor → trigger-engine → detector
3. Asserts the correct `DiagnosisOutput` with correct confidence and issue class

**Fixture structure:**
```
fixtures/scenarios/
  high_cpu_single_process.json       # One process at 85% CPU
  high_cpu_distributed.json          # Multiple processes, no one at >30%
  high_cpu_post_update.json          # CPU high + recent_update_detected = true
  memory_critical_pressure.json      # macOS: memory_pressure = Critical, swap 2.5 GB
  memory_oom_risk.json               # available_memory_mb = 80
  disk_system_critical.json          # system_disk_free_pct = 3.0
  disk_full_with_pending_update.json # free = 8%, pending_update = true
  disk_io_spotlight.json             # top_disk_process = "mds_stores", queue_depth = 12
  network_dns_failure.json           # dns_resolution_fail_rate = 0.9
  network_gateway_unreachable.json   # gateway_ping_ms = null, network_reachable = false
  network_vpn_dns_split.json         # vpn_active = true, dns_resolution_fail_rate = 0.7
  battery_health_degraded.json       # battery_health_pct = 71.0, cycle_count = 950
  thermal_critical.json              # cpu_temp_celsius = 102, thermal_state = Critical
  startup_excessive.json             # startup_items_count = 22
  app_crash_repeated.json            # app_crash_count_24h = 6, same process
  service_restart_loop.json          # service_restart_count_1h = 12
  update_overdue_reboot.json         # reboot_pending_since_hours = 200
  healthy_baseline.json              # All metrics in normal range — expect zero diagnoses
```

**Example test:**
```rust
#[test]
fn test_diagnose_memory_critical_pressure() {
    let snapshot: NormalizedSignalSnapshot =
        load_fixture("memory_critical_pressure.json");
    let features = extract_features(&snapshot);
    let trigger = evaluate_trigger(&features);
    assert!(trigger.is_some(), "Trigger should fire for critical memory pressure");

    let diagnosis = run_rules(&features);
    assert_eq!(diagnosis.diagnoses.len(), 1);
    assert_eq!(diagnosis.diagnoses[0].issue_class, IssueClass::HighMemoryUsage);
    assert_eq!(diagnosis.diagnoses[0].confidence, Confidence::Confirmed);
    assert_eq!(diagnosis.diagnoses[0].severity, Severity::Critical);
}

#[test]
fn test_healthy_machine_no_diagnosis() {
    let snapshot: NormalizedSignalSnapshot =
        load_fixture("healthy_baseline.json");
    let features = extract_features(&snapshot);
    let trigger = evaluate_trigger(&features);
    assert!(trigger.is_none(), "Healthy machine should not trigger diagnosis");
}
```

---

### Layer 5: False Positive / False Negative Testing

**False positive matrix:**
For each issue class, define "safe" fixture values that are near-threshold but should NOT trigger a diagnosis.

| Issue | Near-Miss Fixture | Expected Result |
|---|---|---|
| High CPU | cpu_avg_5m = 82%, top_process = 50% (video export) | Should not trigger at 82% — threshold is 85% |
| High Memory | memory_used_pct = 88%, pressure = Warning | Probable, not Confirmed; no auto-fix |
| Low Disk | disk_free_pct = 12% | No trigger (threshold is 10%) |
| Disk I/O | queue_depth = 3.5 for 2 samples | No trigger (need 5 sustained samples) |
| Network | dns_fail_rate = 0.15 | No trigger (threshold is 20%) |

**False negative matrix:**
For each issue class, define clear-case fixtures that MUST trigger a diagnosis.

| Issue | Clear-Case Fixture | Expected Confidence |
|---|---|---|
| High CPU | cpu_avg_5m = 92%, top_process = 88% | Confirmed |
| High Memory | memory_pressure = Emergency | Confirmed |
| Low Disk | disk_free_pct = 4% | Confirmed |
| Network | network_reachable = false | Confirmed |
| Thermal | cpu_temp = 105°C | Confirmed |

---

### Layer 6: Remediation Safety Tests

These tests verify the safety gate and executor behavior. They run against mock OS operations — no actual system modification.

```rust
#[test]
fn test_never_kill_protected_process() {
    let plan = RemediationPlan::with_action(ActionType::KillProcess {
        pid: 1, name: "launchd".to_string()
    });
    let gate_result = policy::evaluate(&plan);
    assert_eq!(gate_result, PolicyDecision::Denied);
    assert!(gate_result.reason.contains("protected process"));
}

#[test]
fn test_approval_required_action_blocked_without_consent() {
    let plan = RemediationPlan::with_action(ActionType::KillProcess {
        pid: 4312, name: "Safari".to_string()
    });
    let gate_result = policy::evaluate_without_consent(&plan);
    assert_eq!(gate_result, PolicyDecision::PendingConsent);
}

#[test]
fn test_clear_cache_respects_approved_paths_only() {
    let illegal = ActionType::ClearCache {
        path: PathBuf::from("/System/Library/Caches")
    };
    let gate_result = policy::evaluate(&RemediationPlan::with_action(illegal));
    assert_eq!(gate_result, PolicyDecision::Denied);
    assert!(gate_result.reason.contains("protected path"));
}

#[test]
fn test_never_autofix_cannot_be_elevated() {
    let plan = RemediationPlan::with_action(ActionType::Reboot);
    let gate_result = policy::evaluate_with_elevation(&plan);
    assert_eq!(gate_result, PolicyDecision::NeverAllowed);
}
```

---

### Layer 7: Before/After Verification Tests

Test the `verifier` component against mock post-remediation snapshots.

```rust
#[test]
fn test_verification_passes_when_memory_recovered() {
    let pre = load_fixture("memory_critical_pressure.json");
    let post = load_fixture("healthy_baseline.json");
    let result = verifier::verify(
        IssueClass::HighMemoryUsage,
        &pre.into_features(),
        &post.into_features()
    );
    assert!(result.overall_resolved);
    assert!(result.checks.iter().all(|c| c.passed));
}

#[test]
fn test_verification_fails_when_disk_still_low() {
    let pre = load_fixture("disk_system_critical.json");
    let post = load_fixture("disk_still_low.json");  // 8% free, still below 10%
    let result = verifier::verify(IssueClass::LowDiskSpace, &pre.into_features(), &post.into_features());
    assert!(!result.overall_resolved);
    assert!(result.follow_up_recommended);
}
```

---

### Layer 8: Resource Footprint Benchmarks

These run as CI benchmarks against limits:

| Metric | Limit | Test Method |
|---|---|---|
| Binary size (release) | <15 MB | `stat -c%s target/release/firstaid` |
| Cold start time | <500ms | `time firstaid --version` |
| `diagnose` command latency | <3s | `time firstaid diagnose` on CI runner |
| Peak RSS during diagnose | <100 MB | `/usr/bin/time -v firstaid diagnose` |
| Idle RSS (daemon) | <30 MB | Monitor daemon process after 60s idle |
| SQLite DB size after 7 days | <50 MB | Synthetic 7-day snapshot injection test |
| Model load + inference | <50ms | Microbenchmark in `inference` crate |

---

## Synthetic Scenario Harness

The harness allows injecting synthetic snapshots into the running pipeline without touching the OS:

```
Usage:
  firstaid --test-mode --fixture fixtures/scenarios/high_cpu_single_process.json
  
Output:
  {
    "trigger": { "fired": true, "reason": "cpu_avg_5m_above_85pct" },
    "diagnosis": { ... DiagnosisOutput ... },
    "plan": { ... RemediationPlan ... },
    "gate": { "decision": "ApprovalRequired", "would_execute_if_consented": true },
    "verification": "(simulated)"
  }
```

This mode:
- Skips all OS calls
- Injects the fixture as a `NormalizedSignalSnapshot`
- Runs the full pipeline deterministically
- Prints the complete pipeline output as JSON
- Returns exit code 0 if pipeline succeeded, 1 if any stage errored

This enables the CI scenario matrix to be a simple shell loop:

```bash
for fixture in fixtures/scenarios/*.json; do
  firstaid --test-mode --fixture "$fixture" --expected-diagnosis "$(basename $fixture .json).expected.json"
  if [ $? -ne 0 ]; then
    echo "FAIL: $fixture"
    exit 1
  fi
done
```
