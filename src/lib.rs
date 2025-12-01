use std::collections::VecDeque;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use axum::{
    routing::{get, post},
    Router,
};
use clap::Parser;
use prometheus_client::registry::Registry;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

/// utilities to interact and work with nix store derivations
pub mod nix_derivation;
use crate::nix_derivation::active_gc_roots;
use crate::nix_derivation::{Derivation, DerivationState, JobToplevel};

/// functions to interact with the shared state
pub mod shared_reports_list;
use shared_reports_list::*;

/// utilities to interact with the sqlite database to read and write information about nix derivations
pub mod database;
use database::Database;

/// handlers for REST api endpoints
pub mod http_api;
use http_api::AppState;
use http_api::Metrics;
use http_api::ReportBody;

/// functions to make message body and interact with gitlab api
pub mod gitlab;
use gitlab::Gitlab;

/// create, merge and fmt markdown for human readable report content
mod report_message;
use crate::report_message::{HistoryEntry, ReportMessage};

/// A service to rebuild every element of a Nix build closures sent to it and report the results as a GitLab merge request comment.
#[derive(Parser, Debug, Clone)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// sqlite filepath to track tested store derivations; e.g. "/your/path/server.db"
    #[arg(short, long)]
    pub db_file: PathBuf,
    /// socket address the tracking server is listening on; e.g. 127.0.0.1:8080
    #[arg(short, long)]
    pub socket_address: SocketAddr,
    /// Gitlab domain to send merge request comments via api; e.g. "https://mygit.domain.com"
    #[arg(short, long)]
    pub gitlab_url: String,
    /// used with `nix-build [paths] ... --store <...>`
    #[arg(short, long, default_value_t = String::from("ssh-ng://localhost"))]
    pub nix_store: String,
    /// used to run multiple nix-build commands at once
    /// depending on the machine you can balance I/O wait times and out of memory
    #[arg(long, default_value_t = 1)]
    pub simultaneous_builds: usize,
    /// the location where symlinks will be placed to protect needed derivation files from automatic garbage collection
    #[arg(long, default_value = PathBuf::from("/var/lib/linchpin/gc-roots").into_os_string())]
    pub gc_links_path: PathBuf,
    /// load and continue reports that were not finished after restarting the program
    #[arg(long, default_value_t = false)]
    pub persistent_reports: bool,
    /// filepath for saving unfinished reports
    #[arg(long, default_value = PathBuf::from("/var/lib/linchpin/savefile.json").into_os_string())]
    pub savefile_path: PathBuf,
    /// filepath for saving unfinished reports
    #[arg(long, default_value = PathBuf::from("/var/lib/linchpin/comment-history.json").into_os_string())]
    pub savefile_history_path: PathBuf,
    /// how often given the chance a rebuild should be done until it will be skipped
    /// when skipped the database entry is used at face value
    #[arg(long, default_value_t = 10)]
    pub max_rebuild_tries: i32,
}

