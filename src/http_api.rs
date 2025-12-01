use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

use axum::{
    body::{Body, Bytes},
    extract::{Multipart, State},
    http,
    http::{header::CONTENT_TYPE, StatusCode},
    response::{IntoResponse, Response},
};
use prometheus_client::encoding::{EncodeLabelSet, EncodeLabelValue};
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::gauge::Gauge;
use prometheus_client::registry::Registry;
use serde::{Deserialize, Serialize};

use crate::active_gc_roots;
use crate::shared_reports_list_pipeline_ids;
use crate::{add_report, healthy_length, nix_derivation, save_shared_reports_list};

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
pub struct ReportBody {
    /// toplevel store derivation
    pub store_derivation: String,
    /// closure of toplevel store derivation
    pub store_derivation_closure: Vec<String>,
    /// repository id
    pub ci_merge_request_project_id: String,
    /// merge request id
    pub ci_merge_request_iid: String,
    /// hash of commit used
    pub ci_commit_sha: String,
    /// name of the job shown in the pipeline webinterface
    pub ci_job_name: String,
    /// The instance-level ID of the current pipeline. This ID is unique across all projects on the GitLab instance.
    pub ci_pipeline_id: String,
}

/// axum's Router can only take one with_state()
#[derive(Debug, Clone)]
pub struct AppState {
    /// the live list of reports shared between server and rebuilder thread
    pub shared_reports_list: Arc<Mutex<VecDeque<ReportBody>>>,
    /// see config args
    pub gc_links_path: PathBuf,
    /// see config args
    pub savefile_path: PathBuf,

    pub registry: Arc<Mutex<Registry>>,
    pub metrics: Arc<Mutex<Metrics>>,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelValue)]
pub enum Method {
    Get,
    Post,
}
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct MethodLabels {
    pub method: Method,
}
#[derive(Clone, Debug)]
pub struct Metrics {
    pub requests: Family<MethodLabels, Counter>,
    pub current_len: Gauge,
    pub comment_history_len: Gauge,
    pub active_gc_roots: Gauge,
    pub number_of_pipeline_ids: Gauge,
    pub tokio_workers_count: Gauge,
    pub tokio_total_park_count: Gauge,
}
impl Metrics {
    pub fn inc_requests(&self, method: Method) {
        self.requests.get_or_create(&MethodLabels { method }).inc();
    }
}

/// simple check how many items are in the shared_reports_list ("testing todo list")
pub async fn ping(State(app_state): State<AppState>) -> impl IntoResponse {
    println!("/PING");
    app_state
        .metrics
        .lock()
        .expect("editing metrics")
        .inc_requests(Method::Get);

    let shared_reports_list = app_state.shared_reports_list;

    match healthy_length(shared_reports_list) {
        Ok(a) => format!("reports in waitlist: {a}"),
        Err(_) => "poisoned waitlist".to_string(),
    }
}

/// simple check how many items are in the shared_reports_list ("testing todo list")
pub async fn metrics(State(app_state): State<AppState>) -> impl IntoResponse {
    let mut buffer = String::new();
    match app_state.registry.lock() {
        Ok(registry) => {
            prometheus_client::encoding::text::encode(&mut buffer, &registry).unwrap();
            Response::builder()
                .status(StatusCode::OK)
                .header(
                    CONTENT_TYPE,
                    "text/plain; version=0.0.4; charset=utf-8; escaping=underscores",
                )
                .body(Body::from(buffer))
                .unwrap()
        }
        Err(e) => {
            println!("error on metrics url: {e}");
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from(buffer))
                .unwrap()
        }
    }

    //prometheus_client::encoding::text::encode(&mut buffer, &registry).unwrap();
}

