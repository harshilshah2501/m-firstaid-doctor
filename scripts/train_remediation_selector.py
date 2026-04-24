#!/usr/bin/env python3
"""
train_remediation_selector.py — Train an ML model to rank remediation actions
by predicted effectiveness given the current system state.

Architecture
────────────
Input  : float[N_ACTIONS × INPUT_DIM]  where INPUT_DIM = 35 + 10 + 10
           • 35 = current FeatureSet snapshot
           • 10 = one-hot active IssueClass vector
           • 10 = one-hot action type bucket

Output : float[N_ACTIONS]  — predicted effectiveness score per action (sigmoid)

Model  : Per-action MLP with shared weights (like a scoring network):
         INPUT_DIM → 128 → 64 → 1  (applied independently to each action)

Training signal
───────────────
Synthetic: sample a feature vector with an injected issue condition, build
the deterministic planner's action list for that issue, then label each
action with a ground-truth effectiveness score derived from domain rules:
  - AutoSafe + issue-aligned + state-changing  → 0.90–1.00
  - AutoSafe + issue-aligned + report-only     → 0.65–0.80
  - ApprovalRequired + issue-aligned           → 0.50–0.75
  - ElevationRequired                          → 0.30–0.55
  - NeverAutoFix / off-class                   → 0.05–0.30

Loss: MSE between predicted and ground-truth effectiveness scores.
The model learns to generalise beyond the static heuristic by fitting to the
continuous feature space, so it can downrank an action that is usually good
but inappropriate given the current metric values.

Export: ONNX opset 17, updates models/manifest.json SHA-256.

Usage:
    python scripts/train_remediation_selector.py
    python scripts/train_remediation_selector.py --epochs 60 --samples 80000
    python scripts/train_remediation_selector.py --export-only
"""

import argparse
import hashlib
import json
import random
from pathlib import Path

import numpy as np
import torch
import torch.nn as nn
import torch.optim as optim

# ── Constants (must match Rust crates) ───────────────────────────────────────

N_FEATURES    = 35
N_CLASSES     = 10
N_ACTION_TYPES = 10
INPUT_DIM     = N_FEATURES + N_CLASSES + N_ACTION_TYPES   # 55

REPO_ROOT = Path(__file__).parent.parent

CLASS_NAMES = [
    "HighCpu", "HighMemory", "LowDisk", "DiskIo", "StartupSlow",
    "AppCrash", "NetworkDns", "ServiceDown", "UpdateIssue", "BatteryThermal",
]
CLASS_IDX = {n: i for i, n in enumerate(CLASS_NAMES)}

# Action type index — must match encode_action_type() in remediation_selector.rs
ACTION_TYPES = [
    "Report",           # 0
    "FlushDnsCache",    # 1
    "KillProcess",      # 2
    "ReprioritizeProcess",  # 3
    "ClearCache",       # 4
    "DisableStartupItem",   # 5
    "EnableStartupItem",    # 6
    "RestartService",   # 7
    "StopService",      # 8
    "RunConnectivityProbe", # 9
]
ACTION_IDX = {n: i for i, n in enumerate(ACTION_TYPES)}

# ── Ground-truth effectiveness table ─────────────────────────────────────────
# (issue_class, action_type) → (min_score, max_score)
# Scores outside this table default to (0.05, 0.30)

