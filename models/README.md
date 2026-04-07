# Models

ONNX model files for Phase 3 ML augmentation.

Models are NOT committed to the repo (large binary files). They are distributed separately via the installer or a versioned model download.

## Expected files (Phase 3)

| File | Description | Size | SHA-256 |
|---|---|---|---|
| `crash-classifier-v1.onnx` | CharCNN crash log classifier, INT8 quantized | ~300 KB | TBD |
| `process-anomaly-v1.onnx` | Isolation Forest process anomaly detector | ~500 KB | TBD |
| `startup-scorer-v1.onnx` | XGBoost startup item scorer | ~200 KB | TBD |

## Model lifecycle

1. Models are verified by SHA-256 checksum on every load (see `crates/inference/src/lib.rs`)
2. Models are lazy-loaded on trigger and dropped after inference
3. No model resides in memory at idle
4. Inference is always advisory — rules always run first

## Training

Training code and datasets live in a separate `ml-training/` repository (not included in this monorepo). See `docs/slm-strategy.md` for model architecture decisions.