/// constructing the REST server application in the thread by adding sqlite database, socket address, a with rebuilder shared state and REST endpoints
pub async fn server(
    database: database::Database,
    shared_reports_list: Arc<Mutex<VecDeque<ReportBody>>>,
    metrics: Arc<Mutex<Metrics>>,
    args: Args,
) {
    let socket_addr: std::net::SocketAddr = args.socket_address;

    if let Some(parent) = database.db_path.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent).expect("db-file directory can not be created");
        }
    }

    match database.initialize() {
        Ok(_) => {
            println!("Database initialized at {:?}", database.db_path)
        }
        Err(e) => panic!("{}", e),
    };

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| format!("{}=debug", env!("CARGO_CRATE_NAME")).into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let app_state = AppState {
        shared_reports_list: Arc::clone(&shared_reports_list),
        gc_links_path: args.gc_links_path.clone(),
        savefile_path: args.savefile_path.clone(),
        registry: Arc::new(Mutex::new(Registry::default())),
        metrics,
    };

    {
        let mut registry = app_state.registry.lock().expect("registering metrics");
        registry.register(
            "linchpin_axum_requests",
            "Count of requests",
            app_state
                .metrics
                .lock()
                .expect("registering metrics")
                .requests
                .clone(),
        );
        registry.register(
            "linchpin_report_waitlist_len",
            "Number of reports waiting to be tested",
            app_state
                .metrics
                .lock()
                .expect("registering metrics")
                .current_len
                .clone(),
        );
        registry.register(
            "linchpin_comment_history_len",
            "Number of comments that were posted and are still stored locally for edits",
            app_state
                .metrics
                .lock()
                .expect("registering metrics")
                .comment_history_len
                .clone(),
        );
        registry.register(
            "linchpin_active_gc_roots",
            "Number of symlinks that protect toplevel derivation files from garbadge collection",
            app_state
                .metrics
                .lock()
                .expect("registering metrics")
                .active_gc_roots
                .clone(),
        );
        registry.register(
            "linchpin_number_of_pipeline_ids",
            "Number of different pipeline ids in the waitlist; how many different pipeline ordered a report",
            app_state
                .metrics
                .lock()
                .expect("registering metrics")
                .number_of_pipeline_ids
                .clone(),
        );
        registry.register(
            "linchpin_tokio_workers_count",
            "value of tokio_metrics::RuntimeMetric.workers_count",
            app_state
                .metrics
                .lock()
                .expect("registering metrics")
                .tokio_workers_count
                .clone(),
        );
        registry.register(
            "linchpin_tokio_total_park_count",
            "value of tokio_metrics::RuntimeMetric.total_park_count",
            app_state
                .metrics
                .lock()
                .expect("registering metrics")
                .tokio_total_park_count
                .clone(),
        );
    }

    let app = Router::new()
        .route("/ping", get(http_api::ping))
        .route("/metrics", get(http_api::metrics))
        .route("/report", post(http_api::report))
        .with_state(app_state)
        .layer(axum::extract::DefaultBodyLimit::max(1000000000));

    let listener = tokio::net::TcpListener::bind(socket_addr).await.unwrap();
    tracing::debug!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app).await.unwrap();
}

