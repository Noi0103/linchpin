use std::collections::VecDeque;
use std::path::PathBuf;

use anyhow::Error;
use anyhow::Result;
use log::info;
use serde::Deserialize;
use serde::Serialize;

use crate::database::Database;
use crate::report_request::ReportRequest;

/// The grand todo list of report requests that will be rebuillt over time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReportRequestList {
    // active report requests
    report_requests: VecDeque<ReportRequest>,
}

// TODO docstrings
impl Default for ReportRequestList {
    fn default() -> Self {
        Self::new()
    }
}

impl ReportRequestList {
    pub fn new() -> ReportRequestList {
        ReportRequestList {
            report_requests: VecDeque::new(),
        }
    }
    pub fn add_one_report(&mut self, report_request: &ReportRequest) {
        if !self.report_requests.contains(report_request) {
            self.report_requests.push_back(report_request.clone());
        } else {
            info!(
                "already in the list, ignoring duplicate: {}",
                &report_request.store_derivation
            );
        }
    }
    pub fn get_one_report(&self) -> Option<ReportRequest> {
        self.report_requests.front().cloned()
    }
    pub fn remove_one_report(&mut self, report_request: ReportRequest) {
        self.report_requests
            .retain(|x| x.store_derivation != report_request.store_derivation);
    }
    pub fn save(&self, path: &PathBuf) -> Result<(), Error> {
        let json: String =
            serde_json::to_string(&self.report_requests).expect("parse to json-string failed");
        std::fs::write(path, json)?;
        info!(
            "wrote savefile with number of reports: {}",
            self.report_requests.len()
        );
        Ok(())
    }
    pub fn load(&mut self, path: PathBuf) -> Result<(), Error> {
        let data: Vec<u8> = std::fs::read(path)?;
        let mut loaded: VecDeque<ReportRequest> = serde_json::from_slice(&data)
            .expect("parse from json-string failed; savefile corrupted");
        info!("loaded savefile with number of reports: {}", loaded.len());
        while let Some(report_request) = loaded.pop_front() {
            self.report_requests.push_back(report_request);
        }
        info!(
            "loaded, new number of reports is: {}",
            self.report_requests.len()
        );
        Ok(())
    }
    pub fn load_and_lookup(&mut self, path: PathBuf, database: &Database) -> Result<()> {
        self.load(path.clone())?;
        for index in 0..self.report_requests.len() {
            self.report_requests[index].lookup(database);
        }
        Ok(())
    }
    pub fn len(&self) -> usize {
        self.report_requests.len()
    }
    pub fn is_empty(&self) -> bool {
        self.report_requests.is_empty()
    }
}
