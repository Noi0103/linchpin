use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::server::Metrics;
use crate::server::ReportBody;

/// add report as last in line at the back
pub fn add_report(
    shared_reports_list: Arc<Mutex<VecDeque<ReportBody>>>,
    report: ReportBody,
    metrics: Arc<Mutex<Metrics>>,
) -> Result<(), &'static str> {
    {
        let mut list: MutexGuard<VecDeque<ReportBody>> = shared_reports_list.lock().unwrap();
        if !list.contains(&report) {
            list.push_back(report.clone());
        } else {
            println!(
                "ignoring the duplicate: {}",
                &report.clone().store_derivation
            );
            return Err("already in the waitlist");
        }
    }
    {
        let metrics = metrics.lock().expect("get metrics lock");
        metrics.current_len.set(
            healthy_length(shared_reports_list.clone())
                .expect("reading healthy waitlist length")
                .try_into()
                .unwrap(),
        );
    }

    Ok(())
}

/// read the front most report
pub fn get_one_report(shared_reports_list: Arc<Mutex<VecDeque<ReportBody>>>) -> Option<ReportBody> {
    let list: MutexGuard<VecDeque<ReportBody>> = shared_reports_list.lock().unwrap();
    list.front().cloned()
}

pub fn remove_report(
    shared_reports_list: Arc<Mutex<VecDeque<ReportBody>>>,
    report: ReportBody,
    metrics: Arc<Mutex<Metrics>>,
) -> Result<(), &'static str> {
    {
        let mut list: MutexGuard<VecDeque<ReportBody>> = shared_reports_list.lock().unwrap();

        match list.contains(&report) {
            true => {
                //remove it
                let index_front = 0;
                if list.get(index_front).unwrap() == &report {
                    list.remove(index_front);
                } else {
                    return Err("trying to remove the wrong index of shared_reports_list");
                }
            }
            false => return Err("report not in shared_reports_list"),
        }
    }
    {
        let metrics = metrics.lock().expect("get metrics lock");
        metrics.current_len.set(
            healthy_length(shared_reports_list.clone())
                .expect("reading healthy waitlist length")
                .try_into()
                .unwrap(),
        );
    }
    Ok(())
}

pub fn save_shared_reports_list(
    shared_reports_list: Arc<Mutex<VecDeque<ReportBody>>>,
    savefile: PathBuf,
) -> Result<(), std::io::Error> {
    let list: MutexGuard<VecDeque<ReportBody>> = shared_reports_list.lock().unwrap();
    let list_clone: VecDeque<ReportBody> = list.clone();
    let list_json: String =
        serde_json::to_string(&list_clone).expect("parse response json to string");
    std::fs::write(savefile, list_json)?;
    println!("wrote savefile with number of reports: {}", list.len());
    Ok(())
}

pub fn load_shared_reports_list(
    shared_reports_list: Arc<Mutex<VecDeque<ReportBody>>>,
    savefile: PathBuf,
    metrics: Arc<Mutex<Metrics>>,
) -> Result<(), std::io::Error> {
    let data: Vec<u8> = std::fs::read(savefile)?;
    let list_loaded: VecDeque<ReportBody> =
        serde_json::from_slice(&data).expect("savefile corrupted; can not be parsed");

    {
        let mut list: MutexGuard<VecDeque<ReportBody>> = shared_reports_list.lock().unwrap();
        for _e in 0..list_loaded.len() {
            list.push_back(list_loaded.clone().pop_front().unwrap());
        }
        println!(
            "loaded from savefile with number of reports: {}",
            list.len()
        );
    }
    {
        let metrics = metrics.lock().expect("get metrics lock");
        metrics.current_len.set(
            healthy_length(shared_reports_list.clone())
                .expect("reading healthy waitlist length")
                .try_into()
                .unwrap(),
        );
    }

    Ok(())
}

/// count how many of the shared_reports_list items are of the given pipeline_id
pub fn shared_reports_list_entries_of_pipeline(
    shared_reports_list: Arc<Mutex<VecDeque<ReportBody>>>,
    pipeline_id: i64,
) -> i32 {
    let list: MutexGuard<VecDeque<ReportBody>> = shared_reports_list.lock().unwrap();
    let mut counter: i32 = 0;
    for e in list.clone() {
        let ci_pipeline_id: i64 = e
            .ci_pipeline_id
            .parse::<i64>()
            .expect("pipeline id parse to i64");
        if ci_pipeline_id == pipeline_id {
            counter += 1;
        }
    }
    counter
}

/// get all pipeline ids without dublicates
pub fn shared_reports_list_pipeline_ids(
    shared_reports_list: Arc<Mutex<VecDeque<ReportBody>>>,
) -> Vec<i64> {
    let list: MutexGuard<VecDeque<ReportBody>> = shared_reports_list.lock().unwrap();
    let mut id_list: Vec<i64> = vec![];
    for e in list.clone() {
        let ci_pipeline_id: i64 = e
            .ci_pipeline_id
            .parse::<i64>()
            .expect("pipeline id parse to i64");
        if !id_list.contains(&ci_pipeline_id) {
            id_list.push(ci_pipeline_id)
        }
    }
    id_list
}

/// is the shared_reports_list structure still healthy or broken for whatever reason
pub fn healthy_length(
    shared_reports_list: Arc<Mutex<VecDeque<ReportBody>>>,
) -> Result<usize, &'static str> {
    let healthy: bool = !shared_reports_list.is_poisoned();
    match healthy {
        true => {
            let length = {
                let list: MutexGuard<VecDeque<ReportBody>> = shared_reports_list.lock().unwrap();
                list.len()
            };
            Ok(length)
        }
        false => Err("poisoned shared_reports_list"),
    }
}
