//! Validate a deploy locally, and, when the CLI is signed in, ask whether the
//! user's plan allows what its `app.toml` asks for, with the control plane's
//! own rules (`control_api::plans`), as the deploy would. Signed out, it needs
//! no settings or network.

use std::path::Path;

use control_api::plans::{Asker, check_apps, resolve};
use control_api::{MAX_DEPLOY_REQUEST_LEN, Plan};
use naming::public_apps::Namespace;
use naming::{AppAddress, AppName, Username};
use serde::Serialize;
use tokio::time::Instant;

use crate::api::types::ErrorBody;
use crate::app::{self, App};
use crate::app_ref::AppRef;
use crate::control::{self, Control};
use crate::error::CliError;
use crate::output::Output;
use crate::settings::API_TIMEOUT;
use control_api::DeployRequest;

pub struct Checked {
    pub app: App,
    pub request: DeployRequest,
    pub request_bytes: usize,
}

#[derive(Serialize)]
pub struct Summary {
    pub name: String,
    pub title: String,
    /// The web address, including before the first deploy on a named apps
    /// namespace. Unknown signed out or for another owner's app; none for workers.
    pub url: Option<String>,
    /// For an image app, the image each deploy pulls; it sends no files.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    pub file_count: usize,
    pub total_bytes: u64,
    pub request_bytes: usize,
    /// Whether the user's plan was asked: only when the CLI is signed in, for
    /// an app of the user's own.
    pub plan_checked: bool,
    /// The plan that allows the app, when it was asked.
    pub plan: Option<Plan>,
    /// Why the plan wasn't asked, for a person.
    #[serde(skip)]
    pub unchecked: Option<String>,
}

/// `anywhere check`: the app in `dir`, as `--app` names it or else by its
/// folder's name, as `anywhere deploy` names it, checked as [`read`] does,
/// then, when the CLI is signed in and the app is the user's own, held to
/// their plan ([`plan`]). Another owner's app is held to that owner's plan,
/// which only its deploy reads.
pub async fn check(dir: &Path, app: Option<&str>, output: Output) -> Result<Summary, CliError> {
    let named = app.map(AppRef::parse).transpose()?;
    let checked = read(dir, named.as_ref().map(|app| app.name.clone()))?;
    let (plan, url, unchecked) = match named.as_ref().and_then(|app| app.owner.as_ref()) {
        Some(owner) => (
            None,
            None,
            Some(format!(
                "your plan wasn't checked, since {owner}'s plan applies to {}, which its deploy checks",
                named.as_ref().map_or_else(String::new, ToString::to_string)
            )),
        ),
        None => match control::connect_if_signed_in()? {
            Some(control) => {
                let (plan, url) = self::plan(&control, &checked, output).await?;
                (Some(plan), url, None)
            }
            None => (
                None,
                None,
                Some(
                    "your plan wasn't checked, since the CLI isn't signed in (anywhere login)"
                        .to_owned(),
                ),
            ),
        },
    };
    Ok(Summary {
        plan_checked: plan.is_some(),
        plan,
        url,
        unchecked,
        ..checked.summary()
    })
}

/// Refuses `checked`, as the user's own app, where their plan wouldn't let
/// its deploy have what its `app.toml` asks for, with the code and message the
/// deploy would get, or gives the plan. What the app has already, from the
/// list of the user's apps, is never held against it, as the deploy doesn't
/// hold it. The apps and plan may change before the deploy, which checks
/// them again.
async fn plan(
    control: &Control,
    checked: &Checked,
    output: Output,
) -> Result<(Plan, Option<String>), CliError> {
    let deadline = Instant::now() + API_TIMEOUT;
    let me = control.me(deadline, output).await?;
    // The list gives an existing app's address as well as the count.
    let apps = control.list(deadline, output).await?.apps;
    let name = checked.app.name.as_str();
    if me.limits.max_apps.is_some() {
        let new_app = !apps.iter().any(|app| app.name == name);
        check_apps(
            me.plan,
            me.limits.max_apps,
            me.apps,
            new_app,
            Asker::Owner,
            me.billing.upgrade_url.as_deref(),
        )
        .map_err(|error| {
            CliError::Api(ErrorBody {
                diagnostic: None,
                code: error.body.code,
                message: error.body.message,
                retryable: error.body.retryable,
            })
        })?;
    }
    resolve(me.limits.per_app(), checked.app.spec.limits).map_err(|refused| {
        // Me carries this plan's ceilings; the deploy supplies the live Pro
        // offer. A local check can still give the exact requested fix.
        let error = refused.error(me.plan, Asker::Owner, None, None);
        CliError::Api(ErrorBody {
            diagnostic: None,
            code: error.body.code,
            message: error.body.message,
            retryable: error.body.retryable,
        })
    })?;
    let url = if checked.app.spec.kind().map_err(CliError::AppToml)? == appspec::Kind::Worker {
        None
    } else if let Some(url) = apps
        .iter()
        .find(|app| app.name == name)
        .and_then(|app| app.url.as_ref())
    {
        Some(url.clone())
    } else {
        prospective_url(&me.apps_origin, &me.username, checked.app.name.clone())?
    };
    Ok((me.plan, url))
}

