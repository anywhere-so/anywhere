//! Bounded, retryable reads through the generated control API client.

use std::future::Future;

use progenitor_client::ResponseValue;
use tokio::time::Instant;
use uuid::Uuid;

use crate::api::{
    self,
    types::{
        AppDetails, AppList, Me, Operation, SecretList, ShareList, SharedAppList, Ticket,
        TicketRequest, TicketService,
    },
};
use crate::app_ref::AppRef;
use crate::error::{CliError, chain};
use crate::output::Output;
use crate::retry::Retries;
use crate::settings::Settings;

pub struct Control {
    pub client: api::Client,
    pub api: String,
}

pub fn connect() -> Result<(Control, crate::settings::UploadClient), CliError> {
    with(Settings::from_env()?)
}

/// [`connect`], or None when nobody has signed in on this machine: no
/// credentials file, and neither `ANYWHERE_API` nor `ANYWHERE_TOKEN`. Any
/// other problem with the settings is still an error.
pub fn connect_if_signed_in() -> Result<Option<Control>, CliError> {
    match Settings::from_env() {
        Err(CliError::NotSignedIn { .. }) => Ok(None),
        settings => with(settings?).map(|(control, _)| Some(control)),
    }
}

fn with(settings: Settings) -> Result<(Control, crate::settings::UploadClient), CliError> {
    let clients = settings.clients()?;
    Ok((
        Control {
            client: api::Client::new_with_client(&settings.api, clients.api),
            api: settings.api,
        },
        clients.uploads,
    ))
}

/// The control API through a client with no timeout for a whole answer, for
/// answers that stream, whose reader bounds each wait itself.
pub fn connect_streams() -> Result<Control, CliError> {
    let settings = Settings::from_env()?;
    let clients = settings.clients()?;
    Ok(Control {
        client: api::Client::new_with_client(&settings.api, clients.streams),
        api: settings.api,
    })
}

impl Control {
    pub async fn view(
        &self,
        id: Uuid,
        deadline: Instant,
        output: Output,
    ) -> Result<Operation, CliError> {
        let operation = self
            .read(deadline, output, || self.client.operation_view(&id))
            .await?;
        if operation.id != id {
            return Err(CliError::BadAnswer {
                detail: "the answer names another operation".to_owned(),
            });
        }
        Ok(operation)
    }

    pub async fn list(&self, deadline: Instant, output: Output) -> Result<AppList, CliError> {
        self.read(deadline, output, || self.client.app_list()).await
    }

    /// Who's signed in, and their plan.
    pub async fn me(&self, deadline: Instant, output: Output) -> Result<Me, CliError> {
        self.read(deadline, output, || self.client.me_view()).await
    }

    pub async fn inspect(
        &self,
        app: &AppRef,
        deadline: Instant,
        output: Output,
    ) -> Result<AppDetails, CliError> {
        self.read(deadline, output, || {
            self.client.app_view(app.name.as_str(), app.owner())
        })
        .await
    }

    pub async fn secrets(
        &self,
        app: &AppRef,
        deadline: Instant,
        output: Output,
    ) -> Result<SecretList, CliError> {
        self.read(deadline, output, || {
            self.client.secret_list(app.name.as_str(), app.owner())
        })
        .await
    }

    pub async fn account_usage(
        &self,
        deadline: Instant,
        output: Output,
    ) -> Result<control_api::AccountUsage, CliError> {
        self.read(deadline, output, || self.client.account_usage())
            .await
    }

    pub async fn usage(
        &self,
        app: &AppRef,
        range: control_api::UsageRange,
        deadline: Instant,
        output: Output,
    ) -> Result<control_api::AppUsage, CliError> {
        self.read(deadline, output, || {
            self.client
                .app_usage(app.name.as_str(), app.owner(), Some(range.as_str()))
        })
        .await
    }

    pub async fn access(
        &self,
        app: &str,
        deadline: Instant,
        output: Output,
    ) -> Result<control_api::AppAccess, CliError> {
        self.read(deadline, output, || self.client.app_access_get(app))
            .await
    }

    pub async fn shares(
        &self,
        app: &str,
        deadline: Instant,
        output: Output,
    ) -> Result<ShareList, CliError> {
        self.read(deadline, output, || self.client.share_list(app))
            .await
    }

    pub async fn shared_apps(
        &self,
        deadline: Instant,
        output: Output,
    ) -> Result<SharedAppList, CliError> {
        self.read(deadline, output, || self.client.shared_app_list())
            .await
    }

    pub async fn ticket(
        &self,
        app: &AppRef,
        service: TicketService,
        deadline: Instant,
        output: Output,
    ) -> Result<Ticket, CliError> {
        let body = TicketRequest { service };
        self.read(deadline, output, || {
            self.client
                .app_ticket_create(app.name.as_str(), app.owner(), &body)
        })
        .await
    }

    async fn read<T, F, Fut>(
        &self,
        deadline: Instant,
        output: Output,
        mut request: F,
    ) -> Result<T, CliError>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<ResponseValue<T>, api::Error<api::types::Error>>>,
    {
        let mut retries = Retries::with_output(deadline, output);
        loop {
            match tokio::time::timeout_at(deadline, request()).await {
                Ok(Ok(response)) => return Ok(response.into_inner()),
                Ok(Err(error)) => {
                    let error = self.error(error);
                    if !retries.again(&error).await {
                        return Err(error);
                    }
                }
                Err(_) => return Err(CliError::ReadTimedOut),
            }
        }
    }

    pub fn error(&self, error: api::Error<api::types::Error>) -> CliError {
        api_error(&self.api, error)
    }
}

/// What went wrong calling the control API for a free-form answer, such as an
/// app's logs, other than its own error answer, which the caller reads.
pub fn stream_error(api: &str, error: api::Error<progenitor_client::ByteStream>) -> CliError {
    match error {
        api::Error::ErrorResponse(response) => CliError::BadAnswer {
            detail: format!(
                "HTTP {} with no error the CLI could read",
                response.status()
            ),
        },
        api::Error::CommunicationError(error)
        | api::Error::ResponseBodyError(error)
        | api::Error::InvalidUpgrade(error) => CliError::Unreachable {
            api: api.to_owned(),
            detail: chain(&error),
        },
        api::Error::InvalidResponsePayload(_, error) => CliError::BadAnswer {
            detail: error.to_string(),
        },
        api::Error::UnexpectedResponse(response) => CliError::BadAnswer {
            detail: format!("HTTP {}", response.status()),
        },
        api::Error::InvalidRequest(detail) | api::Error::Custom(detail) => {
            CliError::Internal { detail }
        }
    }
}

/// What went wrong calling the control API, as the agent should see it.
pub fn api_error(api: &str, error: api::Error<api::types::Error>) -> CliError {
    match error {
        api::Error::ErrorResponse(response) => CliError::Api(response.into_inner().0),
        api::Error::CommunicationError(error)
        | api::Error::ResponseBodyError(error)
        | api::Error::InvalidUpgrade(error) => CliError::Unreachable {
            api: api.to_owned(),
            detail: chain(&error),
        },
        api::Error::InvalidResponsePayload(_, error) => CliError::BadAnswer {
            detail: error.to_string(),
        },
        api::Error::UnexpectedResponse(response) => CliError::BadAnswer {
            detail: format!("HTTP {}", response.status()),
        },
        api::Error::InvalidRequest(detail) | api::Error::Custom(detail) => {
            CliError::Internal { detail }
        }
    }
}
