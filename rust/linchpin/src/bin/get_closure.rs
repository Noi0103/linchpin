use linchpin::gitlab::PublisherMetadataGitlab;
use linchpin::report_request::Publisher;

use anyhow::{anyhow, Context, Error, Result};
use clap::Parser;
use log::debug;
use log::error;
use log::trace;
use nix_daemon::nix::DaemonStore;
use nix_daemon::Progress;
use nix_daemon::Store;
use reqwest::Client;
use tokio::process::Command;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

use std::env;
use std::fs;
use std::path::PathBuf;

//server uses axum bytes

#[cfg(target_has_atomic = "ptr")]
#[tokio::main]
async fn main() -> Result<(), Error> {
    use linchpin::report_request::{ClosureElement, ReportRequest};

    let cli = Cli::parse();

    let subscriber = FmtSubscriber::builder()
        .with_max_level(match cli.verbose {
            0 => Level::INFO,
            1 => Level::DEBUG,
            _ => Level::TRACE,
        })
        .finish();

    tracing::subscriber::set_global_default(subscriber)
        .context("Failed to set global tracing subscriber")?;

    // Log messages from the log crate as well.
    tracing_log::LogTracer::init()?;

    // get nix daemon socket
    let mut store = DaemonStore::builder()
        .connect_unix("/nix/var/nix/daemon-socket/socket")
        .await?;

    info!("hello world");

    // get store derivation via result symlink
    let store_derivations: Vec<String> = if cli.derivation.is_none() {
        let store_output_path = fs::read_link("result")?;
        info!("using result symlink");

        let store_output_path_str = store_output_path.to_str().unwrap();
        info!("output store path {store_output_path_str}");

        store
            .query_valid_derivers(store_output_path_str)
            .result()
            .await?
    } else {
        vec![cli.derivation.unwrap().to_str().unwrap().into()]
    };

    info!("deriver store paths {:?}", store_derivations);

    // get all build closure derivation paths
    // store_derivation_closure=$(nix-store --query --requisites "$store_derivation")

    let mut build_closure_strings: Vec<String> = vec![];
    let mut todo_derivations: Vec<String> = store_derivations.clone();
    let mut lower_level: Vec<String> = vec![];

    loop {
        lower_level.clear();
        debug!(
            "inspecting dependency tree level with count: {}",
            todo_derivations.len()
        );
        if todo_derivations.is_empty() {
            break;
        }
        for derivation in &todo_derivations {
            debug!("starting derivation: {derivation}");
            // get PathInfo object of given store drv path
            // SAFETY: The derivation path has been queried from the store above, a Some(PathInfo) is expected
            let path_info = store
                .query_pathinfo(derivation.clone())
                .result()
                .await?
                .unwrap();

            // collect next level of store derivation paths
            for reference in path_info.references {
                // if collected then dont do again
                if lower_level.contains(&reference) && build_closure_strings.contains(&reference) {
                    debug!("skip adding known dependency: {reference}");
                    continue;
                }
                debug!("found a new dependency: {reference}");
                lower_level.push(reference);
            }
            build_closure_strings.push(derivation.clone());
            debug!("finished derivation: {derivation}");
        }
        todo_derivations = lower_level.clone();
    }
    build_closure_strings.sort();
    build_closure_strings.dedup();
    info!(
        "build_closure_strings length: {}",
        build_closure_strings.len()
    );

    // TODO this will only use the first deriver and not every known one
    let mut build_closure: Vec<ClosureElement> = vec![];
    for derivation_string in &build_closure_strings {
        let closure_element: ClosureElement = derivation_string.clone().into();
        build_closure.push(closure_element);
    }

    // export nix store
    let mut export_arg = String::new();
    for elem in &build_closure_strings {
        export_arg = format!("{} {}", export_arg, elem);
    }
    debug!("export_arg: {export_arg}");
    let serialized_nix_store = nix_store_export(build_closure_strings)
        .await
        .expect("failed to export neccessary nix store slice")
        .stdout;
    info!(
        "aquired exported store with u8 len: {}",
        serialized_nix_store.len()
    );
    // do the rest specific to the publisher
    if cli.cli {
        handle_cli().expect("handle cli");
        // closure needs to be converted to ClosureElement(Derivation) and ClosureElement(Other)
        let report_request: ReportRequest = ReportRequest {
            store_derivation: store_derivations
                .first()
                .unwrap()
                .clone()
                .try_into()
                .expect("toplevel is not a drv"),
            store_derivation_closure: build_closure,
            publisher_data: Publisher::Cli(),
        };
        let report_request_string =
            serde_json::to_string(&report_request).expect("failed serde_json::to_string");

        let form = reqwest::multipart::Form::new()
            .part(
                "json",
                reqwest::multipart::Part::text(report_request_string),
            )
            .part(
                "closure",
                reqwest::multipart::Part::bytes(serialized_nix_store),
            );

        match Client::new().post(&cli.url).multipart(form).send().await {
            Ok(response) => {
                info!("api response raw: {:?}", response);
            }
            Err(e) => {
                error!("multipart did not send correctly: {e}");
            }
        };
        Ok(())
    } else if cli.gitlab {
        let meta_gitlab: PublisherMetadataGitlab =
            handle_gitlab().expect("collect gitlab metadata");
        let report_request: ReportRequest = ReportRequest {
            store_derivation: store_derivations
                .first()
                .unwrap()
                .clone()
                .try_into()
                .expect("toplevel is not a drv"),
            store_derivation_closure: build_closure,
            publisher_data: Publisher::Gitlab(meta_gitlab.clone()),
        };
        let report_request_string =
            serde_json::to_string(&report_request).expect("failed serde_json::to_string");

        let form = reqwest::multipart::Form::new()
            .part(
                "json",
                reqwest::multipart::Part::text(report_request_string),
            )
            .part(
                "closure",
                reqwest::multipart::Part::bytes(serialized_nix_store),
            );

        match Client::new().post(&cli.url).multipart(form).send().await {
            Ok(response) => {
                info!("api response raw: {:?}", response);
            }
            Err(e) => {
                error!("multipart did not send correctly: {e}");
            }
        };
        Ok(())
    } else {
        error!("not properly handled for a publisher");
        Err(anyhow!("not properly handled for a publisher"))
    }
}

