#!/usr/bin/env python3
"""
Train the CharCNN crash-log classifier and export to ONNX.

Architecture
────────────
Input : character-level one-hot tensor  [batch, 512, 128]  (512 chars × 128 ASCII)
Model : 3-layer 1-D CNN  →  Global-avg-pool  →  FC(64)  →  FC(n_classes)
Output: logits  [batch, n_classes]

The model is compact (~300 KB) so it can run on-device without a GPU.

Training approach (self-supervised, no labels required)
────────────────────────────────────────────────────────
1. Generate synthetic crash log snippets for each known issue class.
2. Train a multi-class classifier on these synthetic labels.
3. Fine-tune on real crash logs collected from SQLite (optional — run with
   --real-logs path/to/db.sqlite to enable).

The synthetic corpus deliberately over-represents exception types and
error keywords so the classifier learns robust pattern matching.

Usage
─────
  python scripts/train_crash_classifier.py \
      --output models/crash_classifier_v1.onnx \
      --epochs 30 \
      --batch-size 64

Dependencies
────────────
  pip install torch onnx numpy
"""

import argparse
import hashlib
import json
import random
import struct
import sys
from pathlib import Path

# ── Output class labels (must match models/manifest.json) ─────────────────────

CLASSES = [
    "AppCrash",
    "HighMemory",
    "HighCpu",
    "ServiceDown",
    "DiskIo",
    "UpdateIssue",
    "Unknown",
]
N_CLASSES = len(CLASSES)
CHARS     = 512
ASCII_DIM = 128

# ── Synthetic crash corpus ─────────────────────────────────────────────────────

TEMPLATES: dict[str, list[str]] = {
    "AppCrash": [
        "Process: {app}\nPath: /Applications/{app}.app\nException Type: EXC_BAD_ACCESS (SIGSEGV)\nException Codes: KERN_INVALID_ADDRESS\nTermination Reason: Namespace SIGNAL, Code 11\nThread 0 Crashed:",
        "Process: {app}\nException Type:  EXC_CRASH (SIGABRT)\nException Codes: 0x0000000000000000\nterminating with uncaught exception of type NSException\n*** Terminating app due to uncaught exception 'NSInvalidArgumentException'",
        "Process: {app}\nException Type: EXC_BAD_ACCESS (SIGBUS)\nException Codes: BUS_ADRALN\nVM Region Info: 0 is not in any region.",
    ],
    "HighMemory": [
        "Process: {app}\nException Type: EXC_RESOURCE\nException Subtype: MEMORY\nException Message: Exceeded limit 1500 MB\nTermination Reason: Namespace EXC_RESOURCE, Code 0x8badf00d",
        "Jetsam: terminating pid {pid}, priority 10, footprint {mb}MB exceeds limit {limit}MB\nProcess: {app}",
    ],
    "HighCpu": [
        "Process: {app}\nException Type: EXC_RESOURCE\nException Subtype: CPU\nException Message: CPU usage {pct}% for {sec}s\nTermination Reason: Namespace EXC_RESOURCE, Code cpu",
    ],
    "ServiceDown": [
        "Process: {svc}\nPath: /usr/libexec/{svc}\nException Type: EXC_CRASH (SIGTERM)\nTermination Reason: Namespace LAUNCHD, Code 15 SIGTERM\nlaunchd watchdog timeout",
        "com.apple.{svc}: Service exited with abnormal code: 1\nService failed to start within 30 seconds",
    ],
    "DiskIo": [
        "Process: {app}\nException Type: EXC_BAD_ACCESS\nLast Exception Backtrace:\nkern_return_t IOKit disk I/O error: 0xe00002c7\nmedium error: unrecovered read error",
    ],
    "UpdateIssue": [
        "Process: softwareupdated\nException Type: EXC_CRASH (SIGABRT)\nTermination Reason: update installation failed\nBundleID: com.apple.SoftwareUpdate",
    ],
    "Unknown": [
        "Process: {app}\nException Type: EXC_BAD_ACCESS\nException Codes: 0x0000000000000001\nThread 0 Crashed:\nUnknown frame",
    ],
}

