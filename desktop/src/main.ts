import { invoke } from "@tauri-apps/api/core";
import "./style.css";

interface UiBundle {
  ui_bundle_schema: string;
  generated_at: string;
  agent_version: string;
  source: string;
  overall_health: "healthy" | "degraded" | "critical";
  cycle: {
    snapshot_id: string;
    triggered: boolean;
    diagnoses_found: string[];
    plan_id: string | null;
    executed: boolean;
  };
  snapshot_summary: Record<string, unknown>;
  diagnosis: {
    diagnoses: Array<{
      issue_class: string;
      severity: string;
      confidence: string;
      title: string;
      explanation: string;
    }>;
  };
}

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className?: string,
  text?: string,
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function renderHealth(health: UiBundle["overall_health"]): HTMLElement {
  const span = el("span", `health health-${health}`, health);
  return span;
}

function renderSnapshot(summary: UiBundle["snapshot_summary"]): HTMLElement {
  const grid = el("div", "grid");
  const entries: [string, string][] = [
    ["Platform", String(summary.platform ?? "—")],
    ["CPU %", summary.cpu_usage_pct != null ? String(summary.cpu_usage_pct) : "—"],
    ["Memory used %", summary.memory_used_pct != null ? String(summary.memory_used_pct) : "—"],
    ["Disk free %", summary.system_disk_free_pct != null ? String(summary.system_disk_free_pct) : "—"],
    ["Network", summary.network_reachable ? "reachable" : "unreachable"],
    ["Crashes (24h)", summary.crashes_24h != null ? String(summary.crashes_24h) : "—"],
    ["Top process", String(summary.top_process_name ?? "—")],
  ];
  for (const [title, value] of entries) {
    const card = el("div", "card");
    card.appendChild(el("h3", "", title));
    card.appendChild(el("p", "", value));
    grid.appendChild(card);
  }
  return grid;
}

function renderDiagnoses(bundle: UiBundle): HTMLElement {
  const wrap = el("div", "diagnoses");
  wrap.appendChild(el("h2", "", "Diagnoses"));

  const list = bundle.diagnosis.diagnoses;
  if (list.length === 0) {
    wrap.appendChild(el("p", "muted", "No active diagnoses — endpoint looks healthy for this cycle."));
    return wrap;
  }

  for (const d of list) {
    const issue = el("div", "issue");
    issue.appendChild(el("h4", "", d.title));
    issue.appendChild(
      el(
        "div",
        "meta",
        `${d.issue_class} · ${d.severity} · ${d.confidence}`,
      ),
    );
    issue.appendChild(el("p", "", d.explanation));
    wrap.appendChild(issue);
  }
  return wrap;
}

function renderBundle(bundle: UiBundle, root: HTMLElement) {
  root.innerHTML = "";
  const meta = el(
    "p",
    "muted",
    `Schema ${bundle.ui_bundle_schema} · Agent ${bundle.agent_version} · ${bundle.source} · ${bundle.generated_at}`,
  );
  root.appendChild(meta);
  root.appendChild(renderSnapshot(bundle.snapshot_summary));
  root.appendChild(renderDiagnoses(bundle));
}

const app = document.querySelector<HTMLDivElement>("#app")!;
app.innerHTML = "";

const header = el("header");
const title = el("h1", "", "FirstAid Doctor");
const healthSlot = el("div");
const runBtn = el("button", "primary", "Run diagnosis") as HTMLButtonElement;
header.appendChild(title);
header.appendChild(healthSlot);
header.appendChild(runBtn);

const main = el("main");
const errBox = el("div", "error");
errBox.style.display = "none";
main.appendChild(errBox);
const content = el("div");
content.appendChild(
  el(
    "p",
    "muted",
    "Build the agent (`cargo build -p agent-core` from repo root), then click Run diagnosis.",
  ),
);
main.appendChild(content);

app.appendChild(header);
app.appendChild(main);

runBtn.addEventListener("click", async () => {
  errBox.style.display = "none";
  runBtn.disabled = true;
  healthSlot.innerHTML = "";
  content.replaceChildren();
  content.appendChild(el("p", "muted", "Running firstaid…"));

  try {
    const json = await invoke<string>("run_diagnosis");
    const bundle = JSON.parse(json) as UiBundle;
    healthSlot.appendChild(renderHealth(bundle.overall_health));
    renderBundle(bundle, content);
  } catch (e) {
    errBox.textContent = e instanceof Error ? e.message : String(e);
    errBox.style.display = "block";
    content.innerHTML = "";
  } finally {
    runBtn.disabled = false;
  }
});
