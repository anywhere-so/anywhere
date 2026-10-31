//! `anywhere usage`: what an app used over a range, against its limits, and
//! the problems it ran into, each with its fix. As text, the same layout in
//! every state, so an agent reads it without `--json` and a person can scan
//! it; with `--json`, totals and problems, adding points with `--detail`.

use control_api::{AppStatus, AppUsage, UsageRange, words};
use serde::Serialize;
use tokio::time::Instant;

use crate::app_ref::AppRef;
use crate::control;
use crate::error::CliError;
use crate::output::{Output, ResultData, plain};
use crate::settings::API_TIMEOUT;

/// The most stretches with no reading the text names, newest first; `--json`
/// lists them all. A starting value, to keep an agent's context short.
const GAPS_SHOWN: usize = 5;

/// An app's API answer with display settings.
pub struct Usage {
    pub app: AppRef,
    pub usage: AppUsage,
    pub detail: bool,
}

impl Serialize for Usage {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut value = serde_json::to_value(&self.usage).map_err(serde::ser::Error::custom)?;
        if let Some(object) = value.as_object_mut() {
            if !self.detail {
                object.remove("points");
            }
            // Paste prompts repeat the message and fix for a person to give
            // to an agent; this reader already is one.
            if let Some(problems) = object.get_mut("problems").and_then(|p| p.as_array_mut()) {
                for problem in problems.iter_mut().filter_map(|p| p.as_object_mut()) {
                    problem.remove("agent_prompt");
                }
            }
        }
        value.serialize(serializer)
    }
}

#[derive(Serialize)]
pub struct Account {
    #[serde(flatten)]
    pub usage: control_api::AccountUsage,
    #[serde(skip)]
    pub detail: bool,
}

pub async fn show(
    app: Option<&str>,
    range: Option<UsageRange>,
    detail: bool,
    output: Output,
) -> Result<ResultData, CliError> {
    if app.is_none() && range.is_some() {
        return Err(CliError::Arguments {
            detail: "--range needs an app; run `anywhere usage NAME --range 7d`, or omit --range to read your account's usage this month".to_owned(),
        });
    }
    let (control, _) = control::connect()?;
    let Some(app) = app else {
        let usage = control
            .account_usage(Instant::now() + API_TIMEOUT, output)
            .await?;
        return Ok(ResultData::AccountUsage(Box::new(Account {
            usage,
            detail,
        })));
    };
    let app = AppRef::parse(app)?;
    let usage = control
        .usage(
            &app,
            range.unwrap_or(UsageRange::Day),
            Instant::now() + API_TIMEOUT,
            output,
        )
        .await?;
    if output.json && usage.status == AppStatus::Crashed {
        let (command, note) = crashed(&app);
        output.next(&command, note);
    }
    Ok(ResultData::Usage(Box::new(Usage { app, usage, detail })))
}

/// CLI dollars are to the cent, rounded down as the portal's are, so the two
/// agree and $2.999 never shows as "$3.00 of $3.00".
pub fn dollars(micro: u64) -> String {
    let cents = u128::from(micro) / 10_000;
    format!("${}.{:02}", cents / 100, cents % 100)
}

fn rate(micro: u64) -> String {
    let mut decimal = format!("{:06}", micro % 1_000_000);
    while decimal.len() > 2 && decimal.ends_with('0') {
        decimal.pop();
    }
    format!("${}.{decimal}", micro / 1_000_000)
}

fn breakdown(cost: &control_api::UsageCost) -> String {
    format!(
        "  memory {} · CPU {} · disk {} · data out {}\n",
        dollars(cost.memory_microusd),
        dollars(cost.cpu_microusd),
        dollars(cost.disk_microusd),
        dollars(cost.egress_microusd)
    )
}

