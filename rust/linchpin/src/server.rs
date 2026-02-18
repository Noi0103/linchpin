use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;

//use tokio::sync::Mutex;
//use tokio::sync::MutexGuard;

use axum::{
    body::Bytes,
    extract::{Multipart, State},
    http,
    response::IntoResponse,
    routing::{get, post},
    Router,
};

use crate::cli;
use crate::cli::Cli;
use crate::report_request::ReportRequest;
use crate::report_request_list::ReportRequestList;
use log::debug;
use log::info;

/// axum's Router can only take one with_state()
#[derive(Debug, Clone)]
pub struct AppState {
    /// the live list of reports shared between server and rebuilder thread
    pub shared_reports_list: Arc<Mutex<ReportRequestList>>,
    /// all cli arguments
    pub cli: Cli,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct MethodLabels {
    pub method: Method,
}

/// constructing the REST server application in the thread by adding sqlite database, socket address, a with rebuilder shared state and REST endpoints
pub async fn server(cli: cli::Cli, shared_reports_list: Arc<Mutex<ReportRequestList>>) {
    info!("HELLO WORLD SERVER");

    let socket_addr: std::net::SocketAddr = cli.socket_address;

    let app_state = AppState {
        shared_reports_list: Arc::clone(&shared_reports_list),
        cli: cli.clone(),
    };

    let app = Router::new()
        .route("/ping", get(handle_ping))
        .route("/report", post(handle_report))
        .with_state(app_state)
        .layer(axum::extract::DefaultBodyLimit::max(1000000000));

    let listener = tokio::net::TcpListener::bind(socket_addr).await.unwrap();
    tracing::debug!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app).await.unwrap();
}

/// simple check how many items are in the shared_reports_list ("testing todo list")
pub async fn handle_ping(State(app_state): State<AppState>) -> impl IntoResponse {
    println!("/PING");
    match app_state.shared_reports_list.is_poisoned() {
        true => "poisoned waitlist".to_string(),
        false => format!(
            "reports in waitlist: {}",
            app_state.shared_reports_list.lock().unwrap().len()
        ),
    }
}

struct ReportRequestMultipart {
    pub report_request_bytes: Bytes,
    pub nix_store_export_bytes: Bytes,
}

/// requesting a test
pub async fn handle_report(
    State(app_state): State<AppState>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    info!("/REPORT");

    let shared_reports_list = app_state.shared_reports_list;
    let cli = app_state.cli;

    // receive all data
    let mut multipart_data: ReportRequestMultipart = ReportRequestMultipart {
        report_request_bytes: Bytes::new(),
        nix_store_export_bytes: Bytes::new(),
    };

    while let Some(mut field) = multipart.next_field().await.unwrap() {
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
            debug!("parsed chunk of {} bytes", buffer.len());
        }

        let name = field.name().unwrap().to_string();
        debug!("Length of `{}` is {} bytes", name, buffer.len());

        match name {
            s if s == "json" => {
                multipart_data.report_request_bytes = Bytes::from(buffer.clone());
            }
            s if s == "closure" => {
                multipart_data.nix_store_export_bytes = Bytes::from(buffer.clone());
            }
            _ => return "malformed http multipart request",
        }
    }

    // TODO unsafe behaviour if multipart has only one of the parts

    // process json
    let report_request_json = match String::from_utf8(multipart_data.report_request_bytes.to_vec())
    {
        Ok(string) => string,
        Err(_) => return "json multipart malformed",
    };

    let report_request: ReportRequest = match serde_json::from_str(&report_request_json) {
        Ok(report_request) => report_request,
        Err(_) => return "json multipart malformed",
    };

    shared_reports_list
        .lock()
        .unwrap()
        .add_one_report(&report_request);
    shared_reports_list
        .lock()
        .unwrap()
        .save(&cli.savefile_path)
        .expect("failed saving");

    info!(
        "received closure paths for: {}",
        &report_request.store_derivation
    );

    // process nix store
    match nix_store_import(multipart_data.nix_store_export_bytes) {
        Ok(a) => match a.status.success() {
            true => {}
            false => return "importing closure failed",
        },
        Err(_) => return "importing closure failed",
    }

    // create gc symlinks
    match report_request
        .store_derivation
        .create_gc_root(&cli.gc_links_dir)
    {
        Ok(_) => {}
        Err(_) => return "gc root could not be created",
    }

    // respond with a success
    "report received"
}

/// run `nix-store --import` on a bytestream from `nix-store --export ...`
fn nix_store_import(exported_nix_store: Bytes) -> Result<std::process::Output, &'static str> {
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
            .write_all(&exported_nix_store)
            .expect("Failed to write to stdin");
    });

    match child.wait_with_output() {
        Ok(a) => Ok(a),
        Err(_) => Err("Failed to execute command"),
    }
}
