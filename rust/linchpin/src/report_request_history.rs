use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::SystemTime;

use anyhow::Result;
use log::info;
use serde::Deserialize;
use serde::Serialize;

use crate::report_request::ReportRequest;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReportRequestHistoryList {
    history_entries: VecDeque<ReportRequestHistoryEntry>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReportRequestHistoryEntry {
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
impl Default for ReportRequestHistoryList {
    fn default() -> Self {
        Self::new()
    }
}

impl ReportRequestHistoryList {
    pub fn new() -> ReportRequestHistoryList {
        ReportRequestHistoryList {
            history_entries: VecDeque::new(),
        }
    }
    //TODO some sort of query through it
    // fn find(self, derivation: Derivation)->Result<ReportRequest, Error> {todo!()}

    pub fn add(&mut self, entry: ReportRequestHistoryEntry) {
        if !self.history_entries.contains(&entry) {
            self.history_entries.push_back(entry.clone());
        } else {
            info!(
                "already in the list, ignoring duplicate: {}",
                &entry.report_request.store_derivation
            );
            // TODO update the history entry (update existing) to handle update functionality
            // for publisher that can update

            // TODO prevent duplicate entries
        }
    }

    /// save history to a file
    pub fn save(&self, path: &PathBuf) -> Result<()> {
        let json: String =
            serde_json::to_string(&self.history_entries).expect("parse to json-string failed");
        std::fs::write(path, json)?;
        info!(
            "wrote history with number of reports: {}",
            self.history_entries.len()
        );
        Ok(())
    }

    ///load history out of the file
    pub fn load(&mut self, path: &PathBuf) -> Result<()> {
        let data: Vec<u8> = std::fs::read(path)?;
        let mut loaded: VecDeque<ReportRequestHistoryEntry> = serde_json::from_slice(&data)
            .expect("parse from json-string failed; savefile corrupted");
        info!("loaded history with number of reports: {}", loaded.len());
        while let Some(history_entries) = loaded.pop_front() {
            self.history_entries.push_back(history_entries);
        }
        info!(
            "loaded, new number of reports is: {}",
            self.history_entries.len()
        );
        Ok(())
    }

    /// clear all of history
    pub fn reset(self, _path: PathBuf) -> Result<()> {
        // clear local
        // clear history
        todo!()
    }
}
