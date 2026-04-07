# MVP Roadmap

---

## Phase 1 — Deterministic MVP (Weeks 1–6)

### Scope

Build a working, shippable tool for the top 5 most impactful issue classes using **rules only** — no ML inference, no history, no tray app. The agent runs on-demand (CLI), not as a persistent background service.

**Issue classes in scope:**
1. High CPU usage
2. High memory / memory pressure
3. Low disk space
4. Network / DNS issues
5. Battery / thermal (read-only diagnosis; no remediation)

**In scope:**
- Cross-platform `collectors` crate with `sysinfo` baseline
- macOS-specific collector for memory pressure (IOKit) and thermal (IOKit)
- Windows-specific collector for memory commit charge (WMI)
- `signal-normalizer`, `feature-extractor`
- `trigger-engine` with threshold triggers (no event-driven yet)
- `detector` with rules engine for 5 issue classes
- `remediation` planner and executor for safe actions only:
  - DNS cache flush
  - Report of top processes
  - Connectivity layered test
  - Temp file size scan (dry-run only — no actual deletion yet)
- `policy` layer with safety gates (all approval gates: just log them in Phase 1, no UI consent yet)
- `storage` with SQLite — persist snapshots and diagnoses
- `audit` logging to file + SQLite
- `ui-cli` with `diagnose`, `status`, `report` commands
- Single binary, <15 MB, <30 MB RAM at idle

**Out of scope in Phase 1:**
- Background service / daemon
- Event-driven collection (crash log watchers)
- ML inference
- Actual remediation execution (any action that modifies system state)
- Tray app / UI
- Windows-specific deep collectors (ETW, event log)
- App crash, service down, startup slow, update issues diagnosis
- macOS Helper binary (elevation)

### Success Criteria

- `firstaid diagnose` produces a correct `DiagnosisOutput` on both macOS and Windows within 3 seconds
- All 5 issue classes produce correct diagnosis for hand-crafted fixture snapshots
- Zero false positives on a healthy machine (idle CPU <20%, memory >50% free, disk >30% free)
- Binary size <15 MB; idle RAM <30 MB
- All audit events written to SQLite
- CI passes on `macos-latest` AND `windows-latest` in GitHub Actions

### Risks

- IOKit bindings for memory pressure on macOS are complex — may need unsafe code or a subprocess call to `memory_pressure` tool as fallback
- WMI queries on Windows can be slow (>1s) — must be async and time-bounded
- `sysinfo` crate coverage differences between platforms may surface unexpected gaps

### What NOT to Build Yet

- Don't design the tray app UI (waste of time until the core is proven)
- Don't add more issue classes until the 5 work perfectly
- Don't implement the helper binary elevation yet
- Don't write model training pipelines yet

---

## Phase 2 — Background Agent + Historical Intelligence (Weeks 7–12)

### Scope

Convert the CLI tool into a persistent background agent. Add event-driven collection. Add the remaining 5 issue classes. Enable actual remediation execution with user consent.

**New in Phase 2:**
- **Background service:** launchd daemon (macOS), Windows Service (Windows)
- **Event-driven collection:** crash log file watcher (macOS: `/Library/Logs/DiagnosticReports`; Windows: Event Log subscription)
- **Remaining issue classes:** Startup slow, App crash, Disk I/O, Service down, Update issues
- **Remediation execution** for approved actions:
  - DNS cache flush (execute)
  - Kill process with consent (execute)
  - Clear temp files with consent (execute)
  - Disable startup item with consent (execute)
  - Restart service with consent (execute)
- **History-aware diagnosis:** Recurrence detection, trend analysis for battery, startup count change tracking
- **macOS Helper binary** for `ElevationRequired` actions
- **Policy config file** (`~/.config/firstaid/policy.toml`) for managed environments
- `firstaid history`, `firstaid plan`, `firstaid fix` CLI commands
- macOS-specific: Spotlight/mds I/O diagnosis, launch agent audit
- Windows-specific: Event Log crash parsing, Registry startup key audit

