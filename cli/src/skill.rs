//! `anywhere skill install`: writes the anywhere skill, which this CLI carries
//! from `skills/anywhere/`, where Claude Code and Codex read a user's
//! skills, so an agent's skill is the one for the CLI it runs.
//!
//! Platform developers link the skill's folder, `~/.claude/skills/anywhere`,
//! into their checkout, so a link there or at its `SKILL.md` is refused and
//! never written through. The folders above it are followed, as the agents
//! follow them, since people keep `~/.claude` with their dotfiles and share one
//! `skills` folder between agents. Paths are checked, then written: another
//! program of the user's that swaps a folder for a link in between isn't
//! guarded against.

use std::env;
use std::fs::{self, DirBuilder, File, OpenOptions, Permissions};
use std::hash::{BuildHasher as _, RandomState};
use std::io::{self, Write as _};
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::CliError;
use crate::output::ResultData;

// Generated from every regular Markdown file in the skill and references.
include!(concat!(env!("OUT_DIR"), "/skill_bundle.rs"));

#[cfg(test)]
const SKILL: &str = include_str!("../../skills/anywhere/SKILL.md");

/// The skill's mode, and its folders', whatever the umask: anyone may read
/// them, and only their owner change what an agent is told.
const FILE_MODE: u32 = 0o644;
const FOLDER_MODE: u32 = 0o755;

/// One agent the skill is installed for.
pub struct Target {
    /// Its name in the result: `claude-code` or `codex`.
    agent: &'static str,
    /// Its name for people.
    name: &'static str,
    /// The folder it reads a user's skills from.
    skills: PathBuf,
}

/// One copy of the skill the command wrote.
#[derive(Debug, Serialize)]
pub struct Installed {
    /// `claude-code` or `codex`.
    pub agent: &'static str,
    #[serde(skip)]
    pub name: &'static str,
    /// The `SKILL.md` written.
    pub path: String,
}

pub fn install() -> Result<ResultData, CliError> {
    let home = absolute("HOME", "your home folder, under which Codex reads skills")?.ok_or(
        CliError::SettingMissing {
            name: "HOME",
            what: "your home folder, under which Claude Code and Codex read skills",
        },
    )?;
    // Claude Code keeps its settings there in place of ~/.claude when it's set.
    let claude = absolute(
        "CLAUDE_CONFIG_DIR",
        "the folder Claude Code keeps its settings in, or unset it",
    )?
    .unwrap_or_else(|| home.join(".claude"));
    install_in(&targets(&home, &claude)).map(|installed| ResultData::Skill { installed })
}

/// The absolute path the environment variable `name` holds, if it's set.
fn absolute(name: &'static str, what: &str) -> Result<Option<PathBuf>, CliError> {
    match env::var_os(name) {
        None => Ok(None),
        Some(path) if path.is_empty() => Ok(None),
        Some(path) => {
            let path = PathBuf::from(path);
            if path.is_absolute() {
                Ok(Some(path))
            } else {
                Err(CliError::SettingInvalid {
                    name,
                    problem: format!("isn't an absolute path; set it to {what}"),
                })
            }
        }
    }
}

/// Claude Code's skills in its folder, `claude`, and Codex's under `home`.
fn targets(home: &Path, claude: &Path) -> [Target; 2] {
    [
        Target {
            agent: "claude-code",
            name: "Claude Code",
            skills: claude.join("skills"),
        },
        Target {
            agent: "codex",
            name: "Codex",
            skills: home.join(".agents/skills"),
        },
    ]
}

/// Writes the skill for every target, checking every path first, so a refusal
/// leaves each copy as it was. A failure after the first copy is written says
/// so.
fn install_in(targets: &[Target]) -> Result<Vec<Installed>, CliError> {
    for target in targets {
        check(&target.skills)?;
    }
    let mut installed: Vec<Installed> = Vec::new();
    for target in targets {
        match write(&target.skills) {
            Ok(path) => installed.push(Installed {
                agent: target.agent,
                name: target.name,
                path: path.display().to_string(),
            }),
            Err(error) if installed.is_empty() => return Err(error),
            Err(error) => {
                return Err(CliError::SkillPartlyInstalled {
                    error: Box::new(error),
                    installed: installed
                        .iter()
                        .map(|copy| format!("{} at {}", copy.name, copy.path))
                        .collect::<Vec<_>>()
                        .join(" and "),
                });
            }
        }
    }
    Ok(installed)
}

