# Footprint Budget

Concrete resource targets and how the architecture stays within them.

---

## Target Budgets

| Metric | Phase 1 Target | Phase 2 Target | Phase 3 Target | Why This Number |
|---|---|---|---|---|
| Binary size (release, stripped) | <15 MB | <15 MB | <20 MB (includes ORT stub) | Comparable to native system utilities |
| Cold start (CLI, `firstaid --version`) | <200ms | <200ms | <200ms | Instant feel |
| `firstaid diagnose` end-to-end | <3s | <5s | <5s | User waiting at CLI |
| Idle CPU (daemon, no trigger) | 0% (polling sleep) | <0.1% avg | <0.1% avg | Background daemon must be invisible |
| Idle RAM (daemon) | n/a (CLI only) | <25 MB RSS | <30 MB RSS | Less than Chrome's idle tab |
| Burst CPU (during diagnosis) | <80% for <3s | <80% for <5s | <80% for <5s | Noticeable but not alarming |
| Peak RAM (during diagnosis) | <80 MB RSS | <100 MB RSS | <130 MB RSS | Model loads contribute during Phase 3 |
| Model load + inference | n/a | n/a | <50ms per model | Transparent to user |
| Model resident memory | n/a | n/a | 0 MB idle; <10 MB during inference | Load → infer → drop |
| SQLite DB size (7-day history) | <20 MB | <50 MB | <50 MB | Reasonable for local storage |
| Log file size (30 days, uncompressed) | <5 MB | <10 MB | <10 MB | With rotation |
| Total disk footprint (install) | <30 MB | <50 MB | <60 MB | Binary + models + DB + logs |

---

## How the Architecture Stays Lean

### Binary Size

**Rust release profile settings:**
```toml
[profile.release]
opt-level     = 3
lto           = "thin"     # Link-time optimization across crates
codegen-units = 1          # Single codegen unit → better dead-code elimination
strip         = true       # Strip debug symbols → 50-70% size reduction
```

**Key choices:**
- ONNX Runtime is **not statically linked** — it is loaded dynamically at runtime (`ort` crate with `load-dynamic` feature). The ORT shared library (~14 MB for macOS, ~12 MB for Windows) is separate from the agent binary and only ships in Phase 3.
- `sysinfo` is the baseline collector — it is a pure-Rust crate with no native library dependencies.
- `rusqlite` uses the `bundled` feature which statically embeds a stripped SQLite ~1.2 MB into the binary. This is intentional — no runtime SQLite dependency.
- Avoid crates with heavy transitive dependencies (e.g., no `tokio-tungstenite` unless needed for tray IPC, no `hyper` for HTTP).

**Estimated binary size breakdown (Phase 2):**
| Component | Contribution |
|---|---|
| Rust std + tokio + async runtime | ~3 MB |
| sysinfo | ~0.8 MB |
| rusqlite (bundled SQLite) | ~1.2 MB |
| serde + serde_json | ~0.4 MB |
| clap | ~0.5 MB |
| tracing + tracing-subscriber | ~0.6 MB |
| chrono + uuid | ~0.3 MB |
| All domain logic crates | ~4 MB |
| **Total estimate** | **~11 MB** |

### Idle CPU

**Key design:** The daemon never polls in a tight loop.

```
Idle loop:
  tokio::time::sleep(Duration::from_secs(30)).await;
  collect_cheap_signals();                    // < 5ms
  normalize();                                // < 1ms  
  evaluate_trigger(snapshot);                 // < 1ms
  if !trigger_fired { continue; }             // → sleep again 99% of the time
  // triggered path: < 5% of 30s cycles
```

Between sleep intervals, the daemon is in wait state — consuming 0 CPU. On a 30-second poll cycle:
- 30s sleep → 0% CPU
- 5ms collection → 0.017% average CPU contribution
- Trigger evaluation → effectively 0%

For event-driven watchers (crash log, event log): OS-native file system events via `notify` crate → epoll/kqueue/ReadDirectoryChangesW under the hood → 0% CPU until event fires.

### Idle RAM

