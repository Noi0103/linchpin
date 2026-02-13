use anyhow::{anyhow, Context, Error, Result};
use log::debug;
use log::error;
use log::info;
use std::collections::VecDeque;
use std::fs;
use std::fs::create_dir_all;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;

/// utilities to interact and work with nix store derivations
pub mod nix_derivation;

/// utilities to interact with the sqlite database to read and write information about nix derivations
pub mod database;
use database::Database;

/// handlers for REST api endpoints
pub mod server;

/// functions to make message body and interact with gitlab api
pub mod gitlab;

pub mod publisher;

pub mod cli;
pub mod report_request;
pub mod report_request_history;
pub mod report_request_list;

use crate::cli::Cli;
use crate::gitlab::Gitlab;
use crate::nix_derivation::Derivation;
use crate::nix_derivation::DerivationState;
use crate::report_request::ClosureElement;
use crate::report_request::Publisher;
use crate::report_request_list::ReportRequestList;

use crate::nix_derivation::reset_gc_root;

pub fn initialize_linchpin(
    cli: &Cli,
    shared_reports_list: Arc<Mutex<ReportRequestList>>,
    shared_reports_history: Arc<Mutex<ReportRequestList>>,
    database: &Database,
) -> Result<()> {
    if !&cli.gc_links_dir.exists() {
        create_dir_all(&cli.gc_links_dir)?;
    }
    if !&cli.savefile_path.parent().unwrap().exists() {
        create_dir_all(&cli.savefile_path.parent().unwrap())?;
    }
    if !&cli.savefile_history_path.parent().unwrap().exists() {
        create_dir_all(&cli.savefile_history_path.parent().unwrap())?;
    }

    let list = shared_reports_list.lock().unwrap();
    // if cli then load running report list else clear gc roots
    if cli.persistent_reports {
        debug!("loading last active report_request_list");
        list.clone()
            .load_and_lookup(cli.savefile_path.clone(), database);
    } else {
        list.save(&cli.savefile_path)?;
        reset_gc_root(cli.gc_links_dir.clone())?;
    }
    // if cli then load done history list else nothing
    let history = shared_reports_history.lock().unwrap();
    history
        .clone()
        .load_and_lookup(cli.savefile_history_path.clone(), database);
    Ok(())
}

/// thread looping to check shared state for reports to process what derivations have what state and might need to be rebuild and documented
pub async fn rebuilder(
    cli: Cli,
    shared_reports_list: Arc<Mutex<ReportRequestList>>,
    history_list: Arc<Mutex<ReportRequestList>>,
    database: Database,
) {
    info!("HELLO WORLD REBUILDER");

    // TODO https://docs.rs/tokio/latest/tokio/sync/mpsc/

    // TBD
    //tokio::time::sleep(std::time::Duration::from_secs(20)).await;

    //TODO have data modeled better and receiving working better before touching this stuff

    loop {
        // mpsc let this wait until message
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;

        // get front report
        let report_request;
        {
            let list = shared_reports_list.lock().unwrap();
            report_request = list.get_one_report();
        }

        if report_request.is_none() {
            //debug!("no report request");
            continue;
        }

        let report_request = report_request.unwrap();
        info!("doing: {}", report_request.store_derivation);

        // lookup what needs to be built (i.e. cli.max_rebuilds > db_write)
        // rebuild and update db
        for closure_element in &report_request.store_derivation_closure {
            //TODO simultaneous builds feature is missing
            match closure_element {
                ClosureElement::Derivation(derivation) => {
                    // TODO do these paths properly
                    if derivation.db_write_count < Some(cli.max_rebuild_tries)
                        && derivation.state != Some(DerivationState::Reproducible)
                    {
                        match derivation
                            .build_rebuild_upsert(&database, &cli.nix_store)
                            .await
                        {
                            Ok(_) => {
                                debug!("successfully rebuilt {derivation}")
                            }
                            Err(e) => {
                                error!("rebuilding failed {derivation} {e}")
                            }
                        }
                    }
                }
                ClosureElement::Other(_) => {}
            }
        }
        // publish results
        match &report_request.publisher_data {
            Publisher::Cli() => {
                info!("publishing to cli:");
                report_request.print_summary();
            }
            Publisher::Gitlab(metadata_gitlab) => {
                // TODO this unwrap can panic
                let url = cli.clone().gitlab.unwrap().gitlab_url.clone().unwrap();

                let token: String = String::from_utf8(
                    fs::read(cli.clone().gitlab.unwrap().gitlab_api_token_file.unwrap())
                        .expect("reading gitlab token"),
                )
                .expect("utf8 to string");
                let gitlab = Gitlab { url, token };
                // TODO check history entries decide: update or publish
                match gitlab.publish_report(&report_request).await {
                    Ok(_) => {
                        info!("published to gitlab");
                    }
                    Err(e) => {
                        error!("failed publishing to gitlab");
                        // TODO how do i handle this case and give feedback?
                        // a user will just wait indefinetly for the comment
                    }
                };
            }
        }
        // move just finished report from todo into history
        {
            history_list.lock().unwrap().add_one_report(&report_request);

            let mut list = shared_reports_list.lock().unwrap();
            list.remove_one_report(&report_request);
            info!("done with {}", report_request.store_derivation);
        }
    }

    /*
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
        /*
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
                    .update_report(
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
        */

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
        /*
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
        */

        // if message of the pipeline_id exists overwrite it with merged_message
        // else create a new message with report_message
        /*
        #[allow(clippy::unnecessary_unwrap)]
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
        */

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
        //let _ = message_history.save(args.savefile_history_path.clone());
        //message_history.remove_older_than(24 * 14, shared_metrics.clone());
        {
            let metrics = shared_metrics.lock().unwrap();
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
    */
}
