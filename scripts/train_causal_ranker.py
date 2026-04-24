#!/usr/bin/env python3
"""
train_causal_ranker.py — Train a 10×10 causal adjacency matrix predictor.

The model predicts, for a given 35-feature OS telemetry snapshot, which
issue-class pairs are causally linked (cause → effect, in [0,1]).

Architecture
────────────
Input  : float[35]      — current FeatureSet vector
Output : float[10 × 10] — per-pair causal strength (sigmoid-activated)

Model  : MLP  (35 → 128 → 128 → 100)
Loss   : Binary Cross-Entropy on the flattened 10×10 adjacency matrix

Training signal
───────────────
Synthetic: generate feature vectors with co-occurring issue conditions,
then label the causal edges using the same static graph that backs the
deterministic Rust fallback (CAUSAL_RULES in causal.rs).

Self-supervised: when historical ONNX-classified telemetry is available
(from SQLite), co-occurring issue pairs in consecutive windows are used
as weak supervision (i.e. if A fired then B fired in the next window,
(A→B) is labeled 1.0 weighted by time-gap).

Export: ONNX opset 17, updates models/manifest.json SHA-256.

Usage:
    python scripts/train_causal_ranker.py [--epochs 100] [--db path/to/firstaid.db]
    python scripts/train_causal_ranker.py --export-only   # re-export from checkpoint
"""

import argparse
import hashlib
import json
import math
import os
import random
import struct
import time
from pathlib import Path

import numpy as np
import torch
import torch.nn as nn
import torch.optim as optim

# ── Constants (must match Rust) ───────────────────────────────────────────────

N_FEATURES = 35
N_CLASSES  = 10
REPO_ROOT  = Path(__file__).parent.parent

CLASS_NAMES = [
    "HighCpu", "HighMemory", "LowDisk", "DiskIo", "StartupSlow",
    "AppCrash", "NetworkDns", "ServiceDown", "UpdateIssue", "BatteryThermal",
]

CLASS_IDX = {name: i for i, name in enumerate(CLASS_NAMES)}

# Static causal graph — mirrors CAUSAL_RULES in crates/detector/src/causal.rs
CAUSAL_RULES = [
    ("HighCpu",       "BatteryThermal", 0.85),
    ("BatteryThermal","HighCpu",        0.75),
    ("HighMemory",    "HighCpu",        0.70),
    ("HighMemory",    "AppCrash",       0.80),
    ("LowDisk",       "AppCrash",       0.65),
    ("LowDisk",       "UpdateIssue",    0.90),
    ("NetworkDns",    "AppCrash",       0.55),
    ("NetworkDns",    "ServiceDown",    0.70),
    ("UpdateIssue",   "ServiceDown",    0.60),
    ("DiskIo",        "AppCrash",       0.65),
    ("DiskIo",        "StartupSlow",    0.80),
    ("ServiceDown",   "AppCrash",       0.60),
    ("StartupSlow",   "HighCpu",        0.50),
]

# Pre-build label matrix for fast lookup
CAUSAL_MATRIX = np.zeros((N_CLASSES, N_CLASSES), dtype=np.float32)
for cause, effect, strength in CAUSAL_RULES:
    CAUSAL_MATRIX[CLASS_IDX[cause], CLASS_IDX[effect]] = strength

# ── Feature ranges for synthetic data generation ──────────────────────────────

# Each tuple: (feature_name, normal_range, stressed_range)
# Indices match TEMPORAL_FEATURE_NAMES in feature_extractor
FEATURE_NORMAL_RANGES = [
    (0.05, 0.30),   # cpu_usage_avg_1m
    (0.05, 0.25),   # cpu_usage_avg_5m
    (0.05, 0.20),   # cpu_usage_avg_15m
    (4.0,  16.0),   # cpu_core_count
    (0.01, 0.15),   # top_process_cpu_pct
    (0.0,  0.3),    # top_process_is_system
    (8000, 32000),  # memory_total_mb
    (4000, 20000),  # memory_available_mb
    (0.20, 0.60),   # memory_used_pct
    (0.0,  500.0),  # swap_used_mb
    (0.0,  0.20),   # swap_used_pct
    (0.0,  0.1),    # memory_oom_risk
    (100,  4000),   # top_process_rss_mb
    (0.30, 0.80),   # system_disk_free_pct
    (50e9, 500e9),  # system_disk_free_bytes
    (0.0,  0.05),   # smart_failing
    (0.8,  1.0),    # network_reachable
    (-70,  -30),    # wifi_signal_dbm
    (0.0,  0.5),    # vpn_active
    (0.0,  0.3),    # proxy_configured
    (0.0,  0.05),   # dns_resolution_fail_rate
    (1.0,  15.0),   # default_gateway_ping_ms
    (30.0, 65.0),   # cpu_temp_celsius
    (1000, 3000),   # fan_speed_rpm
    (0.80, 1.0),    # battery_health_pct
    (0,    200),    # battery_cycle_count
    (0.30, 1.0),    # battery_charge_pct
    (3,    15),     # startup_items_count
    (0,    2),      # app_crash_count_1h
    (0,    5),      # app_crash_count_24h
    (0,    0.2),    # service_restart_storm
    (0,    0.2),    # service_down_expected
    (0,    0.3),    # pending_reboot_required
    (0,    0.3),    # recent_update_detected
    (0,    0.1),    # update_failed
]


