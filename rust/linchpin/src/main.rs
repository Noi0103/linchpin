use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use clap::Parser;
use libsystemd::daemon::{notify, watchdog_enabled, NotifyState};
use tracing::Level;
use tracing_subscriber::FmtSubscriber;

use linchpin::cli::Cli;
use linchpin::database::Database;
use linchpin::report_request_history::ReportRequestHistoryList;
use linchpin::report_request_list::ReportRequestList;

/// spawn two tokio tasks and continue to send keep-alive messages through sd_notify and systemd (watchdog)
#[cfg(target_has_atomic = "ptr")]
#[tokio::main]
async fn main() -> Result<()> {
    use linchpin::initialize_linchpin;

    let cli = Cli::parse();

    let subscriber = FmtSubscriber::builder()
        .with_max_level(match cli.verbose {
            0 => Level::INFO,
            1 => Level::DEBUG,
            _ => Level::TRACE,
        })
        .finish();

    tracing::subscriber::set_global_default(subscriber)
        .context("Failed to set global tracing subscriber")?;

    // Log messages from the log crate as well.
    tracing_log::LogTracer::init()?;

    // tracking object what reports are ongoing and waiting
    let shared_reports_list = Arc::new(Mutex::new(ReportRequestList::new()));
    let shared_reports_history = Arc::new(Mutex::new(ReportRequestHistoryList::new()));
    let database = Database::new(cli.db_file.clone());
    database.initialize().expect("failed initialization");

    initialize_linchpin(
        &cli,
        shared_reports_list.clone(),
        shared_reports_history.clone(),
        &database,
    )
    .expect("failed initialization");

    // two tokio tasks share the main workload between webserver and actual building and rebuilding

    // accept new ReportRequests
    let task_server = tokio::task::spawn(linchpin::server::server(
        cli.clone(),
        shared_reports_list.clone(),
    ));

    // rebuild and work through the ReportRequests
    let task_rebuilder = tokio::task::spawn(linchpin::rebuilder(
        cli.clone(),
        shared_reports_list.clone(),
        shared_reports_history.clone(),
        database.clone(),
    ));

    // bonus: systemd und sd_notify
    match watchdog_enabled(false) {
        Some(a) => {
            println!("watchdog timeout secs: {}", a.as_secs());
        }
        None => {
            println!("no watchdog support");
        }
    };
    match notify(false, &[NotifyState::Ready]) {
        Ok(_) => {}
        Err(_) => {
            println!("libsystemd notify ready failed");
        }
    };
    let mut health: bool = true;
    loop {
        std::thread::sleep(std::time::Duration::from_secs(30));
        if task_server.is_finished() {
            health = false;
        };
        if task_rebuilder.is_finished() {
            health = false;
        };
        if shared_reports_list.is_poisoned() {
            health = false;
        };
        if health {
            match notify(false, &[NotifyState::Watchdog]) {
                Ok(_) => {}
                Err(_) => {
                    println!("can not notify systemd watchdog");
                }
            };
        }
    }
}
