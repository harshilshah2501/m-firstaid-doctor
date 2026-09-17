# FirstAid Doctor — desktop UI

Native shell ([Tauri 2](https://tauri.app/)) that runs the real `firstaid` binary and renders the JSON from `firstaid diagnose --ui-bundle`.

## Prerequisites

- **Rust** (same toolchain as the rest of the repo)
- **Node.js** 18+ and npm
- **Built `firstaid` binary** at `../target/debug/firstaid` (or `release`, or platform-specific `target/<triple>/release/`). Build from repo root:

  ```bash
  cargo build -p agent-core
  ```

  Or set **`FIRSTAID_EXE`** to the full path of `firstaid` if it lives elsewhere.

- **OS libraries** for Tauri: **WebView2** on Windows (standard on Windows 10+); on **Linux** install your distro’s WebKitGTK / libsoup stack (see [Tauri Linux prerequisites](https://v2.tauri.app/start/prerequisites/#linux)).

## Run (development)

From **this directory** (`desktop/`):

```bash
npm install
npm run tauri dev
```

Click **Run diagnosis** to collect live telemetry and show results.

## Production build

```bash
npm run tauri build
```

Bundling is currently **`bundle.active: false`** in `src-tauri/tauri.conf.json` (dev-focused). Turn it on and add icons when you want installers for the UI app itself.
