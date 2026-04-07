# Final Engineering Recommendation

## Blunt Answers

### Is this product best built as rules-first + small models?

**Yes, unequivocally.**

6 of 10 issue classes (high memory, low disk, network/DNS, service down, update issues, disk I/O) are 100% deterministically diagnosable from OS signals. Adding ML to these problems would:
- Introduce non-determinism where determinism is achievable
- Add maintenance burden (model updates, drift)
- Make the system harder to debug when a diagnosis is wrong
- Add load latency for zero accuracy gain

The 4 classes where ML adds genuine value are narrow and well-bounded:
1. **App crash** — crash log text classification (CharCNN, ~300 KB)
2. **High CPU** — process behavior anomaly (Isolation Forest, ~500 KB)
3. **Startup slow** — startup item ranking (XGBoost, ~200 KB)
4. **Battery** — degradation trend (linear regression, inline, no model file)

**Total ML model storage: ~1 MB. Total idle ML memory: 0 MB.**

---

### Which parts should absolutely avoid ML?

| Issue Class | Reason to Avoid ML |
|---|---|
| Low Disk | `free_bytes < threshold` is perfectly deterministic |
| High Memory | macOS exposes `memory_pressure` enum directly; Windows exposes commit charge |
| Network/DNS | Decision tree of network stack layers; 100% rule-expressible |
| Service Down | Binary state + restart count; perfectly deterministic |
| Update Issues | Registry/OS flags; binary state |
| Disk I/O (basic) | Queue depth and SMART are directly observable |

If someone proposes "let's use an LLM to analyze why the disk is slow," fire them. The disk queue depth tells you the answer.

---

### Which 3 issue classes are the best initial candidates for specialized models?

**Priority order:**

**1. App Crash (crash log text classification)**
- Highest user impact
- Most non-deterministic signal (free-text crash logs)
- Clear training data source (Apple OSS crash dumps, publicly available crash logs)
- Clear taxonomy (6 classes)
- ~85% accuracy improvement over rules-only for root cause attribution
- CharCNN: 300 KB, <5ms inference

**2. High CPU (process anomaly detection)**
- The #1 user complaint
- Rules handle the obvious cases; anomaly detection catches the subtle ones (slow-burn runaway, multiple processes)
- Isolation Forest on process feature vector
- Clear value: distinguishes "kernel_task high = thermal throttling" from "unknown binary high = suspicious"
- 500 KB, <10ms inference, cold-start solvable with a shipped baseline model

**3. Startup Slow (startup item scoring)**
- High user frustration, low risk (recommendation-only)
- GBT trained on known-bad agent patterns
- Curated training data from known slow startup items
- Can ship as a static lookup table for Phase 1 → upgrade to model in Phase 3
- 200 KB, <2ms inference

---

### What stack would you personally choose for v1?

```
Language:      Rust 1.78+
Async:         Tokio
Collectors:    sysinfo (baseline) + platform-native subprocesses (Phase 1)
               → IOKit FFI / windows crate (Phase 2)
Storage:       SQLite via rusqlite (bundled)
Audit:         SQLite (append-only table) + tracing-subscriber JSON logs
CLI:           clap 4
ML (Phase 3):  ONNX Runtime via ort (load-dynamic)
UI (Phase 2):  Tauri 2 (not Electron, not pure CLI forever)
CI:            GitHub Actions with macos-latest + windows-latest matrix
Packaging:     cargo-bundle → .pkg (macOS), WiX → .msi (Windows)
```

**What I would NOT use:**

| Rejected | Reason |
|---|---|
| Go | GC pauses; CGo pain for IOKit/WMI |
| C++ | Memory unsafety at remediation time; no advantage |
| Python | Startup time (>500ms), memory footprint, distribution hell |
| Electron | 80–120 MB for a tray icon is insulting |
| Any LLM | Wrong tool. Will never run fast enough on-device, wrong accuracy profile |
| TFLite | ONNX is the better cross-platform export target |
| PostgreSQL | No server, edge-only |
| Custom ML inference | Non-trivial to maintain; ONNX Runtime is battle-tested |

---

## Top Architecture Risks (Ranked)

### 1. Cross-platform collector parity (HIGH)
The hardest engineering problem. macOS and Windows expose the same concepts (memory pressure, thermal state, startup items) through completely different APIs with different granularity and reliability.

**Mitigation:** Platform adapter pattern is the right call. Test on both platforms from Day 1 in CI. Don't let the macOS implementation get too far ahead — you'll pay a painful porting tax.

**The specific traps:**
- Windows WMI queries can be slow (>2s) for some classes — always async with timeout
- macOS IOKit for SMC temperatures requires entitlements on Apple Silicon — use subprocess fallback
- Windows Event Log subscription requires COM initialization on the correct thread
- macOS crash log path changes between OS versions (`/Library/Logs/DiagnosticReports` vs `~/Library/Logs/DiagnosticReports`)

### 2. Remediation safety at edge (CRITICAL)
Any bug in the remediation executor on a live endpoint can cause real damage. The safety model is only as good as its enforcement.

**Mitigation:** Protected list + approved path list is compile-time. Policy evaluator has 100% unit test coverage. Executor pre-checks are mandatory and logged. Dry-run mode for all cache clearing.

**The specific trap:** The `KillProcess` action on macOS with the wrong PID is permanent. Always verify name matches PID before sending the signal. Race condition: process may have died and another spawned with the same PID. Add name verification in pre-check.

### 3. Trigger storm + false positives (HIGH)
A machine under load may cross multiple thresholds simultaneously. Without hysteresis and storm prevention, you flood the user with notifications.

