# Top-10 Endpoint Issue Catalog

Each issue entry defines: observable symptoms, required signals, feature set, trigger conditions, ML necessity, deterministic decision logic, remediation options, verification checks, and safety classification.

---

## Issue 1 — High CPU Usage

**Description:** Sustained CPU utilization above safe thresholds, causing thermal stress, battery drain, and user-perceived sluggishness.

| Field | Detail |
|---|---|
| Observable symptoms | Fan spin-up, thermal throttling, UI lag, battery drain rate spike |
| Primary signals | `cpu_usage_avg_1m`, `cpu_usage_avg_5m`, `top_process_cpu_pct`, `top_process_name`, `cpu_core_count`, `thermal_state` |
| Trigger condition | `cpu_usage_avg_5m > 85%` for 3+ consecutive samples |
| Rules sufficient? | **Yes for detection; ML useful for root cause attribution** |
| Suggested model | Isolation Forest on (pid, cpu%, mem%, io_wait, process_age) — flags anomalous process behavior |
| Safety level | Auto-safe: safe process kill (user-space, non-system); Approval required: kill system-adjacent process |

**Deterministic decision logic:**
```
if top_process_cpu_pct > 80% AND process_is_user_space:
    → "Single process monopolizing CPU"
    → confidence: Confirmed
elif cpu_usage_avg_5m > 85% AND top_process_cpu_pct < 40%:
    → "Distributed CPU load — likely multiple background processes"
    → confidence: Probable
    → trigger ML: process anomaly model
elif recent_update_detected AND cpu_usage_avg_5m > 70%:
    → "Post-update indexing or compilation (mds_stores, WU, etc.)"
    → confidence: Probable
elif thermal_state == Throttling:
    → "Thermal throttling — CPU pegged due to heat"
    → confidence: Confirmed
```

**Safe remediations:**
| Action | Safety Level | Privilege |
|---|---|---|
| Notify user of process name + CPU% | Auto-safe | User |
| Suggest user-initiated quit | Auto-safe | User |
| Kill identified user-space process (with consent) | Approval required | User |
| Reduce process priority (nice/renice) | Approval required | User |
| Disable runaway launch agent | Approval required | User (launchctl) |

**Post-fix verification:** `cpu_usage_avg_1m < 50%` within 60s of action.

---

## Issue 2 — High Memory Usage / Memory Pressure

**Description:** System RAM exhausted or near exhaustion, causing heavy swapping, OOM conditions, and application unresponsiveness.

| Field | Detail |
|---|---|
| Observable symptoms | App unresponsiveness, beachball/spinner, swap usage spike, page fault rate spike |
| Primary signals | `memory_pressure` (macOS: Normal/Warning/Critical/Emergency), `memory_used_pct`, `swap_used_mb`, `page_fault_rate`, `top_process_rss_mb`, `available_memory_mb` |
| Trigger condition | `memory_pressure == Critical OR memory_used_pct > 90%` |
| Rules sufficient? | **Yes — memory pressure is directly observable and deterministic** |
| Suggested model | None needed for MVP; recurrence patterns useful in Phase 2 |
| Safety level | Auto-safe: memory stats report; Approval required: kill or compress process |

**Deterministic decision logic:**
```
if memory_pressure == Emergency OR available_memory_mb < 100:
    → "Critical memory exhaustion — OOM risk"
    → confidence: Confirmed
elif memory_pressure == Critical AND swap_used_mb > 2048:
    → "Severe memory pressure with active swap — system degraded"
    → confidence: Confirmed
elif memory_used_pct > 90% AND swap_used_mb > 500:
    → "High memory usage with swap active"
    → confidence: Probable
elif top_process_rss_mb > (total_memory_mb * 0.40):
    → "Single process consuming >40% RAM"
    → confidence: Confirmed
```

**Safe remediations:**
| Action | Safety Level | Privilege |
|---|---|---|
| Report memory usage breakdown | Auto-safe | User |
| Suggest closing unused apps | Auto-safe | User |
| Kill identified memory-hungry process (consent) | Approval required | User |
| Flush disk cache (macOS: purge / Windows: EmptyWorkingSet) | Approval required | Admin |
| Compress inactive memory (macOS: built-in; Windows: memory compaction) | Auto-safe | User |

**Post-fix verification:** `available_memory_mb > 500` and `memory_pressure != Critical`.

---

## Issue 3 — Low Disk Space

**Description:** One or more volumes approaching full capacity, blocking writes, preventing updates, and degrading system performance.

