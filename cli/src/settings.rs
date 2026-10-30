//! Settings and HTTP clients shared by the online commands: the API's origin
//! and token from `ANYWHERE_API` and `ANYWHERE_TOKEN`, or else from the
//! credentials file `anywhere login` wrote.

use std::env::{self, VarError};
use std::fs;
use std::path::Path;
use std::time::Duration;

use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};

use crate::error::{CliError, chain};

pub const API_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

pub mod credentials;
mod uploads;
pub use uploads::UploadClient;

const WHAT_API: &str = "the control API's address, such as https://anywhere.so";
const WHAT_TOKEN: &str = "your token for the control API";

/// The CLI's settings, from its environment or the credentials file.
pub struct Settings {
    /// The control API's address, without a trailing `/`.
    pub api: String,
    token: String,
    /// More certificate authorities to trust than the system's, such as the
    /// local stack's.
    ca: Option<Vec<reqwest::Certificate>>,
}

impl Settings {
    /// `ANYWHERE_API` and `ANYWHERE_TOKEN` when both are set, and otherwise
    /// the credentials `anywhere login` wrote. Only one of the two is an
    /// error, never a mix with the file.
    pub fn from_env() -> Result<Self, CliError> {
        let api = optional("ANYWHERE_API", WHAT_API)?;
        let token = optional("ANYWHERE_TOKEN", WHAT_TOKEN)?;
        let (api, token) = match (api, token) {
            (Some(api), Some(token)) => {
                uploads::api_origin(&api)?;
                (api, token)
            }
            (Some(_), None) => {
                return Err(CliError::SettingMissing {
                    name: "ANYWHERE_TOKEN",
                    what: "your token for the control API, since ANYWHERE_API is set; or unset ANYWHERE_API to use the credentials `anywhere login` saved",
                });
            }
            (None, Some(_)) => {
                return Err(CliError::SettingMissing {
                    name: "ANYWHERE_API",
                    what: "the control API's address, such as https://anywhere.so, since ANYWHERE_TOKEN is set; or unset ANYWHERE_TOKEN to use the credentials `anywhere login` saved",
                });
            }
            (None, None) => {
                let saved = credentials::read(&credentials::path()?)?;
                (saved.api, saved.token)
            }
        };
        Ok(Self {
            api: api.trim_end_matches('/').to_owned(),
            token,
            ca: ca_from_env()?,
        })
    }

    /// Uploads attach the token only after matching the operation path to the
    /// configured API origin; the gateway checks and strips it before the
    /// sandbox hop.
    pub fn clients(&self) -> Result<Clients, CliError> {
        let mut token = HeaderValue::from_str(&format!("Bearer {}", self.token)).map_err(|_| {
            CliError::SettingInvalid {
                name: "ANYWHERE_TOKEN",
                problem: "has characters a token can't hold; copy it again, whole".to_owned(),
            }
        })?;
        token.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, token.clone());
        let api = self.client().default_headers(headers).timeout(API_TIMEOUT);
        // Each upload has its own timeout: what's left of the deploy's deadline.
        let uploads = self.client();
        Ok(Clients {
            api: build(api)?,
            uploads: UploadClient::new(build(uploads)?, &self.api, token)?,
        })
    }

    /// An HTTP/1.1 client with the bearer token and no request timeout, for
    /// upgrading to an `anywhere exec` WebSocket.
    pub fn exec_client(&self) -> Result<reqwest::Client, CliError> {
        let mut token = HeaderValue::from_str(&format!("Bearer {}", self.token)).map_err(|_| {
            CliError::SettingInvalid {
                name: "ANYWHERE_TOKEN",
                problem: "has characters a token can't hold; copy it again, whole".to_owned(),
            }
        })?;
        token.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, token);
        build(self.client().http1_only().default_headers(headers))
    }

    fn client(&self) -> reqwest::ClientBuilder {
        client(self.ca.as_deref())
    }
}

/// What `anywhere login` reaches the control API with, before it has a token:
/// the API's origin, checked, and a client with no credential.
pub struct Anonymous {
    /// The control API's address, without a trailing `/`.
    pub api: String,
    pub client: reqwest::Client,
    ca: Option<Vec<reqwest::Certificate>>,
}

