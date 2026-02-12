use std::path::PathBuf;
use std::time::SystemTime;

use anyhow::{anyhow, Context, Error, Result};

use crate::report_request::ReportRequest;

#[derive(Debug, Clone, PartialEq)]
struct ReportRequestHistoryList {
    history_entries: Vec<ReportRequestHistoryEntry>,
}
#[derive(Debug, Clone, PartialEq)]
struct ReportRequestHistoryEntry {
    report_request: ReportRequest,
    archival_time: SystemTime,
}
impl From<ReportRequest> for ReportRequestHistoryEntry {
    fn from(value: ReportRequest) -> Self {
        ReportRequestHistoryEntry {
            report_request: value,
            archival_time: SystemTime::now(),
        }
    }
}

impl ReportRequestHistoryList {
    //TODO some sort of query through it
    // fn find(self, derivation: Derivation)->Result<ReportRequest, Error> {todo!()}

    /// save history to a file
    fn save(self, path: PathBuf) -> Result<()> {
        todo!()
    }
    ///load history out of the file
    fn load(self, path: PathBuf) -> Result<ReportRequestHistoryList, Error> {
        todo!()
    }
    /// clear all of history
    fn reset(self, path: PathBuf) -> Result<()> {
        // clear local
        // clear history
        todo!()
    }
}
