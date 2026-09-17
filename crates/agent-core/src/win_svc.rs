//! Entry point when started by the Windows Service Control Manager (`firstaid service`).
#![cfg(windows)]

use anyhow::Context;
use std::ffi::OsString;
use std::time::Duration;
use tokio::sync::watch;
use tracing_subscriber::{fmt, EnvFilter};
use windows_service::{
    define_windows_service,
    service::{
        ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
        ServiceType,
    },
    service_control_handler::{self, ServiceControlHandlerResult},
    service_dispatcher,
};

use crate::orchestrator::Orchestrator;

const SERVICE_NAME: &str = "firstaid";

define_windows_service!(ffi_service_main, service_main);

pub fn run() -> anyhow::Result<()> {
    service_dispatcher::start(SERVICE_NAME, ffi_service_main)
        .map_err(|e| anyhow::anyhow!(e))
}

fn service_main(_arguments: Vec<OsString>) {
    if let Err(e) = run_service_impl() {
        eprintln!("firstaid service failed: {e:#}");
    }
}

fn run_service_impl() -> anyhow::Result<()> {
    let _ = fmt()
        .with_env_filter(
            EnvFilter::from_default_env()
                .add_directive("firstaid=info".parse().expect("static filter"))
                .add_directive("agent_core=info".parse().expect("static filter")),
        )
        .with_target(false)
        .with_ansi(false)
        .try_init();

    let (stop_tx, stop_rx) = watch::channel(false);

    let status_handle = service_control_handler::register(SERVICE_NAME, move |control_event| {
        match control_event {
            ServiceControl::Stop => {
                let _ = stop_tx.send(true);
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    })?;

    status_handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Running,
        controls_accepted: ServiceControlAccept::STOP,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })?;

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("tokio runtime")?;

    let run_result: anyhow::Result<()> = rt.block_on(async move {
        let config = crate::load_config(None)?;
        if let Some(parent) = config.db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = storage::open(&config.db_path)?;
        let mut orch = Orchestrator::new(config);
        orch.run_daemon_with_shutdown(&conn, stop_rx).await
    });

    let exit_code = match &run_result {
        Ok(()) => ServiceExitCode::Win32(0),
        Err(_) => ServiceExitCode::Win32(1),
    };

    let _ = status_handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code,
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    });

    run_result
}
