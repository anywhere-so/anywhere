//! What an app used over a range, against its limits, as
//! `GET /v1/apps/{app}/usage` answers: the portal's Usage section and
//! `anywhere usage` both read it, and its problems, worked out by the control
//! plane, say the same thing in the same words to a person and an agent.

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{AppStatus, Plan};

/// How to read an app's usage.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UsageQuery {
    /// The range: `1h`, `24h`, `7d` or `30d`, up to now. Without it, `24h`;
    /// anything else is `range_invalid`.
    pub range: Option<String>,
    /// The username of the app's owner, for an app whose owner made you a
    /// developer. Without it, the app is yours.
    pub owner: Option<String>,
}

/// A range of usage, up to now, and the step its points are made at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub enum UsageRange {
    /// The last hour: a point for each 30-second reading.
    #[serde(rename = "1h")]
    Hour,
    /// The last 24 hours, in 5-minute steps.
    #[serde(rename = "24h")]
    Day,
    /// The last 7 days, in hours.
    #[serde(rename = "7d")]
    Week,
    /// The last 30 days, in hours.
    #[serde(rename = "30d")]
    Month,
}

impl UsageRange {
    pub const ALL: [UsageRange; 4] = [Self::Hour, Self::Day, Self::Week, Self::Month];

    /// The range a query names, as `1h`, `24h`, `7d` or `30d`.
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|range| range.as_str() == text)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hour => "1h",
            Self::Day => "24h",
            Self::Week => "7d",
            Self::Month => "30d",
        }
    }

    /// How long the range is, in seconds.
    pub fn seconds(self) -> u32 {
        match self {
            Self::Hour => 3600,
            Self::Day => 86_400,
            Self::Week => 7 * 86_400,
            Self::Month => 30 * 86_400,
        }
    }

    /// How long each of its steps is, in seconds.
    pub fn step_seconds(self) -> u32 {
        match self {
            Self::Hour => 30,
            Self::Day => 300,
            Self::Week | Self::Month => 3600,
        }
    }

    /// The range in words, as "in the last 24 hours" ends.
    pub fn words(self) -> &'static str {
        match self {
            Self::Hour => "the last hour",
            Self::Day => "the last 24 hours",
            Self::Week => "the last 7 days",
            Self::Month => "the last 30 days",
        }
    }
}

impl std::fmt::Display for UsageRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What an app used over a range, against its limits, and what to change if it
/// ran into them.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct AppUsage {
    /// The owner's current usage period's dollars; only the owner sees them.
    pub cost: Option<UsageCost>,
    /// The app's name.
    pub app: String,
    /// How the app is doing now, as on `App`.
    pub status: AppStatus,
    /// How many times its host has restarted it since its last deploy or
    /// change to its secrets, as on `App`.
    pub restarts: u32,
    pub range: UsageRange,
    /// Where the range starts: `to` minus the range.
    pub from: DateTime<Utc>,
    /// When the answer was made.
    pub to: DateTime<Utc>,
    /// How long each point's step is: 30, 300 or 3600 seconds.
    pub step_seconds: u32,
    pub limits: UsageLimits,
    /// The newest reading, if it's under 90 seconds old. None while the app
    /// isn't running. It's "now" in every range.
    pub latest: Option<LatestUsage>,
    /// What its disk holds, from the newest reading that had it, whether or
    /// not the app runs now. None before its first.
    pub disk_used_bytes: Option<u64>,
    /// Over the whole range, from the points.
    pub totals: UsageTotals,
    /// Each stretch of whole steps with no reading, since the app's first
    /// reading, oldest first, such as while it was stopped or had crashed, or
    /// its host couldn't read it. Before its first reading there's no stretch.
    pub gaps: Vec<UsageGap>,
    /// Where `problems` count from: the app's newest deploy, if it's in the
    /// range, and else `from`. Their counts start with the first step that
    /// starts at or after it, so a problem a deploy fixed doesn't linger, and
    /// its words use the limits that ran.
    pub problems_from: DateTime<Utc>,
    /// What the app ran into since `problems_from`, each with what to change.
    pub problems: Vec<UsageProblem>,
    /// One for each step that had a reading, oldest first. A step with none
    /// has no point, never one of zeros.
    pub points: Vec<UsagePoint>,
}

