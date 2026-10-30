//! Generates the control API's client from its OpenAPI spec, `api/control.json`,
//! with progenitor, each time the crate builds. It's never checked in.

use std::error::Error;
use std::path::Path;

fn main() -> Result<(), Box<dyn Error>> {
    bundle_skill()?;
    let spec_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../api/control.json");
    println!("cargo::rerun-if-changed={}", spec_path.display());
    let spec: openapiv3::OpenAPI = serde_json::from_slice(&std::fs::read(&spec_path)?)?;

    let mut settings = progenitor::GenerationSettings::default();
    settings
        .with_interface(progenitor::InterfaceStyle::Positional)
        // The spec's own types for these come from the crates the CLI checks
        // them with, so the client sends exactly what was checked.
        .with_replacement("AppSpec", "appspec::AppSpec", std::iter::empty())
        .with_replacement("Resources", "appspec::Resources", std::iter::empty())
        .with_replacement("FileEntry", "manifest::FileEntry", std::iter::empty())
        .with_replacement(
            "SecretRequest",
            "control_api::SecretRequest",
            std::iter::empty(),
        )
        .with_replacement("PullBundle", "control_api::PullBundle", std::iter::empty())
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

/// Embed the complete Markdown bundle; watch directories so added/removed references rebuild.
fn bundle_skill() -> Result<(), Box<dyn Error>> {
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
