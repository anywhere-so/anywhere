//! General-purpose app sizes: limits on each app's sandbox. These are product
//! defaults, not measured needs.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::SpecError;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Size {
    #[default]
    Small,
    Medium,
    Large,
    Xl,
}

impl Size {
    pub fn memory_bytes(self) -> u64 {
        u64::from(self.millicpus() / 1000) * (2 << 30)
    }

    pub fn millicpus(self) -> u32 {
        match self {
            Self::Small => 1000,
            Self::Medium => 2000,
            Self::Large => 4000,
            Self::Xl => 8000,
        }
    }
}

impl std::fmt::Display for Size {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Small => "small",
            Self::Medium => "medium",
            Self::Large => "large",
            Self::Xl => "xl",
        })
    }
}

impl std::str::FromStr for Size {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "small" => Ok(Self::Small),
            "medium" => Ok(Self::Medium),
            "large" => Ok(Self::Large),
            "xl" => Ok(Self::Xl),
            _ => Err("choose small, medium, large or xl"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Resources {
    /// Shared CPUs and the memory limit; small, the default, is 1 vCPU and
    /// 2 GiB.
    pub size: Size,
    /// Persistent disk quota in GiB, independent of compute. Defaults to 10.
    pub disk_gib: u64,
}

impl Default for Resources {
    fn default() -> Self {
        Self {
            size: Size::Small,
            disk_gib: 10,
        }
    }
}

impl Resources {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    pub fn check(&self) -> Result<(), SpecError> {
        if self.disk_gib == 0 || self.disk_gib > (i64::MAX as u64) / (1 << 30) {
            return Err(SpecError::InvalidDisk);
        }
        Ok(())
    }

    pub fn disk_bytes(self) -> Result<u64, SpecError> {
        self.check()?;
        Ok(self.disk_gib * (1 << 30))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_explicit_resources_survive_toml_and_json() {
        let base = "title = \"Site\"\nstatic = \"public\"\n";
        let default = crate::parse(base).unwrap();
        assert_eq!(default.resources, Resources::default());
        assert_eq!(default.resources.size, Size::Small);
        // The default is left out of the JSON, and anything else is written,
        // medium included, so a stored spec without `resources` is small.
        assert_eq!(
            serde_json::to_value(&default).unwrap().get("resources"),
            None
        );
        let medium = crate::parse(&format!("{base}[resources]\nsize = \"medium\"\n")).unwrap();
        assert_eq!(
            serde_json::to_value(&medium).unwrap()["resources"],
            serde_json::json!({ "size": "medium", "disk_gib": 10 })
        );
        for (size, cpus, gib) in [
            ("small", 1, 2),
            ("medium", 2, 4),
            ("large", 4, 8),
            ("xl", 8, 16),
        ] {
            let spec = crate::parse(&format!(
                "{base}[resources]\nsize = \"{size}\"\ndisk_gib = 30\n"
            ))
            .unwrap();
            assert_eq!(spec.resources.size.millicpus(), cpus * 1000);
            assert_eq!(spec.resources.size.memory_bytes(), gib << 30);
            assert_eq!(spec.resources.disk_bytes().unwrap(), 30 << 30);
            let json = serde_json::to_value(&spec).unwrap();
            assert_eq!(
                serde_json::from_value::<crate::AppSpec>(json).unwrap(),
                spec
            );
        }
    }

    #[test]
    fn refuses_unknown_sizes_fields_and_invalid_disk_quotas() {
        for fields in [
            "size = \"huge\"",
            "cpu = 4",
            "disk_gib = 0",
            "disk_gib = -1",
            "disk_gib = 0.5",
            "disk_gib = 8589934592",
        ] {
            assert!(
                crate::parse(&format!(
                    "title = \"Site\"\nstatic = \"public\"\n[resources]\n{fields}"
                ))
                .is_err(),
                "{fields}"
            );
        }
        assert_eq!(
            Resources {
                size: Size::Medium,
                disk_gib: u64::MAX
            }
            .check(),
            Err(SpecError::InvalidDisk)
        );
    }
}
