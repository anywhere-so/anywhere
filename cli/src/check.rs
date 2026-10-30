//! Validate a deploy locally, without API settings or network access.

use std::path::Path;

use control_api::MAX_DEPLOY_REQUEST_LEN;
use naming::AppName;
use serde::Serialize;

use crate::api::types::DeployRequest;
use crate::app::{self, App};
use crate::error::CliError;

pub struct Checked {
    pub app: App,
    pub request: DeployRequest,
    pub request_bytes: usize,
}

#[derive(Serialize)]
pub struct Summary {
    pub name: String,
    pub title: String,
    pub file_count: usize,
    pub total_bytes: u64,
    pub request_bytes: usize,
}

/// Reads and checks the app in `dir`, as `name`, or, without one, as its
/// folder's name.
pub fn read(dir: &Path, name: Option<AppName>) -> Result<Checked, CliError> {
    let name = match name {
        Some(name) => name,
        None => app::name(dir, &app::working_dir()?)?,
    };
    let app = App::read(dir, name)?;
    let request = DeployRequest {
        spec: app.spec.clone(),
        files: app.files.clone(),
    };
    let request_bytes = check_size(&request)?;
    Ok(Checked {
        app,
        request,
        request_bytes,
    })
}

impl Checked {
    pub fn summary(&self) -> Summary {
        Summary {
            name: self.app.name.as_str().to_owned(),
            title: self.app.spec.title.clone(),
            file_count: self.app.files.len(),
            total_bytes: self.app.files.iter().map(|file| file.size).sum(),
            request_bytes: self.request_bytes,
        }
    }
}

/// Refuses a request that's bigger, as JSON, than a deploy may send, before
/// it's sent. The client sends the same JSON.
fn check_size(request: &DeployRequest) -> Result<usize, CliError> {
    let json = serde_json::to_vec(request).map_err(|error| CliError::Internal {
        detail: format!("the deploy's request couldn't be written as JSON: {error}"),
    })?;
    if json.len() > MAX_DEPLOY_REQUEST_LEN {
        return Err(CliError::RequestTooLarge { len: json.len() });
    }
    Ok(json.len())
}

#[cfg(test)]
mod tests {
    use manifest::FileEntry;

    use super::*;

    /// A request with one file, whose path is `len` bytes long.
    fn request(len: usize) -> DeployRequest {
        DeployRequest {
            spec: appspec::parse("title = \"Hi\"\n[processes]\nweb = \"run\"\n").unwrap(),
            files: vec![FileEntry {
                path: "p".repeat(len),
                hash: "0".repeat(64),
                size: 0,
                executable: false,
            }],
        }
    }

    #[test]
    fn a_request_is_at_most_32_mib_of_json() {
        let around = serde_json::to_vec(&request(0)).unwrap().len();
        let largest = request(MAX_DEPLOY_REQUEST_LEN - around);
        assert_eq!(
            serde_json::to_vec(&largest).unwrap().len(),
            MAX_DEPLOY_REQUEST_LEN
        );
        assert!(check_size(&largest).is_ok());

        let error = check_size(&request(MAX_DEPLOY_REQUEST_LEN - around + 1)).unwrap_err();
        assert_eq!(error.code(), "deploy_request_too_large");
        assert!(error.to_string().contains(".gitignore"), "{error}");
    }
}
