# m-firstaid-doctor

**Local-first endpoint health diagnostics and remediation for macOS and Windows.**

A lightweight background agent that observes, diagnoses, and safely remediates the top 10 ITSM-class endpoint issues — without cloud dependencies, without a large LLM, and without heavy resource overhead.

---

## Design Principles

- **Rules-first:** Deterministic logic handles all common cases. ML is targeted augmentation, not the engine.
- **Edge-only:** No cloud calls required. Works fully offline.
- **Safety-gated:** Every remediation action is classified (AutoSafe / ApprovalRequired / ElevationRequired / NeverAutoFix). No destructive action without explicit user consent.
- **Minimal footprint:** <30 MB idle RAM, <15 MB binary, <0.1% idle CPU.
- **Auditable:** Append-only audit log with event chain for every action.

## Desktop UI (Tauri)

A minimal **native window** lives under [`desktop/`](desktop/README.md). It shells out to `firstaid diagnose --ui-bundle` and displays health, snapshot summary, and diagnoses.

```bash
cargo build -p agent-core
cd desktop && npm install && npm run tauri dev
```

## Quick Start (Phase 1 CLI)

```bash
# Diagnose current endpoint
firstaid diagnose

# Diagnose from a fixture (no OS access required)
firstaid --fixture fixtures/scenarios/memory_critical_pressure.json diagnose --json

# Show what remediation would do (no execution)
firstaid plan

# Verify audit log integrity
firstaid verify-audit
```

## Documentation

| Document | Description |
|---|---|
| [Architecture](docs/architecture.md) | Full system design, component breakdown, tradeoffs |
| [Issue Catalog](docs/issue-catalog.md) | Top-10 issue classes with diagnosis and remediation logic |
| [Feature Schema](docs/feature-schema.md) | Normalized feature set definition |
| [Data Contracts](docs/data-contracts.md) | JSON schemas for all inter-component types |
| [SLM Strategy](docs/slm-strategy.md) | Where models help, where they are overkill |
| [Safety Model](docs/safety-model.md) | Action classification, consent model, protected resources |
| [Roadmap](docs/roadmap.md) | Phased MVP plan (Phase 1–4) |
| [Evaluation](docs/evaluation.md) | Test plan and synthetic harness |
| [Footprint Budget](docs/footprint-budget.md) | Resource targets and how we stay lean |

## Repo Structure

```
crates/
  agent-core/          Main binary (firstaid) — orchestrator + CLI
  collectors/          Cross-platform signal collection (sysinfo baseline)
  collectors-mac/      macOS: IOKit, pmset, launchctl, airport
  collectors-win/      Windows: WMI, Registry, Event Log, PowerShell
  signal-normalizer/   Raw → NormalizedSignalSnapshot
  feature-extractor/   Snapshot → FeatureSet (derived, typed)
  trigger-engine/      Threshold + event triggers with hysteresis
  detector/            Rules engine for all 10 issue classes
  inference/           ONNX model lazy-load runner (Phase 3)
  remediation/         Plan, execute, verify remediations
  policy/              Safety gates + consent model
  storage/             SQLite: snapshots, diagnoses, audit log
  audit/               Append-only event audit logger
  ui-cli/              Human-readable CLI rendering

docs/                  Architecture and engineering docs
fixtures/              Test snapshots and synthetic scenario fixtures
models/                ONNX model files (Phase 3, distributed separately)
scripts/               Build and test scripts
```

## Supported Issue Classes

| # | Issue | Detection | Safe Auto-Fix? |
|---|---|---|---|
| 1 | High CPU | Rules + optional anomaly model | Report only (kill: approval) |
| 2 | High Memory | Rules (deterministic) | Report only (kill: approval) |
| 3 | Low Disk | Rules (deterministic) | Cache clear (approval) |
| 4 | Disk I/O | Rules + z-score | SMART alert (recommendation) |
| 5 | Startup Slow | Rules + optional GBT scorer | Disable items (approval) |
| 6 | App Crash | Rules + optional text classifier | Cache clear (approval) |
| 7 | Network/DNS | Rules (layered test) | DNS flush (auto) |
| 8 | Service Down | Rules (deterministic) | Restart (approval) |
| 9 | Update Issues | Rules (registry flags) | Never auto (reboot) |
| 10 | Battery/Thermal | Rules + trend analysis | Alert only |

## Building

```bash
# macOS (universal binary)
./scripts/build-mac.sh

# Windows
.\scripts\build-win.ps1

# Development
cargo build

# Tests
cargo test

# Synthetic scenario tests
./scripts/run-scenario-tests.sh
```

## Technology Stack

| Component | Choice | Why |
|---|---|---|
| Agent core | Rust + Tokio | Memory-safe, minimal runtime, cross-platform |
| Collectors | sysinfo + OS-native | sysinfo for baseline; platform crates for depth |
| Model runtime | ONNX Runtime (ort) | Best cross-platform ML inference; load-dynamic |
| Storage | SQLite (rusqlite, bundled) | Zero deployment dependency, WAL mode |
| CLI | clap | Ergonomic, well-maintained |
| Phase 2 UI | Tauri | Native tray, minimal footprint (vs. Electron) |

## Safety

This agent will NEVER automatically:
- Reboot or shut down the system
- Install or uninstall software
- Delete files outside approved temp/cache paths
- Modify system integrity settings
- Act on protected system processes
- Execute any elevated action without user consent

See [Safety Model](docs/safety-model.md) for the complete framework.
