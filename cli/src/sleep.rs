use crate::{
    app_ref::AppRef,
    control,
    error::CliError,
    operations,
    output::{Output, ResultData},
};
use clap::{Args as ClapArgs, ValueEnum};
use control_api::{SleepOverrides, SleepSettings};

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Mode {
    Auto,
    Never,
}

#[derive(Debug, ClapArgs)]
pub struct Args {
    /// App selector; omitted means your personal account defaults.
    pub app: Option<String>,
    #[arg(long, value_enum, conflicts_with = "inherit")]
    pub mode: Option<Mode>,
    /// Base idle timeout in whole seconds, greater than zero.
    #[arg(long, value_parser=clap::value_parser!(u32).range(1..), conflicts_with="inherit")]
    pub after: Option<u32>,
    /// Clear both app overrides and follow account defaults.
    #[arg(long, requires = "app")]
    pub inherit: bool,
}

pub async fn run(args: Args, output: &mut Output) -> Result<ResultData, CliError> {
    let (control, _) = control::connect()?;
    let mode = args.mode.map(|mode| match mode {
        Mode::Auto => appspec::Sleep::Auto,
        Mode::Never => appspec::Sleep::Never,
    });
    let changing = mode.is_some() || args.after.is_some() || args.inherit;
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
        if !changing {
            return data(current);
        }
        let overrides = if args.inherit {
            SleepOverrides::default()
        } else {
            SleepOverrides {
                sleep: mode.or(current.overrides.sleep),
                sleep_after_seconds: args.after.or(current.overrides.sleep_after_seconds),
            }
        };
        request!(
            control
                .client
                .app_sleep_set(app.name.as_str(), app.owner(), &overrides)
        )
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