EFFECTIVENESS_TABLE = {
    ("HighCpu",        "Report"):              (0.65, 0.80),
    ("HighCpu",        "KillProcess"):         (0.75, 0.95),
    ("HighCpu",        "ReprioritizeProcess"): (0.55, 0.75),
    ("HighMemory",     "Report"):              (0.60, 0.75),
    ("HighMemory",     "KillProcess"):         (0.70, 0.90),
    ("LowDisk",        "ClearCache"):          (0.80, 0.95),
    ("LowDisk",        "Report"):              (0.60, 0.75),
    ("DiskIo",         "Report"):              (0.65, 0.80),
    ("StartupSlow",    "DisableStartupItem"):  (0.75, 0.92),
    ("StartupSlow",    "Report"):              (0.60, 0.75),
    ("AppCrash",       "Report"):              (0.55, 0.70),
    ("AppCrash",       "RestartService"):      (0.65, 0.85),
    ("NetworkDns",     "FlushDnsCache"):       (0.85, 0.97),
    ("NetworkDns",     "RunConnectivityProbe"):(0.80, 0.92),
    ("NetworkDns",     "Report"):              (0.55, 0.70),
    ("ServiceDown",    "RestartService"):      (0.70, 0.90),
    ("ServiceDown",    "StopService"):         (0.50, 0.70),
    ("ServiceDown",    "Report"):              (0.55, 0.70),
    ("UpdateIssue",    "Report"):              (0.60, 0.75),
    ("BatteryThermal", "Report"):              (0.65, 0.80),
    ("BatteryThermal", "KillProcess"):         (0.60, 0.80),
}


def ground_truth_score(issue_class: str, action_type: str) -> float:
    key = (issue_class, action_type)
    lo, hi = EFFECTIVENESS_TABLE.get(key, (0.05, 0.30))
    return random.uniform(lo, hi)


# ── Synthetic data generation ─────────────────────────────────────────────────

FEATURE_NORMAL_RANGES = [
    (0.05, 0.30), (0.05, 0.25), (0.05, 0.20), (4.0, 16.0), (0.01, 0.15),
    (0.0, 0.3),   (8000, 32000),(4000, 20000),(0.20, 0.60), (0.0, 500.0),
    (0.0, 0.20),  (0.0, 0.1),  (100, 4000),  (0.30, 0.80), (50e9, 500e9),
    (0.0, 0.05),  (0.8, 1.0),  (-70, -30),   (0.0, 0.5),   (0.0, 0.3),
    (0.0, 0.05),  (1.0, 15.0), (30.0, 65.0), (1000, 3000), (0.80, 1.0),
    (0, 200),     (0.30, 1.0), (3, 15),      (0, 2),       (0, 5),
    (0, 0.2),     (0, 0.2),   (0, 0.3),     (0, 0.3),     (0, 0.1),
]


def sample_feature_vec(issue: str) -> np.ndarray:
    vec = np.array([random.uniform(lo, hi) for lo, hi in FEATURE_NORMAL_RANGES], dtype=np.float32)
    # Stress the relevant metrics for the injected issue
    perturbations = {
        "HighCpu":        [(0, 0.80, 0.99), (1, 0.75, 0.95), (4, 0.60, 0.95)],
        "HighMemory":     [(8, 0.88, 0.99), (10, 0.60, 0.95), (11, 1.0, 1.0)],
        "LowDisk":        [(13, 0.0, 0.08)],
        "DiskIo":         [(15, 0.7, 1.0)],
        "StartupSlow":    [(27, 20, 80)],
        "AppCrash":       [(28, 3, 15)],
        "NetworkDns":     [(16, 0.0, 0.0), (20, 0.5, 1.0)],
        "ServiceDown":    [(30, 1.0, 1.0), (31, 0.0, 0.0)],
        "UpdateIssue":    [(32, 1.0, 1.0), (34, 1.0, 1.0)],
        "BatteryThermal": [(22, 80.0, 105.0), (23, 4000, 6500), (24, 0.50, 0.75)],
    }
    for idx, lo, hi in perturbations.get(issue, []):
        vec[idx] = random.uniform(lo, hi)
    return vec


def encode_class_vec(active: list) -> np.ndarray:
    v = np.zeros(N_CLASSES, dtype=np.float32)
    for cls in active:
        v[CLASS_IDX[cls]] = 1.0
    return v


def encode_action_type(action_type: str) -> np.ndarray:
    v = np.zeros(N_ACTION_TYPES, dtype=np.float32)
    v[ACTION_IDX[action_type]] = 1.0
    return v


