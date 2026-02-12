use std::fmt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use crate::nix_derivation::{self};
use crate::server::Metrics;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct History {
    pub history: Vec<HistoryEntry>,
}
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct HistoryEntry {
    pub report_message: ReportMessage,
    pub datetime: SystemTime,
    pub project_id: i64,
    pub merge_id: i64,
    pub pipeline_id: i64,
    pub comment_id: i64,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct ReportMessage {
    pub report_summary: ReportSummary,
    pub report_detailed: ReportDetailed,
    pub commit_hash: String,
}
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct ReportSummary {
    pub closure_full: Vec<String>,
    pub derivation: Vec<nix_derivation::Derivation>,
    pub ci_jobs_done: i32,
    pub ci_jobs_waiting: i32,
    pub ci_jobs_sum: i32,
}
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct ReportDetailed {
    pub reproducible: Vec<nix_derivation::Derivation>,
    pub non_reproducible: Vec<nix_derivation::Derivation>,
    pub build_error: Vec<nix_derivation::Derivation>,
    pub no_entry: Vec<nix_derivation::Derivation>,
}

impl History {
    pub fn add_entry(&mut self, entry: HistoryEntry, metrics: Arc<Mutex<Metrics>>) {
        self.history.push(entry);
        {
            let metrics = metrics.lock().expect("get metrics lock");
            metrics
                .comment_history_len
                .set(self.history.len().try_into().unwrap());
        }
    }
    // TODO additional checks besides a time y/N?
    pub fn remove_older_than(&mut self, hours: u64, metrics: Arc<Mutex<Metrics>>) {
        let cutoff = Duration::from_secs(60 * 60 * hours); // secs*min*hrs
        let now = SystemTime::now();

        self.history.retain(|e| {
            match now.duration_since(e.datetime) {
                Ok(duration) => duration <= cutoff,
                Err(_) => true, // clock went backwards
            }
        });
        {
            let metrics = metrics.lock().expect("get metrics lock");
            metrics
                .comment_history_len
                .set(self.history.len().try_into().unwrap());
        }
    }
    pub fn remove_all_entries(&mut self, metrics: Arc<Mutex<Metrics>>) {
        self.history.clear();
        {
            let metrics = metrics.lock().expect("get metrics lock");
            metrics
                .comment_history_len
                .set(self.history.len().try_into().unwrap());
        }
    }
    pub fn save(&self, savefile: PathBuf) -> Result<(), std::io::Error> {
        let list_json: String = serde_json::to_string(self).expect("parse response json to string");
        std::fs::write(savefile, list_json)?;
        println!(
            "wrote history file with number of entries: {}",
            self.history.len()
        );
        Ok(())
    }
    pub fn load(
        &mut self,
        savefile: PathBuf,
        metrics: Arc<Mutex<Metrics>>,
    ) -> Result<(), std::io::Error> {
        let data: Vec<u8> = std::fs::read(savefile)?;
        let parsed: History =
            serde_json::from_slice(&data).expect("savefile corrupted; can not be parsed");
        *self = parsed.clone();
        println!(
            "loaded history file with number of entries: {}",
            self.history.len()
        );
        {
            let metrics = metrics.lock().expect("get metrics lock");
            metrics
                .comment_history_len
                .set(self.history.len().try_into().unwrap());
        }
        Ok(())
    }
}

impl ReportMessage {
    // this is very much not a perfomant solution
    // but messages are not merged often enough for it to be a real issue
    pub fn merge(&self, new: &mut ReportMessage) -> ReportMessage {
        let mut merged = self.clone();

        //summary
        for e in &mut new.report_summary.closure_full {
            if !merged.report_summary.closure_full.contains(e) {
                merged.report_summary.closure_full.push(e.clone())
            }
        }
        for e_new in &mut new.report_summary.derivation {
            let mut contains = false;
            for e in merged.report_summary.derivation.clone() {
                if e.file_path == e_new.file_path {
                    contains = true;
                }
            }
            if !contains {
                merged.report_summary.derivation.push(e_new.clone());
            }
        }

        merged.report_summary.ci_jobs_done += new.report_summary.ci_jobs_done;
        merged.report_summary.ci_jobs_waiting = new.report_summary.ci_jobs_waiting;
        merged.report_summary.ci_jobs_sum =
            merged.report_summary.ci_jobs_done + merged.report_summary.ci_jobs_waiting;

        // detailed
        for e_new in &mut new.report_detailed.reproducible {
            let mut contains = false;
            for e in &mut merged.report_detailed.reproducible {
                if e.file_path == e_new.file_path {
                    contains = true;

                    let mut tmp = e.clone().job_toplevel.unwrap();
                    let mut new = e_new.clone().job_toplevel.unwrap();
                    tmp.append(&mut new);
                    tmp.sort_by(|a, b| a.job.cmp(&b.job));
                    tmp.dedup();
                    e.job_toplevel = Some(tmp);
                }
            }
            if !contains {
                merged.report_detailed.reproducible.push(e_new.clone());
            }
        }
        for e_new in &mut new.report_detailed.non_reproducible {
            let mut contains = false;
            for e in &mut merged.report_detailed.non_reproducible {
                if e.file_path == e_new.file_path {
                    contains = true;

                    let mut tmp = e.clone().job_toplevel.unwrap();
                    let mut new = e_new.clone().job_toplevel.unwrap();
                    tmp.append(&mut new);
                    tmp.sort_by(|a, b| a.job.cmp(&b.job));
                    tmp.dedup();
                    e.job_toplevel = Some(tmp);
                }
            }
            if !contains {
                merged.report_detailed.non_reproducible.push(e_new.clone());
            }
        }
        for e_new in &mut new.report_detailed.build_error {
            let mut contains = false;
            for e in &mut merged.report_detailed.build_error {
                if e.file_path == e_new.file_path {
                    contains = true;

                    let mut tmp = e.clone().job_toplevel.unwrap();
                    let mut new = e_new.clone().job_toplevel.unwrap();
                    tmp.append(&mut new);
                    tmp.sort_by(|a, b| a.job.cmp(&b.job));
                    tmp.dedup();
                    e.job_toplevel = Some(tmp);
                }
            }
            if !contains {
                merged.report_detailed.build_error.push(e_new.clone());
            }
        }

        for e_new in &mut new.report_detailed.no_entry {
            let mut contains = false;
            for e in &mut merged.report_detailed.no_entry {
                if e.file_path == e_new.file_path {
                    contains = true;

                    let mut tmp = e.clone().job_toplevel.unwrap();
                    let mut new = e_new.clone().job_toplevel.unwrap();
                    tmp.append(&mut new);
                    tmp.sort_by(|a, b| a.job.cmp(&b.job));
                    tmp.dedup();
                    e.job_toplevel = Some(tmp);
                }
            }
            if !contains {
                merged.report_detailed.no_entry.push(e_new.clone());
            }
        }

        merged
    }
    fn is_fully_reproducible(&self) -> bool {
        if self.report_detailed.non_reproducible.is_empty()
            && self.report_detailed.build_error.is_empty()
            && self.report_detailed.no_entry.is_empty()
        {
            return true;
        }
        false
    }
}

impl fmt::Display for ReportMessage {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.is_fully_reproducible() {
            write!(
                f,
                r#"# Reproducibility Report
{}
{}"#,
                self.report_summary, self.report_detailed,
            )
        } else {
            write!(
                f,
                r#"# Reproducibility Report
{}
{}

## tips to inspect
* checkout this commit `{}`
* build the package: `nix build .#default` (actual build command can vary see pipeline script)
* rebuild to get all logs for one derivation: `nix-build --check --option run-diff-hook true <store-derivation-path>`
* compare both versions with the diffoscope tool `nix run nixpkgs#diffoscope -- <outPath> <outPath>.check`"#,
                self.report_summary, self.report_detailed, self.commit_hash
            )
        }
    }
}

