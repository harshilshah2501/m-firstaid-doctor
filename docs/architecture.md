# m-firstaid-doctor — System Architecture

## Executive Summary

This is a local-first, edge-deployed endpoint health agent for macOS and Windows. It diagnoses, explains, and safely remediates the top 10 ITSM-class endpoint issues without a cloud dependency, without a large LLM at runtime, and without persistent high CPU/RAM overhead.

**Core thesis:** 80% of endpoint issues are deterministically diagnosable from OS signals plus threshold rules. ML is a targeted enhancement for pattern recognition in unstructured signals (crash logs, process behavior anomalies), not the primary engine. The system is rules-first, ML-augmented, and safety-gated.

**What it is NOT:**
- Not a copilot / chat interface
- Not cloud-first with local fallback
- Not a general-purpose LLM wrapped in a tray icon
- Not a heavy monitoring agent running expensive queries in a loop

---

## Top Risks (Call Out First)

| Risk | Severity | Mitigation |
|---|---|---|
| Cross-platform parity gap | HIGH | OS adapter abstraction layer; CI matrix tests on both platforms |
| Remediation causes new damage | CRITICAL | Pre-check → safe-gate → rollback note → post-check; never auto-fix destructive actions |
| Idle resource creep | HIGH | Event-driven collection; no constant ML inference; strict footprint CI |
| False positive flood | HIGH | Conservative trigger thresholds; hysteresis; recurrence tracking |
| Model size / load time | MEDIUM | ONNX quantized models <8MB each; lazy-load on trigger, unload after inference |
| Privilege escalation abuse | CRITICAL | Explicit privilege model; all elevated actions require user consent |
| Startup/service reliability | MEDIUM | Watchdog process; graceful degradation if sub-component fails |
| Crash log parsing unreliability | MEDIUM | Classifier is advisory, not gating; rules always run first |