/// thread looping to check shared state for reports to process what derivations have what state and might need to be rebuild and documented
pub async fn rebuilder(
    database: Database,
    gitlab: Gitlab,
    shared_reports_list: Arc<Mutex<VecDeque<ReportBody>>>,
    shared_metrics: Arc<Mutex<Metrics>>,
    args: Args,
) {
    // to edit merge comments originating from same pipeline
    // instead of creating new comments on every toplevel
    let mut message_history: report_message::History = report_message::History { history: vec![] };

    // how to handle saved file contents after restarting the service
    // load saved state
    if args.persistent_reports {
        match load_shared_reports_list(
            shared_reports_list.clone(),
            args.savefile_path.clone(),
            shared_metrics.clone(),
        ) {
            Ok(_) => {}
            Err(e) => {
                println!("error: {e}")
            }
        };
        match message_history.load(args.savefile_history_path.clone(), shared_metrics.clone()) {
            Ok(_) => {}
            Err(e) => {
                println!("error: {e}")
            }
        };
        {
            let metrics = shared_metrics.lock().expect("lock and get metrics");
            let active_gc_symlinks: i64 = active_gc_roots(args.gc_links_path.clone())
                .expect("gc dir read error")
                .len()
                .try_into()
                .unwrap();
            metrics.active_gc_roots.set(active_gc_symlinks);

            let pipeline_ids: i64 = shared_reports_list_pipeline_ids(shared_reports_list.clone())
                .len()
                .try_into()
                .unwrap();
            metrics.number_of_pipeline_ids.set(pipeline_ids);
        }
    // overwrite saved state with a blank slate
    } else {
        match nix_derivation::reset_gc_root(args.gc_links_path.clone()) {
            Ok(_) => {}
            Err(e) => {
                println!("error: {e}")
            }
        };
        match save_shared_reports_list(shared_reports_list.clone(), args.savefile_path.clone()) {
            Ok(_) => {}
            Err(e) => {
                println!("error: {e}")
            }
        };
        message_history.remove_all_entries(shared_metrics.clone());
        match message_history.save(args.savefile_history_path.clone()) {
            Ok(_) => {}
            Err(e) => {
                println!("error: {e}")
            }
        };
    }

    // used for testing multiple waitlist items prior to removing them
    // just for debugging
    tokio::time::sleep(std::time::Duration::from_secs(20)).await;

    loop {
        let report: ReportBody = match get_one_report(shared_reports_list.clone()) {
            Some(e) => e,
            None => {
                tokio::time::sleep(std::time::Duration::from_secs(10)).await; // sleep when waitlist empty
                continue;
            }
        };

        // parse the report
        let toplevel =
            nix_derivation::Derivation::new(PathBuf::from(report.store_derivation.clone()))
                .expect("getting toplevel derivation");
        let closure: Vec<String> = report.store_derivation_closure.clone();
        let derivations_from_closure: Vec<Derivation> =
            match filter_for_store_derivations(closure.clone()) {
                Some(a) => a,
                None => {
                    println!("no store derivations in this closure");
                    Vec::new()
                }
            };
        let project_id = report
            .ci_merge_request_project_id
            .parse::<i64>()
            .expect("parse to i64");
        let merge_id = report
            .ci_merge_request_iid
            .parse::<i64>()
            .expect("parse to i64");
        let pipeline_id = report
            .ci_pipeline_id
            .parse::<i64>()
            .expect("parse pipeline id error");

        // edit previously posted merge comment to set known pending jobs
        for e in &mut message_history.history {
            if e.pipeline_id
                == report
                    .ci_pipeline_id
                    .parse::<i64>()
                    .expect("parse pipeline id error")
            {
                // refresh the waiting jobs
                let mut tmp_report_message = e.report_message.clone();
                let ci_jobs_waiting = shared_reports_list_entries_of_pipeline(
                    shared_reports_list.clone(),
                    pipeline_id,
                );
                tmp_report_message.report_summary.ci_jobs_waiting = ci_jobs_waiting;
                tmp_report_message.report_summary.ci_jobs_sum += ci_jobs_waiting;
                match gitlab
                    .overwrite_merge_comment(
                        tmp_report_message.clone(),
                        e.project_id,
                        e.merge_id,
                        e.comment_id,
                    )
                    .await
                {
                    Ok(_) => {
                        // waiting job is written in message
                        // do not update history entry
                        // in case of interruptions it will falsify the content
                    }
                    Err(e) => {
                        println!("overwrite merge comment error: {e:?}")
                    }
                }
            }
        }

        // filter out .sh/.patch/... files and get a list of derivations
        println!("the number of closure elements: {}", closure.len());
        println!(
            "the number of derivations:      {}",
            derivations_from_closure.len()
        );

        let to_test = filter_need_testing(
            database.clone(),
            derivations_from_closure.clone(),
            args.max_rebuild_tries,
        );

        // rebuilding whatever needs to be rebuild
        let semaphore = Arc::new(tokio::sync::Semaphore::new(args.simultaneous_builds));
        let mut jhs = Vec::new();

        for element in to_test {
            let semaphore = semaphore.clone();

            let tmp_database = database.clone();
            let tmp_element = element.clone();
            let tmp_nix_store = args.nix_store.clone();

            let jh = tokio::spawn(async move {
                let _permit = semaphore.acquire().await.unwrap();

                let result = build_rebuild_upsert(tmp_database, tmp_element, tmp_nix_store).await;

                drop(_permit);
                result
            });
            jhs.push(jh);
        }
        let mut responses = Vec::new();
        for jh in jhs {
            let response = jh.await.unwrap();
            responses.push(response);
        }

        // reporting back findings
        let ci_jobs_waiting =
            shared_reports_list_entries_of_pipeline(shared_reports_list.clone(), pipeline_id) - 1;
        let mut report_message = ReportMessage {
            report_summary: report_message::ReportSummary {
                closure_full: closure.clone(),
                derivation: derivations_from_closure.clone(),
                ci_jobs_sum: 1,
                ci_jobs_done: 1,
                ci_jobs_waiting,
            },
            report_detailed: report_message::ReportDetailed {
                reproducible: vec![],
                non_reproducible: vec![],
                build_error: vec![],
                no_entry: vec![],
            },
            commit_hash: report.ci_commit_sha.clone(),
        };
        let report_result: Vec<Derivation> =
            database.collect_report_results(derivations_from_closure.clone());
        for e in report_result {
            let job_toplevel: JobToplevel = JobToplevel {
                job: report.ci_job_name.clone(),
                toplevels: vec![toplevel.to_string()],
            };
            // all derivations are ordered into the message type according to their determinism state
            match e.state {
                Some(DerivationState::Reproducible) => {
                    report_message
                        .report_detailed
                        .reproducible
                        .push(Derivation {
                            file_path: e.file_path,
                            state: e.state,
                            error_reason: e.error_reason,
                            db_write_count: e.db_write_count,
                            job_toplevel: Some(vec![job_toplevel]),
                        });
                }
                Some(DerivationState::NonReproducible) => {
                    report_message
                        .report_detailed
                        .non_reproducible
                        .push(Derivation {
                            file_path: e.file_path,
                            state: e.state,
                            error_reason: e.error_reason,
                            db_write_count: e.db_write_count,
                            job_toplevel: Some(vec![job_toplevel]),
                        });
                }
                Some(DerivationState::Error) => {
                    report_message.report_detailed.build_error.push(Derivation {
                        file_path: e.file_path,
                        state: e.state,
                        error_reason: e.error_reason,
                        db_write_count: e.db_write_count,
                        job_toplevel: Some(vec![job_toplevel]),
                    });
                }
                Some(DerivationState::NotTested) => {
                    report_message.report_detailed.no_entry.push(Derivation {
                        file_path: e.file_path,
                        state: e.state,
                        error_reason: e.error_reason,
                        db_write_count: e.db_write_count,
                        job_toplevel: Some(vec![job_toplevel]),
                    });
                }
                None => {
                    report_message.report_detailed.no_entry.push(Derivation {
                        file_path: e.file_path,
                        state: e.state,
                        error_reason: e.error_reason,
                        db_write_count: e.db_write_count,
                        job_toplevel: Some(vec![job_toplevel]),
                    });
                }
            }
        }

        // check history for an existing comment triggered by this pipeline id
        // make a merged_message and get the comment id to overwrite
        let mut merged_message: Option<ReportMessage> = None;
        let mut comment_id: Option<i64> = None;
        for e in &mut message_history.history {
            if e.pipeline_id
                == report
                    .ci_pipeline_id
                    .parse::<i64>()
                    .expect("parse pipeline id error")
            {
                merged_message = Some(e.report_message.merge(&mut report_message));
                comment_id = Some(e.comment_id);
            }
        }

        // if message of the pipeline_id exists overwrite it with merged_message
        // else create a new message with report_message
        if merged_message.is_some() && comment_id.is_some() {
            // overwrite
            match gitlab
                .overwrite_merge_comment(
                    merged_message.clone().unwrap(),
                    project_id,
                    merge_id,
                    comment_id.unwrap(),
                )
                .await
            {
                Ok(_) => {
                    for e in &mut message_history.history {
                        if e.pipeline_id == pipeline_id {
                            e.report_message = merged_message.clone().unwrap();
                        }
                    }
                }
                Err(e) => {
                    println!("overwrite merge error: {e:?}")
                }
            }
        } else {
            // create
            match gitlab
                .create_merge_comment(report_message.clone(), project_id, merge_id)
                .await
            {
                Ok(a) => {
                    message_history.add_entry(
                        HistoryEntry {
                            report_message: report_message.clone(),
                            datetime: SystemTime::now(),
                            project_id,
                            merge_id,
                            pipeline_id,
                            comment_id: a.id,
                        },
                        shared_metrics.clone(),
                    );
                }
                Err(e) => {
                    println!("create merge error: {e:?}")
                }
            }
        }

        // cleaning
        match toplevel.delete_gc_root(args.gc_links_path.clone()) {
            Ok(_) => (),
            Err(out) => println!("failed deletting gc root: {out:?}"),
        }
        match remove_report(shared_reports_list.clone(), report, shared_metrics.clone()) {
            Ok(_) => (),
            Err(out) => println!("failed deletting gc root: {out:?}"),
        }
        match save_shared_reports_list(shared_reports_list.clone(), args.savefile_path.clone()) {
            Ok(_) => (),
            Err(out) => println!("failed to write savefile: {out:?}"),
        }
        let _ = message_history.save(args.savefile_history_path.clone());
        message_history.remove_older_than(24 * 14, shared_metrics.clone());
        {
            let metrics = shared_metrics.lock().expect("lock and get metrics");
            let active_gc_symlinks: i64 = active_gc_roots(args.gc_links_path.clone())
                .expect("gc dir read error")
                .len()
                .try_into()
                .unwrap();
            metrics.active_gc_roots.set(active_gc_symlinks);

            let pipeline_ids: i64 = shared_reports_list_pipeline_ids(shared_reports_list.clone())
                .len()
                .try_into()
                .unwrap();
            metrics.number_of_pipeline_ids.set(pipeline_ids);
        }
        println!("finished one report");
    }
}

