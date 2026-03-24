use anyhow::Result;
use log::debug;
use log::error;
use log::info;
use log::trace;
use log::warn;
use std::fs;
use std::fs::create_dir_all;
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

pub mod cli;
pub mod report_request;
pub mod report_request_history;
pub mod report_request_list;

use crate::cli::Cli;
use crate::gitlab::Gitlab;
use crate::nix_derivation::DerivationState;
use crate::report_request::ClosureElement;
use crate::report_request::Publisher;
use crate::report_request_history::ReportRequestHistoryList;
use crate::report_request_list::ReportRequestList;

use crate::nix_derivation::reset_gc_root;

pub fn initialize_linchpin(
    cli: &Cli,
    shared_reports_list: Arc<Mutex<ReportRequestList>>,
    shared_reports_history: Arc<Mutex<ReportRequestHistoryList>>,
    database: &Database,
) -> Result<()> {
    if !&cli.gc_links_dir.exists() {
        debug!("creating gc links dir");
        create_dir_all(&cli.gc_links_dir)?;
    }
    if !&cli.savefile_path.parent().unwrap().exists() {
        debug!("creating savefile path parent");
        create_dir_all(cli.savefile_path.parent().unwrap())?;
    }
    if !&cli.savefile_history_path.parent().unwrap().exists() {
        debug!("creating history path parent");
        create_dir_all(cli.savefile_history_path.parent().unwrap())?;
    }

    let mut list = shared_reports_list.lock().unwrap();

    // if cli then load running report list else clear gc roots
    if cli.persistent_reports {
        debug!("loading last active report_request_list");
        match list.load_and_lookup(cli.savefile_path.clone(), database) {
            Ok(_) => {}
            Err(e) => {
                warn!("loading given savefile path failed: {}", e);
            }
        };
    } else {
        list.save(&cli.savefile_path)?;
        reset_gc_root(&cli.gc_links_dir)?;
    }

    // if cli then load done history list else nothing
    let mut history = shared_reports_history.lock().unwrap();
    if cli.savefile_history_path.exists() {
        debug!("loading history");
        match history.load(&cli.savefile_history_path) {
            Ok(_) => {}
            Err(e) => {
                warn!("loading given history path failed: {}", e);
            }
        };
    } else {
        debug!("no history found");
        history.save(&cli.savefile_history_path)?;
    }

    Ok(())
}

