use std::path::Path;
use std::path::PathBuf;
use std::time;

use log::debug;
use rusqlite::Connection;

use crate::nix_derivation;
use crate::nix_derivation::Derivation;
use crate::nix_derivation::DerivationState;

#[derive(Clone, Debug)]
pub struct Database {
    pub db_path: PathBuf,
}

impl Database {
    pub fn new(db_path: PathBuf) -> Database {
        Database { db_path }
    }
    /// if it does not exist, create the database with
    /// - the main table itself
    /// - the trigger for a retries count and a last modified date
    pub fn initialize(&self) -> Result<(), rusqlite::Error> {
        let conn = Connection::open(&self.db_path)?;
        conn.execute(
            "
            CREATE TABLE IF NOT EXISTS store_derivations(
              store_derivation TEXT PRIMARY KEY,
              store_derivation_state TEXT,
              error_reason TEXT,
              count_writes INTEGER DEFAULT 1,
              last_modified DATETIME DEFAULT CURRENT_TIMESTAMP
            );
            ",
            [],
        )?;
        conn.execute(
            "
            CREATE TRIGGER IF NOT EXISTS last_modified_timestamp
            AFTER UPDATE ON store_derivations
            FOR EACH ROW
            BEGIN
              UPDATE store_derivations SET count_writes = count_writes + 1 WHERE store_derivation = OLD.store_derivation;
              UPDATE store_derivations SET last_modified = CURRENT_TIMESTAMP WHERE store_derivation = OLD.store_derivation;
            END;
            ",
            [],
        )?;
        Ok(())
    }

    /// get all entries that fit the store_derivation_path key
    /// this should only be one but the return type for lookups is always vec
    /// can return an empty vector
    pub fn lookup_store_derivation(
        &self,
        store_derivation_path: String,
    ) -> Result<Vec<nix_derivation::Derivation>, rusqlite::Error> {
        let conn = Connection::open(&self.db_path)?;
        conn.busy_timeout(time::Duration::new(60, 0))
            .expect("failed to set sqlite busy timeout");

        let mut stmt =
            conn.prepare("SELECT * FROM store_derivations WHERE store_derivation = ?1;")?;
        let result = stmt
            .query_map([store_derivation_path], |row| {
                let d = nix_derivation::Derivation {
                    file_path: PathBuf::from(row.get::<_, String>(0)?),
                    state: row.get(1)?,
                    error_reason: row.get(2)?,
                    db_write_count: row.get(3)?,
                    job_toplevel: None,
                };
                Ok(d)
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(result)
    }

    /// inserted values: store_derivation TEXT PRIMARY KEY, store_derivation_state TEXT, error_reason TEXT,
    pub fn upsert_store_derivation(
        &self,
        entry: nix_derivation::Derivation,
    ) -> Result<(), rusqlite::Error> {
        let conn = rusqlite::Connection::open(&self.db_path)?;
        conn.busy_timeout(time::Duration::new(60, 0))
            .expect("failed to set sqlite busy timeout");

        let error_reason: String = match entry.error_reason {
            Some(e) => e.to_string(),
            None => String::new(),
        };
        let _updated_rows = conn.execute(
            "
            INSERT INTO store_derivations(store_derivation, store_derivation_state, error_reason)
            VALUES (?1, ?2, ?3)
            ON CONFLICT(store_derivation) DO
            UPDATE SET store_derivation_state = ?2, error_reason = ?3
            ",
            rusqlite::params![entry.file_path.to_str().unwrap(), entry.state, error_reason],
        )?;
        Ok(())
    }

    /// database lookups for every derivation in the list
    pub fn collect_report_results(
        &self,
        derivations_from_closure: Vec<nix_derivation::Derivation>,
    ) -> Vec<nix_derivation::Derivation> {
        // collect all new lookup entries and form report
        let mut lookup_sum: Vec<nix_derivation::Derivation> = vec![];

        for element in derivations_from_closure.clone() {
            let lookup: Vec<nix_derivation::Derivation> = self
                .lookup_store_derivation(element.file_path.to_str().unwrap().to_string())
                .expect("sqlite lookup error");
            match lookup.is_empty() {
                true => {
                    lookup_sum.push(Derivation {
                        file_path: element.file_path.clone(),
                        state: Some(DerivationState::NotTested),
                        error_reason: element.error_reason.clone(),
                        db_write_count: element.db_write_count,
                        job_toplevel: element.job_toplevel.clone(),
                    });
                }
                false => {
                    // lookup always returns a vector even if only one entry is found
                    for lookup_entry in lookup {
                        lookup_sum.push(lookup_entry);
                    }
                }
            }
        }

        lookup_sum
    }
}

impl rusqlite::ToSql for nix_derivation::DerivationState {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        let value = self.to_string();
        Ok(rusqlite::types::ToSqlOutput::from(value))
    }
}

impl rusqlite::types::FromSql for nix_derivation::DerivationState {
    fn column_result(
        value: rusqlite::types::ValueRef<'_>,
    ) -> Result<Self, rusqlite::types::FromSqlError> {
        match value.as_str()? {
            "Error" => Ok(nix_derivation::DerivationState::BuildError),
            "NotTested" => Ok(nix_derivation::DerivationState::NotTested),
            "Reproducible" => Ok(nix_derivation::DerivationState::Reproducible),
            "NonReproducible" => Ok(nix_derivation::DerivationState::NonReproducible),
            e => {
                debug!("invalid at nix_derivation::DerivationState FromSql {e:?}");
                Err(rusqlite::types::FromSqlError::InvalidType)
            }
        }
    }
}

impl rusqlite::ToSql for nix_derivation::BuildError {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        let value = self.to_string();
        Ok(rusqlite::types::ToSqlOutput::from(value))
    }
}

