use reqwest::{
    Url,
    header::{AUTHORIZATION, HeaderValue},
};
use uuid::Uuid;

use crate::{api::types::Upload, error::CliError};

pub struct UploadClient {
    client: reqwest::Client,
    origin: Url,
    token: HeaderValue,
}

/// Only this module constructs validated targets. A target can never name
/// another origin or an operation other than the one the caller is following.
pub struct Target(Url);

impl UploadClient {
    pub(super) fn new(
        client: reqwest::Client,
        api: &str,
        token: HeaderValue,
    ) -> Result<Self, CliError> {
        Ok(Self {
            client,
            origin: api_origin(api)?,
            token,
        })
    }

    pub fn target(&self, operation: Uuid, upload: &Upload) -> Result<Target, CliError> {
        let expected = format!("/v1/uploads/{operation}");
        if upload.path.as_deref() != Some(expected.as_str()) {
            return Err(invalid_target());
        }
        let url = self.origin.join(&expected).map_err(|_| invalid_target())?;
        Ok(Target(url))
    }

    pub fn request(&self, target: &Target) -> reqwest::RequestBuilder {
        self.client
            .put(target.0.clone())
            .header(AUTHORIZATION, self.token.clone())
    }
}

fn invalid_target() -> CliError {
    // Never quote the response URL/path: it may contain a malicious credential.
    CliError::BadAnswer {
        detail: "the upload destination does not match this operation and the API origin"
            .to_owned(),
    }
}

/// Checks that `ANYWHERE_API` is a canonical origin the token may go to:
/// HTTPS, or plain HTTP to a loopback host only, where the token never leaves
/// the machine.
pub(super) fn api_origin(api: &str) -> Result<Url, CliError> {
    let invalid = || CliError::SettingInvalid {
        name: "ANYWHERE_API",
        problem: "must be the control API's canonical HTTPS origin, such as https://anywhere.so, \
                  in lowercase with no credentials, path, query, fragment or default port \
                  (http:// is allowed only for 127.0.0.1, [::1] or localhost)"
            .to_owned(),
    };
    let url = Url::parse(api).map_err(|_| invalid())?;
    let loopback = matches!(url.host_str(), Some("127.0.0.1" | "[::1]" | "localhost"));
    let scheme_allowed = match url.scheme() {
        "https" => true,
        "http" => loopback,
        _ => false,
    };
    if !scheme_allowed
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || ![
            url.origin().ascii_serialization(),
            format!("{}/", url.origin().ascii_serialization()),
        ]
        .contains(&api.to_owned())
    {
        return Err(invalid());
    }
    Ok(url)
}

#[cfg(test)]
mod tests;