/// sort closure to only get .drv paths (removing `.patch`, `.sh` or other)
fn filter_for_store_derivations(
    closure_full: Vec<String>,
) -> Option<Vec<nix_derivation::Derivation>> {
    let mut closure_derivations: Vec<nix_derivation::Derivation> = Vec::new();
    if closure_full.is_empty() {
        return None;
    };
    for element in &closure_full {
        let element_path = std::path::PathBuf::from(&element);
        match nix_derivation::Derivation::new(element_path) {
            Ok(a) => closure_derivations.push(a),
            Err(_) => continue,
        };
    }
    Some(closure_derivations)
}

/// lookup what has been tested already and determine what is either not yet tested or has attempts left in case a network error previously caused a failure
fn filter_need_testing(
    database: Database,
    derivations: Vec<nix_derivation::Derivation>,
    max_rebuild_tries: i32,
) -> Vec<nix_derivation::Derivation> {
    let mut result: Vec<nix_derivation::Derivation> = vec![];

    for derivation in derivations.clone() {
        let lookup: Vec<nix_derivation::Derivation> = database
            .lookup_store_derivation(derivation.file_path.to_str().unwrap().to_string())
            .expect("sqlite lookup error");
        match lookup.is_empty() {
            true => {
                // sqlite entry does not exist
                result.push(derivation.clone());
            }
            false => {
                // sqlite entry does exist
                for lookup_entry in lookup {
                    // rusqlite provides query results as a vector
                    // since the key being the query filter will only yield one entry as result
                    match lookup_entry.state {
                        Some(nix_derivation::DerivationState::NotTested) => {
                            result.push(derivation.clone());
                        }
                        Some(nix_derivation::DerivationState::Error) => {
                            println!(
                                "entry found with {}, initial build probably failed, trying again {}",
                                nix_derivation::DerivationState::Error,
                                derivation
                            );
                            result.push(derivation.clone());
                        }
                        Some(nix_derivation::DerivationState::Reproducible) => {
                            println!(
                                "entry found with {}, skipping {}",
                                nix_derivation::DerivationState::Reproducible,
                                derivation
                            );
                        }
                        Some(nix_derivation::DerivationState::NonReproducible) => {
                            let past_rebuilds = lookup_entry.db_write_count.unwrap();
                            if max_rebuild_tries > past_rebuilds {
                                println!(
                                    "entry found with {} and {} attempts, trying again {} ",
                                    nix_derivation::DerivationState::NonReproducible,
                                    past_rebuilds,
                                    derivation,
                                );
                                result.push(derivation.clone());
                            } else {
                                println!(
                                    "entry found with {} and {} attempts, skipping {} ",
                                    nix_derivation::DerivationState::NonReproducible,
                                    past_rebuilds,
                                    derivation,
                                );
                            }
                        }
                        None => {}
                    }
                }
            }
        }
    }
    result
}

