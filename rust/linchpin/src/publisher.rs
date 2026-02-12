use reqwest::Result;
use std::{any::Any, fmt::Debug};
use typetag;

use crate::report_request::ReportRequest;

pub enum PublisherName {
    Gitlab,
}

pub trait Publisher {
    type PublisherArgs;

    // TODO the lookup in published history happen? -> something like match is_known
    // TODO keep history in sqlite as hash:json_blob pairs?
    //fn check_report_history(&self) -> Option<Self::PublisherRequestBody>;

    /// 1. take a ReportRequest to the right publisher
    /// 2. construct the specific http request
    /// 3. send it
    fn publish_report(
        &self,
        request: ReportRequest,
    ) -> impl std::future::Future<Output = std::result::Result<(), reqwest::Error>>;

    /// lookup the history of posted reports and replace the one that ist the same project and pr/branch
    fn update_report(
        &self,
        request: ReportRequest,
    ) -> impl std::future::Future<Output = std::result::Result<(), reqwest::Error>>;
}

pub trait PublisherMetadata {}
