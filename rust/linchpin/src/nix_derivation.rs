use std::path::{Path, PathBuf};
use std::{fs, io, process};

use anyhow::{anyhow, Error, Ok, Result};
use log::{debug, warn};
use serde::{Deserialize, Serialize};

use crate::database::Database;

/// determinism of the last documented build
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub enum DerivationState {
    // initial build failed for some reason
    BuildError,
    /// recorded in the database without any information from a test
    NotTested,
    Reproducible,
    NonReproducible,
}

/// to make distinctions between other general errors and known errors
/// such as http error, hash mismatch that prevent to even try a build
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub enum BuildError {
    None,
    UnknownError,
    NonDeterministic,
    HTTPError,
    HashMismatch,
    InitialBuildError,
}

/// store derivation representation
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Derivation {
    pub file_path: PathBuf,
    pub state: Option<DerivationState>,
    pub error_reason: Option<BuildError>,
    pub db_write_count: Option<i32>,
    // pub last_modified: Option<?>
    pub job_toplevel: Option<Vec<JobToplevel>>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct JobToplevel {
    pub job: String,
    pub toplevels: Vec<String>,
}

impl TryFrom<String> for Derivation {
    type Error = Error;
    fn try_from(string: String) -> Result<Derivation> {
        debug!("TryFrom<String> for Derivation: {string}");
        if !string.ends_with(".drv") {
            return Err(anyhow!("not a .drv file"));
        }
        let file_path = PathBuf::from(string);
        Ok(Derivation {
            file_path,
            state: None,
            error_reason: None,
            db_write_count: None,
            job_toplevel: None,
        })
    }
}
impl From<Derivation> for String {
    fn from(value: Derivation) -> Self {
        // SAFETY: a derivation is always created with a filepath ending on .drv
        String::from(value.file_path.to_str().unwrap())
    }
}
impl Derivation {
    /// run `nix-build ...`
    pub async fn nix_build_remote(&self, nix_store: String) -> process::Output {
        let store_derivation_path = &self.file_path.to_str().expect("PathBuf to str error");

        tokio::process::Command::new("nix-build")
            .args([
                store_derivation_path,
                "--eval-store",
                "auto",
                "--store",
                &nix_store,
                "--max-jobs",
                "0",
            ])
            .output()
            .await
            .expect("Failed to execute command")
    }

    /// run `nix-build --check ...`
    pub async fn nix_build_check_remote(&self, nix_store: &str) -> process::Output {
        let store_derivation_path = &self.file_path.to_str().expect("PathBuf to str error");

        tokio::process::Command::new("nix-build")
            .args([
                store_derivation_path,
                "--eval-store",
                "auto",
                "--store",
                nix_store,
                "--max-jobs",
                "0",
                "--check",
            ])
            .output()
            .await
            .expect("Failed to execute command")
    }

    /// should be used with the toplevel store derivation
    pub fn create_gc_root(&self, gc_links_path: &PathBuf) -> Result<std::process::Output, Error> {
        if !gc_links_path.exists() {
            fs::create_dir_all(gc_links_path)
                .expect("gc root symlinks directory can not be created");
        }

        let gc_link: PathBuf = Path::new(&gc_links_path).join(
            self.file_path
                .file_name()
                .expect("missing store derivation in path"),
        );
        if gc_link.exists() {
            warn!("symlink already exists for {}", self);
            return Err(anyhow!("symlink already exists"));
        };

        debug!("creating new symlink at {gc_link:?}");

        let store_derivation_path = &self.file_path.to_str().expect("PathBuf to str error");
        let output = process::Command::new("nix")
            .args([
                "build",
                "--out-link",
                gc_link.to_str().unwrap(),
                store_derivation_path,
            ])
            .output()
            .expect("Failed to execute command");
        Ok(output)
    }

    pub fn delete_gc_root(&self, gc_links_path: &PathBuf) -> io::Result<()> {
        let gc_link: PathBuf = Path::new(gc_links_path).join(
            self.file_path
                .file_name()
                .expect("missing store derivation in path"),
        );

        fs::remove_file(&gc_link)
    }

    /// helper function to do the initial `nix-build``, the `nix-build --check`` and the sqlite database upsert
    pub async fn build_rebuild_upsert(
        &self,
        database: &Database,
        nix_store: &str,
    ) -> Result<Derivation> {
        debug!("building {self}");
        let mut derivation = self.clone();
        let result = self.nix_build_remote(nix_store.to_owned()).await;

        // initial build failed
        if !result.status.success() {
            derivation.state = Some(DerivationState::BuildError);
            derivation.error_reason = Some(BuildError::InitialBuildError);

            database
                .upsert_store_derivation(derivation)
                .expect("sqlite update error");
            return Err(anyhow!("initial build failed"));
        };

        debug!("rebuilding: {self}");
        let result = self.nix_build_check_remote(nix_store).await;

        if result.status.success() {
            debug!("seems reproducible: {self}");
            derivation.state = Some(DerivationState::Reproducible);
        } else {
            debug!("is non-reproducible (or build error): {self}");
            let stderr: String = String::from_utf8_lossy(&result.stderr).to_string();
            let build_error: BuildError = parse_nix_build_error(stderr);
            derivation.state = Some(DerivationState::NonReproducible);
            derivation.error_reason = Some(build_error);
        }

        database
            .upsert_store_derivation(derivation.clone())
            .expect("sqlite update error");

        Ok(derivation)
    }
}

/// delete all symlinks that prevent garbadge collection left by a prior process
pub fn reset_gc_root(gc_links_path: &PathBuf) -> Result<()> {
    debug!("reset gc links");
    if !gc_links_path.exists() {
        return Ok(());
    }

    let content = fs::read_dir(gc_links_path)?;

    for entry in content {
        let path = entry.expect("reset gc failed").path();
        fs::remove_file(&path)?;
        debug!("removed gc: {:?}", path);
    }
    Ok(())
}

/// get all derivation paths that are protected by a symlink in the configured directory
pub fn active_gc_roots(gc_links_path: PathBuf) -> Result<Vec<PathBuf>, Error> {
    let mut gc_symlinks: Vec<PathBuf> = vec![];
    for entry in (fs::read_dir(gc_links_path)?).flatten() {
        if entry.file_type()?.is_symlink() {
            gc_symlinks.push(entry.path());
        }
    }
    Ok(gc_symlinks)
}

pub fn parse_nix_build_error(text: String) -> BuildError {
    if text.contains("URL returned error:") || text.contains("HTTP error") {
        return BuildError::HTTPError;
    }
    if text.contains("hash mismatch") {
        return BuildError::HashMismatch;
    }
    if text.contains("may not be deterministic") {
        return BuildError::NonDeterministic;
    }
    BuildError::UnknownError
}

impl std::fmt::Display for DerivationState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state_str = match self {
            DerivationState::BuildError => "Error",
            DerivationState::NotTested => "NotTested",
            DerivationState::Reproducible => "Reproducible",
            DerivationState::NonReproducible => "NonReproducible",
        };
        write!(f, "{state_str}")
    }
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state_str = match self {
            BuildError::None => "",
            BuildError::UnknownError => "UnknownError",
            BuildError::HTTPError => "HTTPError",
            BuildError::HashMismatch => "HashMismatch",
            BuildError::NonDeterministic => "NonDeterministic",
            BuildError::InitialBuildError => "InitialBuildError",
        };
        write!(f, "{state_str}")
    }
}