**Trigger engine upgrade:**
- File system watcher for crash log directories
- Windows Event Log subscription for EventID 1000/1001
- Hysteresis and storm prevention

**Storage upgrade:**
- 7-day rolling snapshot window
- `known_issues` table for recurrence
- Per-process crash history

### Success Criteria

- Agent starts in <2 seconds and uses <25 MB RAM at idle
- Crash log event triggers diagnosis within 10 seconds of crash
- Remediation consent prompt appears for every `ApprovalRequired` action
- No action executes without audit log entry
- All 10 issue classes produce correct diagnosis on synthetic fixtures
- Recurrence is detected and escalated correctly
- Background agent survives 24h on CI without memory growth or crash

### Risks

- Background service reliability on Windows: service registration, restart policy, handling service stop during active diagnosis
- Crash log file watching has platform-specific gotchas (file locks on Windows, permission model on macOS)
- User consent via CLI is awkward for background agent — this is the motivation for Phase 2's tray app, but tray app is risky scope creep
- macOS Helper binary code signing requires Apple Developer ID — budget for certificate and notarization

### What NOT to Build Yet

- ML inference (still Phase 3)
- Tray app (can be done in parallel but is not gating)
- Fleet management / remote policy
- Encrypted audit log

---

## Phase 3 — Specialized Model Augmentation (Weeks 13–20)

### Scope

Add the 3 targeted ML models where they materially improve diagnosis quality. Validate against real-world crash logs and process traces.

**New in Phase 3:**
- `inference` crate with ONNX Runtime (ort crate, load-dynamic)
- `crash-classifier` model: CharCNN, 300 KB INT8 ONNX
  - Training data: macOS crash log dataset (Apple OSS + manually curated)
  - Taxonomy: 6 classes (MemoryCorruption, ResourceExhaustion, NullDereference, ExternalKill, AssertionFailure, Unknown)
  - Integration: fires when rules return `Probable` on App Crash; advisory output
- `process-anomaly` model: Isolation Forest, 500 KB ONNX
  - Training data: 30-day process trace on developer machines (opt-in)
  - Integration: fires when CPU > 75% and rule says "Possible"
  - Advisory: surfaces anomaly score and contributing features
- `startup-scorer` model: XGBoost 200 trees, 200 KB ONNX
  - Training data: known slow/heavy startup items curated list
  - Integration: ranks startup items by "remove priority"
- Lazy-load model lifecycle (load on trigger, drop after inference)
- Checksum verification on model files before every load
- Rules-only fallback when ORT is unavailable

**Tray App (Tauri) — can run in parallel:**
- System tray icon with health status indicator
- Native notification for new diagnosis
- Settings page (consent preferences, monitored services, policy)
- Diagnosis history view
- Talks to agent-core via Unix domain socket / Named Pipe JSON API

### Success Criteria

- ML models improve precision on ambiguous cases:
  - Crash classification: >85% accuracy on held-out crash log test set
  - Process anomaly: <5% false positive rate on healthy machine baselines
  - Startup scorer: top 3 items should include the actual slow item in >80% of synthetic tests
- Model load + inference < 50ms per trigger
- Idle ML memory = 0 (no resident model memory)
- Graceful degradation: all models can be disabled independently without breaking agent startup

### Risks

- Crash log dataset quality is the biggest risk — OS-provided crash logs are often noisy, truncated, or stripped of useful context. Validate early.
- Isolation Forest needs per-machine baseline training data — cold start problem on first install. Mitigation: ship a pre-trained baseline + fine-tune over first 7 days of local data.
- ONNX Runtime dynamic loading on Windows requires the ORT DLL to be included in the installer. Track DLL size in footprint budget.

### What NOT to Build Yet

- General-purpose LLM for any part of this system
- Cloud model updates / telemetry
- Multi-device fleet coordination