/// helper function to do the initial `nix-build``, the `nix-build --check`` and the sqlite database upsert
async fn build_rebuild_upsert(
    database: database::Database,
    element: nix_derivation::Derivation,
    nix_store: String,
) -> std::result::Result<(), ()> {
    println!("building:   {:?}", element.file_path);
    let result = element.nix_build_remote(nix_store.clone()).await;
    match result.status.success() {
        true => {
            // initial build or substitution worked
        }
        false => {
            let db_entry: nix_derivation::Derivation = nix_derivation::Derivation {
                file_path: element.file_path.clone(),
                state: Some(nix_derivation::DerivationState::Error),
                error_reason: None, // TODO initial build failure reason Y/N?
                db_write_count: None,
                job_toplevel: None,
            };
            database
                .upsert_store_derivation(db_entry)
                .expect("sqlite update error");
        }
    };

    println!("rebuilding: {:?}", element.file_path);
    let result = element.nix_build_check_remote(nix_store.clone()).await;

    match result.status.success() {
        true => {
            let db_entry: nix_derivation::Derivation = nix_derivation::Derivation {
                file_path: element.file_path.clone(),
                state: Some(nix_derivation::DerivationState::Reproducible),
                error_reason: None,
                db_write_count: None,
                job_toplevel: None,
            };
            database
                .upsert_store_derivation(db_entry)
                .expect("sqlite update error");
        }
        false => {
            println!("non reproducible (or error)");
            let text: String = String::from_utf8_lossy(&result.clone().stderr).to_string();
            let status: Option<nix_derivation::BuildError> =
                nix_derivation::parse_nix_build_error(text);
            let db_entry: nix_derivation::Derivation = nix_derivation::Derivation {
                file_path: element.file_path.clone(),
                state: Some(nix_derivation::DerivationState::NonReproducible),
                error_reason: status,
                db_write_count: None,
                job_toplevel: None,
            };
            database
                .upsert_store_derivation(db_entry)
                .expect("sqlite update error");
        }
    }
    Ok(())
}