pub fn account_text(account: &Account) -> String {
    let u = &account.usage;
    let percent =
        |n: Option<u64>| n.map_or_else(|| "no included usage".to_owned(), |n| format!("{n}%"));
    let last_day = u.period.resets_at - chrono::TimeDelta::days(1);
    let outlook = if u.charged_through.is_some() {
        format!(
            "on track for about {} by {}",
            percent(u.forecast_percent),
            last_day.format("%-d %b")
        )
    } else {
        "nothing charged yet: each hour is charged after it ends".to_owned()
    };
    let mut out = format!(
        "Usage this month ({}): {}, {} of {} · {outlook} · resets {}\n",
        match u.plan {
            control_api::Plan::Free => "Free",
            control_api::Plan::Pro => "Pro",
        },
        percent(u.percent_used),
        dollars(u.used_microusd),
        dollars(u.available_microusd),
        u.period.resets_at.format("%-d %b")
    );
    out.push_str(&format!(
        "State: {}{}\n",
        u.state.as_str(),
        if u.enforced { "" } else { " (enforcement off)" }
    ));
    let names = u
        .apps
        .iter()
        .map(|a| {
            format!(
                "{}{}",
                plain(&a.app),
                if a.deleted { " (deleted)" } else { "" }
            )
        })
        .collect::<Vec<_>>();
    let width = names.iter().map(String::len).max().unwrap_or(3).max(3) + 2;
    out.push_str(&format!(
        "{:width$}{:<13}{}\n",
        "APP", "THIS MONTH", "BY MONTH END"
    ));
    for (app, name) in u.apps.iter().zip(names) {
        out.push_str(&format!(
            "{name:width$}{:<13}about {}\n",
            dollars(app.cost.used_microusd),
            dollars(app.cost.forecast_microusd)
        ));
        if account.detail {
            out.push_str(&breakdown(&app.cost));
        }
    }
    let r = u.rates;
    out.push_str(&format!(
        "Rates: memory {}/GiB-hour · CPU {}/vCPU-hour · disk {}/GiB-month · data out {}/GiB\n",
        rate(r.memory_gib_hour_microusd),
        rate(r.cpu_vcpu_hour_microusd),
        rate(r.disk_gib_month_microusd),
        rate(r.egress_gib_microusd)
    ));
    // While an operator has stopped enforcement, the state line says so and
    // nothing pauses, so there's no pause to describe.
    if u.enforced {
        out.push_str(&format!(
            "At 100%, your apps pause and nothing is lost; they start again on {}, or at once {}.\n",
            u.period.resets_at.format("%-d %b"),
            match u.plan {
                control_api::Plan::Free => "on upgrading to Pro",
                control_api::Plan::Pro => "after a top-up from the portal's usage page",
            }
        ));
    }
    out
}

fn crashed(app: &AppRef) -> (String, &'static str) {
    (
        format!("anywhere logs --app {app}"),
        "the logs say why it crashed; its next deploy or change to its secrets starts it again.",
    )
}