| Field | Detail |
|---|---|
| Observable symptoms | "Startup disk almost full" alerts, failed downloads, app crashes on save |
| Primary signals | `disk_free_pct` per volume, `disk_free_bytes`, `largest_directory_sizes` (on-demand), `recent_update_pending` |
| Trigger condition | Any mounted volume: `disk_free_pct < 10%` |
| Rules sufficient? | **Yes — completely deterministic** |
| Suggested model | None. This is a measurement problem, not a pattern problem. |
| Safety level | Mostly auto-safe or recommendation-only |

**Deterministic decision logic:**
```
if disk_free_pct < 5%:
    → "Critical: disk nearly full — writes may fail"
    → confidence: Confirmed
elif disk_free_pct < 10%:
    → "Low disk space — performance and reliability at risk"
    → confidence: Confirmed
if recent_update_pending AND disk_free_pct < 15%:
    → "Pending OS update may fail due to disk space"
    → confidence: Confirmed
```

**Safe remediations:**
| Action | Safety Level | Privilege |
|---|---|---|
| Report disk usage by top directories | Auto-safe | User |
| Empty user Trash/Recycle Bin | Approval required | User |
| Clear system caches (macOS: ~/Library/Caches) | Approval required | User |
| Clear temp files (%TEMP%, /tmp) | Auto-safe | User |
| Identify and report large files >500 MB | Auto-safe | User |
| Remove old iOS/device backups | Approval required | User |
| Suggest moving files to external storage | Auto-safe (recommendation) | User |
| Uninstall identified large unused apps | Never auto-fix | User |

**Post-fix verification:** `disk_free_pct > 15%` after action.

---

## Issue 4 — Abnormal Disk I/O / Disk Slowness

**Description:** Disk read/write operations are unusually slow or queue depth is high, causing application hangs and sluggish file operations.

| Field | Detail |
|---|---|
| Observable symptoms | Spinning beach ball, long file open times, app hangs on save |
| Primary signals | `disk_queue_depth`, `disk_read_latency_ms`, `disk_write_latency_ms`, `disk_io_util_pct`, `top_disk_process_name`, `disk_type` (SSD/HDD), `smart_status` |
| Trigger condition | `disk_queue_depth > 4` sustained for 5+ samples OR `disk_read_latency_ms > 100` |
| Rules sufficient? | **Yes for detection; anomaly detection useful for distinguishing normal burst from pathological** |
| Suggested model | Statistical: rolling z-score on disk queue depth (no ML needed); or Isolation Forest if behavior is complex |
| Safety level | Mostly recommendation-only; killing offending process is approval-required |

**Deterministic decision logic:**
```
if smart_status == Failing:
    → "SMART reports disk failure — immediate backup recommended"
    → confidence: Confirmed (never auto-fix; critical escalation)
elif disk_type == HDD AND disk_queue_depth > 8:
    → "Disk I/O queue saturated — HDD under heavy load"
    → confidence: Confirmed
elif disk_type == SSD AND disk_read_latency_ms > 200:
    → "SSD read latency abnormal — possible controller issue or bus saturation"
    → confidence: Probable
elif top_disk_process == "mds_stores" OR top_disk_process == "Spotlight":
    → "Spotlight re-indexing causing I/O load"
    → confidence: Confirmed
elif top_disk_process == "TimeMachineBackup" OR top_disk_process == "backupd":
    → "Time Machine backup in progress"
    → confidence: Confirmed
```

**Safe remediations:**
| Action | Safety Level | Privilege |
|---|---|---|
| Report top I/O processes | Auto-safe | User |
| Pause Spotlight indexing temporarily | Approval required | User |
| Pause Time Machine backup | Approval required | User |
| Escalate SMART failure to critical alert | Auto-safe | User |
| Kill offending user process (consent) | Approval required | User |

**Post-fix verification:** `disk_queue_depth < 2` and `disk_read_latency_ms < 50` within 2 minutes.

---

## Issue 5 — Startup / Login Slowdown

**Description:** Boot-to-desktop or login-to-usable time has increased significantly, degrading user experience.