---

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────┐
│                        ENDPOINT HOST                                │
│                                                                     │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │                    agent-core (orchestrator)                 │   │
│  │                                                              │   │
│  │  ┌─────────────┐   ┌──────────────┐   ┌──────────────────┐  │   │
│  │  │  Collectors  │──▶│   Signal     │──▶│  Trigger Engine  │  │   │
│  │  │  (OS-native) │   │  Normalizer  │   │  (threshold +    │  │   │
│  │  │             │   │              │   │   event-driven)  │  │   │
│  │  └─────────────┘   └──────────────┘   └────────┬─────────┘  │   │
│  │        ▲                                        │ FIRED       │   │
│  │        │ poll / event                           ▼             │   │
│  │  ┌─────┴───────────────────────────────────────────────────┐ │   │
│  │  │                   Diagnosis Pipeline                    │ │   │
│  │  │  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  │ │   │
│  │  │  │   Feature    │  │ Deterministic│  │  ML Inference │  │ │   │
│  │  │  │  Extractor   │─▶│ Rules Engine │─▶│  (optional,  │  │ │   │
│  │  │  │              │  │              │  │  lazy-loaded) │  │ │   │
│  │  │  └──────────────┘  └──────────────┘  └──────────────┘  │ │   │
│  │  │                            │                   │         │ │   │
│  │  │                            └─────────┬─────────┘         │ │   │
│  │  │                                      ▼                   │ │   │
│  │  │                          ┌──────────────────┐            │ │   │
│  │  │                          │ DiagnosisOutput  │            │ │   │
│  │  │                          └─────────┬────────┘            │ │   │
│  │  └────────────────────────────────────┼────────────────────-┘ │   │
│  │                                       ▼                        │   │
│  │  ┌─────────────────────────────────────────────────────────┐   │   │
│  │  │                  Remediation Pipeline                    │   │   │
│  │  │  ┌───────────────┐  ┌────────────┐  ┌───────────────┐  │   │   │
│  │  │  │  Remediation  │  │  Policy /  │  │  Remediation  │  │   │   │
│  │  │  │   Planner     │─▶│  Safety    │─▶│   Executor    │  │   │   │
│  │  │  │               │  │   Gate     │  │               │  │   │   │
│  │  │  └───────────────┘  └────────────┘  └───────┬───────┘  │   │   │
│  │  │                                             │           │   │   │
│  │  │                            ┌────────────────▼────────┐ │   │   │
│  │  │                            │   Verification Engine   │ │   │   │
│  │  │                            └────────────────┬────────┘ │   │   │
│  │  └─────────────────────────────────────────────┼──────────┘   │   │
│  │                                                 │               │   │
│  │  ┌──────────────────────────────────────────────▼────────────┐ │   │
│  │  │              Storage + Audit Layer                         │ │   │
│  │  │   SQLite DB  │  Audit Log  │  Signal History  │  Config   │ │   │
│  │  └───────────────────────────────────────────────────────────┘ │   │
│  │                                                                  │   │
│  │  ┌─────────────────────────────────────────────────────────┐   │   │
│  │  │                     UI Surface                           │   │   │
│  │  │   CLI (ui-cli)  │  Tray App (Phase 2)  │  JSON API      │   │   │
│  │  └─────────────────────────────────────────────────────────┘   │   │
│  └──────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────┘
```

---

## Component Breakdown

### 1. Collectors (`crates/collectors`, `crates/collectors-mac`, `crates/collectors-win`)

**Responsibility:** Gather raw OS signals. These are purely observational — no analysis, no interpretation.

**Design rules:**
- Must be non-blocking (async)
- Fail gracefully — a collector error never stops the agent
- Cheap collectors run on a poll cadence; expensive collectors run on-demand
- Platform-specific code lives in platform crates; shared types in `collectors`

**Collection modes:**
| Mode | When | Examples |
|---|---|---|
| `Polling` (30s) | Always | CPU%, memory, disk free |
| `Polling` (5m) | Always | Process list, startup items |
| `Event-driven` | On OS event | Crash log written, event log entry |
| `On-demand` | Triggered by engine | Full disk I/O scan, detailed process tree |

**Cross-platform baseline via `sysinfo`:**
- CPU per-core + total usage
- Memory: total, available, used, swap
- Disk: partitions, free/used, read/write bytes
- Process list: pid, name, cpu%, mem, status

**macOS-specific (via IOKit, `system_profiler`, `log` subsystem):**
- Memory pressure level (IOKit: `kIOResourceMemoryPressure`)
- Thermal state (IOKit: `IOPMCopySystemPowerStatus`)
- Battery: cycle count, health percentage, amperage, temperature
- Unified log (OSLog): crash entries, assertion failures
- Launch agents/daemons (launchctl list)
- `mds_stores` / Spotlight load

**Windows-specific (via WMI, ETW, Event Log, Registry):**
- Memory: working set pressure, commit charge, page faults/sec
- Disk: average queue length (Perfmon), disk I/O latency (ETW)
- Startup programs: Registry Run keys, Task Scheduler
- Application Event Log: crash events (EventID 1000, 1001)
- Windows Update: pending reboot flag (registry)
- Battery: WMI Win32_Battery
- Thermal: WMI MSAcpi_ThermalZoneTemperature

---

### 2. Signal Normalizer (`crates/signal-normalizer`)

**Responsibility:** Map raw platform-specific signals to a typed, versioned `NormalizedSignalSnapshot`. This is the lingua franca of the system — everything downstream consumes this.

**Key invariants:**
- Normalization is pure and deterministic — same input always produces same output
- Fields absent on a platform are `Option<T>` with documented reasons
- Timestamps are always UTC
- Units are standardized: percentages as `f32` 0.0–100.0, sizes in bytes, temperatures in Celsius

---

### 3. Trigger Engine (`crates/trigger-engine`)

**Responsibility:** Decide whether a diagnostic pipeline run is warranted, based on normalized signals and configured thresholds.

**Design:**
- Stateful — tracks recent signal history to detect trends and prevent trigger storms
- Supports hysteresis: a trigger fires when threshold crossed, re-arms only after recovery
- Event-based triggers bypass polling (crash log write → immediate trigger)
- Multiple issue classes can trigger simultaneously; they are batched into one diagnostic run

**Trigger types:**
| Type | Example | Hysteresis |
|---|---|---|
| `ThresholdHigh` | cpu_avg_5m > 85% | Re-arm when < 70% for 2+ samples |
| `ThresholdLow` | disk_free_pct < 10% | Re-arm when > 15% |
| `EventArrival` | crash log written | 60s cooldown per-process |
| `RateSpike` | disk_queue_depth > 8 for 3+ samples | Rolling window |
| `Scheduled` | Daily startup scan | Calendar-based |

---

### 4. Feature Extractor (`crates/feature-extractor`)

**Responsibility:** Compute derived features from a `NormalizedSignalSnapshot`. These features are inputs to both the rules engine and any ML inference.

**Design:**
- Stateless computation over a snapshot + optional history window
- Features are typed and annotated with source fields
- Designed to be serializable for offline analysis and model training

---

### 5. Detector / Rules Engine (`crates/detector`)

**Responsibility:** Produce a `DiagnosisOutput` from features. Rules run first. ML inference runs only when rules are inconclusive and an applicable model is configured.

**Rule design principles:**
- Rules are encoded as Rust match/if trees — not a separate DSL, not runtime-evaluated strings
- Each rule names its evidence (which features drove the conclusion)
- Confidence levels: `Confirmed` (>95%), `Probable` (70–95%), `Possible` (40–70%), `Inconclusive`
- Multiple diagnoses can coexist with independent confidence scores

**ML integration:**
- Rules first: if confidence ≥ `Confirmed`, skip ML
- If `Probable` or lower and a model is registered for the issue class → lazy-load → infer → merge confidences
- Model verdict is advisory: it can promote Possible → Probable, not Inconclusive → Confirmed alone

---

### 6. Inference (`crates/inference`)

**Responsibility:** Lazy-load ONNX models, run inference, unload. Never keeps models resident at idle.

**Model lifecycle:**
```
Trigger → load model from disk (mmap) → pre-process input
→ session.run() → post-process output → drop session → done
```

**Model registry:**
- Models registered in config with path, version, input/output schema
- Checksummed on load — tampering detected before use
- Quantized INT8 or FP16 — never full FP32 for edge deployment

**Target models (Phase 3):**
| Model | Problem Class | Type | Est. Size | Latency |
|---|---|---|---|---|
| `crash-classifier` | App crash / freeze | CharCNN / TinyBERT ONNX | ~3 MB | <50ms |
| `process-anomaly` | High CPU, app freeze | Isolation Forest ONNX | ~0.5 MB | <10ms |
| `startup-scorer` | Startup slowdown | GBT ONNX (XGBoost export) | ~0.3 MB | <5ms |

---

### 7. Remediation (`crates/remediation`)

**Responsibility:** Translate a `DiagnosisOutput` into a `RemediationPlan`, execute actions safely, verify outcomes.

**Remediation recipe structure:**
- Each recipe is a Rust struct implementing `RemediationRecipe` trait
- Recipes declare: issue_class, safety_level, required_privilege, estimated_risk, pre_checks, steps, post_checks, rollback_note
- Steps are typed OS operations — no shell string exec unless explicitly documented
- Platform adapters: Mac recipes use Rust + IOKit/launchctl bindings; Windows recipes use Windows crate + WMI

**Executor:**
- Pre-check: verify pre-conditions are still true before acting
- Gate: safety policy approval (auto, user-consent, or blocked)
- Execute each step atomically where possible; stop on first failure
- Post-check: verify outcome matches expected state
- Emit `ExecutionResult` and `VerificationResult` regardless of success/failure

---

### 8. Policy / Safety Gate (`crates/policy`)

**Responsibility:** Enforce action safety levels before any remediation step executes.

See `docs/safety-model.md` for the full safety framework.

---

### 9. Storage (`crates/storage`)

**Responsibility:** Local SQLite database for signal history, diagnosis history, remediation audit trail, and configuration.

**Schema tables:**
- `signal_snapshots` — recent normalized snapshots (rolling 7-day window)
- `diagnoses` — diagnosis outputs with timestamp and confidence
- `remediation_plans` — planned actions before execution
- `execution_results` — per-action results
- `audit_log` — tamper-evident sequential event log
- `known_issues` — recurrence tracking per issue class

**Design decisions:**
- SQLite bundled statically — zero runtime dependency
- WAL mode — concurrent reads without blocking writes
- 7-day snapshot retention with configurable extension
- Audit log rows are append-only; no UPDATE/DELETE on audit table
- DB encrypted at rest using SQLCipher in enterprise tier (Phase 4)

---

### 10. Audit (`crates/audit`)

**Responsibility:** Write structured, tamper-evident audit events to both SQLite and a rotating log file.

**Every diagnostic run produces at minimum:**
- `DiagnosisStarted` event
- `DiagnosisCompleted` event (with outcome)
- `RemediationProposed` event (if any)
- `RemediationApproved` / `RemediationRejected` event
- `RemediationExecuted` event (per-action)
- `VerificationCompleted` event

---

### 11. UI (`crates/ui-cli`)

**Phase 1 (MVP):** CLI tool that can:
- `firstaid diagnose` — run a full diagnostic and print report
- `firstaid status` — show last known health state
- `firstaid history` — show recent issues and remediations
- `firstaid plan <issue>` — show remediation plan without executing
- `firstaid fix <issue>` — execute a remediation with consent prompt
- `firstaid report` — export JSON diagnostic bundle

**Phase 2:** System tray app (Tauri) with native notifications. Tray app talks to agent-core via a local socket (Unix domain socket / Named Pipe).

---

## Layer Separation: What Lives Where

```
┌──────────────────────────────────────────────────────────┐
│                  CROSS-PLATFORM CORE                     │
│  signal-normalizer, feature-extractor, trigger-engine,   │
│  detector (rules + ML), remediation (planner, executor), │
│  policy, storage, audit, inference, agent-core, ui-cli   │
├──────────────────────────────────────────────────────────┤
│               OS-SPECIFIC ADAPTERS                       │
│   collectors-mac  │           collectors-win             │
│   (IOKit, OSLog,  │  (WMI, ETW, EventLog, Registry,     │
│   launchctl,      │   Perfmon, Win32 APIs, PowerShell    │
│   pmset, syslog)  │   where no safe Rust binding exists) │
├──────────────────────────────────────────────────────────┤
│             SHARED SIGNAL ABSTRACTION                    │
│  collectors (RawSnapshot, CollectorTrait, common types)  │
└──────────────────────────────────────────────────────────┘
```

The rule: if it touches an OS-specific API, it lives in a platform crate behind a trait. The trait lives in `collectors`. Everything above that layer is cross-platform.

---

## Data Flow (End-to-End)

```
[OS Events / 30s Poll]
        │
        ▼
