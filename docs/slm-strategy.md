# SLM / Specialized Model Strategy

## The Hard Rule First

**Do not use ML where a deterministic rule or a simple statistical measure works.** ML adds:
- Non-determinism at inference boundary
- Model size and load latency
- Training data and maintenance burden
- Explainability debt (harder to debug wrong diagnoses)

ML is justified when:
1. The signal is unstructured (text, time-series) and hard to threshold
2. The pattern is multivariate and threshold rules would be an unmaintainable combinatorial explosion
3. There is historical evidence that rules alone produce excessive false positives or miss real issues

With those constraints, here is the honest per-issue assessment:

---

## Per-Issue Decision Matrix

| # | Issue | Approach | ML Type | Justification |
|---|---|---|---|---|
| 1 | High CPU | Rules + optional Isolation Forest | Anomaly detection | Rules catch single-process spike; IF catches distributed subtle anomalies |
| 2 | High Memory | **Rules only** | — | Memory pressure is directly observable from OS; no ML needed |
| 3 | Low Disk | **Rules only** | — | Pure measurement; zero ambiguity |
| 4 | Disk I/O | **Statistical** (z-score) | Not ML | Rolling z-score on queue depth is enough; no model overhead |
| 5 | Startup Slow | Rules + optional GBT | GBT scoring | Rules detect count; GBT can rank which startup items are causative |
| 6 | App Crash | Rules + optional CharCNN | Text classifier | Exception types are structured; crash log detail needs text classification |
| 7 | Network/DNS | **Rules only** | — | Decision tree of network stack layers; no pattern ambiguity |
| 8 | Service Down | **Rules only** | — | Binary state + count; perfectly deterministic |
| 9 | Update Issues | **Rules only** | — | Registry/OS flags; no ambiguity |
| 10 | Battery/Thermal | Rules + optional linear regression | Time series | Trend for degradation prediction; current state is deterministic |

**Summary: 6 of 10 issue classes need zero ML. The other 4 benefit from targeted, small models.**

---

## Where Tiny Transformers Help (and Where They Are Overkill)

### Crash Log Classification — where a tiny transformer EARNS its place

**Problem:** A crash log is unstructured text. The exception type field (`EXC_BAD_ACCESS`, `SIGKILL`) is structured and handled by rules. But the crash thread stack trace, the reason string, and the subsystem name are free-form text that contains real diagnostic signal:

- "Unable to acquire critical section lock" → resource contention crash
- "malloc: double free for pointer" → memory corruption
- "Bus error at address 0x0" → null pointer dereference
- "Assertion failure in [framework]::validateInput" → bad data from upstream
- "Terminated due to memory issue" → OOM kill (already covered by rules, but good to reinforce)

A 4-layer character-level CNN or a quantized distilbert-tiny (6-layer, 4 attention heads, 14.5M params → ~3 MB INT8 ONNX) can classify these into a 6-class taxonomy:
1. MemoryCorruption
2. ResourceExhaustion
3. NullDereference
4. ExternalKill
5. AssertionFailure
6. Unknown

**This matters** because it drives remediation: a MemoryCorruption crash points toward clearing app data / reinstall; ResourceExhaustion points toward memory limits.

**Why not rules:** The text is free-form and varies across app versions, macOS versions, and developers. A rule would need to enumerate thousands of patterns.

**Alternative to transformer:** CharCNN is actually preferable here — it is smaller (~300 KB ONNX), faster (<5ms), and works well for fixed-taxonomy classification over short text. TinyBERT is only worth it if the input is longer and more semantically complex.

---

### Process Anomaly Detection (CPU) — where Isolation Forest is better than a transformer

**Problem:** CPU usage by a single process is high. But is it a malicious process, a runaway loop, or just a legitimate heavy compute job (video export, ML training)?

