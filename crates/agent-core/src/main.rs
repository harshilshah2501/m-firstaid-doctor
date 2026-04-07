//! m-firstaid-doctor — local endpoint health agent
//! Entry point for the `firstaid` binary.

mod config;
mod orchestrator;

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
    command: Command,

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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
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

    match cli.command {
        Command::Diagnose { json } => {
            cmd_diagnose(&config, &conn, json, cli.fixture).await?;
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
            println!("Daemon mode: Phase 2 — not yet implemented");
        }
    }

    Ok(())
}

async fn cmd_diagnose(
    config: &config::AgentConfig,
    conn: &rusqlite::Connection,
    json_output: bool,
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
        let trigger = trigger_engine::TriggerEngine::new().evaluate(&features);

        if let Some(trigger_event) = trigger {
            let diagnosis = detector::diagnose(&features, &trigger_event.issue_classes);
            if json_output {
                println!("{}", serde_json::to_string_pretty(&diagnosis)?);
            } else {
                print_diagnosis_human(&diagnosis);
            }
        } else {
            println!("✓ No issues detected in fixture snapshot.");
        }
        return Ok(());
    }

    // Real collection mode
    println!("Running endpoint diagnostic...\n");
    let result = orch.run_once(conn).await?;

    if result.diagnoses_found.is_empty() {
        println!("✓ No issues detected. Your endpoint appears healthy.");
    } else {
        println!("Issues detected: {:?}", result.diagnoses_found);
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

fn load_config(path: Option<&std::path::Path>) -> anyhow::Result<config::AgentConfig> {
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