**Why 25 MB RSS:**
- Rust binary mapped into memory: ~8–11 MB (code pages)
- Tokio runtime + async task stack: ~2–4 MB
- SQLite WAL mode + page cache: ~3–5 MB (configurable, cap with `PRAGMA cache_size`)
- Signal history ring buffer (last 10 snapshots, ~20 KB each): ~0.2 MB
- Tracing subscriber + log buffers: ~1 MB
- Other Rust allocations: ~2–3 MB
- **Total: ~18–25 MB RSS**

Controls:
```rust
// Cap SQLite cache to 1000 pages (4 MB)
conn.execute_batch("PRAGMA cache_size = -1000;")?;

// Limit signal history ring buffer
const SIGNAL_HISTORY_CAPACITY: usize = 10;
```

### Burst CPU (During Diagnosis)

The diagnosis pipeline is CPU-intensive for <5s and then idle again. During that window:
- Signal collection: ~5ms (cheap sysinfo calls)
- Normalization: ~1ms
- Feature extraction: ~1ms
- Rules engine: ~2ms (simple comparison trees)
- ML inference (Phase 3): ~10–50ms per model

Total active time: ~60ms for a rules-only diagnosis, ~200ms with one ML model. The "3-second" budget includes waiting for OS-native calls (WMI, IOKit) which are the bottleneck, not our code.

**CPU usage during the 60–200ms active window:** on a modern core, running pure Rust logic at 100% of one core. On a 4-core machine: 25% total CPU. For <200ms. This is not noticeable.

### Model Memory

ONNX Runtime allocates model memory on session creation and releases on session drop:

```rust
let session = Session::builder()?.commit_from_file(&model_path)?;
let outputs = session.run(inputs)?;
drop(session);   // ORT memory freed here
```

The `drop(session)` call releases the memory allocated by ORT for the session, input/output tensors, and workspace. Verified by measuring RSS before and after in integration tests.

### SQLite DB Size

At 30-second poll intervals, one `NormalizedSignalSnapshot` is ~3–5 KB as JSON (or ~1.5–2 KB CBOR if we compress). At 30s intervals over 7 days:
- 7 days × 86400 seconds/day ÷ 30s = 20,160 samples
- 20,160 × 2 KB avg = ~39 MB uncompressed

To stay within 50 MB:
- Use SQLite's built-in BLOB compression (store snapshots as compressed JSON via deflate)
- Or reduce poll interval to 60s for snapshot storage (separate from the 30s trigger evaluation which only keeps in-memory state)
- Or store only the delta from previous snapshot (most fields don't change between samples)

Practical approach: store a compressed snapshot every 60s = 10,080 samples × 1 KB compressed ≈ 10 MB.

### Log File Size

Using `tracing-subscriber` with JSON output + log rotation:
- 1 diagnostic event per hour (typical non-problem machine): ~1 KB per event = 8.7 MB/year
- With `max_files = 7`, `max_file_size = 10 MB` → 70 MB max, well within budget
- Audit log rows are structured (not verbose) → low per-event overhead

---

## Footprint CI Enforcement

Add a CI step that fails if these limits are exceeded:

```yaml
# .github/workflows/footprint.yml
- name: Check binary size
  run: |
    size=$(stat -c%s target/release/firstaid)
    limit=$((20 * 1024 * 1024))  # 20 MB
    if [ $size -gt $limit ]; then
      echo "Binary too large: $size bytes (limit $limit)"
      exit 1
    fi

- name: Check cold start
  run: |
    start=$(date +%s%N)
    ./target/release/firstaid --version
    end=$(date +%s%N)
    elapsed=$(( (end - start) / 1000000 ))
    if [ $elapsed -gt 500 ]; then
      echo "Cold start too slow: ${elapsed}ms"
      exit 1
    fi
```

---

## Comparison Against Alternatives

| Tool | Idle RAM | Footprint | Inference |
|---|---|---|---|
| **This agent** | ~25 MB | ~60 MB total | Rules + tiny ONNX |
| Elastic Agent | ~200–400 MB | ~300 MB+ | None (log shipping) |
| Datadog Agent | ~150–300 MB | ~250 MB+ | None (metric forwarding) |
| Crowdstrike Falcon | ~50–150 MB | kernel driver + agent | Cloud-only |
| LLM-based copilot (e.g., Copilot+ PC) | ~2–4 GB (NPU) | ~5 GB model | Full SLM always hot |
| **Target: invisible to user** | **<30 MB** | **<60 MB** | **Lazy, <50ms** |
