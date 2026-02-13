use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use clap::Parser;
use libsystemd::daemon::{notify, watchdog_enabled, NotifyState};
use log::debug;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

use linchpin::cli::Cli;
use linchpin::database::Database;
use linchpin::{report_request::ReportRequest, report_request_list::ReportRequestList};

/// spawn two tokio tasks and continue to send keep-alive messages through sd_notify and systemd (watchdog)
#[cfg(target_has_atomic = "ptr")]
#[tokio::main]
async fn main() -> Result<()> {
    // sort out args stuff

    use linchpin::{
        gitlab::PublisherMetadataGitlab,
        initialize_linchpin,
        nix_derivation::Derivation,
        report_request::{self, ClosureElement, Publisher},
    };
    use tokio::sync::mpsc::channel;

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

    info!("hello world");

    // TBD
    /*
        let report_request = ReportRequest {
            store_derivation: String::from(
                "/nix/store/dgs88rrngn5kncv2b3zapp200k3dc0fk-getclosure.drv",
            )
            .try_into()
            .expect("2"),
            store_derivation_closure: vec![
                ClosureElement::Derivation(
                    String::from("/nix/store/dgs88rrngn5kncv2b3zapp200k3dc0fk-getclosure.drv")
                        .try_into()
                        .expect("2"),
                ),
                ClosureElement::Other(String::from(
                    "/nix/store/001gp43bjqzx60cg345n2slzg7131za8-nix-nss-open-files.patch",
                )),
            ],
            publisher_data: Publisher::Gitlab(PublisherMetadataGitlab {
                ci_merge_request_project_id: "1".to_string(),
                ci_merge_request_iid: "1".to_string(),
                ci_commit_sha: "1".to_string(),
                ci_job_name: "1".to_string(),
                ci_pipeline_id: "1".to_string(),
            }),
        };

        let mut list: ReportRequestList = ReportRequestList::new();
        list.add_one_report(&report_request);

        info!("{}", serde_json::to_string_pretty(&list).unwrap());

        info!("{}", serde_json::to_string_pretty(&report_request).unwrap());

        let json = String::from(
            r#"
    {
      "store_derivation": {
        "file_path": "/nix/store/dgs88rrngn5kncv2b3zapp200k3dc0fk-getclosure.drv",
        "state": null,
        "error_reason": null,
        "db_write_count": null,
        "job_toplevel": null
      },
      "store_derivation_closure": [
        {
          "file_path": "/nix/store/dgs88rrngn5kncv2b3zapp200k3dc0fk-getclosure.drv",
          "state": null,
          "error_reason": null,
          "db_write_count": null,
          "job_toplevel": null
        },
        "/nix/store/001gp43bjqzx60cg345n2slzg7131za8-nix-nss-open-files.patch"
      ],
      "publisher_data": {
        "publisher": "Gitlab",
        "value": {
          "ci_merge_request_project_id": "1",
          "ci_merge_request_iid": "1",
          "ci_commit_sha": "1",
          "ci_job_name": "1",
          "ci_pipeline_id": "1"
        }
      }
    }
        "#,
        );
        debug!("testing parse a report_request next");
        let request: ReportRequest = serde_json::from_str(&json).unwrap();
        debug!("request parsed: {:#?}", request);
    */

    // tracking object what reports are ongoing and waiting
    let shared_reports_list = Arc::new(Mutex::new(ReportRequestList::new()));
    let shared_reports_history = Arc::new(Mutex::new(ReportRequestList::new()));
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
        database.clone(),
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