/// requesting a test
pub async fn report(
    State(app_state): State<AppState>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    println!("/REPORT");
    app_state
        .metrics
        .lock()
        .expect("editing metrics")
        .inc_requests(Method::Post);

    let shared_reports_list = app_state.shared_reports_list;
    let gc_links_path = app_state.gc_links_path;
    let savefile_path = app_state.savefile_path;
    let metrics = app_state.metrics;

    let mut body: Option<ReportBody> = None;
    while let Some(mut field) = multipart.next_field().await.unwrap() {
        let name = field.name().unwrap().to_string();

        let mut buffer = bytes::BytesMut::with_capacity(0);
        while let Some(chunk) = match field
            .chunk()
            .await
            .map_err(|err| (http::StatusCode::BAD_REQUEST, err.to_string()))
        {
            Ok(a) => a,
            Err(_) => return "data chunk transport error",
        } {
            buffer.extend_from_slice(&chunk);
        }

        println!("parsed chunk of {} bytes", buffer.len());

        let data = Bytes::from(buffer);

        println!("Length of `{}` is {} bytes", name, data.len());

        // parse multipart http request
        match name {
            // part "json" holds metadata and the listed closure
            a if a == "json" => {
                match String::from_utf8(data.to_vec()) {
                    Ok(string) => {
                        body = match serde_json::from_str(&string) {
                            Ok(a) => a,
                            Err(_) => return "json parse error",
                        };
                        let report: ReportBody = body.clone().unwrap();
                        match add_report(
                            shared_reports_list.clone(),
                            report.clone(),
                            metrics.clone(),
                        ) {
                            Ok(()) => (),
                            Err(e) => return e,
                        };

                        println!(
                            "received closure paths for: {}",
                            &body.clone().unwrap().store_derivation
                        );
                    }
                    Err(_) => return "Failed to convert bytes to string: {}",
                };
            }
            // a partial export of the nix store including every closure element
            a if a == "closure" => match nix_store_import(data) {
                Ok(a) => match a.status.success() {
                    true => {}
                    false => return "importing closure failed",
                },
                Err(_) => return "importing closure failed",
            },
            _ => {}
        }
    }
    // symlink derivations against automatic garbadge collection
    match body {
        Some(a) => {
            let derivation = nix_derivation::Derivation::new(PathBuf::from(&a.store_derivation))
                .expect("body couldn't convert");

            match derivation.create_gc_root(gc_links_path.clone()) {
                Ok(_) => {}
                Err("symlink already exists") => {
                    println!("symlink already existing for unknown reasons")
                }
                Err(_) => {
                    panic!("symlink creation error")
                }
            }
            {
                let metrics = metrics.lock().expect("lock and get metrics");
                let active_gc_symlinks: i64 = active_gc_roots(gc_links_path.clone())
                    .expect("gc dir read error")
                    .len()
                    .try_into()
                    .unwrap();
                metrics.active_gc_roots.set(active_gc_symlinks);

                let pipeline_ids: i64 =
                    shared_reports_list_pipeline_ids(shared_reports_list.clone())
                        .len()
                        .try_into()
                        .unwrap();
                metrics.number_of_pipeline_ids.set(pipeline_ids);
            }
        }
        None => return "body not successfully parsed",
    };
    match save_shared_reports_list(shared_reports_list.clone(), savefile_path.clone()) {
        Ok(()) => {}
        Err(_) => return "something went wrong",
    };

    "report received"
}

/// run `nix-store --import` on a bytestream from `nix-store --export ...`
fn nix_store_import(serialized_derivations: Bytes) -> Result<std::process::Output, &'static str> {
    let mut child = match Command::new("nix-store")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .args(["--import"])
        .spawn()
    {
        Ok(a) => a,
        Err(_) => return Err("failed to spawn child process"),
    };

    let mut stdin = match child.stdin.take() {
        Some(a) => a,
        _ => return Err("failed to get stdin of child process"),
    };
    thread::spawn(move || {
        stdin
            .write_all(&serialized_derivations)
            .expect("Failed to write to stdin");
    });

    match child.wait_with_output() {
        Ok(a) => Ok(a),
        Err(_) => Err("Failed to execute command"),
    }
}
