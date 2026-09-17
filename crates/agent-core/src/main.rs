//! m-firstaid-doctor — local endpoint health agent
//! Entry point for the `firstaid` binary.

mod config;
mod orchestrator;
mod ui_bundle;

#[cfg(windows)]
mod win_svc;

use clap::{Parser, Subcommand};
use tracing_subscriber::{EnvFilter, fmt};

#[derive(Parser)]
#[command(
    name = "firstaid",
    about = "Local-first endpoint health diagnostics and remediation",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Path to config file (TOML)
    #[arg(long, global = true)]
    config: Option<std::path::PathBuf>,

    /// Run in test mode with a fixture snapshot (skips OS collection)
    #[arg(long, global = true)]
    fixture: Option<std::path::PathBuf>,

    /// Path to expected diagnosis output for fixture validation
    #[arg(long, global = true)]
    expected_diagnosis: Option<std::path::PathBuf>,
}

#[derive(Subcommand)]
enum Command {
    /// Run a full diagnostic cycle and display results
    Diagnose {
        /// Output as JSON
        #[arg(long)]
        json: bool,
        /// Output versioned UI bundle JSON (for tray / Electron apps)
        #[arg(long)]
        ui_bundle: bool,
    },
    /// Show last known health state
    Status,
    /// Show recent issue history
    History {
        #[arg(long, default_value = "20")]
        limit: usize,
    },
    /// Show remediation plan for a diagnosis without executing
    Plan {
        #[arg(long)]
        diagnosis_id: Option<String>,
    },
    /// Execute remediation for a diagnosis with consent prompt
    Fix {
        #[arg(long)]
        diagnosis_id: String,
        /// Execute AutoSafe actions without prompting
        #[arg(long)]
        auto: bool,
    },
    /// Export a JSON diagnostic bundle
    Report {
        #[arg(long)]
        output: Option<std::path::PathBuf>,
    },
    /// Verify the audit log chain integrity
    VerifyAudit,
    /// Run as background daemon (Phase 2)
    Daemon,
}

fn main() -> anyhow::Result<()> {
    #[cfg(windows)]
    if std::env::args().nth(1).as_deref() == Some("service") {
        return win_svc::run();
    }

    main_async()
}

#[tokio::main]
async fn main_async() -> anyhow::Result<()> {
    // Initialize structured logging
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env()
            .add_directive("firstaid=info".parse()?)
            .add_directive("agent_core=info".parse()?))
        .with_target(false)
        .init();

    let cli = Cli::parse();

    let config = load_config(cli.config.as_deref())?;

    // Ensure data directory exists
    if let Some(parent) = config.db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let conn = storage::open(&config.db_path)?;

    let launched_without_subcommand = cli.command.is_none();
    let command = cli.command.unwrap_or(Command::Diagnose {
        json: false,
        ui_bundle: false,
    });

    if launched_without_subcommand {
        println!("No command supplied. Running a default diagnostic now.\n");
    }

    match command {
        Command::Diagnose { json, ui_bundle } => {
            cmd_diagnose(&config, &conn, json, ui_bundle, cli.fixture).await?;
        }
        Command::Status => {
            cmd_status(&conn).await?;
        }
        Command::History { limit } => {
            cmd_history(&conn, limit).await?;
        }
        Command::Plan { diagnosis_id } => {
            println!("Plan command: Phase 2 — not yet implemented");
        }
        Command::Fix { diagnosis_id, auto } => {
            println!("Fix command: Phase 2 — not yet implemented");
        }
        Command::Report { output } => {
            cmd_report(&conn, output).await?;
        }
        Command::VerifyAudit => {
            cmd_verify_audit(&conn)?;
        }
        Command::Daemon => {
            let mut orch = orchestrator::Orchestrator::new(config.clone());
            orch.run_daemon(&conn).await?;
        }
    }

    #[cfg(target_os = "windows")]
    {
        if launched_without_subcommand {
            pause_before_exit();
        }
    }

    Ok(())
}

#[cfg(target_os = "windows")]
fn pause_before_exit() {
    use std::io::{self, Write};

    print!("\nPress Enter to close this window...");
    let _ = io::stdout().flush();

    let mut input = String::new();
    let _ = io::stdin().read_line(&mut input);
}