Rules detect "CPU > 80% for 5+ minutes." But they cannot distinguish:
- `kernel_task` pegged at 60% (thermal protection — benign; don't kill)
- `mdworker` at 60% post-update (transient re-index — wait it out)
- `mds_stores` at 80% for 3 hours (stuck indexer — restart safe)
- Unknown process at 80% with no known parent (suspicious)
- Chrome GPU process at 75% (legitimate WebGL workload)

**Why Isolation Forest:** It learns a distribution of normal process behavior from historical baselines (feature vector: [cpu_pct, mem_pct, io_bytes/s, process_age_hours, open_file_count, parent_process_hash]). An isolation forest flags deviations from that distribution. It is:
- ~500 KB ONNX export (100 trees, max depth 8)
- <10ms inference
- No GPU required
- Interpretable: can report which features drove the anomaly score

**Why not a transformer:** This is a numerical tabular problem, not a sequence or text problem. Transformers are a terrible fit for tabular data. Tree-based models win here by a large margin.

---

### Startup Item Scoring — where GBT adds optional value

**Problem:** A user has 18 startup items. Which ones should be disabled first? Rules can detect "count is high." But ranking requires knowing which items are historically slow, resource-hungry, or rarely needed.

A gradient-boosted tree (XGBoost/LightGBM → ONNX export) trained on:
- Known slow agents (curated list embedded in model training data)
- Launch agent path patterns (vendor paths, user vs system)
- Binary size (large binary = more load time)
- Historical startup agent profiling data (from aggregated telemetry, never individual)

Output: a 0–1 "remove priority" score per startup item.

This is **advisory only** — the score surfaces recommendations, not automatic actions.

**Model size:** ~300 KB ONNX. Load time: <3ms.

**MVP decision: defer this model to Phase 3.** For Phase 1, a curated known-bad-agent list (embedded in code as a static HashSet) is sufficient.

---

### Battery Degradation Prediction — where linear regression is enough (no transformer)

**Problem:** Battery health is dropping. How fast? When will it hit 80% (Apple's recommended replacement threshold)?

A simple linear regression over `battery_health_pct` time series (90-day window, sampled daily) gives a degradation rate. Extrapolate to find the crossing date.

This is **not an ML problem in any meaningful sense.** It is a linear regression, computable with 30 lines of Rust arithmetic. No ONNX model needed.

If you wanted to be fancy, an Exponential Smoothing / Holt-Winters model handles non-linear degradation. Still computable inline. No model file required.

**Do not use a transformer here. It is massively overkill for a time series with one variable.**

---

## Model Size, Latency, and Memory Budget

| Model | Issue Class | Architecture | Format | File Size | Load Time | Inference Time | Resident Memory |
|---|---|---|---|---|---|---|---|
| `crash-classifier` | App Crash | CharCNN (3 conv layers) | ONNX INT8 | ~300 KB | ~15ms | ~3ms | ~4 MB (session) |
| `process-anomaly` | High CPU | Isolation Forest (100 trees) | ONNX | ~500 KB | ~5ms | <5ms | ~2 MB (session) |
| `startup-scorer` | Startup Slow | XGBoost (200 trees, depth 4) | ONNX | ~200 KB | ~3ms | <2ms | ~1 MB (session) |
| — | Battery | Linear regression (inline) | None | 0 bytes | 0ms | <1ms | 0 |
| — | Memory/Disk/Network/Service/Update | Rules only | None | 0 bytes | 0ms | <1ms | 0 |

**Total model storage:** ~1 MB (all three models).
**Peak inference memory:** ~7 MB (one model at a time, unloaded after inference).
**Idle model memory:** 0 MB — all models are file-mapped, loaded on trigger, dropped after inference.

---

## Inference Strategy

```
Policy: trigger → load specific model → infer → drop session
         ^-- NEVER pre-load. NEVER keep hot.

Implementation:
  1. TriggerEngine fires for IssueClass
  2. If RulesEngine returns Confirmed → skip ML entirely
  3. If RulesEngine returns Probable or Possible:
       a. Check model registry: is a model registered for this issue class?
       b. If yes: load model from disk path (mmap, verify checksum)
       c. Pre-process FeatureSet → model input tensor
       d. Run ORT InferenceSession
       e. Post-process output → confidence + class
       f. Drop InferenceSession (memory released)
       g. Merge ML confidence with rule confidence
  4. Emit DiagnosisOutput
```

**One hot model at a time.** The system is single-threaded for diagnosis (one pipeline active). No concurrent model loading.

---

## Quantization Strategy

- **INT8 post-training quantization** for all classification models
- Use ONNX Runtime's built-in quantization tools (`onnxruntime.quantization`)
- Target: within 1% accuracy of FP32 baseline on held-out test set
- Always verify quantized model against FP32 on test fixtures before shipping
- CharCNN: INT8 quantization is lossless for this task
- Isolation Forest / XGBoost: ONNX opset 12+, quantization applies to the leaf output layer; tree structures are inherently discrete

---

## What to Not Build

| Temptation | Why Not |
|---|---|
| Fine-tune a 1B-param SLM (Phi-3, Gemma-2B) for diagnosis | 2 GB RAM, 500ms+ inference, wrong tool |
| Use an embedding model for log search | No retrieval needed; classification is the right abstraction |
| Build a streaming anomaly detection pipeline | Most issues are threshold-detectable; Kafka/Flink overhead is insane for this use case |
| Train issue-specific models for memory/disk/network | Rules are 100% accurate for these; ML adds zero value |
| Use a transformer for startup item scoring | Tabular data; GBT is correct choice, and even that can wait until Phase 3 |
| Build a recommendation LLM that explains diagnoses | Hardcoded templates are better: deterministic, translatable, no hallucination risk |

---

## Risks in the Model Layer

| Risk | Impact | Mitigation |
|---|---|---|
| Model file tampered with | Wrong diagnosis, possible unsafe remediation | SHA-256 checksum verification on every load |
| Model produces wrong label for an unusual crash | Remediation directed at wrong root cause | Model output is advisory only; rules-based evidence always shown |
| ONNX Runtime library is unavailable | ML features non-functional | Graceful degradation: rules-only mode when ORT load fails |
| Isolation Forest trained on biased process data | Legitimate processes flagged as anomalous | Conservative threshold (top 5% isolation score, not top 20%); always shows user evidence |
| Model update breaks existing behavior | Regression in diagnosis quality | Model versioning; A/B test on fixtures before shipping; pin model hash in config |
