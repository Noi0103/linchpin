use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{Args, Parser};

/// A service to rebuild every element of a Nix build closures sent to it and report the results as a GitLab merge request comment.
#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    /// Enable verbose logging. Can be specified multiple times to increase verbosity.
    #[arg(short, long, action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// sqlite filepath to track tested store derivations; e.g. "/your/path/server.db"
    #[arg(short, long)]
    pub db_file: PathBuf,
    /// socket address the tracking server is listening on; e.g. 127.0.0.1:8080
    #[arg(short, long)]
    pub socket_address: SocketAddr,

    /// enable gitlab as publisher and supply args for it
    #[command(flatten)]
    pub gitlab: Option<Gitlab>,

    /// used with `nix-build [paths] ... --store <...>`
    #[arg(short, long, default_value_t = String::from("ssh-ng://localhost"))]
    pub nix_store: String,
    /// used to run multiple nix-build commands at once
    /// depending on the machine you can balance I/O wait times and out of memory
    #[arg(long, default_value_t = 1)]
    pub simultaneous_builds: usize,
    /// the location where symlinks will be placed to protect needed derivation files from automatic garbage collection
    #[arg(long, default_value = PathBuf::from("/tmp/linchpin/gc-roots").into_os_string())]
    pub gc_links_dir: PathBuf,
    /// load and continue reports that were not finished after restarting the program
    #[arg(long, default_value_t = false)]
    pub persistent_reports: bool,
    /// filepath for saving unfinished reports
    #[arg(long, default_value = PathBuf::from("/tmp/linchpin/savefile.json").into_os_string())]
    pub savefile_path: PathBuf,
    /// filepath for saving unfinished reports
    #[arg(long, default_value = PathBuf::from("/tmp/linchpin/comment-history.json").into_os_string())]
    pub savefile_history_path: PathBuf,
    /// how often given the chance a rebuild should be done until it will be skipped
    /// when skipped the database entry is used at face value
    #[arg(long, default_value_t = 10)]
    pub max_rebuild_tries: i32,
}

#[derive(Args, Debug, Clone)]
pub struct Gitlab {
    /// enable gitlab and supply new parameters
    #[arg(long, requires = "gitlab_url", requires = "gitlab_url")]
    pub gitlab: bool,
    /// Gitlab domain to send merge request comments via api; e.g. "https://mygit.domain.com"
    #[arg(long)]
    pub gitlab_url: Option<String>,
    /// A path to a file containing the Gitlab API token.
    #[arg(long, default_value = PathBuf::from("/tmp/linchpin/gc-roots").into_os_string())]
    pub gitlab_api_token_file: Option<PathBuf>,
}