/// The usage as text: what goes on standard output, then the `next:` and
/// `note:` lines for standard error, if any.
pub fn text(usage: &Usage) -> (String, Option<(String, &'static str)>) {
    let Usage {
        app,
        usage: u,
        detail,
    } = usage;
    let mut out = String::new();
    let line = |out: &mut String, text: &str| {
        out.push_str(text.trim_end());
        out.push('\n');
    };
    let cpu = words::cpu_limit(u.limits.cpu_millicores);
    line(
        &mut out,
        &format!(
            "{}: up to {cpu}, {} memory, {} disk; {}",
            plain(&app.to_string()),
            words::byte_limit(u.limits.memory_bytes),
            words::byte_limit(u.limits.disk_bytes),
            status(u),
        ),
    );
    if let Some(cost) = &u.cost {
        line(
            &mut out,
            &format!(
                "This month: {} · about {} by month end",
                dollars(cost.used_microusd),
                dollars(cost.forecast_microusd)
            ),
        );
        if *detail {
            out.push_str(&breakdown(cost));
        }
    }
    let read = !u.points.is_empty();
    // Never read at all, as just after a first deploy, or only before this range.
    let never = !read && u.disk_used_bytes.is_none();
    let readings = if never {
        "no readings yet".to_owned()
    } else if !read {
        "no readings in it".to_owned()
    } else if u.totals.seconds_with_readings + u64::from(u.step_seconds)
        >= u64::from(u.range.seconds())
    {
        // Short of the range by at most a step, which can be part-read.
        "readings for all of it".to_owned()
    } else {
        format!(
            "readings for {} of it",
            // A point's counts may reach back past the range's start.
            span(
                u.totals
                    .seconds_with_readings
                    .min(u64::from(u.range.seconds()))
            )
        )
    };
    let range = u.range.words().trim_start_matches("the ");
    line(
        &mut out,
        &if read {
            format!(
                "{range}, a point every {}; {readings}",
                step(u.step_seconds)
            )
        } else {
            format!("{range}; {readings}")
        },
    );
    out.push('\n');

    let t = &u.totals;
    let dash = || "-".to_owned();
    let or_dash = |value: Option<String>| value.unwrap_or_else(dash);
    line(&mut out, &row("", "now", "average", "peak", "limit", ""));
    line(
        &mut out,
        &row(
            "cpu",
            &or_dash(u.latest.as_ref().map(|l| words::cpu(l.cpu_millicores))),
            &or_dash(t.cpu_avg_millicores.map(words::cpu)),
            &or_dash(t.cpu_peak_millicores.map(words::cpu)),
            &words::cpu_limit(u.limits.cpu_millicores),
            &if read {
                let share = t.cpu_throttled_seconds / t.seconds_with_readings as f64;
                format!("at its limit {} of the time", words::percent(share))
            } else {
                String::new()
            },
        ),
    );
    line(
        &mut out,
        &row(
            "memory",
            &or_dash(u.latest.as_ref().map(|l| words::bytes(l.memory_bytes))),
            &or_dash(t.memory_avg_bytes.map(words::bytes)),
            &or_dash(t.memory_peak_bytes.map(words::bytes)),
            &words::byte_limit(u.limits.memory_bytes),
            &match (read, t.oom_kills) {
                (false, _) => String::new(),
                (true, 0) => "never ran out".to_owned(),
                (true, kills) => format!("ran out {}", words::times(kills)),
            },
        ),
    );
    let disk = u.disk_used_bytes.map(|used| {
        let share = used as f64 / u.limits.disk_bytes as f64;
        // Rounded down, as the problems' are, so 90% shows only once it's 90%.
        let percent = (share * 100.0).floor();
        let free = words::bytes(u.limits.disk_bytes.saturating_sub(used));
        (words::bytes(used), format!("{percent}% used, {free} free"))
    });
    let (held, note) = disk.unwrap_or_else(|| (dash(), String::new()));
    line(
        &mut out,
        &row(
            "disk",
            &held,
            "",
            "",
            &words::byte_limit(u.limits.disk_bytes),
            &note,
        ),
    );
    let network = if read {
        format!(
            "in {}, out {} over the range",
            words::traffic(t.bytes_in),
            words::traffic(t.bytes_out)
        )
    } else if never {
        "none yet".to_owned()
    } else {
        "none in the range".to_owned()
    };
    line(&mut out, &format!("{:<10}{network}", "network"));
    out.push('\n');

    if never {
        line(
            &mut out,
            "No readings yet: its host reads what it uses every 30 seconds while it runs.",
        );
    } else {
        let hidden = u.gaps.len().saturating_sub(GAPS_SHOWN);
        if hidden > 0 {
            line(
                &mut out,
                &format!(
                    "no readings: {hidden} more {} earlier in the range; --json lists them",
                    if hidden == 1 { "time" } else { "times" }
                ),
            );
        }
        for gap in u.gaps.iter().skip(hidden) {
            line(
                &mut out,
                &match gap.to {
                    // Both ends carry their date if either is on another day
                    // than now, so a stretch never reads as going backwards.
                    // Its ends are steps' ends, which no reading fell between.
                    Some(to) if gap.from.date_naive() != u.to.date_naive() => format!(
                        "no readings: {} to {} UTC",
                        words::dated(gap.from),
                        words::dated(to)
                    ),
                    Some(to) => format!(
                        "no readings: {} to {} UTC",
                        words::clock(gap.from, u.to),
                        words::clock(to, u.to)
                    ),
                    None => format!("no readings: since {} UTC", words::clock(gap.from, u.to)),
                },
            );
        }
    }
    for problem in &u.problems {
        let last = problem
            .last_at
            .map(|at| format!(" The last was counted {} UTC.", step_at(at, u)))
            .unwrap_or_default();
        line(
            &mut out,
            &format!(
                "problem: {}: {}{last}",
                problem.code.as_str(),
                plain(&lower(&problem.message))
            ),
        );
        line(&mut out, &format!("fix: {}", plain(&lower(&problem.fix))));
    }
    let next = (u.status == AppStatus::Crashed).then(|| crashed(app));
    if u.problems.is_empty() && read && next.is_none() {
        line(
            &mut out,
            &if u.problems_from > u.from {
                "No problems since its last deploy.".to_owned()
            } else {
                format!("No problems in {}.", u.range.words())
            },
        );
    }
    (out, next)
}

/// One row of the table: its name, then now, average, peak and limit, then a
/// note.
fn row(name: &str, now: &str, average: &str, peak: &str, limit: &str, note: &str) -> String {
    format!("{name:<10}{now:<12}{average:<12}{peak:<12}{limit:<18}{note}")
}

fn status(u: &AppUsage) -> String {
    let status = match u.status {
        AppStatus::Starting => "starting",
        AppStatus::Running => "running",
        AppStatus::Restarting => "restarting",
        AppStatus::Crashed => "crashed",
        AppStatus::Stopped => "stopped",
        AppStatus::Failed => "failed",
        AppStatus::Asleep => "asleep",
    };
    match u.restarts {
        0 => status.to_owned(),
        1 => format!("{status}, restarted once"),
        n => format!("{status}, restarted {n} times"),
    }
}

/// Which point counted something, as a sentence ends with it: in the 1h range
/// the reading at its time, and in the others the step from its start, so a
/// time is never stated more exactly than it's known.
fn step_at(at: chrono::DateTime<chrono::Utc>, u: &AppUsage) -> String {
    if u.range == control_api::UsageRange::Hour {
        let time = if at.date_naive() == u.to.date_naive() {
            at.format("%H:%M:%S").to_string()
        } else {
            at.format("%-d %b %H:%M:%S").to_string()
        };
        return format!("by the reading at {time}");
    }
    format!(
        "in the {} from {}",
        step(u.step_seconds),
        words::clock(at, u.to)
    )
}

fn step(seconds: u32) -> String {
    match seconds {
        3600 => "hour".to_owned(),
        s if s % 60 == 0 => format!("{} minutes", s / 60),
        s => format!("{s} seconds"),
    }
}

/// How long, such as `19 h 3 min` or `4 d 2 h`.
fn span(seconds: u64) -> String {
    let (days, hours, minutes) = (seconds / 86_400, seconds / 3600 % 24, seconds / 60 % 60);
    match (days, hours, minutes) {
        (0, 0, 0) => format!("{seconds} s"),
        (0, 0, m) => format!("{m} min"),
        (0, h, 0) => format!("{h} h"),
        (0, h, m) => format!("{h} h {m} min"),
        (d, 0, _) => format!("{d} d"),
        (d, h, _) => format!("{d} d {h} h"),
    }
}

/// A sentence the API starts with a capital, as a line's second part: `It ran
/// out` becomes `it ran out`, but a word in capitals stays as it is.
fn lower(text: &str) -> String {
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(first), Some(second)) if first.is_uppercase() && !second.is_uppercase() => {
            first.to_lowercase().chain(text.chars().skip(1)).collect()
        }
        _ => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_and_steps_read_as_cli_md_writes_them() {
        assert_eq!(span(19 * 3600 + 3 * 60 + 20), "19 h 3 min");
        assert_eq!(span(45 * 60), "45 min");
        assert_eq!(span(30), "30 s");
        assert_eq!(span(4 * 86_400 + 2 * 3600 + 59), "4 d 2 h");
        assert_eq!(step(300), "5 minutes");
        assert_eq!(step(30), "30 seconds");
        assert_eq!(step(3600), "hour");
        assert_eq!(lower("It ran out"), "it ran out");
        assert_eq!(lower("10 GiB is the most"), "10 GiB is the most");
        assert_eq!(lower("OK then"), "OK then");
    }
}
