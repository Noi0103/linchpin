// metadata per request type
// struct for url etc?
// trait publisher implementations
// calling formatters for md or json from here

use anyhow::{anyhow, Error};
use reqwest::Client;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

use crate::report_request::Publisher;
use crate::{
    nix_derivation::{Derivation, DerivationState},
    report_request::{ClosureElement, ReportRequest},
};

#[derive(Clone)]
pub struct Gitlab {
    /// base url of gitlab instance `https://git.domain.com`
    pub url: String,
    /// access token allowing api use to post comments to a merge request
    pub token: String,
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
pub struct PublisherMetadataGitlab {
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

/// used to simplify json serialize and deserialize
#[derive(Debug, Deserialize, Serialize)]
pub struct GitlabApiBody {
    body: String,
}

/// response json format from gitlab notes api
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

pub struct Args {
    url: String,
}

// TODO take refs instead
impl Gitlab {
    pub async fn publish_report(&self, request: &ReportRequest) -> Result<(), Error> {
        let project_id = match &request.publisher_data {
            Publisher::Gitlab(meta) => meta.ci_merge_request_project_id.clone(),
            _ => return Err(anyhow!("missing metadata")),
        };

        let iid = match &request.publisher_data {
            Publisher::Gitlab(meta) => meta.ci_merge_request_iid.clone(),
            _ => return Err(anyhow!("missing metadata")),
        };

        let url = format!(
            "{}/api/v4/projects/{}/merge_requests/{}/notes",
            self.url, project_id, iid,
        );
        let body: String = serde_json::to_string(&GitlabApiBody {
            body: Gitlab::to_markdown(&request).to_string(),
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

                Ok(())
            }
            Err(e) => Err(e.into()),
        }
    }
    /// you already found a fitting past event in the history and want to overwrite the post
    pub async fn update_report(&self, request: &ReportRequest) -> Result<(), reqwest::Error> {
        // overwrite old published message with the updated one
        // on fail: call publish?
        todo!();
    }

    fn to_markdown(request: &ReportRequest) -> String {
        let mut mardown_string = String::from("# Reproducibility Report");

        let md_summary = Self::make_markdown_summary(request);
        let md_detailed = Self::make_markdown_detailed(request);

        mardown_string = format!("{mardown_string}{md_summary}");
        mardown_string = format!("{mardown_string}{md_detailed}");

        mardown_string
    }
    fn make_markdown_summary(request: &ReportRequest) -> String {
        let closure_count = request.store_derivation_closure.len();
        let derivations: Vec<&Derivation> = request
            .store_derivation_closure
            .iter()
            .filter_map(|ce| {
                if let ClosureElement::Derivation(ref d) = ce {
                    Some(d)
                } else {
                    None
                }
            })
            .collect();
        let derivation_count = derivations.len();

        format![
            "## Summary\nOut of {closure_count} Closure Elements, {derivation_count} are Derivations.",
        ]
    }
    fn make_markdown_detailed(request: &ReportRequest) -> String {
        let derivation_count = request.get_derivations().len();

        let mut body =
            format!["## Detail\nFYI: {derivation_count} Derivations tested Reproducible",];

        let non_reproducible = request.get_derivations_filtered(DerivationState::NonReproducible);
        let non_reproducible_count = non_reproducible.len();

        body = format![
            "{body}\n### Non-Reproducible\nCounting {non_reproducible_count}\n<details><summary>store derivations</summary>"
        ];
        for elem in non_reproducible {
            // if it's reproducibility status is known these fields are filled
            let reason = elem.error_reason.as_ref().unwrap().to_string();
            let try_count = elem.db_write_count.unwrap();
            body = format!["{body}\n<details><summary>{elem}</summary>\nDocumented Reason: {reason}\nTested {try_count} times"];

            // TODO list what toplevel they were included in when
            // more than one toplevel is in one report/pipeline (CI with 50 toplevel drv for parallelism)
            // BLOCKED: report_request history to keep publishermetadata with ci pipeline stuff
        }

        body
    }
}

/*
/// count how many of the shared_reports_list items are of the given pipeline_id
pub fn shared_reports_list_entries_of_pipeline(
    shared_reports_list: Arc<Mutex<VecDeque<ReportBody>>>,
    pipeline_id: i64,
) -> i32 {
    let list: MutexGuard<VecDeque<ReportBody>> = shared_reports_list.lock().unwrap();
    let mut counter: i32 = 0;
    for e in list.clone() {
        let ci_pipeline_id: i64 = e
            .ci_pipeline_id
            .parse::<i64>()
            .expect("pipeline id parse to i64");
        if ci_pipeline_id == pipeline_id {
            counter += 1;
        }
    }
    counter
}

/// get all pipeline ids without dublicates
pub fn shared_reports_list_pipeline_ids(
    shared_reports_list: Arc<Mutex<VecDeque<ReportBody>>>,
) -> Vec<i64> {
    let list: MutexGuard<VecDeque<ReportBody>> = shared_reports_list.lock().unwrap();
    let mut id_list: Vec<i64> = vec![];
    for e in list.clone() {
        let ci_pipeline_id: i64 = e
            .ci_pipeline_id
            .parse::<i64>()
            .expect("pipeline id parse to i64");
        if !id_list.contains(&ci_pipeline_id) {
            id_list.push(ci_pipeline_id)
        }
    }
    id_list
}
*/
