//! Requests stay optional until deploy; stored specs carry all three numbers.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::SpecError;

// Match hostd's CPU and memory minimums; disk must have a positive quota.
pub const MIN_CPU: u32 = 10;
pub const MIN_MEMORY: u64 = 16 << 20;
pub const MIN_DISK: u64 = 1;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LimitRequests {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_millicores: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disk_bytes: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub cpu_millicores: u32,
    pub memory_bytes: u64,
    pub disk_bytes: u64,
}

impl LimitRequests {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
    pub fn check(&self) -> Result<(), SpecError> {
        for (name, value, min, max) in [
            (
                "cpu",
                self.cpu_millicores.map(u64::from),
                u64::from(MIN_CPU),
                u64::from(u32::MAX),
            ),
            ("memory", self.memory_bytes, MIN_MEMORY, i64::MAX as u64),
            ("disk", self.disk_bytes, MIN_DISK, i64::MAX as u64),
        ] {
            if value.is_some_and(|value| value < min || value > max) {
                return Err(bad(
                    name,
                    &format!(
                        "must be between {min} and {max} {}",
                        if name == "cpu" { "millicores" } else { "bytes" }
                    ),
                ));
            }
        }
        Ok(())
    }
    pub fn resolve(self, ceiling: Limits) -> Limits {
        Limits {
            cpu_millicores: self.cpu_millicores.unwrap_or(ceiling.cpu_millicores),
            memory_bytes: self.memory_bytes.unwrap_or(ceiling.memory_bytes),
            disk_bytes: self.disk_bytes.unwrap_or(ceiling.disk_bytes),
        }
    }
    /// Stored specs must already be resolved. Missing values are a data error.
    pub fn resolved(self) -> Result<Limits, SpecError> {
        Ok(Limits {
            cpu_millicores: self
                .cpu_millicores
                .ok_or_else(|| bad("cpu", "is unresolved in the stored spec"))?,
            memory_bytes: self
                .memory_bytes
                .ok_or_else(|| bad("memory", "is unresolved in the stored spec"))?,
            disk_bytes: self
                .disk_bytes
                .ok_or_else(|| bad("disk", "is unresolved in the stored spec"))?,
        })
    }
}
impl From<Limits> for LimitRequests {
    fn from(value: Limits) -> Self {
        Self {
            cpu_millicores: Some(value.cpu_millicores),
            memory_bytes: Some(value.memory_bytes),
            disk_bytes: Some(value.disk_bytes),
        }
    }
}
impl Limits {
    pub fn check(self) -> Result<(), SpecError> {
        LimitRequests::from(self).check()
    }
}
fn bad(limit: &str, reason: &str) -> SpecError {
    SpecError::InvalidLimit {
        limit: limit.to_owned(),
        reason: reason.to_owned(),
    }
}
/// The smallest positive app.toml disk value that holds these measured bytes.
pub fn disk_limit_value(used: u64) -> String {
    let mib = used.div_ceil(1 << 20).max(1);
    if mib.is_multiple_of(1024) {
        format!("{} GiB", mib / 1024)
    } else {
        format!("{mib} MiB")
    }
}
/// Whole binary units only, with exactly one space and no fractional bytes.
pub fn parse_bytes(text: &str) -> Result<u64, String> {
    let invalid = || {
        "must be a whole number followed by MiB or GiB, such as \"512 MiB\" or \"4 GiB\"".to_owned()
    };
    let (number, unit) = text.split_once(' ').ok_or_else(invalid)?;
    if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid());
    }
    let multiplier = match unit {
        "MiB" => 1 << 20,
        "GiB" => 1 << 30,
        _ => return Err(invalid()),
    };
    number
        .parse::<u64>()
        .map_err(|_| invalid())?
        .checked_mul(multiplier)
        .filter(|n| *n <= i64::MAX as u64)
        .ok_or_else(invalid)
}
/// Parse vCPUs without rounding away a requested fraction of a millicore.
pub fn parse_cpu(text: &str) -> Result<u32, String> {
    let invalid = || "must be vCPUs with at most 3 decimal places, such as 0.5 or 2".to_owned();
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || fraction.len() > 3
        || !fraction.bytes().all(|b| b.is_ascii_digit())
        || text.ends_with('.')
    {
        return Err(invalid());
    }
    let part = if fraction.is_empty() {
        0
    } else {
        fraction.parse::<u32>().map_err(|_| invalid())? * 10u32.pow(3 - fraction.len() as u32)
    };
    whole
        .parse::<u32>()
        .map_err(|_| invalid())?
        .checked_mul(1000)
        .and_then(|n| n.checked_add(part))
        .ok_or_else(invalid)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TomlLimits {
    cpu: Option<toml::Value>,
    memory: Option<String>,
    disk: Option<String>,
}
impl TomlLimits {
    pub(crate) fn requests(self) -> Result<LimitRequests, SpecError> {
        let cpu_millicores = self
            .cpu
            .map(|v| {
                match v {
                    toml::Value::Integer(n) => parse_cpu(&n.to_string()),
                    toml::Value::Float(n) => parse_cpu(&n.to_string()),
                    _ => Err("must be a number of vCPUs, such as 0.5 or 2".to_owned()),
                }
                .map_err(|reason| bad("cpu", &reason))
            })
            .transpose()?;
        let bytes = |name, v: Option<String>| {
            v.map(|s| parse_bytes(&s).map_err(|reason| bad(name, &reason)))
                .transpose()
        };
        Ok(LimitRequests {
            cpu_millicores,
            memory_bytes: bytes("memory", self.memory)?,
            disk_bytes: bytes("disk", self.disk)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const BASE: &str = "title = \"Site\"\nstatic = \"public\"\n";
    #[test]
    fn optional_limits_parse_units_and_keep_exact_numbers_on_the_wire() {
        assert!(crate::parse(BASE).unwrap().limits.is_empty());
        for (cpu, millis) in [("0.01", 10), ("0.5", 500), ("1", 1000), ("1.001", 1001)] {
            for (memory, bytes) in [("16 MiB", 16 << 20), ("2 GiB", 2 << 30)] {
                for (disk, disk_bytes) in [("1 MiB", 1 << 20), ("4 GiB", 4 << 30)] {
                    let spec = crate::parse(&format!(
                        "{BASE}[limits]\ncpu = {cpu}\nmemory = \"{memory}\"\ndisk = \"{disk}\"\n"
                    ))
                    .unwrap();
                    let expected = Limits {
                        cpu_millicores: millis,
                        memory_bytes: bytes,
                        disk_bytes,
                    };
                    assert_eq!(spec.limits.resolved().unwrap(), expected);
                    let json = serde_json::to_value(&spec).unwrap();
                    assert_eq!(json["limits"], serde_json::to_value(expected).unwrap());
                    assert_eq!(
                        serde_json::from_value::<crate::AppSpec>(json).unwrap(),
                        spec
                    );
                }
            }
        }
    }
    #[test]
    fn refuses_old_sections_bad_units_fractions_and_limits_too_small_to_start() {
        for fields in [
            "cpu = 0",
            "cpu = 0.009",
            "cpu = 0.1234",
            "cpu = -1",
            "cpu = nan",
            "cpu = \"0.5\"",
            "memory = \"15 MiB\"",
            "memory = \"0 GiB\"",
            "disk = \"0 MiB\"",
            "disk = \"1.5 GiB\"",
            "memory = \"0.5 GiB\"",
            "disk = \"4 GB\"",
            "memory = \"512 MB\"",
            "disk = \"4GiB\"",
            "disk = \"4  GiB\"",
            "disk = \"4 GiB \"",
            "disk = \"8589934592 GiB\"",
            "size = \"small\"",
            "disk_gib = 10",
            "memory_bytes = 536870912",
        ] {
            assert!(
                crate::parse(&format!("{BASE}[limits]\n{fields}")).is_err(),
                "{fields}"
            );
        }
        let error = crate::parse(&format!("{BASE}[resources]\nsize = \"small\""))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("[resources] was replaced by [limits]"),
            "{error}"
        );
    }
}