/// run `nix-store --import` on a bytestream from `nix-store --export ...`
async fn nix_store_export(export_arg: Vec<String>) -> Result<std::process::Output, std::io::Error> {
    info!("making subprocess process");
    Command::new("nix-store")
        .arg("--export")
        .args(export_arg)
        .output()
        .await
}

fn handle_cli() -> Result<()> {
    Ok(())
}

fn handle_gitlab() -> Result<PublisherMetadataGitlab> {
    // collect chosen publisher info -> early fail
    let metadata = PublisherMetadataGitlab {
        ci_merge_request_project_id: env::var("ci_merge_request_project_id")?,
        ci_merge_request_iid: env::var("ci_merge_request_iid")?,
        ci_commit_sha: env::var("ci_commit_sha")?,
        ci_job_name: env::var("ci_job_name")?,
        ci_pipeline_id: env::var("ci_pipeline_id")?,
    };
    trace!("collected meta: {:#?}", metadata);

    Ok(metadata)
}

#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Enable verbose logging. Can be specified multiple times to increase verbosity.
    #[arg(short, long, action = clap::ArgAction::Count)]
    pub verbose: u8,
    /// socket address the tracking server is listening on; e.g. 127.0.0.1:8080
    #[arg(short, long)]
    pub url: String,

    // publishers
    /// publish report results to stdout
    #[arg(long, default_value_t = false, conflicts_with = "gitlab")]
    pub cli: bool,
    /// publish report results to gitlab, collect gitlab CI pipeline environment
    #[arg(long, default_value_t = false, conflicts_with = "cli")]
    pub gitlab: bool,

    /// derivation to use instead of the result symlink
    #[arg(short, long)]
    pub derivation: Option<PathBuf>,
}