impl fmt::Display for ReportSummary {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let out: String = format!(
            r#"## Summary
Out of {} Closure Elements, {} are Derivations.

The CI Pipeline has {} known report requests.
{} done
{} ongoing"#,
            self.closure_full.len(),
            self.derivation.len(),
            self.ci_jobs_sum,
            self.ci_jobs_done,
            self.ci_jobs_waiting,
        );
        write!(f, "{out}")
    }
}

impl fmt::Display for ReportDetailed {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut out = format!(
            r#"## Detail
FYI: {} Derivations tested Reproducible"#,
            self.reproducible.len()
        );

        // create string blocks from all vectors in the detailed report
        let mut non_reproducible = String::from("");
        if !self.non_reproducible.is_empty() {
            non_reproducible = format!(
                r#"### Non-Reproducible
Counting {}
<details><summary>store derivations</summary>"#,
                self.non_reproducible.len()
            );
            for e in &self.non_reproducible {
                non_reproducible = format!(
                    r#"{}
<details><summary>{}</summary>

Documented Reason: {}
Tested {} times"#,
                    non_reproducible,
                    e,
                    e.error_reason.as_ref().unwrap(),
                    e.db_write_count.unwrap(),
                );
                for jobs in e.job_toplevel.clone().unwrap() {
                    non_reproducible = format!("{}\n* CI-Job: {}", non_reproducible, jobs.job,);
                    for toplevel in jobs.toplevels.clone() {
                        non_reproducible =
                            format!("{non_reproducible}\n\t* Derivation: {toplevel}",);
                    }
                }
                non_reproducible = format!("{non_reproducible}</details>",);
            }
            non_reproducible = format!("{non_reproducible}\n</details>",);
        }

        let mut build_error = String::from("");
        if !self.build_error.is_empty() {
            build_error = format!(
                r#"### Build-Error
Counting {}
After the CI pipeline should have successfully built this a failure to make an initial build or substitute is unlikely and an error that should be investigated.
<details><summary>store derivations</summary>"#,
                self.build_error.len()
            );
            for e in &self.build_error {
                build_error = format!("{build_error}\n* {e}");
            }
            build_error = format!("{build_error}\n</details>",);
        };

        let mut no_entry = String::from("");
        if !self.no_entry.is_empty() {
            no_entry = format!(
                r#"### No-Entry/Record
Counting {}
After testing there should always be some sort of record for each store derivation path unless some unknown error occured.
<details><summary>store derivations</summary>"#,
                self.build_error.len()
            );
            for e in &self.no_entry {
                no_entry = format!("{no_entry}\n* {e}",);
            }
            no_entry = format!("{no_entry}\n</details>",);
        };

        // connect string blocks
        if !non_reproducible.is_empty() {
            out = format!("{out}\n{non_reproducible}");
        }
        if !build_error.is_empty() {
            out = format!("{out}\n{build_error}");
        }
        if !no_entry.is_empty() {
            out = format!("{out}\n{no_entry}");
        }
        write!(f, "{out}")
    }
}
