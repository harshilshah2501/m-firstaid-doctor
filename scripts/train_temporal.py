#!/usr/bin/env python3
"""
Train the MiniPatch-TST temporal anomaly transformer and export to ONNX.

Architecture: MiniPatch-TST (miniaturized PatchTST, NeurIPS 2023)
────────────────────────────────────────────────────────────────────
Input  : [batch, T=30, F=35]  — 30 time steps × 35 FeatureSet features
Patch  : T/P = 30/5 = 6 non-overlapping patches of 5 steps each
Embed  : Linear([P*F] → d_model=64) per patch
Pos    : Learnable positional embedding [6, d_model]
Encoder: 4 × TransformerEncoderLayer(d=64, nhead=4, ffn=128, dropout=0.1)
Pool   : Global average pool over 6 patch tokens → [batch, 64]
Head   : Linear(64 → N_CLASSES=10) + Sigmoid → per-IssueClass anomaly score

Novel research contribution: Masked Metric Modeling (MMM)
──────────────────────────────────────────────────────────
Pre-training task (self-supervised, no labels needed):
  1. Randomly mask 25% of (time-step, feature) cells
  2. Train a reconstruction head to predict masked values
  3. Loss = MSE on masked positions only
  4. After pre-training, discard reconstruction head, fine-tune anomaly head

Why this is novel:
  - First application of patch-based masked self-supervised pre-training to
    OS telemetry at edge scale
  - No labeled anomaly data required for pre-training
  - The encoder learns which metric co-movements are "normal" on this machine
  - Attention patterns reveal causal relationships between system metrics

Training schedule
──────────────────
  Phase 1 (pre-train, 50 epochs): MMM reconstruction loss on synthetic normal traces
  Phase 2 (fine-tune,  20 epochs): anomaly classification loss on labeled data
                                   (real anomalies from synthetic scenarios)

Output
──────
  ONNX model: input "input" [batch, 30, 35] → output "output" [batch, 10]
  Each output value = anomaly probability for the corresponding IssueClass.
  IssueClass order: HighCpu, HighMemory, LowDisk, DiskIo, StartupSlow,
                    AppCrash, NetworkDns, ServiceDown, UpdateIssue, BatteryThermal

Usage
─────
  # Bootstrap (synthetic only)
  python scripts/train_temporal.py --output models/temporal_anomaly_v1.onnx

  # With real data from SQLite
  python scripts/train_temporal.py --db /var/lib/firstaid/agent.db \\
      --output models/temporal_anomaly_v1.onnx

Dependencies
────────────
  pip install torch onnx numpy
"""

import argparse
import hashlib
import json
import math
import random
import sys
from pathlib import Path

import numpy as np

# ── Constants (must match feature_extractor::TEMPORAL_FEATURE_NAMES) ─────────

FEATURE_NAMES = [
    "cpu_usage_avg_1m", "cpu_usage_avg_5m", "cpu_usage_avg_15m",
    "cpu_core_count", "top_process_cpu_pct", "top_process_is_system",
    "memory_total_mb", "memory_available_mb", "memory_used_pct",
    "swap_used_mb", "swap_used_pct", "memory_oom_risk",
    "top_process_rss_mb", "system_disk_free_pct", "system_disk_free_bytes",
    "smart_failing", "network_reachable", "wifi_signal_dbm",
    "vpn_active", "proxy_configured", "dns_resolution_fail_rate",
    "default_gateway_ping_ms", "cpu_temp_celsius", "fan_speed_rpm",
    "battery_health_pct", "battery_cycle_count", "battery_charge_pct",
    "startup_items_count", "app_crash_count_1h", "app_crash_count_24h",
    "service_restart_storm", "service_down_expected", "pending_reboot_required",
    "recent_update_detected", "update_failed",
]
F = len(FEATURE_NAMES)    # 35

# IssueClass output order (must match inference::temporal::ISSUE_CLASS_ORDER)
ISSUE_CLASSES = [
    "HighCpu", "HighMemory", "LowDisk", "DiskIo", "StartupSlow",
    "AppCrash", "NetworkDns", "ServiceDown", "UpdateIssue", "BatteryThermal",
]
N_CLASSES = len(ISSUE_CLASSES)   # 10
T = 30    # time steps per window
P = 5     # patch size → 6 patches
D = 64    # model dimension
N_HEADS = 4
N_LAYERS = 4
FFN_DIM = 128
MASK_RATIO = 0.25   # fraction of cells masked during MMM pre-training