/// The containing skills directory may be linked, but no bundle path may be.
fn check(skills: &Path) -> Result<(), CliError> {
    match fs::metadata(skills) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => return Err(unwritable(skills, "it isn't a folder")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(unwritable(skills, &format!("it can't be read ({error})"))),
    }
    let root = skills.join("anywhere");
    check_path(&root, true)?;
    for (name, _) in SKILL_FILES {
        let mut path = root.clone();
        let parts: Vec<_> = Path::new(name).components().collect();
        for (index, part) in parts.iter().enumerate() {
            path.push(part);
            check_path(&path, index + 1 < parts.len())?;
        }
    }
    Ok(())
}

fn check_path(path: &Path, folder: bool) -> Result<(), CliError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(unwritable(path, &format!("it can't be read ({error})"))),
    };
    if metadata.file_type().is_symlink() {
        return Err(CliError::SkillLinked {
            path: path.display().to_string(),
            target: match fs::read_link(path) {
                Ok(target) => target.display().to_string(),
                Err(error) => format!("somewhere it couldn't read ({error})"),
            },
        });
    }
    if folder && !metadata.is_dir() {
        return Err(unwritable(path, "it isn't a folder"));
    }
    if !folder && !metadata.is_file() {
        return Err(unwritable(path, "it isn't a regular file"));
    }
    Ok(())
}

/// Replace each bundled file atomically, references first and entrypoint last.
/// Unrelated files are preserved. Preflight checks both agents before writing either.
fn write(skills: &Path) -> Result<PathBuf, CliError> {
    let root = skills.join("anywhere");
    for (name, contents) in SKILL_FILES {
        let path = root.join(name);
        let folder = path
            .parent()
            .ok_or_else(|| unwritable(&path, "it has no parent folder"))?;
        make_folder(folder).map_err(|error| {
            unwritable(&path, &format!("its folder couldn't be made ({error})"))
        })?;
        check(skills)?;
        write_file(&path, contents)?;
    }
    Ok(root.join("SKILL.md"))
}

fn write_file(path: &Path, contents: &str) -> Result<(), CliError> {
    let folder = path
        .parent()
        .ok_or_else(|| unwritable(path, "it has no parent folder"))?;
    // The standard library's hasher has random keys, so this names a file no
    // other run picks.
    let unique = RandomState::new().hash_one(std::process::id());
    let temporary = folder.join(format!(".skill.{unique:016x}.tmp"));
    let written = (|| {
        // `create_new` never follows a link at its own path.
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(FILE_MODE)
            .open(&temporary)?;
        // The umask may have taken bits from the mode it was made with.
        file.set_permissions(Permissions::from_mode(FILE_MODE))?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        // The rename itself lasts only once the folder is on disk too.
        File::open(folder)?.sync_all()
    })();
    if let Err(error) = written {
        let detail = match fs::remove_file(&temporary) {
            Ok(()) => error.to_string(),
            Err(removing) if removing.kind() == io::ErrorKind::NotFound => error.to_string(),
            Err(removing) => format!(
                "{error}, and the partial file {} couldn't be removed ({removing})",
                temporary.display()
            ),
        };
        return Err(unwritable(path, &detail));
    }
    Ok(())
}