**Mitigation:** TriggerEngine has built-in hysteresis (recovery threshold must be crossed before re-arming) and cooldown (5 minutes between triggers for the same class). Batch simultaneous triggers into one diagnostic run.

### 4. macOS Helper binary complexity (MEDIUM — deceptive)
The Helper binary for elevation looks simple. It is not. Apple's security model for privileged helpers has changed significantly across macOS versions. SMJobBless (the old way) is deprecated. The new way involves XPC services with Launchd plist co-signing.

**Mitigation:** Phase 1 has zero elevation actions. Phase 2 adds one or two carefully chosen elevation actions with proper XPC design. Don't underestimate this.

### 5. Crash log dataset quality (MEDIUM — Phase 3)
The crash classifier is only as good as its training data. macOS crash logs from real-world apps vary enormously in quality — some are truncated, some are stripped, some contain only hex addresses.

**Mitigation:** Build the dataset pipeline before committing to the model. Validate on a held-out test set with logs from different macOS versions and app types before shipping.

---

## Where "AI" Is Unnecessary

| Proposed | Reality | Better Approach |
|---|---|---|
| LLM to explain diagnoses | Hardcoded templates are more accurate, translatable, and never hallucinate | Template strings per rule |
| ML for disk free % monitoring | `free_bytes < threshold` | One line of Rust |
| ML for memory pressure | OS already classifies it for us | Read the IOKit value |
| ML for network reachability | Binary: reachable or not | `network_reachable` bool |
| Embeddings for log similarity | Classification is the right abstraction | CharCNN or rules |
| A "health score" | Gamified metrics mislead; "82% healthy" is meaningless | Issue list with severities |

---

## What Can Be Built Quickly vs. What Is Deceptively Hard

### Quick (1–3 days each, staff engineer)
- sysinfo-based CPU/memory/disk collection
- Rules engine for low disk and memory pressure (thresholds + pattern matching)
- SQLite schema and audit log
- CLI with `diagnose` subcommand
- JSON fixture loading for test mode
- DNS flush action (single command call)
- Synthetic scenario test fixtures

### Moderate (1–2 weeks each)
- IOKit memory pressure on macOS (unsafe FFI, requires testing on M1 and Intel)
- Trigger engine with hysteresis (stateful, needs careful test coverage)
- Remediation executor with pre-checks (must handle all failure modes)
- Process kill with protected list enforcement

### Deceptively Hard (2–4+ weeks each)
- macOS Helper binary with XPC for elevation (Apple security model complexity)
- Crash log file watcher (platform-specific file event APIs, lock contention)
- Windows Event Log subscription (COM, threading model, session lifecycle)
- Isolation Forest per-machine baseline (cold-start, requires local data accumulation)
- Tauri tray app that works across dark mode, multiple monitors, and notch on both platforms

### Dangerous Distractions (Do Not Build)
- A general NLP interface to the agent
- Remote telemetry / cloud sync
- A browser extension
- A "health score" dashboard
- Plugin system for third-party detectors
- Backwards-compatible API versioning before v1 is shipped

---

## First 2–3 Week Milestone Plan

### Week 1: Core types + collectors + rules

**Days 1–2:**
- [ ] Set up workspace, CI on macOS + Windows
- [ ] `collectors` crate: sysinfo CPU/memory/disk baseline compiling and tested on both platforms
- [ ] `signal-normalizer` crate: unit tests with fixture JSON

**Days 3–4:**
- [ ] `feature-extractor` crate: all fields computed, unit tests
- [ ] `trigger-engine` crate: threshold triggers, hysteresis tested

**Day 5:**
- [ ] `detector/rules/cpu.rs` and `detector/rules/memory.rs`
- [ ] Fixture tests: `high_cpu_single_process.json` → correct diagnosis

### Week 2: Remaining rules + storage + CLI

**Days 6–7:**
- [ ] `detector/rules/disk.rs`, `network.rs`, `battery.rs`
- [ ] All 5 in-scope issue classes produce correct diagnosis from fixtures

**Days 8–9:**
- [ ] `storage` crate: SQLite schema, snapshot save/load, round-trip tests
- [ ] `audit` crate: append-only events, chain verification test

**Day 10:**
- [ ] `agent-core` + `ui-cli`: `firstaid diagnose` runs end-to-end
- [ ] `firstaid --fixture <path> diagnose --json` works

### Week 3: Policy + remediation + integration

**Days 11–12:**
- [ ] `policy/safety.rs`: gate tests (all cases in test suite)
- [ ] `remediation/planner.rs`: plans generated for each issue class

**Days 13–14:**
- [ ] `remediation/executor.rs`: DNS flush + report actions execute
- [ ] Cross-platform CI green on both platforms

**Day 15:**
- [ ] `scripts/run-scenario-tests.sh` passes all 6 fixtures
- [ ] Binary size check in CI: <15 MB
- [ ] Tag v0.1.0-alpha
- [ ] Write "what's next" for Phase 2 scoping

---

## Final Verdict

This is a buildable, valuable product. The architecture is sound. The rules-first approach is correct. The ML strategy is appropriately conservative.

**Build it in this order:**
1. Deterministic rules for 5 issue classes (Phase 1) — proves the concept
2. Background daemon + remaining 5 classes (Phase 2) — makes it real
3. Crash classifier + process anomaly model (Phase 3) — adds intelligence where it earns its keep

**Do not let anyone add a general LLM to this system.** It would destroy the footprint budget, introduce hallucination risk at remediation time, and solve nothing that the rules don't already handle better.

The product's moat is its safety model, deterministic core, and zero-cloud-dependency design. Those are harder to copy than a fine-tuned model.