async fn cmd_diagnose(
    config: &config::AgentConfig,
    conn: &rusqlite::Connection,
    json_output: bool,
    ui_bundle_output: bool,
    fixture: Option<std::path::PathBuf>,
) -> anyhow::Result<()> {
    let mut orch = orchestrator::Orchestrator::new(config.clone());

    if let Some(fixture_path) = fixture {
        // Test mode: load fixture instead of collecting from OS
        let fixture_json = std::fs::read_to_string(&fixture_path)?;
        let snapshot: signal_normalizer::NormalizedSignalSnapshot =
            serde_json::from_str(&fixture_json)?;

        println!("Running diagnostic on fixture: {}", fixture_path.display());

        let features = feature_extractor::extract(&snapshot, &[]);

        // Fixture mode: feed the same sample enough times to satisfy hysteresis
        // (samples_required=3) so the trigger engine can fire on a single snapshot.
        // No adaptive thresholds in fixture mode — use rule defaults only.
        let mut engine = trigger_engine::TriggerEngine::new();
        let trigger = (0..3).fold(None, |_, _| engine.evaluate(&features, None));

        let diagnosis = if let Some(trigger_event) = &trigger {
            detector::diagnose(&features, &trigger_event.issue_classes)
        } else {
            detector::diagnose(&features, &[])
        };

        if ui_bundle_output {
            let cycle = orchestrator::DiagnosticCycleResult {
                snapshot_id: diagnosis.snapshot_id,
                triggered: trigger.is_some(),
                diagnoses_found: diagnosis
                    .diagnoses
                    .iter()
                    .map(|d| format!("{:?}", d.issue_class))
                    .collect(),
                plan_id: None,
                executed: false,
            };
            let bundle = ui_bundle::UiBundle::from_fixture(snapshot, diagnosis, cycle);
            println!("{}", serde_json::to_string_pretty(&bundle)?);
            return Ok(());
        }

        if json_output {
            println!("{}", serde_json::to_string_pretty(&diagnosis)?);
        } else if diagnosis.diagnoses.is_empty() {
            println!("✓ No issues detected in fixture snapshot.");
        } else {
            print_diagnosis_human(&diagnosis);
        }
        return Ok(());
    }

    // Real collection mode
    if !ui_bundle_output {
        println!("Running endpoint diagnostic...\n");
    }
    let detail = orch.run_once(conn).await?;

    if ui_bundle_output {
        let bundle = ui_bundle::UiBundle::from_live_cycle(detail);
        println!("{}", serde_json::to_string_pretty(&bundle)?);
        return Ok(());
    }

    if detail.inner.diagnoses_found.is_empty() {
        println!("✓ No issues detected. Your endpoint appears healthy.");
    } else {
        println!("Issues detected: {:?}", detail.inner.diagnoses_found);
        println!("\nRun 'firstaid plan' to see recommended actions.");
    }

    Ok(())
}

async fn cmd_status(conn: &rusqlite::Connection) -> anyhow::Result<()> {
    // Phase 1: show last diagnosis from DB if available
    println!("Status: Phase 1 — run 'firstaid diagnose' to get current status.");
    Ok(())
}

async fn cmd_history(conn: &rusqlite::Connection, limit: usize) -> anyhow::Result<()> {
    println!("History (last {} diagnoses): Phase 1 — not yet implemented.", limit);
    Ok(())
}

async fn cmd_report(
    conn: &rusqlite::Connection,
    output: Option<std::path::PathBuf>,
) -> anyhow::Result<()> {
    println!("Report: Phase 1 — not yet implemented.");
    Ok(())
}

fn cmd_verify_audit(conn: &rusqlite::Connection) -> anyhow::Result<()> {
    match audit::verify_chain(conn) {
        Ok(count) => {
            println!("✓ Audit chain verified: {} events, no gaps or breaks.", count);
            Ok(())
        }
        Err(e) => {
            eprintln!("✗ Audit chain integrity failure: {}", e);
            std::process::exit(1);
        }
    }
}

pub(crate) fn load_config(path: Option<&std::path::Path>) -> anyhow::Result<config::AgentConfig> {
    if let Some(p) = path {
        let content = std::fs::read_to_string(p)?;
        Ok(toml::from_str(&content)?)
    } else {
        Ok(config::AgentConfig::default())
    }
}

fn print_diagnosis_human(diagnosis: &detector::types::DiagnosisOutput) {
    if diagnosis.diagnoses.is_empty() {
        println!("✓ No issues detected.");
        return;
    }

    println!("━━━ Diagnosis Results ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    for (i, d) in diagnosis.diagnoses.iter().enumerate() {
        println!("\n[{}/{}] {:?} | Confidence: {:?} | Severity: {:?}",
            i + 1, diagnosis.diagnoses.len(),
            d.issue_class, d.confidence, d.severity
        );
        println!("  {}", d.title);
        println!("  {}", d.explanation);
        println!("  Evidence:");
        for ev in &d.evidence {
            println!("    • {}: {:?}", ev.field, ev.value);
        }
    }
    println!("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("Run 'firstaid plan' to see recommended actions.\n");
}
