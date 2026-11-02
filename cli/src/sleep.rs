use crate::{
    app_ref::AppRef,
    control,
    error::CliError,
    operations,
    output::{Output, ResultData},
};
use clap::{Args as ClapArgs, ValueEnum};
use control_api::SleepSettings;

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Mode {
    Auto,
    Never,
}

#[derive(Debug, ClapArgs)]
pub struct Args {
    /// App selector; omitted means your personal account defaults.
    pub app: Option<String>,
    #[arg(long, value_enum)]
    pub mode: Option<Mode>,
    /// Base idle timeout in whole seconds, greater than zero.
    #[arg(long, value_parser=clap::value_parser!(u32).range(1..))]
    pub after: Option<u32>,
}

pub async fn run(args: Args, output: &mut Output) -> Result<ResultData, CliError> {
    if args.app.is_some() && (args.mode.is_some() || args.after.is_some()) {
        return Err(CliError::Arguments { detail: "Set sleep and sleep_after_seconds in app.toml, then deploy. Account defaults are changed without an app selector.".into() });
    }
    let (control, _) = control::connect()?;
    let mode = args.mode.map(|mode| match mode {
        Mode::Auto => appspec::Sleep::Auto,
        Mode::Never => appspec::Sleep::Never,
    });
    let changing = mode.is_some() || args.after.is_some();
    macro_rules! request {
        ($call:expr) => {
            $call.await.map_err(|e| control.error(e))?.into_inner()
        };
    }
    let operation = if let Some(app) = args.app.as_deref() {
        let app = AppRef::parse(app)?;
        let current = request!(
            control
                .client
                .app_sleep_view(app.name.as_str(), app.owner())
        );
        return data(current);
    } else {
        let current = request!(control.client.sleep_defaults_view());
        if !changing {
            return data(current);
        }
        request!(control.client.sleep_defaults_set(&SleepSettings {
            sleep: mode.unwrap_or(current.sleep),
            sleep_after_seconds: args.after.unwrap_or(current.sleep_after_seconds)
        }))
    };
    output.operation = Some(operation.id);
    output.progress(&format!("Applying sleep settings: {}", operation.id));
    let done = operations::until(
        &control,
        operation.id,
        true,
        operations::longest(&operation.kind),
        *output,
    )
    .await?;
    Ok(ResultData::Operation(done.into()))
}
fn data(value: impl serde::Serialize) -> Result<ResultData, CliError> {
    serde_json::to_value(value)
        .map(ResultData::Workspace)
        .map_err(|e| CliError::Internal {
            detail: e.to_string(),
        })
}