# Actions relevant to each issue class
ISSUE_ACTIONS = {
    "HighCpu":        ["Report", "KillProcess", "ReprioritizeProcess"],
    "HighMemory":     ["Report", "KillProcess"],
    "LowDisk":        ["ClearCache", "Report"],
    "DiskIo":         ["Report"],
    "StartupSlow":    ["DisableStartupItem", "Report"],
    "AppCrash":       ["Report", "RestartService"],
    "NetworkDns":     ["FlushDnsCache", "RunConnectivityProbe", "Report"],
    "ServiceDown":    ["RestartService", "StopService", "Report"],
    "UpdateIssue":    ["Report"],
    "BatteryThermal": ["Report", "KillProcess"],
}

# Unrelated distractor action types that should score low
DISTRACTOR_ACTIONS = ["FlushDnsCache", "EnableStartupItem", "Report"]


def generate_sample():
    """
    Returns (X, Y) where:
      X : float[N_ACTIONS, INPUT_DIM]
      Y : float[N_ACTIONS]
    N_ACTIONS varies per sample; padded to fixed size by DataLoader collate.
    """
    issue = random.choice(CLASS_NAMES)
    feature_vec = sample_feature_vec(issue)
    class_vec   = encode_class_vec([issue])

    # Active actions for this issue + 1-2 distractors
    action_types = list(ISSUE_ACTIONS[issue])
    n_distractors = random.randint(1, 2)
    distractors = random.choices(
        [a for a in DISTRACTOR_ACTIONS if a not in action_types],
        k=n_distractors,
    )
    all_actions = action_types + distractors
    random.shuffle(all_actions)

    X_rows = []
    Y_vals = []
    for at in all_actions:
        action_vec = encode_action_type(at)
        row = np.concatenate([feature_vec, class_vec, action_vec])
        X_rows.append(row)
        # Distractors (not in issue's action list) get low scores
        if at in action_types:
            score = ground_truth_score(issue, at)
        else:
            score = random.uniform(0.05, 0.25)
        Y_vals.append(score)

    return np.array(X_rows, dtype=np.float32), np.array(Y_vals, dtype=np.float32)


# ── Model ─────────────────────────────────────────────────────────────────────

class RemediationScorer(nn.Module):
    """Per-action scoring MLP with shared weights applied across all actions."""
    def __init__(self, input_dim: int = INPUT_DIM):
        super().__init__()
        self.scorer = nn.Sequential(
            nn.Linear(input_dim, 128),
            nn.LayerNorm(128),
            nn.GELU(),
            nn.Dropout(0.1),
            nn.Linear(128, 64),
            nn.LayerNorm(64),
            nn.GELU(),
            nn.Linear(64, 1),
            nn.Sigmoid(),
        )

    def forward(self, x):
        # x: [batch, n_actions, input_dim]  OR  [n_actions, input_dim]
        if x.dim() == 2:
            return self.scorer(x).squeeze(-1)  # [n_actions]
        # Apply scorer independently across the action dimension
        B, N, D = x.shape
        flat = x.view(B * N, D)
        scores = self.scorer(flat).squeeze(-1)
        return scores.view(B, N)


# ── Training ──────────────────────────────────────────────────────────────────