impl std::fmt::Display for Derivation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // SAFETY: initializing a derivation requires path ending on .drv
        let string = String::from(self.file_path.file_name().unwrap().to_str().unwrap());
        write!(f, "/nix/store/{string}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // TODO those might break with version bumps
    #[test]
    fn find_parse_hash_mismatch() {
        let text = String::from("
        error: hash mismatch in fixed-output derivation '/nix/store/1dnnlz39jh7bj21piq0ing8bw5ls8br9-fluidicon.png.drv':
         specified: sha256-MYTYPOfhW0+ecSJfJKMGdPZZkpxiSvcGJaFbwIfPAvI=
            got:    sha256-3Nls7yfhW0+ecSJfJKMGdPZZkpxiSvcGJaFbwIfPAvI=");
        assert_eq!(parse_nix_build_error(text), BuildError::HashMismatch)
    }
    #[test]
    fn find_parse_http_error() {
        let text = String::from("error: builder for '/nix/store/1dnnlz39jh7bj21piq0ing8bw5ls8br9-fluidicon.png.drv' failed with exit code 1;
        last 17 log lines:
       >
       > trying https://github.com/fluidiconHIIAMBREAKINGSTUFF.png
       >   % Total    % Received % Xferd  Average Speed   Time    Time     Time  Current
       >                                  Dload  Upload   Total   Spent    Left  Speed
       >   0     0    0     0    0     0      0      0 --:--:-- --:--:-- --:--:--     0
       > curl: (22) The requested URL returned error: 404
       > Warning: Problem (retrying all errors). Will retry in 1 second. 3 retries left.
       >   0     0    0     0    0     0      0      0 --:--:-- --:--:-- --:--:--     0
       > curl: (22) The requested URL returned error: 404
       > Warning: Problem (retrying all errors). Will retry in 2 seconds. 2 retries
       > Warning: left.
       >   0     0    0     0    0     0      0      0 --:--:-- --:--:-- --:--:--     0
       > curl: (22) The requested URL returned error: 404
       > Warning: Problem (retrying all errors). Will retry in 4 seconds. 1 retry left.
       >   0     0    0     0    0     0      0      0 --:--:-- --:--:-- --:--:--     0
       > curl: (22) The requested URL returned error: 404
       > error: cannot download fluidiconHIIAMBREAKINGSTUFF.png from any mirror");
        assert_eq!(parse_nix_build_error(text), BuildError::HTTPError)
    }

    #[test]
    fn find_parse_non_deterministic_error() {
        let text = String::from("error: derivation '/nix/store/iyx9i1aqh6r4wxd7xc5bbyz1693ifj1r-unstable.drv' may not be deterministic: output '/nix/store/7dy5j86rkc09fhnx6irmpmcx59yaxs9m-unstable' differs");
        assert_eq!(parse_nix_build_error(text), BuildError::NonDeterministic)
    }
}
