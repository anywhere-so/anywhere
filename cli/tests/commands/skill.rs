use super::*;

/// The CLI with `home` as its home folder, or with none.
async fn run_at_home(home: Option<&Path>, args: &[&str]) -> Output {
    let mut command = command();
    command.args(args);
    match home {
        Some(home) => command.env("HOME", home),
        None => command.env_remove("HOME"),
    };
    tokio::time::timeout(Duration::from_secs(20), command.output())
        .await
        .unwrap()
        .unwrap()
}

/// `anywhere skill install` with `home` as its home folder, Claude Code's own
/// in `claude`, and `umask`.
async fn install_with(home: &Path, claude: &Path, umask: &str) -> Output {
    let mut sh = tokio::process::Command::new("/bin/sh");
    sh.args([
        "-c",
        &format!("umask {umask} && exec \"$0\" --json skill install"),
        env!("CARGO_BIN_EXE_anywhere"),
    ])
    .env("HOME", home)
    .env("CLAUDE_CONFIG_DIR", claude);
    tokio::time::timeout(Duration::from_secs(20), sh.output())
        .await
        .unwrap()
        .unwrap()
}

/// Compare the installed bundle to the maintained source, including modes.
fn assert_bundle(installed: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    fn compare(source: &Path, installed: &Path) {
        assert_eq!(
            std::fs::metadata(installed).unwrap().permissions().mode() & 0o777,
            0o755
        );
        for entry in std::fs::read_dir(source).unwrap() {
            let source = entry.unwrap().path();
            let destination = installed.join(source.file_name().unwrap());
            if source.is_dir() {
                compare(&source, &destination);
            } else {
                assert_eq!(
                    std::fs::read(&destination).unwrap(),
                    std::fs::read(&source).unwrap(),
                    "{}",
                    destination.display()
                );
                assert_eq!(
                    std::fs::metadata(&destination)
                        .unwrap()
                        .permissions()
                        .mode()
                        & 0o777,
                    0o644
                );
            }
        }
    }
    compare(
        Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../skills/anywhere")),
        installed,
    );
}

#[tokio::test]
async fn skill_install_writes_the_skill_it_carries_for_claude_code_and_codex() {
    let home = tempfile::tempdir().unwrap();
    let claude = home.path().join(".claude/skills/anywhere/SKILL.md");
    let codex = home.path().join(".agents/skills/anywhere/SKILL.md");
    // Upgrade the old single-file skill, replace a stale bundled reference, preserve user notes.
    std::fs::create_dir_all(claude.parent().unwrap().join("references")).unwrap();
    std::fs::write(&claude, "old entrypoint").unwrap();
    std::fs::write(
        claude.parent().unwrap().join("references/setup.md"),
        "stale",
    )
    .unwrap();
    std::fs::write(claude.parent().unwrap().join("notes.md"), "keep me").unwrap();
    std::fs::write(
        claude.parent().unwrap().join("references/my-notes.md"),
        "also mine",
    )
    .unwrap();
    let output = run_at_home(Some(home.path()), &["--json", "skill", "install"]).await;
    assert_eq!(
        success(&output),
        json!({"installed": [
            {"agent": "claude-code", "path": claude.display().to_string()},
            {"agent": "codex", "path": codex.display().to_string()},
        ]})
    );
    assert!(output.stderr.is_empty(), "{}", text(&output.stderr));
    for path in [&claude, &codex] {
        assert_bundle(path.parent().unwrap());
    }
    assert_eq!(
        std::fs::read_to_string(claude.parent().unwrap().join("notes.md")).unwrap(),
        "keep me"
    );
    assert_eq!(
        std::fs::read_to_string(claude.parent().unwrap().join("references/my-notes.md")).unwrap(),
        "also mine"
    );
    // Without --json, a line for each.
    let output = run_at_home(Some(home.path()), &["skill", "install"]).await;
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(
        text(&output.stdout),
        format!(
            "Installed the anywhere skill for Claude Code in {}\nInstalled the anywhere skill for Codex in {}\n",
            claude.display(),
            codex.display()
        )
    );
    assert_bundle(claude.parent().unwrap());
    assert_bundle(codex.parent().unwrap());
}

#[tokio::test]
async fn skill_install_never_writes_through_a_link_or_without_a_home() {
    let home = tempfile::tempdir().unwrap();
    let checkout = home.path().join("checkout/skills/anywhere");
    std::fs::create_dir_all(&checkout).unwrap();
    std::fs::write(checkout.join("SKILL.md"), "the checkout's").unwrap();
    let link = home.path().join(".claude/skills/anywhere");
    std::fs::create_dir_all(link.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&checkout, &link).unwrap();
    let output = run_at_home(Some(home.path()), &["--json", "skill", "install"]).await;
    let error = failure(&output, "skill_path_linked");
    assert_eq!(error["retryable"], false);
    assert!(error["message"].as_str().unwrap().starts_with(&format!(
        "{} is a symlink to {}",
        link.display(),
        checkout.display()
    )));
    assert_eq!(
        std::fs::read_to_string(checkout.join("SKILL.md")).unwrap(),
        "the checkout's"
    );
    assert!(!home.path().join(".agents").exists());

    let error = &error["message"];
    assert!(
        error.as_str().unwrap().contains("don't remove the link"),
        "{error}"
    );

    let output = run_at_home(None, &["--json", "skill", "install"]).await;
    failure(&output, "setting_missing");
}

/// Claude Code's copy goes in `CLAUDE_CONFIG_DIR` when it's set, and what the
/// command makes others may read but not change, whatever the umask.
#[tokio::test]
async fn skill_install_follows_claude_codes_folder_and_sets_its_own_modes() {
    use std::os::unix::fs::PermissionsExt as _;
    let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    for umask in ["000", "077"] {
        let home = tempfile::tempdir().unwrap();
        let claude = home.path().join("elsewhere/claude");
        let output = install_with(home.path(), &claude, umask).await;
        let skill = claude.join("skills/anywhere/SKILL.md");
        let codex = home.path().join(".agents/skills/anywhere/SKILL.md");
        assert_eq!(
            success(&output),
            json!({"installed": [
                {"agent": "claude-code", "path": skill.display().to_string()},
                {"agent": "codex", "path": codex.display().to_string()},
            ]}),
            "umask {umask}"
        );
        assert!(!home.path().join(".claude").exists());
        for file in [&skill, &codex] {
            assert_eq!(mode(file), 0o644, "umask {umask}: {}", file.display());
            assert_bundle(file.parent().unwrap());
        }
        for folder in [
            "elsewhere",
            "elsewhere/claude",
            "elsewhere/claude/skills",
            "elsewhere/claude/skills/anywhere",
            ".agents",
            ".agents/skills",
            ".agents/skills/anywhere",
        ] {
            assert_eq!(
                mode(&home.path().join(folder)),
                0o755,
                "umask {umask}: {folder}"
            );
        }
    }
}
