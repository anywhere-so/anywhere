//! Generates the control API's client from its OpenAPI spec, `api/control.json`,
//! with progenitor, each time the crate builds. It's never checked in. It also
//! stamps the CLI's version with the commit a release builds it from.

use std::error::Error;
use std::path::Path;

fn main() -> Result<(), Box<dyn Error>> {
    let version = stamp_version()?;
    // The target, for `anywhere update` to ask for the platform's build of it.
    println!(
        "cargo::rustc-env=ANYWHERE_TARGET={}",
        std::env::var("TARGET")?
    );
    bundle_skill(&version)?;
    let spec_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../api/control.json");
    println!("cargo::rerun-if-changed={}", spec_path.display());
    let spec: openapiv3::OpenAPI = serde_json::from_slice(&std::fs::read(&spec_path)?)?;

    let mut settings = progenitor::GenerationSettings::default();
    for name in [
        "AppIdentity",
        "SleepSettings",
        "SleepOverrides",
        "AppSleepSettings",
        "Workspace",
        "WorkspaceKind",
        "WorkspaceRole",
        "WorkspaceList",
        "WorkspaceCreate",
        "WorkspaceMembers",
        "WorkspaceMember",
        "WorkspaceMemberUpdate",
        "WorkspaceInvite",
        "WorkspaceInvitationCreated",
        "WorkspaceInvitationAccept",
        "WorkspaceApps",
        "WorkspaceApp",
        "WorkspaceAppAccess",
        "WorkspaceAppSharing",
        "WorkspaceAppGrantUpdate",
        "AppCapabilities",
    ] {
        settings.with_replacement(name, format!("control_api::{name}"), std::iter::empty());
    }

    settings
        .with_interface(progenitor::InterfaceStyle::Positional)
        // The spec's own types for these come from the crates the CLI checks
        // them with, so the client sends exactly what was checked.
        .with_replacement(
            "DeployRequest",
            "control_api::DeployRequest",
            std::iter::empty(),
        )
        .with_replacement("AppSpec", "appspec::AppSpec", std::iter::empty())
        .with_replacement("Limits", "appspec::Limits", std::iter::empty())
        .with_replacement(
            "LimitRequests",
            "appspec::LimitRequests",
            std::iter::empty(),
        )
        .with_replacement("FileEntry", "manifest::FileEntry", std::iter::empty())
        .with_replacement(
            "SecretRequest",
            "control_api::SecretRequest",
            std::iter::empty(),
        )
        .with_replacement(
            "SecretsRequest",
            "control_api::SecretsRequest",
            std::iter::empty(),
        )
        .with_replacement(
            "NamedSecret",
            "control_api::NamedSecret",
            std::iter::empty(),
        )
        .with_replacement("PullBundle", "control_api::PullBundle", std::iter::empty())
        // What `anywhere update` reads, which every CLI that updates must.
        .with_replacement("CliBuild", "control_api::CliBuild", std::iter::empty())
        // The plan, which `anywhere check` holds the app to with the control
        // plane's own rules (`control_api::plans`).
        .with_replacement("Plan", "control_api::Plan", std::iter::empty())
        .with_replacement("PlanLimits", "control_api::PlanLimits", std::iter::empty())
        // An app's usage, read with the words and rules the control plane
        // writes its problems with.
        .with_replacement(
            "AccountUsage",
            "control_api::AccountUsage",
            std::iter::empty(),
        )
        .with_replacement("AppUsage", "control_api::AppUsage", std::iter::empty())
        // An email, which the crate's own keep out of Debug.
        .with_replacement(
            "ShareRequest",
            "control_api::ShareRequest",
            std::iter::empty(),
        )
        .with_replacement(
            "ShareAddRequest",
            "control_api::ShareAddRequest",
            std::iter::empty(),
        )
        .with_replacement("AppAccess", "control_api::AppAccess", std::iter::empty())
        .with_replacement("Visibility", "control_api::Visibility", std::iter::empty())
        .with_replacement("ShareRole", "control_api::ShareRole", std::iter::empty())
        // These hold secrets, and the crate's own keep them out of Debug.
        .with_replacement(
            "AgentTokenCreated",
            "control_api::AgentTokenCreated",
            std::iter::empty(),
        )
        .with_replacement(
            "CliLoginRequest",
            "control_api::CliLoginRequest",
            std::iter::empty(),
        )
        .with_replacement(
            "CliLoginStarted",
            "control_api::CliLoginStarted",
            std::iter::empty(),
        )
        .with_replacement(
            "CliLoginTokenRequest",
            "control_api::CliLoginTokenRequest",
            std::iter::empty(),
        );
    let code = progenitor::Generator::new(&settings).generate_tokens(&spec)?;

    let out = Path::new(&std::env::var("OUT_DIR")?).join("control_api.rs");
    std::fs::write(out, code.to_string())?;
    Ok(())
}