| Field | Detail |
|---|---|
| Observable symptoms | Long spinning wheel at boot, apps take long to open after login |
| Primary signals | `startup_items_count`, `startup_items_list`, `login_time_seconds` (if measurable), `launch_agents_count`, `launch_daemons_count`, `recent_app_installs` |
| Trigger condition | `startup_items_count > 15` OR `login_time_seconds > 60` (if measurable) OR scheduled weekly check |
| Rules sufficient? | **Mostly yes; GBT model can rank startup items by historical impact** |
| Suggested model | GBT (XGBoost/LightGBM) scoring startup item "dangerousness" based on known bad actors and historical data. ~300 KB ONNX. Advisory only. |
| Safety level | Approval required for disabling startup items |

**Deterministic decision logic:**
```
if startup_items_count > 20:
    → "Excessive startup items — likely causing significant boot delay"
    → confidence: Confirmed
elif startup_items_count > 10:
    → "Above-average startup items count"
    → confidence: Probable
if any(item.name in known_slow_agents for item in startup_items):
    → "Known resource-heavy agent detected in startup"
    → confidence: Confirmed
if recent_app_installs AND startup_items_count increased since last_check:
    → "Recent install added startup items"
    → confidence: Probable
```

**Safe remediations:**
| Action | Safety Level | Privilege |
|---|---|---|
| Report startup items with vendor/purpose | Auto-safe | User |
| Disable identified non-essential startup item | Approval required | User |
| Remove orphaned launch agents | Approval required | User |
| Suggest uninstalling unused apps | Auto-safe (recommendation) | User |

**Post-fix verification:** Scheduled — check `startup_items_count` at next boot, or re-scan immediately.

---

## Issue 6 — App Crash / Repeated Freeze

**Description:** One or more applications crash repeatedly or freeze, requiring force-quit, indicating a software or resource defect.

| Field | Detail |
|---|---|
| Observable symptoms | App disappears without warning, spinning beach ball requiring force-quit, crash reporter popup |
| Primary signals | `app_crash_count_24h`, `app_crash_count_7d`, `crash_process_name`, `crash_exception_type` (macOS), `crash_fault_address`, `event_log_crash_count` (Windows), `crash_log_text` |
| Trigger condition | `app_crash_count_24h > 2` for the same process |
| Rules sufficient? | **Partially — crash log classification needs a model for root cause attribution** |
| Suggested model | CharCNN or TinyBERT on crash log text → classify exception type (memory corruption, assertion failure, resource exhaustion, external process killed). ~3 MB ONNX INT8. |
| Safety level | Mostly recommendation-only; re-permissions, cache clears are approval-required |

**Deterministic decision logic:**
```
if crash_exception_type in [EXC_BAD_ACCESS, SIGSEGV, SIGBUS]:
    → "Memory access violation — likely bug, corrupted library, or bad update"
    → confidence: Confirmed
elif crash_exception_type == EXC_RESOURCE:
    → "Process killed by OS — resource limit exceeded (CPU/memory)"
    → confidence: Confirmed
elif crash_count_24h > 5 AND crash_count_7d > 10:
    → "Repeated systematic crash — persistent defect"
    → confidence: Confirmed
elif recent_update_detected AND crash_started_after_update:
    → "App crash likely related to recent update"
    → confidence: Probable
```

**If ML model available:** Feed crash log text → classify root cause → add to diagnosis evidence.

**Safe remediations:**
| Action | Safety Level | Privilege |
|---|---|---|
| Report crash frequency and exception type | Auto-safe | User |
| Clear app cache / preferences (user-space) | Approval required | User |
| Reset app permissions | Approval required | User |
| Reinstall app (recommendation only) | Never auto-fix | User |
| Roll back app update (if possible) | Approval required | User |
| File crash report with developer | Auto-safe (recommendation) | User |

**Post-fix verification:** `app_crash_count_24h == 0` for the same process over next 24h window.

---

## Issue 7 — Network / DNS / Connectivity Issues

**Description:** Network connectivity is degraded, intermittent, or completely absent, affecting cloud services, updates, and user productivity.

| Field | Detail |
|---|---|
| Observable symptoms | Apps report "no internet", DNS lookups fail, VPN disconnects, captive portal stuck |
| Primary signals | `network_reachable`, `dns_resolution_fail_rate`, `default_gateway_ping_ms`, `dns_server_ip`, `wifi_ssid`, `wifi_signal_dbm`, `proxy_configured`, `vpn_active`, `packet_loss_pct`, `ipv4_addr`, `ipv6_addr` |
| Trigger condition | `dns_resolution_fail_rate > 20%` OR `default_gateway_ping_ms > 500` OR `network_reachable == false` |
| Rules sufficient? | **Yes — connectivity is binary/measurable; no ML needed** |
| Suggested model | None. Network diagnosis is a decision tree problem. |
| Safety level | Auto-safe: diagnostic commands; Approval required: DNS change, network config reset |