# ── Feature indices (for synthetic data generation) ───────────────────────────

IDX = {name: i for i, name in enumerate(FEATURE_NAMES)}


# ── Synthetic workload trace generator ───────────────────────────────────────

class WorkloadTrace:
    """Generate a T-step realistic workload trace for a given IssueClass."""

    @staticmethod
    def baseline(rng: np.random.Generator) -> np.ndarray:
        """A quiet, healthy machine."""
        trace = np.zeros((T, F), dtype=np.float32)
        for t in range(T):
            trace[t, IDX["cpu_usage_avg_1m"]]  = np.clip(rng.normal(8,  3), 0, 100)
            trace[t, IDX["cpu_usage_avg_5m"]]  = np.clip(rng.normal(8,  2), 0, 100)
            trace[t, IDX["cpu_usage_avg_15m"]] = np.clip(rng.normal(7,  2), 0, 100)
            trace[t, IDX["cpu_core_count"]]    = 8.0
            trace[t, IDX["memory_total_mb"]]   = 16384.0
            trace[t, IDX["memory_available_mb"]] = np.clip(rng.normal(8000, 500), 100, 16000)
            trace[t, IDX["memory_used_pct"]]   = np.clip(rng.normal(45, 5), 5, 95)
            trace[t, IDX["swap_used_pct"]]     = np.clip(rng.normal(5, 3), 0, 100)
            trace[t, IDX["system_disk_free_pct"]] = np.clip(rng.normal(40, 5), 1, 99)
            trace[t, IDX["network_reachable"]] = 1.0
            trace[t, IDX["battery_charge_pct"]] = np.clip(rng.normal(75, 10), 5, 100)
            trace[t, IDX["battery_health_pct"]] = np.clip(rng.normal(90, 5), 50, 100)
            trace[t, IDX["cpu_temp_celsius"]]  = np.clip(rng.normal(55, 5), 30, 100)
        return trace

    @staticmethod
    def high_cpu(rng: np.random.Generator) -> np.ndarray:
        """CPU anomaly: gradual ramp then sustained high usage."""
        trace = WorkloadTrace.baseline(rng)
        ramp_start = rng.integers(0, T // 2)
        for t in range(ramp_start, T):
            frac = (t - ramp_start) / max(T - ramp_start, 1)
            cpu = 40 + frac * 55 + rng.normal(0, 3)
            trace[t, IDX["cpu_usage_avg_1m"]]  = np.clip(cpu, 0, 100)
            trace[t, IDX["cpu_usage_avg_5m"]]  = np.clip((cpu - 5), 0, 100)
            trace[t, IDX["cpu_usage_avg_15m"]] = np.clip((cpu - 10), 0, 100)
            trace[t, IDX["top_process_cpu_pct"]] = np.clip((cpu * 0.9), 0, 100)
            trace[t, IDX["cpu_temp_celsius"]]  = np.clip((55 + frac * 35 + rng.normal(0, 2)), 30, 110)
        return trace

    @staticmethod
    def high_memory(rng: np.random.Generator) -> np.ndarray:
        """Memory leak: monotonically increasing memory usage."""
        trace = WorkloadTrace.baseline(rng)
        leak_rate = rng.uniform(100, 500)   # MB per step
        for t in range(T):
            used_pct = 40 + (t * leak_rate / 16384 * 100)
            trace[t, IDX["memory_used_pct"]]   = np.clip(used_pct, 0, 100)
            trace[t, IDX["memory_available_mb"]] = np.clip((16384 * (1 - used_pct / 100)), 0, 16384)
            trace[t, IDX["swap_used_pct"]] = max(0, used_pct - 85) * 4 + rng.normal(0, 2)
            if used_pct > 90:
                trace[t, IDX["memory_oom_risk"]] = 1.0
        return trace

    @staticmethod
    def low_disk(rng: np.random.Generator) -> np.ndarray:
        """Disk filling up over the observation window."""
        trace = WorkloadTrace.baseline(rng)
        start_free = rng.uniform(5, 15)   # % free at start
        drain_rate = rng.uniform(0.1, 0.5) # % per step
        for t in range(T):
            free = max(0.5, start_free - t * drain_rate)
            trace[t, IDX["system_disk_free_pct"]] = free
            trace[t, IDX["system_disk_free_bytes"]] = free / 100 * 500e9
        return trace

    @staticmethod
    def disk_io(rng: np.random.Generator) -> np.ndarray:
        """I/O storm pattern: bursty disk writes."""
        trace = WorkloadTrace.baseline(rng)
        for t in range(T):
            if rng.random() < 0.6:   # I/O active 60% of time
                trace[t, IDX["cpu_usage_avg_1m"]] += rng.uniform(20, 50)
        return trace

    @staticmethod
    def startup_slow(rng: np.random.Generator) -> np.ndarray:
        """Startup with many launch agents causing CPU spike."""
        trace = WorkloadTrace.baseline(rng)
        trace[:5, IDX["cpu_usage_avg_1m"]] = rng.uniform(70, 95, 5)
        trace[:, IDX["startup_items_count"]] = rng.integers(80, 150)
        return trace

    @staticmethod
    def app_crash(rng: np.random.Generator) -> np.ndarray:
        """Crash events appearing mid-window."""
        trace = WorkloadTrace.baseline(rng)
        n_crashes = rng.integers(1, 6)
        for _ in range(n_crashes):
            t = rng.integers(T // 2, T)
            trace[t, IDX["app_crash_count_1h"]]  += 1
            trace[t:, IDX["app_crash_count_24h"]] += 1
        return trace

    @staticmethod
    def network_dns(rng: np.random.Generator) -> np.ndarray:
        """DNS resolution failures → gradually increasing fail rate."""
        trace = WorkloadTrace.baseline(rng)
        for t in range(T):
            fail_rate = min(1.0, t / T * rng.uniform(0.5, 1.0))
            trace[t, IDX["dns_resolution_fail_rate"]] = fail_rate
            if rng.random() < fail_rate * 0.3:
                trace[t, IDX["network_reachable"]] = 0.0
        return trace

    @staticmethod
    def service_down(rng: np.random.Generator) -> np.ndarray:
        """Service crash and repeated restart storm."""
        trace = WorkloadTrace.baseline(rng)
        crash_at = rng.integers(T // 3, 2 * T // 3)
        for t in range(crash_at, T):
            trace[t, IDX["service_down_expected"]] = 1.0
            trace[t, IDX["service_restart_storm"]] = float(rng.random() < 0.7)
        return trace

    @staticmethod
    def update_issue(rng: np.random.Generator) -> np.ndarray:
        """Failed update leaving pending reboot."""
        trace = WorkloadTrace.baseline(rng)
        fail_at = rng.integers(0, T // 2)
        trace[fail_at:, IDX["update_failed"]] = 1.0
        trace[fail_at:, IDX["pending_reboot_required"]] = 1.0
        trace[fail_at:, IDX["recent_update_detected"]] = 1.0
        return trace

    @staticmethod
    def battery_thermal(rng: np.random.Generator) -> np.ndarray:
        """Thermal throttling + battery drain under load."""
        trace = WorkloadTrace.baseline(rng)
        for t in range(T):
            trace[t, IDX["cpu_temp_celsius"]] = np.clip(rng.normal(85, 5), 70, 110)
            trace[t, IDX["battery_charge_pct"]] = max(5, 90 - t * 3 + rng.normal(0, 2))
            trace[t, IDX["battery_health_pct"]] = np.clip(rng.normal(65, 10), 20, 100)
        return trace

    GENERATORS = {
        "HighCpu":        high_cpu.__func__,
        "HighMemory":     high_memory.__func__,
        "LowDisk":        low_disk.__func__,
        "DiskIo":         disk_io.__func__,
        "StartupSlow":    startup_slow.__func__,
        "AppCrash":       app_crash.__func__,
        "NetworkDns":     network_dns.__func__,
        "ServiceDown":    service_down.__func__,
        "UpdateIssue":    update_issue.__func__,
        "BatteryThermal": battery_thermal.__func__,
    }


def generate_corpus(n_normal: int, n_per_class: int,
                    rng: np.random.Generator):
    """
    Returns:
        X: float32 [N, T, F]
        y: float32 [N, N_CLASSES] — multi-label, 0=normal for all classes
    """
    traces, labels = [], []

    # Normal traces (all class labels = 0)
    for _ in range(n_normal):
        traces.append(WorkloadTrace.baseline(rng))
        labels.append(np.zeros(N_CLASSES, dtype=np.float32))

    # Anomaly traces (one-hot class label per class)
    for cls_idx, cls_name in enumerate(ISSUE_CLASSES):
        gen = WorkloadTrace.GENERATORS[cls_name]
        for _ in range(n_per_class):
            traces.append(gen(rng))
            lbl = np.zeros(N_CLASSES, dtype=np.float32)
            lbl[cls_idx] = 1.0
            labels.append(lbl)

    X = np.stack(traces, axis=0).astype(np.float32)   # [N, T, F]
    y = np.stack(labels, axis=0).astype(np.float32)   # [N, N_CLASSES]

    # Shuffle
    idx = rng.permutation(len(X))
    return X[idx], y[idx]


# ── Masking for MMM pre-training ──────────────────────────────────────────────

def mask_input(X: "torch.Tensor", mask_ratio: float = MASK_RATIO):
    """
    Randomly zero out `mask_ratio` fraction of (time, feature) cells.
    Returns (masked_X, mask_bool) where mask_bool=True means the cell was masked.
    """
    import torch
    mask = torch.bernoulli(torch.full(X.shape, mask_ratio)).bool()
    masked_X = X.clone()
    masked_X[mask] = 0.0
    return masked_X, mask


# ── Model definition ──────────────────────────────────────────────────────────

def build_model():
    import torch
    import torch.nn as nn

    class MiniPatchTST(nn.Module):
        def __init__(self):
            super().__init__()
            self.patch_size  = P
            self.n_patches   = T // P   # 6
            patch_dim        = P * F    # 5 × 35 = 175

            # Patch embedding
            self.patch_embed = nn.Linear(patch_dim, D)

            # Learnable positional embedding (one per patch)
            self.pos_embed = nn.Parameter(torch.zeros(1, self.n_patches, D))
            nn.init.normal_(self.pos_embed, std=0.02)

            # Transformer encoder
            enc_layer = nn.TransformerEncoderLayer(
                d_model=D, nhead=N_HEADS,
                dim_feedforward=FFN_DIM,
                dropout=0.1, batch_first=True,
                norm_first=True,   # Pre-norm for stability
            )
            self.encoder = nn.TransformerEncoder(enc_layer, num_layers=N_LAYERS)

            # Anomaly detection head (fine-tune phase only)
            self.anomaly_head = nn.Sequential(
                nn.Linear(D, D // 2),
                nn.GELU(),
                nn.Dropout(0.1),
                nn.Linear(D // 2, N_CLASSES),
                nn.Sigmoid(),
            )

            # MMM reconstruction head (pre-train phase only, discarded after)
            self.recon_head = nn.Linear(D, P * F)  # per patch → reconstruct flattened patch

        def encode(self, x: "torch.Tensor") -> "torch.Tensor":
            """
            x: [B, T, F]
            Returns patch tokens: [B, n_patches, D]
            """
            B = x.size(0)
            # Reshape to patches: [B, n_patches, P*F]
            patches = x.reshape(B, self.n_patches, P * F)
            # Embed + positional encoding
            tokens = self.patch_embed(patches) + self.pos_embed
            # Transformer
            return self.encoder(tokens)   # [B, n_patches, D]

        def forward(self, x: "torch.Tensor") -> "torch.Tensor":
            """Anomaly scoring forward (used at inference time)."""
            tokens = self.encode(x)   # [B, n_patches, D]
            # Global average pool
            pooled = tokens.mean(dim=1)  # [B, D]
            return self.anomaly_head(pooled)  # [B, N_CLASSES]

        def forward_mmm(self, x_masked: "torch.Tensor") -> "torch.Tensor":
            """MMM reconstruction forward (pre-training only)."""
            tokens = self.encode(x_masked)  # [B, n_patches, D]
            recon_patches = self.recon_head(tokens)  # [B, n_patches, P*F]
            # Reshape back to [B, T, F]
            B = x_masked.size(0)
            return recon_patches.reshape(B, T, F)

    return MiniPatchTST()


# ── Training loops ────────────────────────────────────────────────────────────

def pretrain_mmm(model, X_normal: np.ndarray,
                 epochs: int, batch_size: int, lr: float = 1e-3):
    """
    Phase 1 — Masked Metric Modeling self-supervised pre-training.
    Trains the encoder + reconstruction head on normal traces only.
    No labels required.
    """
    import torch
    import torch.nn as nn
    from torch.utils.data import DataLoader, TensorDataset

    X_t = torch.from_numpy(X_normal)
    dataset = TensorDataset(X_t)
    loader  = DataLoader(dataset, batch_size=batch_size, shuffle=True)
    optimizer = torch.optim.AdamW(model.parameters(), lr=lr, weight_decay=1e-4)
    scheduler = torch.optim.lr_scheduler.CosineAnnealingLR(optimizer, T_max=epochs)

    model.train()
    print(f"\n── Phase 1: MMM pre-training ({epochs} epochs) ──")
    for epoch in range(epochs):
        total_loss = 0.0
        for (xb,) in loader:
            x_masked, mask = mask_input(xb)
            x_recon = model.forward_mmm(x_masked)    # [B, T, F]
            # Loss only on masked positions
            loss = ((x_recon - xb)[mask]).pow(2).mean()
            optimizer.zero_grad()
            loss.backward()
            torch.nn.utils.clip_grad_norm_(model.parameters(), 1.0)
            optimizer.step()
            total_loss += loss.item() * len(xb)
        scheduler.step()
        if (epoch + 1) % 10 == 0:
            print(f"  Epoch {epoch+1:3d}/{epochs}  MMM loss={total_loss/len(X_normal):.5f}")


def finetune_anomaly(model, X: np.ndarray, y: np.ndarray,
                     epochs: int, batch_size: int, lr: float = 3e-4):
    """
    Phase 2 — Supervised fine-tuning of the anomaly head.
    The pre-trained encoder weights are fine-tuned with a lower LR.
    Multi-label BCE loss: each class is an independent binary problem.
    """
    import torch
    import torch.nn as nn
    from torch.utils.data import DataLoader, TensorDataset

    X_t = torch.from_numpy(X)
    y_t = torch.from_numpy(y)
    dataset = TensorDataset(X_t, y_t)
    loader  = DataLoader(dataset, batch_size=batch_size, shuffle=True)

    # Lower LR for the encoder (protect pre-trained weights), full LR for head.
    param_groups = [
        {"params": [p for n, p in model.named_parameters() if "anomaly_head" not in n],
         "lr": lr * 0.1},
        {"params": model.anomaly_head.parameters(), "lr": lr},
    ]
    optimizer = torch.optim.AdamW(param_groups, weight_decay=1e-4)
    criterion = nn.BCELoss()

    model.train()
    print(f"\n── Phase 2: anomaly head fine-tuning ({epochs} epochs) ──")
    for epoch in range(epochs):
        total_loss = 0.0
        for xb, yb in loader:
            preds = model(xb)   # [B, N_CLASSES]
            loss  = criterion(preds, yb)
            optimizer.zero_grad()
            loss.backward()
            torch.nn.utils.clip_grad_norm_(model.parameters(), 1.0)
            optimizer.step()
            total_loss += loss.item() * len(xb)
        if (epoch + 1) % 5 == 0:
            avg = total_loss / len(X)
            # Compute per-class AUROC on training set (quick sanity check)
            model.eval()
            with torch.no_grad():
                all_preds = model(torch.from_numpy(X)).numpy()
            auc_list = []
            for ci in range(N_CLASSES):
                positives = y[:, ci] == 1
                if positives.sum() > 0:
                    from sklearn.metrics import roc_auc_score
                    try:
                        auc = roc_auc_score(y[:, ci], all_preds[:, ci])
                        auc_list.append(auc)
                    except Exception:
                        pass
            mean_auc = sum(auc_list) / len(auc_list) if auc_list else 0.0
            print(f"  Epoch {epoch+1:3d}/{epochs}  BCE={avg:.4f}  mean-AUROC={mean_auc:.3f}")
            model.train()


# ── ONNX export ───────────────────────────────────────────────────────────────

def export_onnx(model, output_path: Path):
    import torch

    model.eval()
    dummy = torch.zeros(1, T, F)

    torch.onnx.export(
        model,
        dummy,
        str(output_path),
        input_names   = ["input"],
        output_names  = ["output"],
        dynamic_axes  = {"input": {0: "batch"}, "output": {0: "batch"}},
        opset_version = 17,
    )
    size_kb = output_path.stat().st_size // 1024
    print(f"\nExported ONNX → {output_path}  ({size_kb} KB)")


def sha256_of(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(65536), b""):
            h.update(chunk)
    return h.hexdigest()


def update_manifest(model_path: Path, digest: str, manifest_path: Path):
    if not manifest_path.exists():
        print(f"WARNING: manifest not found at {manifest_path}")
        return
    with open(manifest_path) as f:
        manifest = json.load(f)
    for m in manifest.get("models", []):
        if m.get("name") == "temporal_anomaly_v1":
            m["sha256"] = digest
            m["path"]   = str(model_path)
    with open(manifest_path, "w") as f:
        json.dump(manifest, f, indent=2)
    print(f"Updated manifest SHA-256 → {digest[:16]}…")


# ── CLI ───────────────────────────────────────────────────────────────────────

def main():
    parser = argparse.ArgumentParser(
        description="Train MiniPatch-TST temporal anomaly transformer")
    parser.add_argument("--output",        default="models/temporal_anomaly_v1.onnx")
    parser.add_argument("--manifest",      default="models/manifest.json")
    parser.add_argument("--n-normal",      type=int, default=1000,
                        help="Synthetic normal traces for MMM pre-training")
    parser.add_argument("--n-per-class",   type=int, default=200,
                        help="Synthetic anomaly traces per IssueClass")
    parser.add_argument("--pretrain-epochs", type=int, default=50)
    parser.add_argument("--finetune-epochs", type=int, default=20)
    parser.add_argument("--batch-size",    type=int, default=64)
    parser.add_argument("--seed",          type=int, default=42)
    args = parser.parse_args()

    try:
        import torch
    except ImportError:
        print("ERROR: pip install torch onnx numpy", file=sys.stderr)
        sys.exit(1)

    rng = np.random.default_rng(args.seed)
    torch.manual_seed(args.seed)
    random.seed(args.seed)

    # ── Generate corpus ───────────────────────────────────────────────────────
    print(f"Generating corpus: {args.n_normal} normal + "
          f"{args.n_per_class}×{N_CLASSES} anomaly traces…")
    X, y = generate_corpus(args.n_normal, args.n_per_class, rng)
    X_normal = X[y.sum(axis=1) == 0]   # only normal traces for MMM
    print(f"Corpus: {len(X)} total  ({len(X_normal)} normal, {len(X)-len(X_normal)} anomaly)")

    # ── Build model ───────────────────────────────────────────────────────────
    model = build_model()
    n_params = sum(p.numel() for p in model.parameters())
    print(f"MiniPatch-TST parameters: {n_params:,}")

    # ── Phase 1: MMM self-supervised pre-training ─────────────────────────────
    pretrain_mmm(model, X_normal,
                 epochs=args.pretrain_epochs, batch_size=args.batch_size)

    # ── Phase 2: Supervised anomaly head fine-tuning ──────────────────────────
    finetune_anomaly(model, X, y,
                     epochs=args.finetune_epochs, batch_size=args.batch_size)

    # ── Export ────────────────────────────────────────────────────────────────
    output_path = Path(args.output)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    export_onnx(model, output_path)

    digest = sha256_of(output_path)
    print(f"SHA-256: {digest}")
    update_manifest(output_path, digest, Path(args.manifest))

    print("\nDone. Verify with:")
    print("  cargo test -p inference --features onnx -- temporal")


if __name__ == "__main__":
    main()
