# Safety Model

This is the most critical design document. Any remediation action that the agent takes on a live endpoint can cause damage. The safety model defines the rules under which the agent may act, how it gates actions, how it handles failures, and what can never be automated.

---

## Safety Levels

Every remediation action is statically classified into one of four safety levels. This classification is hardcoded at compile time — it is never computed dynamically and never overridable by configuration.

| Level | Code | Description |
|---|---|---|
| `AutoSafe` | 0 | Agent may execute without asking. Risk of damage is negligible or zero. |
| `ApprovalRequired` | 1 | Agent must present the action to the user and receive explicit consent before executing. |
| `ElevationRequired` | 2 | Action requires elevated privilege (admin/sudo). Always requires approval. No elevation prompt without user seeing the plan first. |
| `NeverAutoFix` | 3 | Agent will never execute this action. It is recommendation-only. This classification is permanent. |

**Important:** `AutoSafe` actions still produce audit events. They are not invisible.

---

## Action Categories and Default Safety Level

| Category | Examples | Default Level |
|---|---|---|
| Reporting / diagnostics | Print CPU stats, list startup items, check disk | `AutoSafe` |
| Cache and temp clearing | Clear ~/Library/Caches (user-space), empty /tmp | `ApprovalRequired` |
| Process termination | SIGTERM/SIGKILL a user-space process | `ApprovalRequired` |
| Process priority change | Renice a process | `ApprovalRequired` |
| Service restart | Restart a monitored user-space service | `ApprovalRequired` |
| Network probe | Ping, DNS resolution test, traceroute | `AutoSafe` |
| DNS cache flush | dscacheutil -flushcache, ipconfig /flushdns | `AutoSafe` |
| DNS server change | Switch to fallback DNS server | `ElevationRequired` |
| DHCP renewal | ipconfig renew / networksetup -renewdhcp | `ElevationRequired` |
| Startup item disable | launchctl disable / remove Registry Run key | `ApprovalRequired` |
| Startup item remove | Delete a plist / registry key | `ElevationRequired` |
| App cache clear | Clear app-specific ~/Library/Application Support/AppName/Cache | `ApprovalRequired` |
| Update trigger | macOS: softwareupdate --download-only | `ApprovalRequired` |
| Reboot | macOS: sudo reboot / Windows: shutdown /r | `NeverAutoFix` |
| Update install | Any OS update installation | `NeverAutoFix` |
| App uninstall | Remove any application | `NeverAutoFix` |
| File deletion outside known-safe paths | Delete outside ~/Library/Caches, /tmp, Trash | `NeverAutoFix` |
| Kernel extension / driver | Any kext or driver modification | `NeverAutoFix` |
| System Integrity Protection | SIP disable/enable | `NeverAutoFix` |
| Network stack reset | netsh reset / networksetup -setdhcp | `NeverAutoFix` |
| OS reinstall / restore | Recovery mode, reinstall | `NeverAutoFix` |

---

## Pre-Check Requirements

Every action, including `AutoSafe` ones, must pass its declared pre-checks before execution. If any pre-check fails, the action is skipped and the failure is audit-logged.

**Standard pre-checks (run for every action):**
1. Agent is still running and not shutting down
2. Target resource (PID, path, service name) still exists
3. Target resource state matches what the diagnosis found (e.g., PID still at high CPU, not already killed)
4. System disk is not completely full (preventing audit log write)

**Action-specific pre-checks (defined per recipe):**
- `KillProcess`: PID exists, name matches, not a protected process, process is not in D-state
- `ClearCache`: Path is in approved-paths list, no open file handles from other processes
- `RestartService`: Service exists, expected_running=true in config, not already restarting
- `FlushDNS`: Interface is up
- `DisableStartupItem`: Item is not on protected list, current user owns the item (or has elevation)

---

## Protected Paths and Protected Processes

### Protected Paths (never delete or modify files in these paths)

```toml
protected_paths = [
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
  "C:\\Program Files\\Windows Defender",
  # User home is protected except specific known-safe subdirs:
  "~/Documents",
  "~/Desktop",
  "~/Downloads",
  "~/Pictures",
  "~/Movies",
  "~/Music",
]

# Approved paths for cache/temp clearing:
approved_clear_paths = [
  "~/Library/Caches",
  "/private/tmp",
  "/tmp",
  "%TEMP%",
  "%LOCALAPPDATA%\\Temp",
]
```

### Protected Processes (never kill or modify)

```toml
protected_process_names = [
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
]
```

Any action targeting a process in this list is **automatically denied** regardless of safety level, before policy evaluation.

