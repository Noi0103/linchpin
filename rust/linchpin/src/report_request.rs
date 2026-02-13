// https://serde.rs/enum-representations.html
// TODO externally tagged (new multipart), adjacently tagged (type-value pair),

// TODO history

use anyhow::Error;
use anyhow::Result;
use log::debug;
use log::info;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::gitlab::PublisherMetadataGitlab;
use crate::nix_derivation::{Derivation, DerivationState};
use crate::Database;

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct ReportRequest {
    /// toplevel store derivation
    pub store_derivation: Derivation,
    /// closure of toplevel store derivation
    pub store_derivation_closure: Vec<ClosureElement>,
    // generic metadata object to be able to publish results on the respective publishers
    pub publisher_data: Publisher,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ClosureElement {
    Other(String),
    Derivation(Derivation),
}

impl From<String> for ClosureElement {
    fn from(string: String) -> Self {
        debug!("From<String> for ClosureElement: {string}");
        match Derivation::try_from(string.clone()) {
            Ok(derivation) => ClosureElement::Derivation(derivation),
            Err(_) => ClosureElement::Other(string),
        }
    }
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[serde(tag = "publisher", content = "value")]
pub enum Publisher {
    Cli(),
    Gitlab(PublisherMetadataGitlab),
}

impl ReportRequest {
    pub fn get_toplevel_derivations(&self, derivation: Derivation) -> &Derivation {
        todo!()
    }

    /// Return a Vector of all Derivations of the Closure.
    pub fn get_derivations(&self) -> Vec<&Derivation> {
        self.store_derivation_closure
            .iter()
            .filter_map(|ce| {
                if let ClosureElement::Derivation(ref d) = ce {
                    Some(d)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Return a Vector of all Derivations of the Closure with the current state given.
    pub fn get_derivations_filtered(&self, wanted_state: DerivationState) -> Vec<&Derivation> {
        match wanted_state {
            DerivationState::Reproducible => self
                .get_derivations()
                .iter()
                .copied()
                .filter(|d| d.state == Some(DerivationState::Reproducible))
                .collect(),
            DerivationState::NonReproducible => self
                .get_derivations()
                .iter()
                .copied()
                .filter(|d| d.state == Some(DerivationState::NonReproducible))
                .collect(),
            DerivationState::BuildError => self
                .get_derivations()
                .iter()
                .copied()
                .filter(|d| d.state == Some(DerivationState::BuildError))
                .collect(),
            DerivationState::NotTested => self
                .get_derivations()
                .iter()
                .copied()
                .filter(|d| d.state == Some(DerivationState::NotTested))
                .collect(),
        }
    }

    /// Store a report_request to prevent forgetting it in a crash.
    pub fn save(&self, path: PathBuf) -> Result<(), Error> {
        let json: String = serde_json::to_string(&self).expect("parse to json-string failed");
        std::fs::write(path, json)?;
        info!("wrote report_request savefile: {}", self.store_derivation);
        Ok(())
    }

    /// Load a report_request and lookup the status in the database
    pub fn lookup(&mut self, path: PathBuf, database: &Database) {
        for index in 0..self.store_derivation_closure.len() {
            match &self.store_derivation_closure[index] {
                ClosureElement::Derivation(derivation) => {
                    let mut lookup = database
                        .lookup_store_derivation(derivation.clone().try_into().unwrap())
                        .expect("lookup in database failed");
                    match lookup.pop() {
                        Some(lookup_derivation) => {
                            self.store_derivation_closure[index] =
                                ClosureElement::Derivation(lookup_derivation);
                        }
                        None => {}
                    }
                }
                ClosureElement::Other(_) => {}
            }
        }
    }

    /// lookup what has been tested already and determine what is either not yet tested or has attempts left in case a network error previously caused a failure
    pub fn get_untested_derivations(
        self,
        database: Database,
        max_rebuild_tries: i32,
    ) -> Result<Vec<Derivation>, Error> {
        let mut untested: Vec<Derivation> = vec![];
        let closure = self.store_derivation_closure.clone();
        for elem in closure {
            match elem {
                ClosureElement::Derivation(drv) => {
                    let derivation_string = drv.file_path.display().to_string();
                    let mut lookup = database.lookup_store_derivation(derivation_string)?;
                    if lookup.len() > 0 {
                        debug!("no entry found {drv}");
                        untested.push(drv);
                    }
                    for elem in lookup {
                        match elem.state {
                            Some(DerivationState::BuildError) => {
                                debug!("lookup found with BuildError: {elem}");
                                untested.push(elem);
                            }
                            Some(DerivationState::NonReproducible) => {
                                debug!("lookup found with NonReproducible: {elem}");
                                // TODO why is this unwrap safe or not safe
                                let db_write_count = elem.db_write_count.unwrap();
                                if db_write_count > max_rebuild_tries {
                                    debug!("already did enough attempts: {}", db_write_count);
                                    continue;
                                }
                            }
                            Some(DerivationState::NotTested) => {
                                debug!("lookup found with NotTested: {elem}");
                                untested.push(elem);
                            }
                            Some(DerivationState::Reproducible) => {
                                debug!("lookup found with Reproducible: {elem}");
                                info!("known to be reproducible and skipping: {elem}")
                            }
                            None => {
                                debug!("found entry missing a state value: {elem}");
                                untested.push(elem);
                            }
                        }
                    }
                }
                ClosureElement::Other(_) => {}
            }
        }
        Ok(untested)
    }

    pub fn print_summary(&self) {
        info!(
            "full closure count: {}",
            self.store_derivation_closure.len()
        );

        // all derivations count
        let derivations: Vec<Derivation> = self
            .store_derivation_closure
            .iter()
            .cloned()
            .filter_map(|x| {
                if let ClosureElement::Derivation(derivation) = x {
                    Some(derivation.clone())
                } else {
                    None
                }
            })
            .collect();
        info!("derivation count {}", derivations.len());

        // reproducible derivations count
        let derivations_reproducible: Vec<Derivation> = derivations
            .iter()
            .cloned()
            .filter(|x| {
                &DerivationState::Reproducible
                    == x.state.as_ref().unwrap_or(&DerivationState::NotTested)
            })
            .collect();
        info!("reproducible count {}", derivations_reproducible.len());

        // non reproducible derivations count
        let derivations_non_reproducible: Vec<Derivation> = derivations
            .iter()
            .cloned()
            .filter(|x| {
                &DerivationState::NonReproducible
                    == x.state
                        .as_ref()
                        .unwrap_or(&DerivationState::NonReproducible)
            })
            .collect();
        info!(
            "non-reproducible count {}",
            derivations_non_reproducible.len()
        );
    }
}
