# Data Contracts

Concrete JSON schemas and examples for all inter-component data structures. These are the machine-usable contracts. All types map directly to Rust structs with `#[derive(Serialize, Deserialize)]`.

---

## 1. Raw Collector Snapshot

**Produced by:** `collectors` + platform adapters
**Consumed by:** `signal-normalizer`

```json
{
  "schema_version": "1.0",
  "collected_at": "2025-04-07T14:23:01.532Z",
  "platform": "macos",
  "collector_errors": [],
  "cpu": {
    "usage_total_pct": 87.3,
    "usage_per_core_pct": [92.1, 88.4, 81.2, 87.5],
    "user_pct": 71.2,
    "system_pct": 16.1,
    "idle_pct": 12.7
  },
  "memory": {
    "total_bytes": 17179869184,
    "available_bytes": 524288000,
    "used_bytes": 16655581184,
    "swap_total_bytes": 4294967296,
    "swap_used_bytes": 2684354560,
    "platform_specific": {
      "macos_pressure_level": 2
    }
  },
  "disks": [
    {
      "mount_point": "/",
      "device": "disk3s1",
      "fs_type": "apfs",
      "total_bytes": 494384795648,
      "free_bytes": 12884901888,
      "read_bytes_total": 987654321,
      "write_bytes_total": 123456789
    }
  ],
  "network": {
    "interfaces": [
      {
        "name": "en0",
        "ipv4": "192.168.1.42",
        "ipv6": "fe80::1",
        "mac": "aa:bb:cc:dd:ee:ff",
        "rx_bytes": 104857600,
        "tx_bytes": 52428800,
        "wifi_ssid": "HomeNetwork",
        "wifi_signal_dbm": -62
      }
    ],
    "dns_servers": ["192.168.1.1", "8.8.8.8"]
  },
  "processes": [
    {
      "pid": 4312,
      "name": "com.apple.WebKit.WebContent",
      "cpu_pct": 78.4,
      "rss_bytes": 1073741824,
      "status": "running",
      "parent_pid": 412,
      "start_time": "2025-04-07T08:11:00Z"
    }
  ],
  "platform_specific": {
    "macos": {
      "thermal_state": 1,
      "battery_charge_pct": 61.0,
      "battery_health_pct": 87.3,
      "battery_cycle_count": 412,
      "battery_current_ma": -2100,
      "cpu_temp_celsius": 88.5,
      "fan_speed_rpm": 5400,
      "launch_agents_count": 14,
      "launch_daemons_count": 42,
      "memory_pressure_level": 2
    }
  }
}
```

**Schema notes:**
- `collector_errors`: list of `{collector: string, error: string}` — partial collection is OK
- `platform_specific` is a tagged union; only one of `macos` / `windows` is populated
- macOS `thermal_state`: 0=Normal, 1=Fair, 2=Serious, 3=Critical
- macOS `memory_pressure_level`: 0=Normal, 1=Warning, 2=Critical, 3=Emergency

---

## 2. Normalized Signal Snapshot

**Produced by:** `signal-normalizer`
**Consumed by:** `trigger-engine`, `feature-extractor`, `storage`