/// thread looping to check shared state for reports to process what derivations have what state and might need to be rebuild and documented
pub async fn rebuilder(
    cli: Cli,
    shared_reports_list: Arc<Mutex<ReportRequestList>>,
    history: Arc<Mutex<ReportRequestHistoryList>>,
    database: Database,
) {
    // TODO https://docs.rs/tokio/latest/tokio/sync/mpsc/

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
            continue;
        }

        let mut report_request = report_request.unwrap();
        info!("doing: {}", report_request.store_derivation);

        // do database lookup and if found take the state to memory
        let mut derivations = 0;
        let mut non_derivations = 0;

        let mut db_hits = 0;
        let mut db_hits_reproducible = 0;
        let mut db_misses = 0;
        let mut db_error = 0;

        for closure_element in &mut report_request.store_derivation_closure {
            match closure_element {
                ClosureElement::Derivation(derivation) => {
                    derivations += 1;
                    match database.lookup_store_derivation(derivation.to_string()) {
                        Ok(Some(lookup_derivation)) => {
                            trace!("db hit: {derivation}");

                            if lookup_derivation.state == Some(DerivationState::Reproducible) {
                                db_hits_reproducible += 1;
                            }
                            db_hits += 1;

                            *closure_element = ClosureElement::Derivation(lookup_derivation);
                        }
                        Ok(None) => {
                            trace!("db miss: {derivation}");
                            db_misses += 1;
                        }
                        Err(e) => {
                            warn!("lookup error: {e}");
                            db_error += 1;
                        }
                    }
                }
                ClosureElement::Other(_) => {
                    non_derivations += 1;
                }
            }
        }

        info!("derivation count is {derivations}");
        info!("non_derivation count is {non_derivations}");

        info!("db hit count is {db_hits}");
        info!("db miss count is {db_misses}");
        info!("will not rebuild {db_hits_reproducible} as reproducible marked db hits");

        if db_error > 0 {
            warn!("db lookup errors: {db_error}");
        }

        // if necessary rebuild and update db
        use std::sync::Arc;
        use tokio::sync::Semaphore;
        use tokio::task;

        let semaphore = Arc::new(Semaphore::new(cli.simultaneous_builds));
        let mut jhs = Vec::new();
        for closure_element in &mut report_request.store_derivation_closure {
            let permit = semaphore.clone().acquire_owned().await.unwrap();

            let database_clone = database.clone();
            let nix_store_clone = cli.nix_store.clone();
            let closure_element_clone = closure_element.clone();
            let max_rebuild_tries = cli.max_rebuild_tries;

            let jh = task::spawn(async move {
                trace!("spawned new task");
                let closure_element = match closure_element_clone {
                    ClosureElement::Derivation(derivation) => {
                        info!("looking at a derivation: {derivation}");
                        // skip if tested too often
                        if derivation.db_write_count.unwrap_or_default() >= max_rebuild_tries {
                            return ClosureElement::Derivation(derivation);
                        }
                        // do stuff for every derivation
                        let tmp = match derivation.clone().state {
                            Some(DerivationState::Reproducible) => derivation,
                            _ => derivation
                                .build_rebuild_upsert(&database_clone, &nix_store_clone)
                                .await
                                .expect("build_rebuild_upsert failed"),
                        };
                        trace!("done with derivation: {tmp}");
                        ClosureElement::Derivation(tmp)
                    }
                    ClosureElement::Other(other) => {
                        info!("not a derivation: {other}");
                        ClosureElement::Other(other)
                    }
                };
                drop(permit);
                closure_element
            });
            jhs.push(jh);
        }
        let mut responses = Vec::new();
        for jh in jhs {
            let response = jh.await.unwrap();
            responses.push(response);
        }
        report_request.store_derivation_closure = responses;

        // publish results
        let history_entry = history.lock().unwrap().try_find(&report_request);
        match history_entry {
            Some(_) => {
                info!("this report_request is found in the history");
            }
            None => {
                debug!("this toplevel derivation is not yet in the history");
            }
        };

        match &report_request.publisher_data {
            Publisher::Cli() => {
                info!("publishing to cli:");
                report_request.print_summary();
            }
            Publisher::Gitlab(_) => {
                // TODO this unwrap can panic
                let url = cli
                    .clone()
                    .gitlab
                    .expect("gitlab is not available as a publisher")
                    .gitlab_url
                    .clone()
                    .expect("gitlab is not available as a publisher");

                let token: String = String::from_utf8(
                    fs::read(cli.clone().gitlab.unwrap().gitlab_api_token_file.unwrap())
                        .expect("reading gitlab token failed"),
                )
                .expect("utf8 to string");
                let gitlab = Gitlab { url, token };

                match gitlab.publish_report(&report_request).await {
                    Ok(_) => {
                        info!("published to gitlab");
                    }
                    Err(e) => {
                        error!("failed publishing to gitlab: {e}");
                        // TODO how do i handle this case and give feedback?
                        // a user will just wait indefinetly for the comment
                    }
                };
            }
        }

        // move just finished report from (todo) list into history
        {
            history.lock().unwrap().add(report_request.clone().into());
            history
                .lock()
                .unwrap()
                .save(&cli.savefile_history_path)
                .expect("saving history");

            let mut list = shared_reports_list.lock().unwrap();
            list.remove_one_report(report_request.clone());
            list.save(&cli.savefile_path).expect("saving list");
            info!("done with {}", report_request.store_derivation);
        }
        report_request
            .store_derivation
            .delete_gc_root(&cli.gc_links_dir)
            .expect("removing gc symlink");
        debug!("removed gc symlink for {}", report_request.store_derivation);
    }
}