/// Makes `folder` and any of its parents that are missing, each with exactly
/// [`FOLDER_MODE`]. A folder already there, or a link to one, is left as it
/// is.
fn make_folder(folder: &Path) -> io::Result<()> {
    match fs::metadata(folder) {
        Ok(metadata) if metadata.is_dir() => return Ok(()),
        Ok(_) => {
            return Err(io::Error::other(format!(
                "{} isn't a folder",
                folder.display()
            )));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    if let Some(parent) = folder.parent() {
        make_folder(parent)?;
    }
    match DirBuilder::new().mode(FOLDER_MODE).create(folder) {
        Ok(()) => fs::set_permissions(folder, Permissions::from_mode(FOLDER_MODE)),
        // Another program made it meanwhile, or it's a link to nothing.
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            if fs::metadata(folder)?.is_dir() {
                Ok(())
            } else {
                Err(error)
            }
        }
        Err(error) => Err(error),
    }
}

fn unwritable(path: &Path, detail: &str) -> CliError {
    CliError::SkillUnwritable {
        path: path.display().to_string(),
        detail: detail.to_owned(),
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "test failures stop at their assertion, as clippy.toml allows"
)]
mod tests {
    use std::os::unix::fs::symlink;

    use super::*;

    const OLD: &str = "---\nname: anywhere\n---\nan older skill\n";

    fn skill_at(home: &Path, skills: &str) -> PathBuf {
        home.join(skills).join("anywhere/SKILL.md")
    }

    /// Installs for both agents under `home`, Claude Code's in `~/.claude`.
    fn install_at(home: &Path) -> Result<Vec<Installed>, CliError> {
        install_in(&targets(home, &home.join(".claude")))
    }

    fn mode(path: &Path) -> u32 {
        fs::symlink_metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn the_skill_carried_is_the_anywhere_skill() {
        assert!(SKILL.starts_with("---\n"), "{SKILL:.80}");
        assert!(SKILL.contains("\nname: anywhere\n"), "{SKILL:.80}");
    }

    #[test]
    fn it_makes_the_folders_and_replaces_an_older_copy_for_both_agents() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        // Claude Code's has an older copy and another file, Codex's nothing.
        let claude = skill_at(home, ".claude/skills");
        fs::create_dir_all(claude.parent().unwrap()).unwrap();
        fs::set_permissions(home.join(".claude"), Permissions::from_mode(0o700)).unwrap();
        fs::write(&claude, OLD).unwrap();
        fs::write(claude.with_file_name("notes.md"), "mine").unwrap();
        let installed = install_at(home).unwrap();
        let paths: Vec<_> = installed
            .iter()
            .map(|installed| (installed.agent, installed.path.as_str()))
            .collect();
        let codex = skill_at(home, ".agents/skills");
        assert_eq!(
            paths,
            [
                ("claude-code", claude.to_str().unwrap()),
                ("codex", codex.to_str().unwrap())
            ]
        );
        for path in [&claude, &codex] {
            assert_eq!(fs::read_to_string(path).unwrap(), SKILL);
            assert_eq!(mode(path), FILE_MODE);
        }
        // The folders it made are 0755, and one already there keeps its mode.
        for made in [".agents", ".agents/skills", ".agents/skills/anywhere"] {
            assert_eq!(mode(&home.join(made)), FOLDER_MODE, "{made}");
        }
        assert_eq!(mode(&home.join(".claude")), 0o700);
        // Only the skill changed, and no temporary file is left behind.
        let mut left: Vec<_> = fs::read_dir(claude.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        left.sort();
        assert_eq!(left, ["SKILL.md", "notes.md", "references"]);
        assert_eq!(
            fs::read_to_string(claude.with_file_name("notes.md")).unwrap(),
            "mine"
        );
        // Installing again changes nothing.
        install_at(home).unwrap();
        assert_eq!(fs::read_to_string(&codex).unwrap(), SKILL);
        assert_eq!(fs::read_dir(codex.parent().unwrap()).unwrap().count(), 2);
    }

    #[test]
    fn a_linked_skill_folder_is_refused_and_nothing_is_written() {
        for linked in [
            ".claude/skills/anywhere",
            ".agents/skills/anywhere",
            ".agents/skills/anywhere/references",
        ] {
            let home = tempfile::tempdir().unwrap();
            let home = home.path();
            // A checkout's own skill, which a platform developer links to.
            let checkout = home.join("checkout/skills/anywhere");
            fs::create_dir_all(&checkout).unwrap();
            fs::write(checkout.join("SKILL.md"), OLD).unwrap();
            let link = home.join(linked);
            fs::create_dir_all(link.parent().unwrap()).unwrap();
            symlink(&checkout, &link).unwrap();
            let error = install_at(home).unwrap_err();
            assert_eq!(error.code(), "skill_path_linked", "{linked}");
            let message = error.to_string();
            assert!(
                message.starts_with(&format!(
                    "{} is a symlink to {}",
                    link.display(),
                    checkout.display()
                )),
                "{message}"
            );
            assert!(message.contains("don't remove the link"), "{message}");
            assert_eq!(fs::read_to_string(checkout.join("SKILL.md")).unwrap(), OLD);
            assert!(link.is_symlink());
            // Neither agent's copy was written: each is missing, or the
            // checkout's, through the link.
            for skills in [".claude/skills", ".agents/skills"] {
                match fs::read_to_string(skill_at(home, skills)) {
                    Ok(text) => assert_eq!(text, OLD, "{linked}: {skills}"),
                    Err(error) => assert_eq!(error.kind(), io::ErrorKind::NotFound),
                }
            }
        }
    }

    #[test]
    fn a_linked_skill_file_is_refused_and_left_as_it_is() {
        for relative in [
            ".agents/skills/anywhere/SKILL.md",
            ".agents/skills/anywhere/references/setup.md",
        ] {
            let home = tempfile::tempdir().unwrap();
            let home = home.path();
            let theirs = home.join("theirs.md");
            fs::write(&theirs, OLD).unwrap();
            let file = home.join(relative);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            symlink(&theirs, &file).unwrap();
            assert_eq!(install_at(home).unwrap_err().code(), "skill_path_linked");
            assert_eq!(fs::read_to_string(&theirs).unwrap(), OLD);
            assert!(file.is_symlink());
            assert!(!skill_at(home, ".claude/skills").exists());
        }
    }

    /// People keep `~/.claude` with their dotfiles, and share one `skills`
    /// folder between agents.
    #[test]
    fn the_folders_above_the_skills_own_are_followed_as_the_agents_follow_them() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        let dotfiles = home.join("dotfiles/claude");
        fs::create_dir_all(&dotfiles).unwrap();
        symlink(&dotfiles, home.join(".claude")).unwrap();
        install_at(home).unwrap();
        assert_eq!(
            fs::read_to_string(dotfiles.join("skills/anywhere/SKILL.md")).unwrap(),
            SKILL
        );

        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        fs::create_dir_all(home.join(".agents/skills")).unwrap();
        fs::create_dir_all(home.join(".claude")).unwrap();
        symlink(home.join(".agents/skills"), home.join(".claude/skills")).unwrap();
        install_at(home).unwrap();
        assert!(home.join(".claude/skills").is_symlink());
        assert_eq!(
            fs::read_to_string(skill_at(home, ".agents/skills")).unwrap(),
            SKILL
        );
    }

    #[test]
    fn claude_codes_copy_goes_in_its_own_folder_when_it_has_another() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        let claude = home.join("elsewhere/claude");
        let installed = install_in(&targets(home, &claude)).unwrap();
        let written = claude.join("skills/anywhere/SKILL.md");
        assert_eq!(installed[0].path, written.display().to_string());
        assert_eq!(fs::read_to_string(&written).unwrap(), SKILL);
        assert!(!home.join(".claude").exists());
    }

    #[test]
    fn a_failure_after_the_first_copy_says_which_was_written() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        // Codex's folder can't take its `skills`, which the check can't see.
        fs::create_dir_all(home.join(".agents")).unwrap();
        fs::set_permissions(home.join(".agents"), Permissions::from_mode(0o555)).unwrap();
        let error = install_at(home).unwrap_err();
        fs::set_permissions(home.join(".agents"), Permissions::from_mode(0o755)).unwrap();
        assert_eq!(error.code(), "skill_write_failed");
        let claude = skill_at(home, ".claude/skills");
        assert!(
            error.to_string().ends_with(&format!(
                "; the skill for Claude Code at {} is installed already",
                claude.display()
            )),
            "{error}"
        );
        assert_eq!(fs::read_to_string(&claude).unwrap(), SKILL);
    }

    #[test]
    fn a_file_where_a_folder_goes_or_a_folder_where_the_skill_goes_is_refused() {
        for (path, says) in [
            (".claude/skills", "it isn't a folder"),
            (".agents/skills/anywhere", "it isn't a folder"),
            (".agents/skills/anywhere/references", "it isn't a folder"),
        ] {
            let home = tempfile::tempdir().unwrap();
            let home = home.path();
            fs::create_dir_all(home.join(path).parent().unwrap()).unwrap();
            fs::write(home.join(path), "").unwrap();
            let error = install_at(home).unwrap_err();
            assert_eq!(error.code(), "skill_write_failed");
            assert!(error.to_string().contains(says), "{error}");
        }
        let home = tempfile::tempdir().unwrap();
        fs::create_dir_all(skill_at(home.path(), ".claude/skills")).unwrap();
        let error = install_at(home.path()).unwrap_err();
        assert_eq!(error.code(), "skill_write_failed");
        assert!(
            error.to_string().contains("it isn't a regular file"),
            "{error}"
        );
    }
}