/// Sets `ANYWHERE_VERSION`, what `--version` and the User-Agent say: the
/// crate's version, and for a release, `+` and the first 12 characters of
/// `ANYWHERE_SOURCE_REVISION`, the commit it's built from, which the release
/// bundle's builds set, so one release's CLI tells itself apart from
/// another's. A revision that isn't a full commit ID stops the build.
fn stamp_version() -> Result<String, Box<dyn Error>> {
    println!("cargo::rerun-if-env-changed=ANYWHERE_SOURCE_REVISION");
    let package = env!("CARGO_PKG_VERSION");
    let version = match std::env::var("ANYWHERE_SOURCE_REVISION") {
        Err(std::env::VarError::NotPresent) => package.to_owned(),
        Ok(revision)
            if revision.len() == 40
                && revision
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) =>
        {
            format!("{package}+{}", &revision[..12])
        }
        Ok(revision) => {
            return Err(format!(
                "ANYWHERE_SOURCE_REVISION is {revision:?}, not a commit's 40 lowercase hex digits; set it to the commit's full ID, or unset it"
            )
            .into());
        }
        Err(error) => return Err(format!("ANYWHERE_SOURCE_REVISION: {error}").into()),
    };
    println!("cargo::rustc-env=ANYWHERE_VERSION={version}");
    Ok(version)
}

/// Embed the complete Markdown bundle; watch directories so added/removed references rebuild.
/// SKILL.md says, in its frontmatter's `metadata`, the version of the CLI that
/// carries it, so an agent can tell which it has, and the CLI when to replace it.
fn bundle_skill(version: &str) -> Result<(), Box<dyn Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../skills/anywhere");
    let mut files = Vec::new();
    skill_files(&root, &root, &mut files)?;
    if !files.iter().any(|name| name == "SKILL.md") {
        return Err("the skill bundle needs SKILL.md".into());
    }
    // References are installed before the entrypoint that points at them.
    files.sort_by_key(|name| (name == "SKILL.md", name.clone()));
    let mut generated = String::from("const SKILL_FILES: &[(&str, &str)] = &[\n");
    for name in files {
        let path = root.join(&name);
        if name == "SKILL.md" {
            let stamped = stamp_skill(&std::fs::read_to_string(&path)?, version)?;
            generated.push_str(&format!("({name:?}, {stamped:?}),\n"));
            continue;
        }
        let path = path.to_str().ok_or("the skill path must be UTF-8")?;
        generated.push_str(&format!("({name:?}, include_str!({path:?})),\n"));
    }
    generated.push_str("];\n");
    std::fs::write(
        Path::new(&std::env::var("OUT_DIR")?).join("skill_bundle.rs"),
        generated,
    )?;
    Ok(())
}

/// `skill` with `metadata:` and its `version: "<version>"` at the end of its
/// frontmatter, which must have no `metadata` of its own.
fn stamp_skill(skill: &str, version: &str) -> Result<String, Box<dyn Error>> {
    let rest = skill
        .strip_prefix("---\n")
        .ok_or("SKILL.md must start with its frontmatter, `---`")?;
    let end = rest
        .find("\n---\n")
        .ok_or("SKILL.md's frontmatter must end with a `---` line")?;
    let frontmatter = &rest[..end + 1];
    if frontmatter
        .lines()
        .any(|line| line.starts_with("metadata:"))
    {
        return Err(
            "SKILL.md's frontmatter must not have `metadata`, which the build writes".into(),
        );
    }
    let at = 4 + end + 1;
    Ok(format!(
        "{}metadata:\n  version: \"{version}\"\n{}",
        &skill[..at],
        &skill[at..]
    ))
}

fn skill_files(root: &Path, folder: &Path, files: &mut Vec<String>) -> Result<(), Box<dyn Error>> {
    println!("cargo::rerun-if-changed={}", folder.display());
    for entry in std::fs::read_dir(folder)? {
        let entry = entry?;
        let path = entry.path();
        let name = path
            .strip_prefix(root)?
            .to_str()
            .ok_or("skill filenames must be UTF-8")?
            .to_owned();
        let kind = entry.file_type()?;
        if kind.is_dir() && (name == "references" || name.starts_with("references/")) {
            skill_files(root, &path, files)?;
        } else if kind.is_file()
            && path.extension().is_some_and(|ext| ext == "md")
            && (name == "SKILL.md" || name.starts_with("references/"))
        {
            files.push(name);
        } else {
            return Err(format!("unsupported skill bundle entry: {name}; use SKILL.md and regular Markdown references").into());
        }
    }
    Ok(())
}