impl rusqlite::types::FromSql for nix_derivation::BuildError {
    fn column_result(
        value: rusqlite::types::ValueRef<'_>,
    ) -> Result<Self, rusqlite::types::FromSqlError> {
        match value.as_str()? {
            "" => Ok(nix_derivation::BuildError::None),
            "UnknownError" => Ok(nix_derivation::BuildError::UnknownError),
            "HTTPError" => Ok(nix_derivation::BuildError::HTTPError),
            "HashMismatch" => Ok(nix_derivation::BuildError::HashMismatch),
            "NonDeterministic" => Ok(nix_derivation::BuildError::NonDeterministic),
            e => {
                debug!("invalid at nix_derivation::BuildError FromSql {e:?}");
                Err(rusqlite::types::FromSqlError::InvalidType)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::nix_derivation::BuildError;

    use super::*;
    use rusqlite::types::{FromSql, ToSql, ToSqlOutput, ValueRef};

    #[test]
    fn conversion_derivation_state_sql_roundtrip() {
        let states = [
            DerivationState::BuildError,
            DerivationState::NotTested,
            DerivationState::Reproducible,
            DerivationState::NonReproducible,
        ];

        for state in &states {
            let to_sql_output = state.to_sql().unwrap();

            // get string from ToSqlOutput
            let value_str = match to_sql_output {
                ToSqlOutput::Owned(rusqlite::types::Value::Text(s)) => s,
                ToSqlOutput::Borrowed(ValueRef::Text(s)) => String::from_utf8_lossy(s).to_string(),
                _ => panic!("Unexpected ToSqlOutput variant"),
            };

            // get back enum var
            let value_ref = ValueRef::Text(value_str.as_bytes());
            let roundtrip = DerivationState::column_result(value_ref).unwrap();

            assert_eq!(*state, roundtrip, "Roundtrip failed for {state:?}");
        }
    }

    #[test]
    fn conversion_build_error_sql_roundtrip() {
        let states = [
            BuildError::None,
            BuildError::UnknownError,
            BuildError::HTTPError,
            BuildError::HashMismatch,
            BuildError::NonDeterministic,
        ];

        for state in &states {
            let to_sql_output = state.to_sql().unwrap();

            // get string from ToSqlOutput
            let value_str = match to_sql_output {
                ToSqlOutput::Owned(rusqlite::types::Value::Text(s)) => s,
                ToSqlOutput::Borrowed(ValueRef::Text(s)) => String::from_utf8_lossy(s).to_string(),
                _ => panic!("Unexpected ToSqlOutput variant"),
            };

            // get back enum var
            let value_ref = ValueRef::Text(value_str.as_bytes());
            let roundtrip = BuildError::column_result(value_ref).unwrap();

            assert_eq!(*state, roundtrip, "Roundtrip failed for {state:?}");
        }
    }
}