```json
{
  "schema_version": "1.0",
  "snapshot_id": "018ed3f1-b2a4-7c91-8c12-1a2b3c4d5e6f",
  "collected_at": "2025-04-07T14:23:01.532Z",
  "platform": "macos",
  "cpu": {
    "usage_avg_1m": 87.3,
    "usage_avg_5m": 81.6,
    "usage_avg_15m": 72.1,
    "core_count": 4,
    "user_pct": 71.2,
    "system_pct": 16.1,
    "top_process": {
      "pid": 4312,
      "name": "com.apple.WebKit.WebContent",
      "cpu_pct": 78.4,
      "is_system": true
    }
  },
  "memory": {
    "total_mb": 16384,
    "available_mb": 500,
    "used_pct": 96.9,
    "swap_used_mb": 2560,
    "swap_used_pct": 60.0,
    "pressure": "Critical",
    "top_process": {
      "pid": 4312,
      "name": "com.apple.WebKit.WebContent",
      "rss_mb": 1024
    }
  },
  "disks": [
    {
      "mount_point": "/",
      "is_system": true,
      "free_pct": 2.6,
      "free_bytes": 12884901888,
      "total_bytes": 494384795648,
      "disk_type": "SSD",
      "smart_status": "Ok"
    }
  ],
  "network": {
    "reachable": true,
    "ipv4": "192.168.1.42",
    "wifi_ssid": "HomeNetwork",
    "wifi_signal_dbm": -62,
    "dns_servers": ["192.168.1.1", "8.8.8.8"],
    "vpn_active": false,
    "proxy_configured": false
  },
  "thermal": {
    "state": "Serious",
    "cpu_temp_celsius": 88.5,
    "fan_speed_rpm": 5400
  },
  "battery": {
    "present": true,
    "charge_pct": 61.0,
    "charging": false,
    "health_pct": 87.3,
    "cycle_count": 412,
    "current_ma": -2100,
    "time_remaining_min": 87
  },
  "startup": {
    "items_count": 14,
    "launch_agents_count": 14,
    "launch_daemons_count": 42
  },
  "updates": {
    "pending_reboot_required": false,
    "recent_update_detected": false
  }
}
```

---

## 3. Feature Set

**Produced by:** `feature-extractor`
**Consumed by:** `trigger-engine`, `detector` (rules + ML)

```json
{
  "schema_version": "1.0",
  "computed_at": "2025-04-07T14:23:02.101Z",
  "snapshot_id": "018ed3f1-b2a4-7c91-8c12-1a2b3c4d5e6f",
  "cpu_usage_avg_1m": 87.3,
  "cpu_usage_avg_5m": 81.6,
  "cpu_usage_avg_15m": 72.1,
  "cpu_core_count": 4,
  "top_process_cpu_pct": 78.4,
  "top_process_name": "com.apple.WebKit.WebContent",
  "top_process_is_system": true,
  "memory_used_pct": 96.9,
  "memory_available_mb": 500,
  "swap_used_mb": 2560,
  "swap_used_pct": 60.0,
  "memory_pressure": "Critical",
  "top_process_rss_mb": 1024,
  "top_process_mem_name": "com.apple.WebKit.WebContent",
  "system_disk_free_pct": 2.6,
  "system_disk_free_gb": 12.0,
  "disk_queue_depth": null,
  "disk_read_latency_ms": null,
  "network_reachable": true,
  "wifi_signal_dbm": -62,
  "dns_resolution_fail_rate": 0.0,
  "vpn_active": false,
  "thermal_state": "Serious",
  "cpu_temp_celsius": 88.5,
  "battery_health_pct": 87.3,
  "battery_cycle_count": 412,
  "app_crash_count_24h": 0,
  "startup_items_count": 14,
  "pending_reboot_required": false,
  "recent_update_detected": false
}
```

---

## 4. Diagnosis Output

**Produced by:** `detector`
**Consumed by:** `remediation`, `audit`, `storage`, `ui`

```json
{
  "schema_version": "1.0",
  "diagnosis_id": "018ed3f2-aaab-7c91-9d10-aabbccddeeff",
  "snapshot_id": "018ed3f1-b2a4-7c91-8c12-1a2b3c4d5e6f",
  "diagnosed_at": "2025-04-07T14:23:02.890Z",
  "diagnoses": [
    {
      "issue_class": "HighMemoryUsage",
      "confidence": "Confirmed",
      "severity": "Critical",
      "title": "Critical memory pressure — swap active, OOM risk",
      "explanation": "System is at 96.9% memory utilization with 2.5 GB of swap in use and memory pressure at Critical level. The process 'com.apple.WebKit.WebContent' (PID 4312) is consuming 1 GB of RAM. OOM kill risk is elevated.",
      "evidence": [
        {"field": "memory_pressure", "value": "Critical", "threshold": "Warning"},
        {"field": "memory_used_pct", "value": 96.9, "threshold": 90.0},
        {"field": "swap_used_mb", "value": 2560, "threshold": 500},
        {"field": "top_process_rss_mb", "value": 1024, "context": "16384 MB total"}
      ],
      "contributing_ml": null,
      "rule_name": "memory::critical_pressure_with_swap"
    },
    {
      "issue_class": "LowDiskSpace",
      "confidence": "Confirmed",
      "severity": "High",
      "title": "Low disk space — system volume at 2.6% free",
      "explanation": "The system volume has only 12 GB free (2.6%). This is below the safe threshold of 10% and may prevent OS operations, update installations, and cause application write failures.",
      "evidence": [
        {"field": "system_disk_free_pct", "value": 2.6, "threshold": 10.0}
      ],
      "contributing_ml": null,
      "rule_name": "disk::system_volume_critical"
    }
  ],
  "detection_method": "RulesOnly",
  "ml_models_invoked": [],
  "elapsed_ms": 12
}
```

