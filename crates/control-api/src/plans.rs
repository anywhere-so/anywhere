//! Resolve optional requests against the plan terms read at deploy time.
use crate::{Plan, PlanError as ApiError, words};
use appspec::{LimitRequests, Limits};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Asker {
    Owner,
    Developer,
}

/// Disk readings need rounding up to a value app.toml can actually express.
pub fn disk_below_used(
    requested: u64,
    used: u64,
    plan: Plan,
    asker: Asker,
    ceiling: Limits,
    pro: Option<Limits>,
    upgrade_url: Option<&str>,
) -> ApiError {
    let minimum = appspec::disk_limit_value(used);
    let fits = used.div_ceil(1 << 20) <= ceiling.disk_bytes / (1 << 20);
    let mut message = format!(
        "the app holds {used} bytes, more than the requested disk limit of {requested} bytes; the smallest disk value that holds it is \"{minimum}\""
    );
    if asker == Asker::Developer {
        message.push_str("; delete files the app no longer needs first, or ask the owner");
        if fits {
            message.push_str(&format!(
                " to set disk = \"{minimum}\" under [limits] in app.toml"
            ));
        }
    } else if fits {
        message.push_str(&format!("; set disk = \"{minimum}\" under [limits] in app.toml, or delete files the app no longer needs first"));
    } else {
        message.push_str(&format!(
            "; this exceeds the app's ceiling of {}, so delete files the app no longer needs first",
            words::byte_limit(ceiling.disk_bytes)
        ));
        if plan == Plan::Free
            && let Some(pro) = pro.filter(|pro| pro.disk_bytes > ceiling.disk_bytes)
        {
            message.push_str(&format!(
                "; Pro allows up to {} per app",
                words::byte_limit(pro.disk_bytes)
            ));
            if let Some(url) = upgrade_url {
                message.push_str(&format!("; upgrade at {url}"));
            }
        }
    }
    message.push_str(", then deploy again");
    ApiError::bad_request("disk_limit_below_used", message)
}

/// Shared by deploy admission and the signed-in CLI's check.
pub fn check_apps(
    plan: Plan,
    max_apps: Option<u32>,
    apps: u32,
    new_app: bool,
    asker: Asker,
    upgrade_url: Option<&str>,
) -> Result<(), ApiError> {
    let Some(max) = max_apps.filter(|max| new_app && apps >= *max) else {
        return Ok(());
    };
    let mut message = if asker == Asker::Developer {
        "this app's owner has reached their plan's number of apps".to_owned()
    } else {
        let plan = match plan {
            Plan::Free => "Free",
            Plan::Pro => "Pro",
        };
        format!("{plan} includes {max} apps; delete one, or upgrade to Pro for any number.")
    };
    if asker == Asker::Owner
        && let Some(url) = upgrade_url
    {
        message.push_str(&format!(" Upgrade at {url}"));
    }
    Err(ApiError::forbidden("plan_app_limit", message))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub limit: &'static str,
    pub asked: u64,
    pub ceiling: u64,
}