/// Discover the namespace rather than guessing it from the API's hostname.
/// Google's aliases contain the app's ID, which doesn't exist before deploy.
fn prospective_url(
    origin: &str,
    username: &str,
    name: AppName,
) -> Result<Option<String>, CliError> {
    let invalid = || CliError::BadAnswer {
        detail: "apps_origin must be a valid public apps namespace; contact the operator"
            .to_owned(),
    };
    let namespace = Namespace::parse(origin).map_err(|_| invalid())?;
    let user = Username::parse(username).map_err(|_| invalid())?;
    let address = AppAddress::new(user, name).map_err(|source| CliError::LookupName { source })?;
    namespace
        .address_origin(&address.to_string())
        .map(|origin| origin.map(|origin| origin.as_str().to_owned()))
        .map_err(|_| invalid())
}

/// Reads and checks the app in `dir`, as `name`, or, without one, as its
/// folder's name.
pub fn read(dir: &Path, name: Option<AppName>) -> Result<Checked, CliError> {
    let name = match name {
        Some(name) => name,
        None => app::name(dir, &app::working_dir()?)?,
    };
    Checked::new(App::read(dir, name)?, false)
}

fn read_icon(app: &App) -> Result<Option<Vec<u8>>, CliError> {
    use std::io::Read;
    let Some(path) = &app.spec.icon else {
        return Ok(None);
    };
    let size = if app.spec.image.is_some() {
        app::image_icon_file(app, path)?
    } else {
        app.files
            .iter()
            .find(|file| &file.path == path)
            .ok_or_else(|| {
                CliError::AppIcon(appspec::icon::Error(
                    "icon must be included in the deploy; check .gitignore".to_owned(),
                ))
            })?
            .size
    };
    if size > appspec::icon::MAX_BYTES as u64 {
        return Err(CliError::AppIcon(appspec::icon::Error(
            "the file exceeds 1 MiB".to_owned(),
        )));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(app.dir.join(path))
        .and_then(|file| {
            file.take(appspec::icon::MAX_BYTES as u64 + 1)
                .read_to_end(&mut bytes)
        })
        .map_err(|_| {
            CliError::AppIcon(appspec::icon::Error(
                "the icon file could not be read; check its permissions".to_owned(),
            ))
        })?;
    Ok(Some(bytes))
}

impl Checked {
    /// The deploy of `app`, which with `new_app` must make it.
    pub fn new(app: App, new_app: bool) -> Result<Self, CliError> {
        let request = DeployRequest {
            spec: app.spec.clone(),
            files: app.files.clone(),
            new_app,
            icon_png: read_icon(&app)?,
        };
        request.sanitized_icon().map_err(CliError::AppIcon)?;
        let request_bytes = check_size(&request)?;
        Ok(Self {
            app,
            request,
            request_bytes,
        })
    }

    pub fn summary(&self) -> Summary {
        Summary {
            name: self.app.name.as_str().to_owned(),
            title: self.app.spec.title.clone(),
            url: None,
            image: self.app.spec.image.clone(),
            file_count: self.app.files.len(),
            total_bytes: self.app.files.iter().map(|file| file.size).sum(),
            request_bytes: self.request_bytes,
            plan_checked: false,
            plan: None,
            unchecked: None,
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
            icon_png: None,
            spec: appspec::parse("title = \"Hi\"\n[processes]\nweb = \"run\"\n").unwrap(),
            files: vec![FileEntry {
                path: "p".repeat(len),
                hash: "0".repeat(64),
                size: 0,
                executable: false,
            }],
            new_app: false,
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