**`confidence` enum:** `Confirmed | Probable | Possible | Inconclusive`
**`severity` enum:** `Critical | High | Medium | Low | Info`
**`detection_method` enum:** `RulesOnly | RulesPlusMl | MlOnly`

---

## 5. Remediation Plan

**Produced by:** `remediation::planner`
**Consumed by:** `policy`, `remediation::executor`, `audit`, `ui`

```json
{
  "schema_version": "1.0",
  "plan_id": "018ed3f3-cccc-7c91-0011-112233445566",
  "diagnosis_id": "018ed3f2-aaab-7c91-9d10-aabbccddeeff",
  "planned_at": "2025-04-07T14:23:03.100Z",
  "actions": [
    {
      "action_id": "018ed3f3-cccc-7c91-0011-aabbcc000001",
      "issue_class": "HighMemoryUsage",
      "action_type": "KillProcess",
      "title": "Offer to quit Safari/WebKit process consuming 1 GB RAM",
      "description": "Send SIGTERM to PID 4312 (com.apple.WebKit.WebContent). User will be notified first. The application (Safari) will be asked to close its renderer.",
      "safety_level": "ApprovalRequired",
      "privilege_required": "User",
      "estimated_risk": "Low",
      "reversible": false,
      "rollback_note": "Reopen Safari to restore the process. No data will be lost if Safari was not mid-write.",
      "pre_checks": [
        "Verify PID 4312 is still running",
        "Verify process name matches expected (com.apple.WebKit.WebContent)",
        "Verify process is not in D-state (uninterruptible wait)"
      ],
      "params": {
        "pid": 4312,
        "signal": "SIGTERM",
        "fallback_signal": "SIGKILL",
        "fallback_delay_ms": 5000
      }
    },
    {
      "action_id": "018ed3f3-cccc-7c91-0011-aabbcc000002",
      "issue_class": "LowDiskSpace",
      "action_type": "ClearTempFiles",
      "title": "Clear user temp files and caches",
      "description": "Remove files from ~/Library/Caches that are older than 7 days and not in use. Excludes application support data.",
      "safety_level": "ApprovalRequired",
      "privilege_required": "User",
      "estimated_risk": "Low",
      "reversible": false,
      "rollback_note": "Deleted cache files cannot be restored, but will be recreated by applications on next use.",
      "pre_checks": [
        "Verify ~/Library/Caches is accessible",
        "Enumerate files to be deleted before deletion",
        "Verify no files are currently open by running processes"
      ],
      "params": {
        "target_paths": ["~/Library/Caches"],
        "min_age_days": 7,
        "exclude_patterns": ["com.apple.*", "*.db"],
        "dry_run_first": true
      }
    }
  ],
  "recommended_only": [
    {
      "issue_class": "LowDiskSpace",
      "title": "Consider uninstalling unused large applications",
      "description": "Applications over 1 GB that have not been opened in 90+ days were found. Review in System Preferences > Storage.",
      "safety_level": "NeverAutoFix"
    }
  ]
}
```

---

## 6. Remediation Execution Result

**Produced by:** `remediation::executor`
**Consumed by:** `verifier`, `audit`, `storage`