/// The app's resolved limits.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct UsageLimits {
    /// Its shared CPUs, in thousandths of a CPU.
    pub cpu_millicores: u64,
    /// Its memory cap.
    pub memory_bytes: u64,
    /// Its disk quota, in bytes.
    pub disk_bytes: u64,
}

/// The newest reading.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct LatestUsage {
    pub at: DateTime<Utc>,
    /// Its CPU over the time since the reading before, in thousandths of a
    /// CPU.
    pub cpu_millicores: u64,
    /// What its sandbox holds on the host, gVisor's own included but not the
    /// files it has cached, so below what `free` shows inside the app.
    pub memory_bytes: u64,
}

/// What the app used over the whole range. Averages and peaks are None with
/// no readings; sums are 0.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct UsageTotals {
    /// The time the points' counts cover, the sum of their `seconds`. It
    /// starts at an app's first reading, which only starts its counts.
    pub seconds_with_readings: u64,
    /// Its CPU over the seconds with readings, in thousandths of a CPU.
    pub cpu_avg_millicores: Option<u64>,
    /// The highest of its readings', each over the time it covers.
    pub cpu_peak_millicores: Option<u64>,
    /// How long it was held back for wanting more than its CPU limit.
    pub cpu_throttled_seconds: f64,
    pub memory_avg_bytes: Option<u64>,
    pub memory_peak_bytes: Option<u64>,
    /// How many times it ran out of memory, each time ended and restarted.
    pub oom_kills: u64,
    /// What it received and sent over its network link.
    pub bytes_in: u64,
    pub bytes_out: u64,
}

/// A stretch of whole steps with no reading.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct UsageGap {
    pub from: DateTime<Utc>,
    /// None while it goes on now.
    pub to: Option<DateTime<Utc>>,
}

/// What the app used in one step.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct UsagePoint {
    /// The step's cost at the price book effective at `at`, in integer
    /// micro-dollars, with each component rounded down. Priced from this
    /// step's averages and seconds, so it may differ from the hourly ledger.
    /// None for a developer; only the owner sees costs.
    pub cost_microusd: Option<u64>,
    /// The step's start, or in the 1h range, the reading's time, its counts
    /// covering the `seconds` before it.
    pub at: DateTime<Utc>,
    /// The time its counts cover: for each reading, the time since the one
    /// before, so more than a step after missed reports while the app ran on,
    /// and at most 30 for a run's first reading. Divide rates by this, not
    /// `step_seconds`, so a step the app ran part of isn't diluted.
    pub seconds: u32,
    /// Its CPU over `seconds`, in thousandths of a CPU.
    pub cpu_avg_millicores: u64,
    /// The highest of its readings' in the step, each over the time it
    /// covers.
    pub cpu_peak_millicores: u64,
    pub cpu_throttled_seconds: f64,
    pub memory_avg_bytes: u64,
    pub memory_peak_bytes: u64,
    /// The most its disk held in the step, or None if no reading in it had
    /// its disk.
    pub disk_used_bytes: Option<u64>,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub oom_kills: u64,
}

/// Something the app ran into over the range, and what to do about it.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct UsageProblem {
    pub code: UsageProblemCode,
    pub severity: UsageSeverity,
    /// What happened, with its numbers, in a sentence or two, and no time of
    /// day: `last_at` has that.
    pub message: String,
    /// What to do, in words for a person and an agent alike: the `app.toml`
    /// line to change and `anywhere deploy`, or what else would help.
    pub fix: String,
    /// The one `app.toml` edit that fixes it, which an agent can apply and
    /// deploy, or None when no edit can, as for a large app that runs out of
    /// memory.
    pub change: Option<ResourceChange>,
    /// The plan `change` needs, which the app's owner doesn't have: theirs to
    /// buy, never an agent's.
    pub needs_plan: Option<Plan>,
    /// Where the owner buys it, where this platform sells it.
    pub upgrade_url: Option<String>,
    /// For `oom_kills`: the `at` of the last point that counted a kill: in
    /// the 1h range, the reading after it, as the run that ran out is never
    /// read itself; in the others, the start of the step of `step_seconds`
    /// whose readings counted it.
    pub last_at: Option<DateTime<Utc>>,
    /// What "Copy for your agent" copies: the app, what happened and what to
    /// change, with the app's title quoted as JSON writes a string.
    pub agent_prompt: String,
}