impl Anonymous {
    /// For the API at `api`, which `name` set, with the certificate
    /// authorities in `ANYWHERE_CA`.
    pub fn new(api: &str, name: &'static str) -> Result<Self, CliError> {
        uploads::api_origin(api).map_err(|error| match error {
            CliError::SettingInvalid { problem, .. } => CliError::SettingInvalid { name, problem },
            error => error,
        })?;
        let ca = ca_from_env()?;
        Ok(Self {
            api: api.trim_end_matches('/').to_owned(),
            client: build(client(ca.as_deref()).timeout(API_TIMEOUT))?,
            ca,
        })
    }

    /// A client for the same API with `token`, as the other commands send it.
    pub fn with_token(&self, token: &str) -> Result<reqwest::Client, CliError> {
        let mut token =
            HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| CliError::BadAnswer {
                detail: "the token has characters a token can't hold".to_owned(),
            })?;
        token.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, token);
        build(
            client(self.ca.as_deref())
                .default_headers(headers)
                .timeout(API_TIMEOUT),
        )
    }
}

/// What every client shares: who's asking, with the contract it was built
/// for, `anywhere/<version> (contract <N>)`, which the gateway refuses if it
/// isn't the platform's, how long connecting may take, which certificate
/// authorities to trust, and never following a redirect, so the token goes
/// only to the configured origin. Exec's and pull's WebSockets, and uploads,
/// are made with it too.
fn client(ca: Option<&[reqwest::Certificate]>) -> reqwest::ClientBuilder {
    let client = reqwest::Client::builder()
        .user_agent(control_api::cli_user_agent(env!("CARGO_PKG_VERSION")))
        .connect_timeout(CONNECT_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none());
    match ca {
        Some(ca) => client.tls_certs_merge(ca.iter().cloned()),
        None => client,
    }
}

/// The certificate authorities in the PEM file `ANYWHERE_CA` names, if it's set.
fn ca_from_env() -> Result<Option<Vec<reqwest::Certificate>>, CliError> {
    match env::var_os("ANYWHERE_CA") {
        None => Ok(None),
        Some(path) => Ok(Some(read_ca(Path::new(&path))?)),
    }
}

pub struct Clients {
    /// For the control API, with the token.
    pub api: reqwest::Client,
    /// For uploads, which go only to the API origin, with the token.
    pub uploads: UploadClient,
}

fn build(client: reqwest::ClientBuilder) -> Result<reqwest::Client, CliError> {
    client.build().map_err(|error| CliError::Tls {
        detail: chain(&error),
    })
}

/// An environment variable's value, if it's set: never empty or not UTF-8.
pub fn optional(name: &'static str, what: &'static str) -> Result<Option<String>, CliError> {
    match env::var(name) {
        Ok(value) if value.is_empty() => Err(CliError::SettingInvalid {
            name,
            problem: format!("is empty; set it to {what}, or unset it"),
        }),
        Ok(value) => Ok(Some(value)),
        Err(VarError::NotPresent) => Ok(None),
        Err(VarError::NotUnicode(_)) => Err(CliError::SettingInvalid {
            name,
            problem: format!("isn't valid UTF-8; set it to {what}"),
        }),
    }
}

/// Reads the certificate authorities in a PEM file.
fn read_ca(path: &Path) -> Result<Vec<reqwest::Certificate>, CliError> {
    let invalid = |problem: String| CliError::SettingInvalid {
        name: "ANYWHERE_CA",
        problem: format!(
            "{problem}; set it to a PEM file of the certificate authority to trust, or unset it"
        ),
    };
    if path.as_os_str().is_empty() {
        return Err(invalid("is set but empty".to_owned()));
    }
    let shown = path.display();
    let pem = fs::read(path)
        .map_err(|error| invalid(format!("names {shown}, which can't be read ({error})")))?;
    let certificates = reqwest::Certificate::from_pem_bundle(&pem).map_err(|error| {
        invalid(format!(
            "names {shown}, which isn't a PEM file of certificates ({})",
            chain(&error)
        ))
    })?;
    if certificates.is_empty() {
        return Err(invalid(format!(
            "names {shown}, which holds no certificate"
        )));
    }
    Ok(certificates)
}
