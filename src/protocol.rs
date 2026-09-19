use serde::{Deserialize, Serialize};

use crate::{CheckReport, CheckRequest, ReportTarget};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "command")]
pub enum DaemonRequest {
    #[serde(rename = "check")]
    Check(CheckRequest),
    #[serde(rename = "list")]
    List(ReportTarget),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DaemonResponse {
    pub ok: bool,
    pub report: Option<CheckReport>,
    pub error: Option<String>,
}

impl DaemonResponse {
    pub fn success(report: CheckReport) -> Self {
        Self {
            ok: true,
            report: Some(report),
            error: None,
        }
    }

    pub fn failure(error: impl Into<String>) -> Self {
        Self {
            ok: false,
            report: None,
            error: Some(error.into()),
        }
    }
}