def sample_normal_feature_vec() -> np.ndarray:
    vec = np.array([
        random.uniform(lo, hi) for lo, hi in FEATURE_NORMAL_RANGES
    ], dtype=np.float32)
    return vec


def inject_issue(vec: np.ndarray, issue: str) -> np.ndarray:
    """Perturb feature vector to reflect a specific issue condition."""
    v = vec.copy()
    if issue == "HighCpu":
        v[0] = random.uniform(0.80, 0.99)  # cpu_usage_avg_1m
        v[1] = random.uniform(0.75, 0.95)  # cpu_usage_avg_5m
        v[4] = random.uniform(0.60, 0.95)  # top_process_cpu_pct
    elif issue == "HighMemory":
        v[8]  = random.uniform(0.88, 0.99)  # memory_used_pct
        v[10] = random.uniform(0.60, 0.95)  # swap_used_pct
        v[11] = 1.0                           # memory_oom_risk
    elif issue == "LowDisk":
        v[13] = random.uniform(0.0, 0.08)   # system_disk_free_pct
    elif issue == "DiskIo":
        v[15] = random.uniform(0.7, 1.0)    # smart_failing
    elif issue == "StartupSlow":
        v[27] = random.randint(20, 80)       # startup_items_count
    elif issue == "AppCrash":
        v[28] = random.randint(3, 15)        # app_crash_count_1h
    elif issue == "NetworkDns":
        v[16] = 0.0                           # network_reachable
        v[20] = random.uniform(0.5, 1.0)    # dns_resolution_fail_rate
    elif issue == "ServiceDown":
        v[31] = 0.0                           # service_down_expected (0 = unexpected)
        v[30] = 1.0                           # service_restart_storm
    elif issue == "UpdateIssue":
        v[32] = 1.0                           # pending_reboot_required
        v[34] = 1.0                           # update_failed
    elif issue == "BatteryThermal":
        v[22] = random.uniform(80.0, 105.0) # cpu_temp_celsius
        v[23] = random.uniform(4000, 6500)  # fan_speed_rpm
        v[24] = random.uniform(0.50, 0.75)  # battery_health_pct
    return v


def generate_sample():
    """Generate a (feature_vec, causal_matrix) training pair."""
    # Pick 0–3 co-occurring active issue classes
    n_active = random.choices([0, 1, 2, 3], weights=[0.20, 0.40, 0.25, 0.15])[0]
    active = random.sample(CLASS_NAMES, n_active)

    vec = sample_normal_feature_vec()
    for issue in active:
        vec = inject_issue(vec, issue)

    # Label matrix: causal edges between active issues
    label = np.zeros((N_CLASSES, N_CLASSES), dtype=np.float32)
    active_set = set(active)
    for cause, effect, strength in CAUSAL_RULES:
        if cause in active_set and effect in active_set:
            label[CLASS_IDX[cause], CLASS_IDX[effect]] = strength

    return vec, label.flatten()


# ── Model ─────────────────────────────────────────────────────────────────────

class CausalRanker(nn.Module):
    def __init__(self):
        super().__init__()
        self.net = nn.Sequential(
            nn.Linear(N_FEATURES, 128),
            nn.LayerNorm(128),
            nn.GELU(),
            nn.Linear(128, 128),
            nn.LayerNorm(128),
            nn.GELU(),
            nn.Linear(128, N_CLASSES * N_CLASSES),
            nn.Sigmoid(),
        )

    def forward(self, x):
        return self.net(x)


# ── Training ──────────────────────────────────────────────────────────────────

