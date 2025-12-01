use std::path::{Path, PathBuf};
use std::{fs, io, process};

use serde::{Deserialize, Serialize};

/// determinsim state of the documented build inside the derivation
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub enum DerivationState {
    // initial build failed for some reason
    Error,
    /// recorded in the database without any information from a test
    NotTested,
    Reproducible,
    NonReproducible,
}

/// to make distinctions between other general errors and known errors
/// such as http error, hash mismatch that prevent a build
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub enum BuildError {
    None,
    UnknownError,
    NonDeterministic,
    HTTPError,
    HashMismatch,
    //InitialBuildError
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

impl Derivation {
    /// create a derivation without any optional values
    pub fn new(filepath: PathBuf) -> Result<Derivation, &'static str> {
        if is_derivation(&filepath) != Ok(true) {
            return Err("not a derivation file");
        }
        Ok(Derivation {
            file_path: filepath,
            state: None,
            error_reason: None,
            db_write_count: None,
            job_toplevel: None,
        })
    }

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
    pub async fn nix_build_check_remote(&self, nix_store: String) -> process::Output {
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
                "--check",
            ])
            .output()
            .await
            .expect("Failed to execute command")
    }

    /// should be used with the toplevel store derivation
    pub fn create_gc_root(
        &self,
        gc_links_path: PathBuf,
    ) -> Result<std::process::Output, &'static str> {
        if !gc_links_path.exists() {
            fs::create_dir_all(&gc_links_path)
                .expect("gc root symlinks directory can not be created");
        }

        let gc_link: PathBuf = Path::new(&gc_links_path).join(
            self.file_path
                .file_name()
                .expect("missing store derivation in path"),
        );
        if gc_link.exists() {
            return Err("symlink already exists");
        };

        println!("creating new symlink preventing garbage collection {gc_link:?}");

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

    pub fn delete_gc_root(&self, gc_links_path: PathBuf) -> Result<PathBuf, io::Error> {
        let gc_link: PathBuf = Path::new(&gc_links_path).join(
            self.file_path
                .file_name()
                .expect("missing store derivation in path"),
        );

        println!("deleting garbage collection link {gc_link:?}");

        match fs::remove_file(&gc_link) {
            Ok(_) => Ok(gc_link),
            Err(e) => Err(e),
        }
    }
}
/// delete all symlinks that prevent garbadge collection left by a prior process
pub fn reset_gc_root(gc_links_path: PathBuf) -> Result<bool, io::Error> {
    if !gc_links_path.exists() {
        fs::create_dir_all(&gc_links_path).expect("gc root symlinks directory can not be created");
    }

    let content = fs::read_dir(gc_links_path)?;

    for entry in content {
        let path = entry.expect("reset gc failed").path();
        match fs::remove_file(&path) {
            Ok(_) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(true)
}

/// get all derivation paths that are protected by a symlink in the configured directory
pub fn active_gc_roots(gc_links_path: PathBuf) -> Result<Vec<PathBuf>, io::Error> {
    let mut gc_symlinks: Vec<PathBuf> = vec![];
    for entry in (fs::read_dir(gc_links_path)?).flatten() {
        if entry.file_type()?.is_symlink() {
            gc_symlinks.push(entry.path());
        }
    }
    Ok(gc_symlinks)
}

fn is_derivation(store_derivation: &std::path::Path) -> Result<bool, ()> {
    let derivation_file_extension = std::ffi::OsStr::new("drv");
    match store_derivation.extension() {
        Some(some_extension) => {
            if some_extension == derivation_file_extension {
                Ok(true)
            } else {
                Ok(false)
            }
        }
        _ => Ok(false),
    }
}

pub fn parse_nix_build_error(text: String) -> Option<BuildError> {
    if text.contains("URL returned error:") || text.contains("HTTP error") {
        return Some(BuildError::HTTPError);
    }

    if text.contains("hash mismatch") {
        return Some(BuildError::HashMismatch);
    }

    if text.contains("may not be deterministic") {
        return Some(BuildError::NonDeterministic);
    }

    Some(BuildError::UnknownError)
}

impl std::fmt::Display for DerivationState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state_str = match self {
            DerivationState::Error => "Error",
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
        };
        write!(f, "{state_str}")
    }
}

impl std::fmt::Display for Derivation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let string = String::from(self.file_path.to_str().unwrap());
        write!(f, "{string}")
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
        assert_eq!(parse_nix_build_error(text), Some(BuildError::HashMismatch))
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
        assert_eq!(parse_nix_build_error(text), Some(BuildError::HTTPError))
    }

    #[test]
    fn find_parse_non_deterministic_error() {
        let text = String::from("error: derivation '/nix/store/iyx9i1aqh6r4wxd7xc5bbyz1693ifj1r-unstable.drv' may not be deterministic: output '/nix/store/7dy5j86rkc09fhnx6irmpmcx59yaxs9m-unstable' differs");
        assert_eq!(
            parse_nix_build_error(text),
            Some(BuildError::NonDeterministic)
        )
    }

    #[test]
    fn filepath_is_derivation() {
        let filepath = Path::new("/tmp/file.drv");
        assert_eq!(is_derivation(filepath), Ok(true));

        let filepath = Path::new("/tmp/file.json");
        assert_eq!(is_derivation(filepath), Ok(false));
    }
}