pub fn resolve(ceilings: Limits, wants: LimitRequests) -> Result<Limits, Refusal> {
    let resolved = wants.resolve(ceilings);
    for (limit, asked, ceiling) in [
        (
            "cpu",
            u64::from(resolved.cpu_millicores),
            u64::from(ceilings.cpu_millicores),
        ),
        ("memory", resolved.memory_bytes, ceilings.memory_bytes),
        ("disk", resolved.disk_bytes, ceilings.disk_bytes),
    ] {
        if asked > ceiling {
            return Err(Refusal {
                limit,
                asked,
                ceiling,
            });
        }
    }
    Ok(resolved)
}
impl Refusal {
    pub fn error(
        self,
        plan: Plan,
        asker: Asker,
        pro: Option<Limits>,
        upgrade_url: Option<&str>,
    ) -> ApiError {
        let describe = |n| {
            if self.limit == "cpu" {
                words::cpu_limit(n)
            } else {
                words::byte_limit(n)
            }
        };
        let mut message = format!(
            "[limits] {} asks for {}, above this app's ceiling of {}; lower {} in app.toml to at most {}, or remove it to use the ceiling, then deploy again",
            self.limit,
            describe(self.asked),
            describe(self.ceiling),
            self.limit,
            describe(self.ceiling)
        );
        if asker == Asker::Owner
            && plan == Plan::Free
            && let Some(pro) = pro
        {
            let value = match self.limit {
                "cpu" => u64::from(pro.cpu_millicores),
                "memory" => pro.memory_bytes,
                _ => pro.disk_bytes,
            };
            message.push_str(&format!("; Pro allows up to {} per app", describe(value)));
            if let Some(url) = upgrade_url {
                message.push_str(&format!("; upgrade at {url}"));
            }
        }
        ApiError::forbidden("limit_above_plan", message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const FREE: Limits = Limits {
        cpu_millicores: 1000,
        memory_bytes: 2 << 30,
        disk_bytes: 10 << 30,
    };
    const PRO: Limits = Limits {
        cpu_millicores: 4000,
        memory_bytes: 16 << 30,
        disk_bytes: 100 << 30,
    };
    #[test]
    fn disk_advice_rounds_up_and_respects_the_ceiling_and_the_callers_role() {
        for (used, value, fits) in [
            (3 << 30, "3 GiB", true),
            ((3 << 30) + 1, "3073 MiB", true),
            ((10 << 30) + 1, "10241 MiB", false),
        ] {
            for asker in [Asker::Owner, Asker::Developer] {
                let error = disk_below_used(
                    1 << 30,
                    used,
                    Plan::Free,
                    asker,
                    FREE,
                    Some(PRO),
                    Some("https://anywhere.test/billing"),
                );
                assert_eq!(error.body.code, "disk_limit_below_used");
                assert!(!error.body.retryable);
                let message = error.body.message;
                assert!(message.contains(&format!("\"{value}\"")), "{message}");
                assert_eq!(message.contains("set disk ="), fits, "{message}");
                assert_eq!(
                    message.contains("Pro allows up to 100 GiB"),
                    !fits && asker == Asker::Owner,
                    "{message}"
                );
                assert_eq!(
                    message.contains("https://anywhere.test/billing"),
                    !fits && asker == Asker::Owner,
                    "{message}"
                );
                if !fits {
                    assert!(
                        message.contains("delete files the app no longer needs first"),
                        "{message}"
                    );
                }
                if asker == Asker::Developer {
                    assert!(message.contains("ask the owner"), "{message}");
                }
            }
        }
        let message = disk_below_used(
            1,
            (100 << 30) + 1,
            Plan::Pro,
            Asker::Owner,
            PRO,
            Some(PRO),
            None,
        )
        .body
        .message;
        assert!(
            !message.contains("set disk =") && !message.contains("Pro allows"),
            "{message}"
        );
        assert!(
            message.contains("delete files the app no longer needs first"),
            "{message}"
        );
    }
    #[test]
    fn app_count_advice_keeps_the_owners_plan_private_from_developers() {
        let error = check_apps(
            Plan::Free,
            Some(3),
            3,
            true,
            Asker::Developer,
            Some("https://anywhere.test/billing"),
        )
        .unwrap_err();
        assert_eq!(error.body.code, "plan_app_limit");
        assert!(!error.body.retryable);
        assert_eq!(
            error.body.message,
            "this app's owner has reached their plan's number of apps"
        );
    }

    #[test]
    fn omissions_take_the_current_ceiling_and_explicit_requests_cannot_exceed_it() {
        assert_eq!(resolve(FREE, LimitRequests::default()), Ok(FREE));
        assert_eq!(resolve(PRO, LimitRequests::default()), Ok(PRO));
        for wants in [
            LimitRequests {
                cpu_millicores: Some(1001),
                ..Default::default()
            },
            LimitRequests {
                memory_bytes: Some(3 << 30),
                ..Default::default()
            },
            LimitRequests {
                disk_bytes: Some(11 << 30),
                ..Default::default()
            },
        ] {
            let refusal = resolve(FREE, wants).unwrap_err();
            for asker in [Asker::Owner, Asker::Developer] {
                let error = refusal.error(
                    Plan::Free,
                    asker,
                    Some(PRO),
                    Some("https://anywhere.test/billing"),
                );
                assert_eq!(error.body.code, "limit_above_plan");
                assert!(!error.body.retryable);
                for text in ["app.toml", "remove", "ceiling", refusal.limit] {
                    assert!(error.body.message.contains(text), "{}", error.body.message);
                }
                assert_eq!(
                    error.body.message.contains("Pro allows"),
                    asker == Asker::Owner
                );
                assert_eq!(
                    error.body.message.contains("billing"),
                    asker == Asker::Owner
                );
                assert!(!error.body.message.contains("Free"));
            }
        }
    }
}

#[cfg(test)]
mod precision_tests {
    use super::*;
    #[test]
    fn refusal_fixes_use_exact_configurable_cpu_and_byte_limits() {
        for (refusal, asked, ceiling) in [
            (
                Refusal {
                    limit: "cpu",
                    asked: 130,
                    ceiling: 126,
                },
                "0.13 shared vCPU",
                "0.126 shared vCPU",
            ),
            (
                Refusal {
                    limit: "memory",
                    asked: 1537 << 20,
                    ceiling: 1536 << 20,
                },
                "1537 MiB",
                "1536 MiB",
            ),
        ] {
            let message = refusal
                .error(Plan::Free, Asker::Developer, None, None)
                .body
                .message;
            assert!(message.contains(&format!("asks for {asked}")), "{message}");
            assert!(
                message.contains(&format!("ceiling of {ceiling}")),
                "{message}"
            );
            assert!(
                message.contains(&format!("to at most {ceiling}")),
                "{message}"
            );
        }
    }
}