/// A problem's stable code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UsageProblemCode {
    /// It ran out of memory at least once, and was ended and restarted.
    OomKills,
    /// It was held at its CPU limit at least 5% of the time it ran.
    CpuLimited,
    /// Its disk is at least 90% full.
    DiskNearlyFull,
    /// Its disk is at least 99% full, so its writes are failing.
    DiskFull,
}

impl UsageProblemCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OomKills => "oom_kills",
            Self::CpuLimited => "cpu_limited",
            Self::DiskNearlyFull => "disk_nearly_full",
            Self::DiskFull => "disk_full",
        }
    }
}

/// How bad a problem is: a failure ended something, such as the app's
/// processes or its writes; a caution slowed it or will soon.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UsageSeverity {
    Failure,
    Caution,
}

/// One edit of the app's `app.toml`: set `key` to `value` under `[table]`.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct ResourceChange {
    /// Always `app.toml`.
    pub file: String,
    /// Always `limits`.
    pub table: String,
    pub key: ResourceKey,
    /// vCPUs for CPU, or a whole MiB or GiB string for memory and disk.
    pub value: ResourceValue,
}

impl ResourceChange {
    pub fn new(key: ResourceKey, value: ResourceValue) -> Self {
        Self {
            file: "app.toml".to_owned(),
            table: "limits".to_owned(),
            key,
            value,
        }
    }

