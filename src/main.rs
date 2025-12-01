use std::sync::{Arc, Mutex};
use std::{env, fs, path};

use clap::Parser;
use libsystemd::daemon::{notify, watchdog_enabled, NotifyState};
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::gauge::Gauge;

use gitlab::Gitlab;
use reproducibility_automation::http_api::Metrics;
use reproducibility_automation::*;

/// spawn two tokio tasks and continue to send keep-alive messages through sd_notify and systemd (watchdog)
#[cfg(target_has_atomic = "ptr")]
#[tokio::main]
async fn main() {
    println!("hello world: async fn main");

    let gitlab_token: String = match env::var("CREDENTIALS_DIRECTORY") {
        Ok(value) => {
            let path = path::Path::new(&value);
            let path = path.join("gitlab_token");

            println!("The token path is: {path:?}");
            fs::read_to_string(path)
                .expect("secrets io error")
                .trim()
                .to_string()
        }
        Err(e) => {
            eprintln!("Couldn't read CREDENTIALS_DIRECTORY: {e}");
            return;
        }
    };

    // sort out args stuff
    let args = Args::parse();

    let database = database::Database {
        db_path: args.clone().db_file,
    };
    let gitlab = Gitlab {
        url: args.clone().gitlab_url,
        token: gitlab_token.clone(),
    };

    let shared_reports_list = Arc::new(Mutex::new(std::collections::VecDeque::new()));

    let shared_metrics = Arc::new(Mutex::new(Metrics {
        requests: Family::default(),
        current_len: Gauge::default(),
        comment_history_len: Gauge::default(),
        active_gc_roots: Gauge::default(),
        number_of_pipeline_ids: Gauge::default(),
        tokio_workers_count: Gauge::default(),
        tokio_total_park_count: Gauge::default(),
    }));

    // tokio metrics update loop
    let handle = tokio::runtime::Handle::current();
    let runtime_monitor: tokio_metrics::RuntimeMonitor =
        tokio_metrics::RuntimeMonitor::new(&handle);
    tokio::spawn(tokio_metrics(runtime_monitor, shared_metrics.clone()));

    // two tokio tasks share the main workload between webserver and actual building and rebuilding
    let task_server = tokio::task::spawn(reproducibility_automation::server(
        database.clone(),
        shared_reports_list.clone(),
        shared_metrics.clone(),
        args.clone(),
    ));
    let task_rebuilder = tokio::task::spawn(reproducibility_automation::rebuilder(
        database,
        gitlab,
        shared_reports_list.clone(),
        shared_metrics.clone(),
        args,
    ));

    // systemd und sd_notify
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
        match shared_reports_list::healthy_length(shared_reports_list.clone()) {
            Ok(_) => {}
            Err(_) => {
                health = false;
            }
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

/// keep tokio metrics values up to date
pub async fn tokio_metrics(
    runtime_monitor: tokio_metrics::RuntimeMonitor,
    metrics: Arc<Mutex<Metrics>>,
) {
    for interval in runtime_monitor.intervals() {
        {
            let metrics = metrics.as_ref().lock().expect("lock on metrics");
            metrics
                .tokio_workers_count
                .set(interval.workers_count.try_into().unwrap());
            metrics
                .tokio_total_park_count
                .set(interval.total_park_count.try_into().unwrap());
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}