**Deterministic decision logic (layered network stack test):**
```
Step 1: Check physical link
  if wifi_signal_dbm < -80: → "Weak WiFi signal"
  if no_ip_address: → "DHCP failure — no IP assigned"

Step 2: Check gateway
  if gateway_ping_ms > 1000 OR gateway_unreachable:
    → "Default gateway unreachable — local network issue"

Step 3: Check DNS
  if dns_resolution_fail_rate > 50%:
    → "DNS resolution failure"
    → sub-diagnose: test 8.8.8.8 vs configured DNS server
      if 8.8.8.8 works: → "ISP/configured DNS server failing"
      if both fail: → "Network-level block or routing issue"

Step 4: Check proxy / VPN
  if proxy_configured AND connectivity_fails:
    → "Proxy may be blocking traffic"
  if vpn_active AND dns_fails:
    → "VPN DNS split-tunneling issue"
```

**Safe remediations:**
| Action | Safety Level | Privilege |
|---|---|---|
| Run layered connectivity diagnostic | Auto-safe | User |
| Flush DNS cache (macOS: dscacheutil / Windows: ipconfig /flushdns) | Auto-safe | User |
| Switch to fallback DNS (8.8.8.8) temporarily | Approval required | Admin |
| Renew DHCP lease | Approval required | Admin |
| Toggle WiFi off/on | Approval required | User |
| Disable proxy temporarily | Approval required | User |
| Reset network stack (macOS: networksetup / Windows: netsh reset) | Never auto-fix | Admin |

**Post-fix verification:** DNS resolution succeeds for 3 test domains; `default_gateway_ping_ms < 100`.

---

## Issue 8 — Service / Agent Not Running or Restart Loop

**Description:** A background service or system agent is not running when it should be, or is crashing and restarting repeatedly (restart storm).

| Field | Detail |
|---|---|
| Observable symptoms | Functionality suddenly absent (backup not running, AV disabled), high CPU from respawn attempts |
| Primary signals | `service_status` per service list, `service_restart_count_1h`, `service_exit_code`, `service_uptime_seconds` |
| Trigger condition | `service_status == Stopped` for expected-running service OR `service_restart_count_1h > 5` |
| Rules sufficient? | **Yes — service state is binary; restart count is measurable** |
| Suggested model | None needed |
| Safety level | Auto-safe: restart known-safe services; Approval required: modify service config |

**Deterministic decision logic:**
```
if service_restart_count_1h > 10:
    → "Restart storm — service is crash-looping"
    → stop restart attempts; escalate
elif service_status == Stopped AND expected_running:
    → "Service unexpectedly stopped"
    → sub-diagnose: check exit code
      if exit_code == 0: → "Clean exit — may have been intentionally stopped"
      if exit_code != 0: → "Service crashed — check logs"
elif service_uptime_seconds < 30 AND restart_count > 3:
    → "Service fails within 30 seconds of start — likely config or dependency issue"
```

**Safe remediations:**
| Action | Safety Level | Privilege |
|---|---|---|
| Report service status and last exit code | Auto-safe | User |
| Restart service (known-safe list only) | Approval required | Admin |
| Stop restart storm (prevent rapid respawn) | Approval required | Admin |
| Check service log for error | Auto-safe | User |
| Re-enable service if disabled | Approval required | Admin |

**Post-fix verification:** `service_status == Running` AND `service_uptime_seconds > 60` after action.

---

## Issue 9 — Update-Related Issues

**Description:** OS or application updates are pending, failed, or causing post-update instability.

| Field | Detail |
|---|---|
| Observable symptoms | "Update failed" notifications, post-update crashes, pending reboot banner, degraded performance after update |
| Primary signals | `pending_updates_count`, `pending_reboot_required`, `last_update_timestamp`, `update_failed_flag`, `reboot_pending_since_hours` |
| Trigger condition | `reboot_pending_since_hours > 72` OR `update_failed_flag == true` OR `pending_updates_count > 0` (scheduled check) |
| Rules sufficient? | **Yes — all conditions are directly observable state flags** |
| Suggested model | None |
| Safety level | Never auto-fix (reboots and updates require user consent) |