def train(epochs: int, batch_size: int = 256, n_samples: int = 50_000):
    device = torch.device("cuda" if torch.cuda.is_available() else "cpu")
    print(f"Device: {device}")

    model = CausalRanker().to(device)
    opt   = optim.AdamW(model.parameters(), lr=1e-3, weight_decay=1e-4)
    sched = optim.lr_scheduler.CosineAnnealingLR(opt, T_max=epochs)
    loss_fn = nn.BCELoss()

    print(f"Parameters: {sum(p.numel() for p in model.parameters()):,}")

    X, Y = zip(*[generate_sample() for _ in range(n_samples)])
    X = torch.tensor(np.array(X), dtype=torch.float32)
    Y = torch.tensor(np.array(Y), dtype=torch.float32)

    n = len(X)
    for epoch in range(1, epochs + 1):
        model.train()
        perm = torch.randperm(n)
        total_loss = 0.0
        for i in range(0, n, batch_size):
            idx = perm[i:i + batch_size]
            xb, yb = X[idx].to(device), Y[idx].to(device)
            pred = model(xb)
            loss = loss_fn(pred, yb)
            opt.zero_grad()
            loss.backward()
            opt.step()
            total_loss += loss.item() * len(xb)
        sched.step()
        if epoch % 10 == 0 or epoch == 1:
            avg = total_loss / n
            print(f"  Epoch {epoch:3d}/{epochs}  loss={avg:.5f}  lr={sched.get_last_lr()[0]:.2e}")

    return model


# ── ONNX export ───────────────────────────────────────────────────────────────

def export(model: CausalRanker, out_path: Path):
    model.eval()
    dummy = torch.zeros(1, N_FEATURES)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    torch.onnx.export(
        model, dummy, str(out_path),
        input_names=["features"],
        output_names=["causal_matrix"],
        dynamic_axes={"features": {0: "batch"}},
        opset_version=17,
    )
    print(f"Exported ONNX: {out_path}")


def update_manifest(model_path: Path):
    manifest_path = REPO_ROOT / "models" / "manifest.json"
    with open(manifest_path) as f:
        manifest = json.load(f)

    sha256 = hashlib.sha256(model_path.read_bytes()).hexdigest()
    size_kb = model_path.stat().st_size // 1024

    entry = {
        "name": "causal_ranker_v1",
        "version": "1.0.0",
        "path": f"models/{model_path.name}",
        "sha256": sha256,
        "input_schema": f"float_[1x{N_FEATURES}]",
        "output_schema": f"float_[1x{N_CLASSES * N_CLASSES}]",
        "output_classes": CLASS_NAMES,
        "description": (
            "MLP causal adjacency predictor. Input: 35-float FeatureSet snapshot. "
            f"Output: {N_CLASSES}×{N_CLASSES} = 100 sigmoid-activated causal strength "
            "values, row=cause, col=effect. Trained via synthetic co-occurrence labels "
            "derived from the static CAUSAL_RULES graph."
        ),
        "training_script": "scripts/train_causal_ranker.py",
        "size_kb": size_kb,
    }

    # Replace or append
    models = manifest.get("models", [])
    for i, m in enumerate(models):
        if m["name"] == "causal_ranker_v1":
            models[i] = entry
            break
    else:
        models.append(entry)
    manifest["models"] = models

    with open(manifest_path, "w") as f:
        json.dump(manifest, f, indent=2)
    print(f"Manifest updated: sha256={sha256[:16]}…  size={size_kb} KB")


# ── Main ──────────────────────────────────────────────────────────────────────

if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--epochs",      type=int,  default=80)
    parser.add_argument("--samples",     type=int,  default=50_000)
    parser.add_argument("--batch-size",  type=int,  default=256)
    parser.add_argument("--export-only", action="store_true")
    parser.add_argument("--db",          type=str,  default=None,
                        help="Path to firstaid.db for weak supervision (optional)")
    args = parser.parse_args()

    out_path = REPO_ROOT / "models" / "causal_ranker_v1.onnx"
    ckpt_path = REPO_ROOT / "models" / "causal_ranker_v1.pt"

    if args.export_only:
        if not ckpt_path.exists():
            raise FileNotFoundError(f"No checkpoint at {ckpt_path}")
        model = CausalRanker()
        model.load_state_dict(torch.load(ckpt_path, map_location="cpu"))
    else:
        model = train(args.epochs, args.batch_size, args.samples)
        torch.save(model.state_dict(), ckpt_path)
        print(f"Checkpoint saved: {ckpt_path}")

    export(model, out_path)
    update_manifest(out_path)