def train(epochs: int, n_samples: int, batch_size: int):
    device = torch.device("cuda" if torch.cuda.is_available() else "cpu")
    print(f"Device: {device}")

    model  = RemediationScorer().to(device)
    opt    = optim.AdamW(model.parameters(), lr=1e-3, weight_decay=1e-4)
    sched  = optim.lr_scheduler.CosineAnnealingLR(opt, T_max=epochs)
    loss_fn = nn.MSELoss()

    print(f"Parameters: {sum(p.numel() for p in model.parameters()):,}")

    # Pre-generate flat (single-action) training pairs for efficient batching
    samples = [generate_sample() for _ in range(n_samples)]
    X_flat = np.vstack([x for x, _ in samples])   # [total_actions, INPUT_DIM]
    Y_flat = np.concatenate([y for _, y in samples])

    X_t = torch.tensor(X_flat, dtype=torch.float32)
    Y_t = torch.tensor(Y_flat, dtype=torch.float32)
    n   = len(X_t)

    for epoch in range(1, epochs + 1):
        model.train()
        perm       = torch.randperm(n)
        total_loss = 0.0
        for i in range(0, n, batch_size):
            idx = perm[i : i + batch_size]
            xb  = X_t[idx].to(device)
            yb  = Y_t[idx].to(device)
            pred = model(xb)
            loss = loss_fn(pred, yb)
            opt.zero_grad()
            loss.backward()
            opt.step()
            total_loss += loss.item() * len(xb)
        sched.step()

        if epoch % 10 == 0 or epoch == 1:
            avg = total_loss / n
            lr  = sched.get_last_lr()[0]
            print(f"  Epoch {epoch:3d}/{epochs}  mse={avg:.5f}  lr={lr:.2e}")

    return model


# ── ONNX export ───────────────────────────────────────────────────────────────

def export(model: RemediationScorer, out_path: Path):
    model.eval()
    # Export with a fixed n_actions=8 (max realistic action count per plan)
    # The Rust side passes exactly as many actions as the plan contains (≤8).
    dummy = torch.zeros(1, 8, INPUT_DIM)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    torch.onnx.export(
        model, dummy, str(out_path),
        input_names=["action_features"],
        output_names=["effectiveness_scores"],
        dynamic_axes={
            "action_features":      {0: "batch", 1: "n_actions"},
            "effectiveness_scores": {0: "batch", 1: "n_actions"},
        },
        opset_version=17,
    )
    print(f"Exported ONNX: {out_path}")


def update_manifest(model_path: Path):
    manifest_path = REPO_ROOT / "models" / "manifest.json"
    with open(manifest_path) as f:
        manifest = json.load(f)

    sha256  = hashlib.sha256(model_path.read_bytes()).hexdigest()
    size_kb = model_path.stat().st_size // 1024

    entry = {
        "name":    "remediation_selector_v1",
        "version": "1.0.0",
        "path":    f"models/{model_path.name}",
        "sha256":  sha256,
        "input_schema":  f"float_[1 x N_actions x {INPUT_DIM}]",
        "output_schema": "float_[1 x N_actions]",
        "description": (
            f"Per-action scoring MLP (shared weights). "
            f"Input per action: {N_FEATURES}-float FeatureSet || "
            f"{N_CLASSES}-float active-class one-hot || "
            f"{N_ACTION_TYPES}-float action-type one-hot. "
            "Output: predicted effectiveness score in [0,1] per action. "
            "Trained on synthetic issue/action pairs labeled by domain-rule "
            "effectiveness table. Falls back to heuristic scorer when absent."
        ),
        "training_script": "scripts/train_remediation_selector.py",
        "size_kb": size_kb,
    }

    models = manifest.get("models", [])
    for i, m in enumerate(models):
        if m["name"] == "remediation_selector_v1":
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
    parser.add_argument("--epochs",      type=int, default=60)
    parser.add_argument("--samples",     type=int, default=80_000)
    parser.add_argument("--batch-size",  type=int, default=512)
    parser.add_argument("--export-only", action="store_true")
    args = parser.parse_args()

    out_path  = REPO_ROOT / "models" / "remediation_selector_v1.onnx"
    ckpt_path = REPO_ROOT / "models" / "remediation_selector_v1.pt"

    if args.export_only:
        if not ckpt_path.exists():
            raise FileNotFoundError(f"No checkpoint at {ckpt_path}")
        model = RemediationScorer()
        model.load_state_dict(torch.load(ckpt_path, map_location="cpu"))
    else:
        model = train(args.epochs, args.samples, args.batch_size)
        torch.save(model.state_dict(), ckpt_path)
        print(f"Checkpoint saved: {ckpt_path}")

    export(model, out_path)
    update_manifest(out_path)