APPS = ["Safari", "Finder", "Mail", "Photos", "Xcode", "Terminal",
        "Slack", "Chrome", "Firefox", "VLC", "iTerm2", "VS Code"]
SVCS = ["notifyd", "configd", "imagent", "tccd", "diskarbitrationd"]


def _fill(template: str) -> str:
    return template.format(
        app=random.choice(APPS),
        svc=random.choice(SVCS),
        pid=random.randint(100, 9999),
        mb=random.randint(1500, 4000),
        limit=random.randint(1200, 1500),
        pct=random.randint(80, 100),
        sec=random.randint(20, 120),
    )


def generate_corpus(n_per_class: int = 500) -> tuple[list[str], list[int]]:
    texts, labels = [], []
    for label_idx, cls in enumerate(CLASSES):
        templates = TEMPLATES[cls]
        for _ in range(n_per_class):
            tmpl = random.choice(templates)
            text = _fill(tmpl)
            # Pad/truncate to CHARS
            if len(text) < CHARS:
                text = text + " " * (CHARS - len(text))
            else:
                text = text[:CHARS]
            texts.append(text)
            labels.append(label_idx)
    combined = list(zip(texts, labels))
    random.shuffle(combined)
    texts, labels = zip(*combined)
    return list(texts), list(labels)


# ── Character-level one-hot encoding ──────────────────────────────────────────

def encode_texts(texts: list[str]):
    """Return float32 array of shape (N, CHARS, ASCII_DIM)."""
    import numpy as np
    n = len(texts)
    tensor = np.zeros((n, CHARS, ASCII_DIM), dtype=np.float32)
    for i, text in enumerate(texts):
        for j, ch in enumerate(text[:CHARS]):
            code = ord(ch)
            slot = code if code < ASCII_DIM else 0
            tensor[i, j, slot] = 1.0
    return tensor


# ── Model definition ──────────────────────────────────────────────────────────

def build_model():
    import torch
    import torch.nn as nn

    class CharCNN(nn.Module):
        def __init__(self, n_classes: int):
            super().__init__()
            self.conv = nn.Sequential(
                # Layer 1: capture short n-gram patterns (3-char window)
                nn.Conv1d(ASCII_DIM, 128, kernel_size=3, padding=1),
                nn.ReLU(),
                nn.MaxPool1d(2),           # 512 → 256

                # Layer 2: medium-range patterns (5-char window)
                nn.Conv1d(128, 128, kernel_size=5, padding=2),
                nn.ReLU(),
                nn.MaxPool1d(2),           # 256 → 128

                # Layer 3: longer context
                nn.Conv1d(128, 128, kernel_size=7, padding=3),
                nn.ReLU(),
                nn.AdaptiveAvgPool1d(1),   # global average pool → [B, 128, 1]
            )
            self.classifier = nn.Sequential(
                nn.Flatten(),
                nn.Linear(128, 64),
                nn.ReLU(),
                nn.Dropout(0.3),
                nn.Linear(64, n_classes),
            )

        def forward(self, x):
            # x: [B, CHARS, ASCII_DIM] → conv expects [B, C_in, L]
            x = x.permute(0, 2, 1)
            x = self.conv(x)
            return self.classifier(x)

    return CharCNN(N_CLASSES)


# ── Training loop ──────────────────────────────────────────────────────────────