```json
{
  "schema_version": "1.0",
  "result_id": "018ed3f4-dddd-7c91-2233-aabbccddeeff",
  "plan_id": "018ed3f3-cccc-7c91-0011-112233445566",
  "executed_at": "2025-04-07T14:25:11.430Z",
  "consent_obtained": true,
  "consent_method": "UserApproval",
  "actions": [
    {
      "action_id": "018ed3f3-cccc-7c91-0011-aabbcc000001",
      "status": "Success",
      "started_at": "2025-04-07T14:25:11.430Z",
      "completed_at": "2025-04-07T14:25:11.521Z",
      "pre_check_results": [
        {"check": "PID still running", "passed": true},
        {"check": "Process name matches", "passed": true},
        {"check": "Not in D-state", "passed": true}
      ],
      "output": {
        "signal_sent": "SIGTERM",
        "process_exited": true,
        "exit_code": 0,
        "elapsed_ms": 91
      },
      "error": null
    },
    {
      "action_id": "018ed3f3-cccc-7c91-0011-aabbcc000002",
      "status": "Success",
      "started_at": "2025-04-07T14:25:11.530Z",
      "completed_at": "2025-04-07T14:25:14.210Z",
      "pre_check_results": [
        {"check": "Cache path accessible", "passed": true},
        {"check": "No open file handles in target", "passed": true}
      ],
      "output": {
        "files_deleted": 847,
        "bytes_freed": 2416979968,
        "dry_run_bytes": 2416979968,
        "elapsed_ms": 2680
      },
      "error": null
    }
  ],
  "overall_status": "Success"
}
```

**`status` enum:** `Success | Failed | SkippedPreCheck | Denied | Timeout`

---

## 7. Verification Result

**Produced by:** `verifier`
**Consumed by:** `audit`, `storage`, `ui`

```json
{
  "schema_version": "1.0",
  "verification_id": "018ed3f5-eeee-7c91-3344-aabbccddeeff",
  "result_id": "018ed3f4-dddd-7c91-2233-aabbccddeeff",
  "verified_at": "2025-04-07T14:26:45.000Z",
  "checks": [
    {
      "issue_class": "HighMemoryUsage",
      "check": "memory_used_pct < 80%",
      "passed": true,
      "observed_value": 68.2,
      "threshold": 80.0
    },
    {
      "issue_class": "HighMemoryUsage",
      "check": "memory_pressure != Critical",
      "passed": true,
      "observed_value": "Normal",
      "threshold": "not Critical"
    },
    {
      "issue_class": "LowDiskSpace",
      "check": "system_disk_free_pct > 10%",
      "passed": true,
      "observed_value": 7.2,
      "threshold": 10.0
    }
  ],
  "overall_resolved": false,
  "unresolved_issues": ["LowDiskSpace"],
  "follow_up_recommended": true,
  "follow_up_note": "Disk space improved but still below 10% free. Additional cleanup recommended."
}
```

---

## 8. Audit Log Event

**Produced by:** `audit`
**Consumed by:** `storage`, log file, enterprise SIEM (Phase 4)

```json
{
  "schema_version": "1.0",
  "event_id": "018ed3f6-ffff-7c91-4455-aabbccddeeff",
  "sequence_num": 1042,
  "timestamp": "2025-04-07T14:23:01.532Z",
  "event_type": "DiagnosisCompleted",
  "session_id": "018ede00-0000-7c91-0000-000000000001",
  "hostname": "macbook-pro-alice",
  "platform": "macos",
  "platform_version": "14.4.1",
  "agent_version": "0.1.0",
  "payload": {
    "diagnosis_id": "018ed3f2-aaab-7c91-9d10-aabbccddeeff",
    "issues_found": ["HighMemoryUsage", "LowDiskSpace"],
    "max_severity": "Critical",
    "elapsed_ms": 12
  },
  "actor": "system",
  "prev_event_id": "018ed3f6-ffff-7c91-4455-aabbccddeeee"
}
```

**`event_type` enum values:**
```
CollectionStarted | CollectionCompleted | CollectionFailed
TriggerFired | TriggerCooledDown
DiagnosisStarted | DiagnosisCompleted
RemediationProposed | RemediationApproved | RemediationRejected | RemediationDeniedByPolicy
ActionStarted | ActionCompleted | ActionFailed | ActionSkipped
VerificationStarted | VerificationCompleted
AgentStarted | AgentStopped | ConfigChanged
ModelLoaded | ModelUnloaded | ModelChecksumFailed
```

**Tamper-evidence:** `prev_event_id` chains audit events. Any gap or out-of-order sequence is detectable by the reader. SHA-256 hash chain in Phase 4.