---

## User Consent Model

For `ApprovalRequired` actions:

1. Agent constructs a `RemediationPlan` with clear human-readable descriptions
2. Plan is presented to user via CLI prompt or tray notification
3. User sees: **what** will happen, **why** it is recommended, **what the risk is**, and **how to undo it**
4. User responds: `yes` / `no` / `details` / `later`
5. If `yes`: execute with 30s timeout; if timeout lapses, treat as `no`
6. If `no`: log `RemediationRejected` with user context; do not retry automatically for 24h
7. All consent decisions are audit-logged with timestamp and method

**Phase 1 (CLI):** Interactive prompt. `y/N` with a `--dry-run` flag to show what would happen without executing.

**Phase 2 (Tray):** Native OS notification with action buttons. Never auto-dismiss consent prompts.

---

## Admin / Elevation Consent Model

For `ElevationRequired` actions:

1. User must explicitly approve the action via consent model (same as above)
2. After consent: agent requests privilege elevation using OS mechanism
   - macOS: `AuthorizationExecuteWithPrivileges` or direct `sudo` spawn (authenticated via OS dialog)
   - Windows: UAC elevation prompt via `ShellExecute` with `runas` verb
3. Elevated process is a minimal helper binary (`firstaid-helper`) — not the full agent
4. Helper binary is code-signed (macOS: Developer ID; Windows: Authenticode)
5. Helper accepts only a typed command over a local socket — never a shell string
6. Helper exits immediately after completing its single action

**No persistent elevated process.** The helper spawns, acts, exits.

---

## Rollback Strategy

The agent does not implement automatic rollback. The engineering cost is high and the risk of a bad rollback is comparable to the original action. Instead:

1. Every action that is not fully reversible declares a `rollback_note` — a human-readable instruction for manual recovery
2. The plan presented to the user always includes the rollback note before consent is given
3. For reversible actions (e.g., re-enabling a startup item), the agent can perform the reversal on request via CLI: `firstaid undo <action_id>`
4. Undo is only available for actions marked `reversible: true` in the plan
5. Undo uses the same safety gates as the original action

**Reversible examples:** disable startup item (re-enable it), change DNS (revert to original)
**Irreversible examples:** kill process (process is gone), clear cache (files deleted), empty Trash

---

## Recurrence Handling

The agent tracks issue recurrence in the `known_issues` table:

- If the same issue fires more than 3 times in 7 days, and the same remediation was applied: escalate to `ManualReview`
- The agent will not repeatedly apply the same failed remediation — this is a `RecurrenceLimit` policy
- After `ManualReview` escalation, the agent defers all auto-fix attempts for that issue class for 72h
- The user is notified of the escalation with context ("This issue has occurred 4 times this week")

---

## Escalation Path

| Condition | Agent Action |
|---|---|
| Issue severity == Critical AND no auto-safe remediation | Immediate user notification |
| Remediation failed (pre-check or execution) | Notify user, log failure, recommend manual action |
| Issue recurred >3 times post-remediation | Escalate: "This fix isn't holding. Manual investigation required." |
| SMART disk failure detected | Immediate critical alert, recommend backup NOW, NeverAutoFix |
| Battery temp >100°C or thermal state == Critical | Immediate notification, suggest immediate shutdown |
| ML model checksum mismatch | Disable ML inference, switch to rules-only, log security alert |
| Helper binary signature verification fails | Block all elevation actions, log critical security event |

---

## Audit Trail Requirements

The audit trail is:
- **Append-only:** No UPDATE or DELETE on `audit_log` table. Ever.
- **Event-chained:** Each event references the previous event's ID (`prev_event_id`)
- **Complete:** Every action attempt — successful or not, consented or rejected — produces an event
- **Tamper-detectable:** Gaps in sequence_num or broken prev_event_id chains are surfaced as warnings

In Phase 4, add:
- SHA-256 hash chain (each event hashes its content + previous event hash)
- Export capability for fleet management integration
- Retention policy with cryptographically signed archive

---

## Things That Can NEVER Be Automated (Hardcoded, Not Configurable)

These are enforced at compile time. No config flag, no admin override, no policy file can enable these:

1. OS reboot or shutdown
2. OS update installation
3. Application uninstall
4. Any file deletion outside `approved_clear_paths`
5. Kernel extension modification
6. SIP (System Integrity Protection) modification
7. User account modification (create, delete, password change)
8. Firewall rule modification
9. Network stack reset
10. Any action on a protected process name
11. Any elevated action when helper binary signature is invalid