[Collectors] ──────────────────────▶ RawSnapshot
        │
        ▼
[Signal Normalizer] ───────────────▶ NormalizedSignalSnapshot
        │
        ▼
[Trigger Engine] ──── no trigger ──▶ (idle, save snapshot to DB)
        │ trigger fired
        ▼
[Feature Extractor] ───────────────▶ FeatureSet
        │
        ▼
[Rules Engine] ──── Confirmed ─────▶ DiagnosisOutput
        │ Inconclusive/Probable
        ▼
[Inference Engine] ─────────────────▶ (merge ML confidence)
        │
        ▼
[Remediation Planner] ─────────────▶ RemediationPlan
        │
        ▼
[Policy / Safety Gate] ── blocked ─▶ RecommendationOnly
        │ approved
        ▼
[Remediation Executor] ─────────────▶ ExecutionResult
        │
        ▼
[Verification Engine] ──────────────▶ VerificationResult
        │
        ▼
[Audit Logger + Storage] ───────────▶ AuditEvent + DB persist
        │
        ▼
[UI Surface] ───────────────────────▶ User notification / CLI output
```

---

## Recommended Technical Stack

### Agent Core
**Rust** — memory-safe, zero-runtime, cross-platform, excellent FFI, strong async via Tokio. The binary will be 5–12 MB. Idle memory: 15–25 MB.

**Why not Go:** Go GC pauses are non-deterministic; CGo is painful for IOKit/WMI. Go's runtime adds ~10 MB baseline.

**Why not C++:** Memory unsafety at remediation time is a serious risk. Compilation complexity across platforms is higher. No production safety advantage over Rust here.

### Collectors
- `sysinfo` (cross-platform baseline — CPU, memory, disk, processes)
- macOS: `core-foundation`, `core-foundation-sys` (IOKit access), `system-configuration` (network), direct `proc_info` syscalls
- Windows: `windows` crate (Microsoft's official), `wmi` crate for WMI queries

### Model Runtime
**ONNX Runtime via `ort` crate** (Rust bindings to the C ONNX Runtime library).

**Why not TFLite:** ONNX is the better export target from PyTorch/scikit-learn/XGBoost. Better cross-platform support in Rust. TFLite is mobile-first and has fewer Rust bindings.

**Why not custom inference:** Non-trivial maintenance burden. ONNX Runtime is battle-tested, supports quantization, and has dynamic loading (`load-dynamic` feature) so the ORT shared library is loaded at runtime — meaning the Rust binary does not statically link ORT, keeping binary size small.

### UI — Phase 1
**CLI via `clap`** — fast to build, testable, works everywhere.

### UI — Phase 2
**Tauri** — native system tray, minimal footprint (vs. Electron), Rust backend, WebView for settings UI. Avoids building a native UI in AppKit/WinUI, which is extremely painful.

### Local Storage
**SQLite via `rusqlite` with bundled feature** — zero deployment dependency, WAL mode, sub-millisecond reads. Appropriate for local audit trail and history.

**Why not plain files:** JSON files have no query capability. Concurrent access from CLI + background agent needs proper locking. SQLite handles both.

### Packaging
- macOS: `.pkg` installer via `pkgbuild`; LaunchDaemon plist for background service
- Windows: NSIS installer or WiX toolset; Windows Service registration

### Testing
- Unit: `#[test]` in each crate
- Integration: fixtures-based (`fixtures/snapshots/`, `fixtures/scenarios/`)
- Cross-platform: CI matrix (GitHub Actions) — `macos-latest` + `windows-latest`
- Synthetic scenario harness: inject fake `NormalizedSignalSnapshot` → assert `DiagnosisOutput`

---

## Tradeoffs Table

| Decision | Chosen | Alternative | Why not alternative |
|---|---|---|---|
| Agent language | Rust | Go | GC pauses; CGo FFI pain; larger runtime |
| Agent language | Rust | C++ | Memory unsafety at remediation time; no advantage |
| Model runtime | ONNX Runtime | TFLite | ONNX better PyTorch/sklearn export; better Rust bindings |
| Model runtime | ONNX Runtime | llama.cpp / llama.rs | LLM is wrong tool; massive overhead |
| Storage | SQLite | Plain JSON files | Need queries; need concurrent access safety |
| Storage | SQLite | PostgreSQL | No server, edge-only |
| UI (Phase 1) | CLI | Native UI | CLI is testable, fast to build, zero platform-specific code |
| UI (Phase 2) | Tauri | Electron | 5–15 MB vs 80–120 MB footprint; same capability |
| Collection model | Event-driven + poll | Always-on streaming | CPU/battery cost; not needed for the target issue set |
| ML strategy | Rules-first + targeted models | Full ML pipeline | Determinism required; most issues don't need ML |