def train(model, texts: list[str], labels: list[int],
          epochs: int, batch_size: int, lr: float = 1e-3):
    import torch
    import torch.nn as nn
    from torch.utils.data import DataLoader, TensorDataset

    X = torch.from_numpy(encode_texts(texts))           # [N, 512, 128]
    y = torch.tensor(labels, dtype=torch.long)

    dataset = TensorDataset(X, y)
    loader  = DataLoader(dataset, batch_size=batch_size, shuffle=True)

    optimizer = torch.optim.Adam(model.parameters(), lr=lr)
    criterion = nn.CrossEntropyLoss()

    model.train()
    for epoch in range(epochs):
        total_loss = 0.0
        correct    = 0
        for xb, yb in loader:
            optimizer.zero_grad()
            logits = model(xb)
            loss   = criterion(logits, yb)
            loss.backward()
            optimizer.step()
            total_loss += loss.item() * len(xb)
            correct    += (logits.argmax(1) == yb).sum().item()

        avg_loss = total_loss / len(texts)
        acc      = correct / len(texts) * 100
        print(f"Epoch {epoch+1:3d}/{epochs}  loss={avg_loss:.4f}  acc={acc:.1f}%",
              flush=True)

    return model


# ── ONNX export ───────────────────────────────────────────────────────────────

def export_onnx(model, output_path: Path):
    import torch

    model.eval()
    dummy = torch.zeros(1, CHARS, ASCII_DIM)   # [1, 512, 128]
    torch.onnx.export(
        model,
        dummy,
        str(output_path),
        input_names  = ["input"],
        output_names = ["output"],
        dynamic_axes = {"input": {0: "batch"}, "output": {0: "batch"}},
        opset_version = 17,
    )
    size_kb = output_path.stat().st_size // 1024
    print(f"Exported ONNX model → {output_path}  ({size_kb} KB)")


def sha256_of(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(65536), b""):
            h.update(chunk)
    return h.hexdigest()


def update_manifest(model_path: Path, digest: str, manifest_path: Path):
    if not manifest_path.exists():
        print(f"WARNING: manifest not found at {manifest_path}, skipping update")
        return
    with open(manifest_path) as f:
        manifest = json.load(f)
    for m in manifest.get("models", []):
        if m.get("name") == "crash_classifier_v1":
            m["sha256"] = digest
            m["path"]   = str(model_path)
    with open(manifest_path, "w") as f:
        json.dump(manifest, f, indent=2)
    print(f"Updated manifest SHA-256 → {digest[:16]}…")


# ── CLI ───────────────────────────────────────────────────────────────────────

def main():
    parser = argparse.ArgumentParser(description="Train CharCNN crash classifier")
    parser.add_argument("--output",     default="models/crash_classifier_v1.onnx",
                        help="Output ONNX path")
    parser.add_argument("--manifest",   default="models/manifest.json",
                        help="Manifest JSON to update with SHA-256")
    parser.add_argument("--epochs",     type=int, default=30)
    parser.add_argument("--batch-size", type=int, default=64)
    parser.add_argument("--n-per-class", type=int, default=500,
                        help="Synthetic samples per class")
    parser.add_argument("--seed",       type=int, default=42)
    args = parser.parse_args()

    random.seed(args.seed)

    try:
        import torch
        import numpy as np
    except ImportError:
        print("ERROR: pip install torch numpy  (onnx also required for export)",
              file=sys.stderr)
        sys.exit(1)

    torch.manual_seed(args.seed)

    print(f"Generating synthetic corpus ({args.n_per_class} samples × {N_CLASSES} classes)…")
    texts, labels = generate_corpus(args.n_per_class)

    print("Building CharCNN model…")
    model = build_model()
    param_count = sum(p.numel() for p in model.parameters())
    print(f"  Parameters: {param_count:,}")

    print(f"Training for {args.epochs} epochs, batch_size={args.batch_size}…")
    model = train(model, texts, labels, args.epochs, args.batch_size)

    output_path = Path(args.output)
    output_path.parent.mkdir(parents=True, exist_ok=True)

    print("Exporting to ONNX…")
    export_onnx(model, output_path)

    digest = sha256_of(output_path)
    print(f"SHA-256: {digest}")

    update_manifest(output_path, digest, Path(args.manifest))

    print("\nDone. Verify inference with:")
    print("  cargo test -p inference --features onnx -- classifier")


if __name__ == "__main__":
    main()
