use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::nix_derivation::{self};
use crate::report_message::ReportMessage;

#[derive(Clone)]
pub struct Gitlab {
    /// base url of gitlab instance `https://git.domain.com`
    pub url: String,
    /// access token allowing api use to post comments to a merge request
    pub token: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotesApiResponse {
    pub id: i64,
    #[serde(rename = "type")]
    pub type_field: Value,
    pub body: String,
    pub author: Author,
    #[serde(rename = "created_at")]
    pub created_at: String,
    #[serde(rename = "updated_at")]
    pub updated_at: String,
    pub system: bool,
    #[serde(rename = "noteable_id")]
    pub noteable_id: i64,
    #[serde(rename = "noteable_type")]
    pub noteable_type: String,
    #[serde(rename = "project_id")]
    pub project_id: i64,
    pub resolvable: bool,
    pub confidential: bool,
    pub internal: bool,
    pub imported: bool,
    #[serde(rename = "imported_from")]
    pub imported_from: String,
    #[serde(rename = "noteable_iid")]
    pub noteable_iid: i64,
    #[serde(rename = "commands_changes")]
    pub commands_changes: CommandsChanges,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Author {
    pub id: i64,
    pub username: String,
    #[serde(rename = "public_email")]
    pub public_email: Value,
    pub name: String,
    pub state: String,
    pub locked: bool,
    #[serde(rename = "avatar_url")]
    pub avatar_url: String,
    #[serde(rename = "web_url")]
    pub web_url: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandsChanges {}

/// used to simplify json serialize and deserialize
#[derive(Debug, Deserialize, Serialize)]
pub struct GitlabApiBody {
    body: String,
}

/// testing resulted in a count of reproducible store derivations and
/// a list of non reproducible store deriavtions
/// and maybe even missing store derivation paths (OOM for example)
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ReportResult {
    pub reproducible: i32,
    pub non_reproducible: Vec<nix_derivation::Derivation>,
    pub test_unsuccessful: Vec<nix_derivation::Derivation>,
    pub no_entry: Vec<nix_derivation::Derivation>,
}

impl Gitlab {
    pub async fn create_merge_comment(
        &self,
        report_message: ReportMessage,
        project_id: i64,
        merge_id: i64,
    ) -> Result<NotesApiResponse, reqwest::Error> {
        let url = format!(
            "{}/api/v4/projects/{}/merge_requests/{}/notes",
            self.url, project_id, merge_id,
        );
        let body: String = serde_json::to_string(&GitlabApiBody {
            body: report_message.to_string(),
        })
        .expect("parse response json to string");

        match Client::new()
            .post(&url)
            .header("PRIVATE-TOKEN", &self.token)
            .header("Content-Type", "application/json")
            .body(body.clone())
            .send()
            .await
        {
            Ok(a) => {
                println!("api response raw: {a:?}");
                let response: NotesApiResponse = a.json().await.expect("parse error api response");
                Ok(response)
            }
            Err(e) => Err(e),
        }
    }

    pub async fn overwrite_merge_comment(
        &self,
        report_message: ReportMessage,
        project_id: i64,
        merge_id: i64,
        comment_id: i64,
    ) -> Result<NotesApiResponse, reqwest::Error> {
        let url = format!(
            "{}/api/v4/projects/{}/merge_requests/{}/notes/{}",
            self.url, project_id, merge_id, comment_id,
        );
        let body: String = serde_json::to_string(&GitlabApiBody {
            body: report_message.to_string(),
        })
        .expect("parse response json to string");

        match Client::new()
            .put(&url)
            .header("PRIVATE-TOKEN", &self.token)
            .header("Content-Type", "application/json")
            .body(body.clone())
            .send()
            .await
        {
            Ok(a) => {
                let response: NotesApiResponse = a.json().await.expect("parse error api response");
                Ok(response)
            }
            Err(e) => Err(e),
        }
    }
}