---

## Phase 4 — Enterprise Hardening (Weeks 21–28)

### Scope

Make the agent production-ready for enterprise deployment: policy management, MDM integration, fleet visibility, and security hardening.

**New in Phase 4:**
- **Policy file support:** TOML policy file deployable via MDM (Jamf, Intune)
  - Override safety levels (can downgrade `ApprovalRequired` to `AutoSafe` for IT-managed actions; cannot upgrade `NeverAutoFix`)
  - Whitelist/blacklist specific startup items, services, processes
  - Configure monitored services list
  - Set custom thresholds per org
- **Fleet reporting:** Structured JSON diagnostic exports for SIEM / ITSM integration
- **Encrypted audit log:** SHA-256 hash chain; optional AES-256 encryption with IT-managed key
- **SQLCipher** for encrypted local DB
- **Code signing + notarization** (macOS: Developer ID, Notarize; Windows: Authenticode EV cert)
- **Installer hardening:**
  - macOS: signed `.pkg` with LaunchDaemon, notarized
  - Windows: WiX/NSIS signed installer, Windows Service with restricted ACL
- **Telemetry opt-in:** Anonymized aggregate issue frequency reporting (no PII, no process names)

### Success Criteria

- MDM-deployable policy file controls agent behavior fleet-wide
- Audit log is tamper-detectable and exportable
- Agent passes macOS Gatekeeper and Windows Defender SmartScreen without prompts
- IT team can query recent diagnoses across fleet via ITSM integration
- Installer leaves no leftover files on uninstall

### Risks

- MDM policy override must not allow IT to elevate `NeverAutoFix` — this is a hardcoded constraint. MDM policy can only act within the space the safety model allows.
- Hash chain audit log adds write overhead — benchmark first
- Enterprise procurement / signing certificate acquisition has long lead times — start early

### What NOT to Build

- Remote remediation execution by IT without local user notification (always local notification first)
- Cloud-based diagnostic AI that sends user data to a server
- General-purpose chat interface to the agent

---

## Per-Phase Summary

| Phase | Duration | Deliverable | Risk Level |
|---|---|---|---|
| 1 | 6 weeks | On-demand CLI diagnostic, 5 issue classes, rules only | Low |
| 2 | 6 weeks | Background daemon, all 10 classes, remediation execution | Medium |
| 3 | 8 weeks | ML augmentation, tray app | Medium-High |
| 4 | 8 weeks | Enterprise hardening, MDM, signing | Medium |

---

## What Can Be Built Quickly vs. What Is Deceptively Hard

### Quick (1–3 days each)
- sysinfo-based CPU/memory/disk collection
- Rules engine for low disk and memory pressure
- SQLite schema and audit log writes
- CLI command structure with clap
- JSON report export
- DNS cache flush (single command invocation)

### Moderate (1–2 weeks each)
- IOKit memory pressure and thermal on macOS (Rust FFI to C APIs)
- WMI queries on Windows (async, error handling)
- Trigger engine with hysteresis
- Remediation executor with pre-checks
- Process kill with consent and safety checks
- Background service registration on both platforms

### Deceptively Hard (2–4+ weeks each)
- macOS Helper binary (code signing, privilege handoff, IPC)
- Crash log file watcher with reliable cross-platform behavior
- Windows Event Log subscription (ETW/WinAPI, session management)
- Isolation Forest baseline training on per-machine data (cold start)
- Tauri tray app that works reliably on both platforms (notch, dark mode, multi-monitor)
- Enterprise MDM policy system that doesn't accidentally loosen safety guarantees
- App crash classification with crash log data acquisition pipeline

### Dangerous Distractions (Do Not Build)
- A general natural language interface
- Remote cloud sync of diagnostic data
- A "score" or "health percentage" for the overall machine (gamified metrics)
- Plugin system for third-party detectors
- Browser extension or web dashboard