**Deterministic decision logic:**
```
if reboot_pending_since_hours > 168:
    → "Critical: security update reboot overdue by >7 days"
    → confidence: Confirmed
elif update_failed_flag AND last_update_error_code:
    → "Update failed with error — likely disk space or network issue"
    → map error_code → known_causes table
elif pending_updates_count > 5:
    → "Many updates pending — system may be at security risk"
elif recent_update_timestamp < 2h AND new_crash_patterns:
    → "Post-update regression suspected"
```

**Safe remediations:**
| Action | Safety Level | Privilege |
|---|---|---|
| Report pending updates and ages | Auto-safe | User |
| Diagnose update failure root cause | Auto-safe | User |
| Prompt user to schedule reboot | Auto-safe (notification) | User |
| Trigger update retry (not reboot) | Approval required | Admin |
| Rollback last update | Never auto-fix | Admin |

**Post-fix verification:** `pending_reboot_required == false` and `update_failed_flag == false`.

---

## Issue 10 — Battery / Thermal Abnormality

**Description:** Battery is degrading faster than normal, charging is abnormal, or system temperatures are dangerously high.

| Field | Detail |
|---|---|
| Observable symptoms | Battery drains unusually fast, "Service Battery" warning, fan constantly at max, system hot to touch, thermal throttling |
| Primary signals | `battery_health_pct`, `battery_cycle_count`, `battery_charge_pct`, `battery_current_ma` (charging rate), `battery_temp_celsius`, `cpu_temp_celsius`, `gpu_temp_celsius`, `fan_speed_rpm`, `thermal_state`, `battery_time_remaining_min` |
| Trigger condition | `thermal_state == Critical` OR `battery_health_pct < 80%` OR `cpu_temp_celsius > 95` |
| Rules sufficient? | **Yes for detection; trend analysis useful for battery degradation prediction** |
| Suggested model | Time-series linear regression on `battery_health_pct` over 90-day history to predict replacement timing. Simple, no transformer needed. |
| Safety level | Mostly recommendation-only (hardware-adjacent) |

**Deterministic decision logic:**
```
if cpu_temp_celsius > 100 OR thermal_state == Critical:
    → "Critical thermal condition — risk of thermal shutdown"
    → confidence: Confirmed (immediate alert)
elif thermal_state == Throttling:
    → "Thermal throttling active — performance limited to manage heat"
    → confidence: Confirmed
if battery_health_pct < 70%:
    → "Battery health critically degraded — replacement recommended"
    → confidence: Confirmed
elif battery_health_pct < 80%:
    → "Battery health below optimal — monitor for replacement"
    → confidence: Confirmed
if battery_cycle_count > max_designed_cycles:
    → "Battery has exceeded designed cycle count"
```

**Safe remediations:**
| Action | Safety Level | Privilege |
|---|---|---|
| Report battery health and cycle count | Auto-safe | User |
| Identify CPU-heavy process causing thermal load | Auto-safe | User |
| Suggest removing from confined space / cleaning vents | Auto-safe (recommendation) | User |
| Enable Low Power Mode | Approval required | User |
| Kill process causing thermal load (consent) | Approval required | User |
| Schedule battery replacement reminder | Auto-safe | User |
| Hardware replacement | Never auto-fix | — |

**Post-fix verification:** `cpu_temp_celsius < 85` within 5 minutes; battery health is not auto-verifiable (hardware).

---

## Summary Matrix

| # | Issue | Rules Only? | Model Needed | Model Type | Auto-Fix Safe | Approval Required | Never Auto-Fix |
|---|---|---|---|---|---|---|---|
| 1 | High CPU | Detection yes; attribution no | Isolation Forest (process anomaly) | process kill (user-space) | priority reduction | kernel process kill |
| 2 | High Memory | Yes | None for MVP | flush cache (admin) | process kill (consent) | — |
| 3 | Low Disk | Yes | None | clear temp | clear caches | uninstall apps |
| 4 | Disk I/O | Yes | z-score (statistical) | pause Spotlight/TM | kill process | SMART failure escalation |
| 5 | Startup Slow | Mostly yes | GBT startup scorer | — | disable startup item | — |
| 6 | App Crash | Partial | CharCNN crash log | clear cache | reset permissions | reinstall |
| 7 | Network/DNS | Yes | None | flush DNS | DNS change, DHCP renew | network stack reset |
| 8 | Service Down | Yes | None | — | restart service | service config change |
| 9 | Update Issue | Yes | None | — | update retry | reboot, rollback |
| 10 | Battery/Thermal | Yes | Linear regression (trend) | low power mode | kill thermal process | hardware action |