    /// The line as `app.toml` writes it, such as `memory = "512 MiB"`.
    pub fn line(&self) -> String {
        match &self.value {
            ResourceValue::Text(text) => format!("{} = \"{text}\"", self.key.as_str()),
            ResourceValue::Cpu(cpu) => format!("{} = {cpu}", self.key.as_str()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKey {
    Cpu,
    Memory,
    Disk,
}

impl ResourceKey {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Memory => "memory",
            Self::Disk => "disk",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum ResourceValue {
    Text(String),
    Cpu(f64),
}

/// Usage in words, as the control plane's problems and the CLI write it. Memory
/// and disk are in binary units, as their limits are; traffic in decimal
/// units, as networks count it.
pub mod words {
    use chrono::{DateTime, Utc};

    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;

    /// Memory or disk, such as `412 MiB`, `1.18 GiB` or `18.9 GiB`. The unit
    /// is chosen after rounding, so 1023.6 MiB is `1 GiB`, never `1024 MiB`.
    pub fn bytes(bytes: u64) -> String {
        let b = bytes as f64;
        if b >= 1023.5 * MIB {
            let places = if b >= 10.0 * GIB { 1 } else { 2 };
            format!("{} GiB", trim(b / GIB, places))
        } else if b >= 1023.5 * KIB {
            format!("{} MiB", (b / MIB).round())
        } else {
            format!("{} KiB", (b / KIB).round())
        }
    }

    /// Traffic, such as `1.7 GB`, `48 MB` or `310 kB`. The unit is chosen
    /// after rounding, so 999.6 kB is `1.0 MB`, never `1000 kB`.
    pub fn traffic(bytes: u64) -> String {
        let b = bytes as f64;
        if b >= 999.5e6 {
            format!("{:.1} GB", b / 1e9)
        } else if b >= 99.95e6 {
            format!("{:.0} MB", b / 1e6)
        } else if b >= 999.5e3 {
            format!("{:.1} MB", b / 1e6)
        } else if b >= 999.5 {
            format!("{:.0} kB", b / 1e3)
        } else {
            format!("{bytes} B")
        }
    }

    /// CPU used, in vCPUs: two places under one, such as `0.32 vCPU`, and one
    /// from there, such as `1.1 vCPU`, chosen after rounding, so 999
    /// thousandths are `1.0 vCPU`.
    pub fn cpu(millicores: u64) -> String {
        let places = if millicores < 995 { 2 } else { 1 };
        format!("{:.*} vCPU", places, millicores as f64 / 1000.0)
    }

    /// A CPU limit, as the portal and the site word it: in whole vCPUs where
    /// it's whole, shared, and plural only past one, such as `0.5 shared vCPU`,
    /// `1 shared vCPU` or `2 shared vCPUs`.
    pub fn cpu_limit(millicores: u64) -> String {
        let whole = millicores / 1000;
        let part = millicores % 1000;
        let cpus = if part == 0 {
            whole.to_string()
        } else {
            format!("{whole}.{part:03}")
                .trim_end_matches('0')
                .to_owned()
        };
        let plural = if millicores > 1000 { "s" } else { "" };
        format!("{cpus} shared vCPU{plural}")
    }

    /// Exact limits in the whole units app.toml accepts. Measurements use
    /// `bytes` instead; rounding a ceiling can prescribe an invalid retry.
    pub fn byte_limit(bytes: u64) -> String {
        if bytes.is_multiple_of(1 << 30) {
            format!("{} GiB", bytes >> 30)
        } else if bytes.is_multiple_of(1 << 20) {
            format!("{} MiB", bytes >> 20)
        } else {
            format!("{bytes} bytes")
        }
    }

    /// A share, as a whole percentage, such as `31%`.
    pub fn percent(share: f64) -> String {
        format!("{:.0}%", share * 100.0)
    }

    /// How many times: `once`, `twice` or `6 times`.
    pub fn times(n: u64) -> String {
        match n {
            1 => "once".to_owned(),
            2 => "twice".to_owned(),
            n => format!("{n} times"),
        }
    }

    /// A moment, in UTC, as a line about a range up to `to` names it: its time
    /// alone on the day `to` is in, such as `14:08`, and its date too before,
    /// such as `26 Oct 14:08`. The caller adds `UTC`.
    pub fn clock(at: DateTime<Utc>, to: DateTime<Utc>) -> String {
        if at.date_naive() == to.date_naive() {
            at.format("%H:%M").to_string()
        } else {
            dated(at)
        }
    }

    /// A moment, in UTC, with its date, such as `26 Oct 14:08`.
    pub fn dated(at: DateTime<Utc>) -> String {
        at.format("%-d %b %H:%M").to_string()
    }

    /// `value` to at most `places` decimal places, without trailing zeros.
    fn trim(value: f64, places: usize) -> String {
        let text = format!("{value:.places$}");
        if text.contains('.') {
            text.trim_end_matches('0').trim_end_matches('.').to_owned()
        } else {
            text
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_match_the_units_limits_are_given_in() {
        use words::*;
        const GIB: u64 = 1 << 30;
        const MIB: u64 = 1 << 20;
        assert_eq!(bytes(412 * MIB), "412 MiB");
        assert_eq!(bytes(1_267_015_680), "1.18 GiB");
        assert_eq!(bytes(2 * GIB), "2 GiB");
        assert_eq!(bytes(2_340_757_504), "2.18 GiB");
        assert_eq!(bytes(GIB * 189 / 10), "18.9 GiB");
        assert_eq!(bytes(0), "0 KiB");
        // The unit is chosen after rounding.
        assert_eq!(bytes(1023 * MIB + 600 * 1024), "1 GiB");
        assert_eq!(bytes(1023 * MIB), "1023 MiB");
        assert_eq!(bytes(1023 * 1024 + 600), "1 MiB");
        assert_eq!(traffic(999_600), "1.0 MB");
        assert_eq!(traffic(99_960_000), "100 MB");
        assert_eq!(traffic(999_600_000), "1.0 GB");
        assert_eq!(traffic(999_400), "999 kB");
        assert_eq!(cpu(999), "1.0 vCPU");
        assert_eq!(cpu(994), "0.99 vCPU");
        assert_eq!(traffic(1_702_334_112), "1.7 GB");
        assert_eq!(traffic(6_593_120_040), "6.6 GB");
        assert_eq!(traffic(310_000), "310 kB");
        assert_eq!(traffic(48_400_000), "48.4 MB");
        assert_eq!(traffic(512), "512 B");
        assert_eq!(cpu(320), "0.32 vCPU");
        assert_eq!(cpu(2000), "2.0 vCPU");
        assert_eq!(cpu(1100), "1.1 vCPU");
        assert_eq!(cpu_limit(500), "0.5 shared vCPU");
        assert_eq!(cpu_limit(1000), "1 shared vCPU");
        assert_eq!(cpu_limit(1500), "1.5 shared vCPUs");
        assert_eq!(cpu_limit(2000), "2 shared vCPUs");
        assert_eq!(percent(0.31), "31%");
        assert_eq!(times(1), "once");
        assert_eq!(times(6), "6 times");
        let to = "2026-10-27T14:30:00Z".parse().unwrap();
        assert_eq!(clock("2026-10-27T14:08:00Z".parse().unwrap(), to), "14:08");
        assert_eq!(
            clock("2026-10-26T02:00:00Z".parse().unwrap(), to),
            "26 Oct 02:00"
        );
    }

    #[test]
    fn ranges_parse_as_queries_name_them_and_nothing_else() {
        for range in UsageRange::ALL {
            assert_eq!(UsageRange::parse(range.as_str()), Some(range));
            assert_eq!(
                serde_json::to_value(range).unwrap(),
                serde_json::json!(range.as_str())
            );
            assert_eq!(range.seconds() % range.step_seconds(), 0);
        }
        for text in ["", "1d", "24H", "30", "1h ", "90d"] {
            assert_eq!(UsageRange::parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn a_change_is_the_line_app_toml_takes() {
        for (key, value, line) in [
            (
                ResourceKey::Memory,
                ResourceValue::Text("512 MiB".to_owned()),
                "memory = \"512 MiB\"",
            ),
            (
                ResourceKey::Disk,
                ResourceValue::Text("30 GiB".to_owned()),
                "disk = \"30 GiB\"",
            ),
            (ResourceKey::Cpu, ResourceValue::Cpu(0.5), "cpu = 0.5"),
        ] {
            let change = ResourceChange::new(key, value);
            assert_eq!(change.line(), line);
            let json = serde_json::to_value(&change).unwrap();
            assert_eq!(json["table"], "limits");
            let text = format!(
                "title = \"Test\"\nstatic = \"public\"\n[limits]\n{}\n",
                change.line()
            );
            assert!(appspec::parse(&text).is_ok(), "{text}");
        }
    }
}

/// Whole micro-dollars charged in the owner's current usage period, and its
/// forecast to the period's end.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct UsageCost {
    pub used_microusd: u64,
    pub forecast_microusd: u64,
    pub memory_microusd: u64,
    pub cpu_microusd: u64,
    pub disk_microusd: u64,
    pub egress_microusd: u64,
}

/// Published rates. A disk month is 730 hours.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct UsageRates {
    pub memory_gib_hour_microusd: u64,
    pub cpu_vcpu_hour_microusd: u64,
    pub disk_gib_month_microusd: u64,
    pub egress_gib_microusd: u64,
}

/// A usage period, a month long: a calendar month in UTC on Free, and from
/// the subscription's billing date on Pro. A change of plan starts a new one.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct UsagePeriod {
    pub from: DateTime<Utc>,
    pub resets_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct AccountAppUsage {
    pub app: String,
    pub title: String,
    pub deleted: bool,
    #[serde(flatten)]
    pub cost: UsageCost,
}

/// The signed-in account alone. Percentages round down and may exceed 100.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct AccountUsage {
    pub state: UsageState,
    pub plan: Plan,
    pub period: UsagePeriod,
    /// The end of the newest charged hour, or null before any charges this period.
    pub charged_through: Option<DateTime<Utc>>,
    pub included_microusd: u64,
    /// Top-up amounts offered by the account's current plan terms.
    pub topup_amounts_microusd: Vec<u64>,
    /// Available usage left after this period's charges, floored at zero.
    pub topup_balance_microusd: u64,
    /// This period's included amount plus grants made before its end, minus
    /// earlier periods' overage, floored at zero. Remaining usage is this
    /// amount minus used_microusd; the limit is reached when used >= available.
    pub available_microusd: u64,
    pub used_microusd: u64,
    /// Null when the available amount is zero: no finite percentage exists.
    pub percent_used: Option<u64>,
    pub forecast_microusd: u64,
    pub forecast_percent: Option<u64>,
    pub apps: Vec<AccountAppUsage>,
    pub rates: UsageRates,
    pub enforced: bool,
}

/// The account's usage against its available amount, even with enforcement off.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UsageState {
    Ok,
    #[serde(rename = "warned_75")]
    Warned75,
    #[serde(rename = "warned_90")]
    Warned90,
    Stopped,
}
impl UsageState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warned75 => "warned_75",
            Self::Warned90 => "warned_90",
            Self::Stopped => "stopped",
        }
    }
}
