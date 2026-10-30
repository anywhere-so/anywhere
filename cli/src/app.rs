//! The app in a folder: its name, its `app.toml` and its files, all read and
//! checked before anything is sent.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};

use appspec::AppSpec;
use ignore::{DirEntry, WalkBuilder};
use manifest::FileEntry;
use naming::AppName;

use crate::error::CliError;

pub struct App {
    pub name: AppName,
    pub spec: AppSpec,
    /// The folder, as it was given.
    pub dir: PathBuf,
    /// Every file the deploy sends, sorted by path: none for an image app,
    /// whose files come from its image.
    pub files: Vec<FileEntry>,
    /// For an image app, whether its folder holds files besides `app.toml`
    /// that a folder's deploy would send, which its deploy doesn't.
    pub unsent: bool,
}

impl App {
    /// Reads the app in `dir`, deployed as `name`.
    pub fn read(dir: &Path, name: AppName) -> Result<Self, CliError> {
        let folder = |problem: String| CliError::AppFolder {
            dir: dir.display().to_string(),
            problem,
        };
        match fs::metadata(dir) {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => return Err(folder("isn't a folder".to_owned())),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(folder("doesn't exist".to_owned()));
            }
            Err(error) => return Err(folder(format!("can't be read ({error})"))),
        }
        let spec = spec(dir)?;
        // An image app's files come from its image, so its folder sends none.
        if spec.image.is_some() {
            let unsent = sends_more_than_app_toml(dir, spec.icon.as_deref())?;
            return Ok(Self {
                name,
                spec,
                dir: dir.to_owned(),
                files: Vec::new(),
                unsent,
            });
        }
        // A static site with no build step arrives built, and projects'
        // .gitignore files often leave out what a build makes, such as Vite's
        // dist, so its folder goes whole.
        let whole = spec
            .static_folder
            .as_deref()
            .filter(|_| spec.build.is_none());
        let files = files(dir, whole)?;
        manifest::check(&files).map_err(CliError::Manifest)?;
        Ok(Self {
            name,
            spec,
            dir: dir.to_owned(),
            files,
            unsent: false,
        })
    }
}

/// Whether a folder's deploy would send files besides `app.toml` from `dir`,
/// by the rules it walks the folder with ([`files`]), leaving out hidden
/// files and folders too, such as `.gitignore` or `.DS_Store`, which nobody
/// means for an image app. It hashes nothing, and stops at the first.
fn sends_more_than_app_toml(dir: &Path, icon: Option<&str>) -> Result<bool, CliError> {
    let walk = walker(dir, true, true).build();
    for entry in walk {
        let entry = entry.map_err(|error| CliError::Walk {
            detail: error.to_string(),
        })?;
        if entry.depth() == 0 || entry.file_type().is_some_and(|kind| kind.is_dir()) {
            continue;
        }
        if icon.is_some_and(|icon| entry.path() == dir.join(icon)) {
            continue;
        }
        if entry.depth() > 1 || entry.file_name() != "app.toml" {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The working folder as the shell names it, in `$PWD`, so a folder reached
/// through a link keeps the link's name. As with `pwd`, `$PWD` counts only when
/// it's absolute and is the same folder as `.`; otherwise the folder's own path
/// is the only name there is.
pub fn working_dir() -> Result<PathBuf, CliError> {
    let physical = std::env::current_dir().map_err(|error| CliError::AppFolder {
        dir: "the working folder".to_owned(),
        problem: format!("can't be read ({error})"),
    })?;
    if let Some(pwd) = std::env::var_os("PWD").map(PathBuf::from)
        && pwd.is_absolute()
        && same_folder(&pwd, &physical)
    {
        return Ok(pwd);
    }
    Ok(physical)
}

fn same_folder(a: &Path, b: &Path) -> bool {
    match (fs::metadata(a), fs::metadata(b)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        // A $PWD that can't be read isn't the working folder.
        _ => false,
    }
}

/// The app's name: the last part of its folder's path as given, made absolute
/// and with `.` and `..` worked out from the path alone, without following
/// links, so a folder that's a link takes the link's name.
pub fn name(dir: &Path, cwd: &Path) -> Result<AppName, CliError> {
    let mut parts: Vec<&OsStr> = Vec::new();
    let absolute = cwd.join(dir);
    for component in absolute.components() {
        match component {
            Component::Prefix(_) | Component::RootDir | Component::CurDir => {}
            Component::ParentDir => {
                parts.pop();
            }
            Component::Normal(part) => parts.push(part),
        }
    }
    let Some(last) = parts.last() else {
        return Err(CliError::AppFolder {
            dir: dir.display().to_string(),
            problem: "has no folder name to take the app's name from".to_owned(),
        });
    };
    let folder = last.to_string_lossy();
    AppName::parse(&folder).map_err(|source| CliError::AppName {
        folder: folder.into_owned(),
        source,
    })
}

fn spec(dir: &Path) -> Result<AppSpec, CliError> {
    let path = dir.join("app.toml");
    let shown = || path.display().to_string();
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(CliError::AppTomlMissing {
                dir: dir.display().to_string(),
            });
        }
        Err(source) => {
            return Err(CliError::AppTomlUnreadable {
                path: shown(),
                source,
            });
        }
    };
    let text = String::from_utf8(bytes).map_err(|_| CliError::AppTomlNotText { path: shown() })?;
    appspec::parse(&text).map_err(CliError::AppToml)
}

/// Every file in the app's folder, hashed. It follows the `.gitignore` files
/// inside the folder, and only those, wherever the folder is and however git
/// is set up, so the same folder always deploys the same files. `whole` is a
/// folder inside it whose every file goes, whatever the `.gitignore` files say.
fn files(dir: &Path, whole: Option<&str>) -> Result<Vec<FileEntry>, CliError> {
    let mut files = BTreeMap::new();
    collect(dir, dir, true, &mut files)?;
    if let Some(folder) = whole
        && let Some(root) = real_folder(dir, folder)?
    {
        collect(dir, &root, false, &mut files)?;
    }
    Ok(files.into_values().collect())
}

/// `folder`, inside the app's folder `dir`, if it's there, and each folder on
/// the way to it is a real one, not a link, so nothing outside the app's folder
/// is walked. One that isn't there adds no files.
fn real_folder(dir: &Path, folder: &str) -> Result<Option<PathBuf>, CliError> {
    let mut path = dir.to_owned();
    let mut shown = String::new();
    for part in folder.split('/') {
        path.push(part);
        if !shown.is_empty() {
            shown.push('/');
        }
        shown.push_str(part);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(metadata) if metadata.is_symlink() => {
                return Err(CliError::Symlink { path: shown });
            }
            Ok(_) => return Err(CliError::NotRegular { path: shown }),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(CliError::FileUnreadable {
                    path: shown,
                    source,
                });
            }
        }
    }
    Ok(Some(path))
}

/// Adds every file under `root`, inside the app's folder `dir`, by its path,
/// following the `.gitignore` files there if `gitignore` says to.
fn collect(
    dir: &Path,
    root: &Path,
    gitignore: bool,
    files: &mut BTreeMap<String, FileEntry>,
) -> Result<(), CliError> {
    let walk = walker(root, gitignore, false).build();
    for entry in walk {
        let entry = entry.map_err(|error| CliError::Walk {
            detail: error.to_string(),
        })?;
        // The folder walked itself, which may be a link.
        if entry.depth() == 0 {
            continue;
        }
        let path = relative(dir, entry.path())?;
        let Some(kind) = entry.file_type() else {
            return Err(CliError::NotRegular { path });
        };
        if kind.is_dir() {
            continue;
        }
        if kind.is_symlink() {
            return Err(CliError::Symlink { path });
        }
        if !kind.is_file() {
            return Err(CliError::NotRegular { path });
        }
        let unreadable = |source| CliError::FileUnreadable {
            path: path.clone(),
            source,
        };
        let metadata = entry
            .metadata()
            .map_err(|error| unreadable(io::Error::other(error)))?;
        let (hash, size) = hash(entry.path()).map_err(unreadable)?;
        files.insert(
            path.clone(),
            FileEntry {
                hash,
                size,
                executable: metadata.permissions().mode() & 0o111 != 0,
                path,
            },
        );
    }
    Ok(())
}

/// Walks `root` as a deploy does: following the `.gitignore` files inside the
/// app's folder alone, if `gitignore` says to, and leaving out what a deploy
/// never carries, and hidden files and folders too if `hidden` says to.
fn walker(root: &Path, gitignore: bool, hidden: bool) -> WalkBuilder {
    let mut walk = WalkBuilder::new(root);
    walk.standard_filters(false)
        .git_ignore(gitignore)
        .require_git(false)
        .filter_entry(move |entry| {
            !left_out(entry)
                && !(hidden
                    && entry.depth() > 0
                    && entry.file_name().as_encoded_bytes().starts_with(b"."))
        });
    walk
}

/// What a deploy never carries: git's own folder, and `.env` files, which
/// hold secrets.
fn left_out(entry: &DirEntry) -> bool {
    let name = entry.file_name();
    name == ".git" || name == ".env" || name.as_encoded_bytes().starts_with(b".env.")
}

/// A file's path inside the app's folder, with `/` between folders.
fn relative(dir: &Path, path: &Path) -> Result<String, CliError> {
    let inside = path.strip_prefix(dir).map_err(|_| CliError::Internal {
        detail: format!("{} isn't inside {}", path.display(), dir.display()),
    })?;
    let mut parts = Vec::new();
    for component in inside.components() {
        let Component::Normal(part) = component else {
            return Err(CliError::Internal {
                detail: format!("{} isn't a plain path", inside.display()),
            });
        };
        let Some(part) = part.to_str() else {
            return Err(CliError::NameNotUtf8 {
                path: inside.display().to_string(),
            });
        };
        parts.push(part);
    }
    Ok(parts.join("/"))
}

/// A file's BLAKE3 hash, in lowercase hex, and its size.
fn hash(path: &Path) -> io::Result<(String, u64)> {
    let mut hasher = blake3::Hasher::new();
    hasher.update_reader(File::open(path)?)?;
    Ok((hasher.finalize().to_hex().to_string(), hasher.count()))
}

/// The configured icon file for an image app, which uploads metadata alone.
pub fn image_icon_file(app: &App, path: &str) -> Result<u64, CliError> {
    for entry in walker(&app.dir, true, false).build() {
        let entry = entry.map_err(|error| CliError::Walk {
            detail: error.to_string(),
        })?;
        if entry.path() == app.dir.join(path) {
            let metadata = std::fs::symlink_metadata(entry.path()).map_err(|source| {
                CliError::FileUnreadable {
                    path: path.to_owned(),
                    source,
                }
            })?;
            if metadata.is_symlink() {
                return Err(CliError::Symlink {
                    path: path.to_owned(),
                });
            }
            if !metadata.is_file() {
                return Err(CliError::NotRegular {
                    path: path.to_owned(),
                });
            }
            return Ok(metadata.len());
        }
    }
    Err(CliError::AppIcon(appspec::icon::Error(
        "icon must be included in the deploy; check .gitignore".to_owned(),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name_of(dir: &str, cwd: &str) -> Result<String, String> {
        name(Path::new(dir), Path::new(cwd))
            .map(|name| name.as_str().to_owned())
            .map_err(|error| error.code().to_owned())
    }

    #[test]
    fn the_name_is_the_folders_last_part_as_given() {
        let cases = [
            (".", "/home/alice/apps/notes", "notes"),
            ("./notes", "/home/alice/apps", "notes"),
            ("notes/", "/home/alice/apps", "notes"),
            ("notes/.", "/home/alice/apps", "notes"),
            ("/srv/apps/my-site", "/anywhere/else", "my-site"),
            ("..", "/home/alice/apps/notes/src", "notes"),
            ("../../budget", "/home/alice/apps/notes/src", "budget"),
            ("notes/src/..", "/home/alice/apps", "notes"),
        ];
        for (dir, cwd, want) in cases {
            assert_eq!(name_of(dir, cwd), Ok(want.to_owned()), "{dir} from {cwd}");
        }
    }

    #[test]
    fn a_folder_without_a_valid_name_is_refused() {
        assert_eq!(name_of("/", "/"), Err("app_folder_invalid".to_owned()));
        assert_eq!(name_of("..", "/a"), Err("app_folder_invalid".to_owned()));
        assert_eq!(
            name_of("My_App", "/home/alice"),
            Err("invalid_app_name".to_owned())
        );
        let error = name(Path::new("My_App"), Path::new("/")).unwrap_err();
        assert_eq!(
            error.to_string(),
            "the app's name comes from its folder's name, \"My_App\", and the app name has 'M' at character 1; use only lowercase letters (a-z), digits and hyphens; rename the folder, or deploy it through a link with the app's name"
        );
    }

    #[test]
    fn a_folder_that_is_a_link_keeps_the_links_name() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("Real_Folder")).unwrap();
        std::os::unix::fs::symlink("Real_Folder", dir.path().join("notes")).unwrap();
        assert_eq!(
            name(&dir.path().join("notes"), Path::new("/"))
                .unwrap()
                .as_str(),
            "notes"
        );
        assert_eq!(
            name(Path::new("."), &dir.path().join("notes"))
                .unwrap()
                .as_str(),
            "notes"
        );
    }

    #[test]
    fn leaves_out_git_and_env_files_and_follows_gitignore_but_sends_data() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for path in [
            "app.toml",
            "index.html",
            ".hidden",
            "settings.env",
            ".env.example",
            "src/data/seed.json",
            "src/.env",
            ".git/config",
            "data/app.db",
            ".env",
            ".env.local",
            "node_modules/x/index.js",
            "build.log",
        ] {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"x").unwrap();
        }
        fs::write(root.join(".gitignore"), "node_modules/\n*.log\n").unwrap();
        let paths: Vec<String> = files(root, None)
            .unwrap()
            .into_iter()
            .map(|file| file.path)
            .collect();
        // An app's own data/ is just files now: a deploy sends it like any
        // other folder, unless .gitignore leaves it out.
        assert_eq!(
            paths,
            [
                ".gitignore",
                ".hidden",
                "app.toml",
                "data/app.db",
                "index.html",
                "settings.env",
                "src/data/seed.json"
            ]
        );
        fs::write(root.join(".gitignore"), "node_modules/\n*.log\n/data/\n").unwrap();
        let paths: Vec<String> = files(root, None)
            .unwrap()
            .into_iter()
            .map(|file| file.path)
            .collect();
        assert!(!paths.contains(&"data/app.db".to_owned()), "{paths:?}");
        assert!(
            paths.contains(&"src/data/seed.json".to_owned()),
            "{paths:?}"
        );
    }

    #[test]
    fn a_static_site_with_no_build_step_carries_its_folder_whatever_gitignore_says() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for path in [
            "app.toml",
            "dist/index.html",
            "dist/data/menu.json",
            "dist/.env",
            "dist/.git/config",
            "src/main.ts",
            "node_modules/x/index.js",
        ] {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"x").unwrap();
        }
        fs::write(root.join(".gitignore"), "dist\nnode_modules/\n").unwrap();
        let paths = |whole| -> Vec<String> {
            files(root, whole)
                .unwrap()
                .into_iter()
                .map(|file| file.path)
                .collect()
        };
        assert_eq!(paths(None), [".gitignore", "app.toml", "src/main.ts"]);
        assert_eq!(
            paths(Some("dist")),
            [
                ".gitignore",
                "app.toml",
                "dist/data/menu.json",
                "dist/index.html",
                "src/main.ts"
            ]
        );
        // A folder that isn't there adds nothing, and a link to one, or to a
        // folder on the way to it, is refused, so nothing outside the app's
        // folder is uploaded.
        assert_eq!(
            paths(Some("public")),
            [".gitignore", "app.toml", "src/main.ts"]
        );
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir_all(outside.path().join("b")).unwrap();
        fs::write(outside.path().join("b/secret.txt"), b"not the app's").unwrap();
        std::os::unix::fs::symlink(outside.path(), root.join("public")).unwrap();
        fs::write(root.join(".gitignore"), "dist\nnode_modules/\npublic\n").unwrap();
        for folder in ["public", "public/b"] {
            let error = files(root, Some(folder)).unwrap_err();
            assert_eq!(error.code(), "app_file_symlink", "{folder}");
            assert!(
                error.to_string().starts_with("public is a symlink"),
                "{error}"
            );
        }
    }

    #[test]
    fn an_image_apps_folder_sends_no_files_and_notes_only_those_a_deploy_would_have_sent() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("seo");
        let image = "title = \"SEO\"\nimage = \"ghcr.io/every-app/open-seo:latest\"\n";
        for (path, text) in [
            ("app.toml", image),
            (".gitignore", "node_modules/\n*.log\n"),
            (".DS_Store", "x"),
            (".git/HEAD", "ref: refs/heads/main\n"),
            (".env", "SECRET=1\n"),
            (".vscode/settings.json", "{}"),
            ("node_modules/x/index.js", "x"),
            ("debug.log", "x"),
        ] {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        let read = || App::read(&root, AppName::parse("seo").unwrap()).unwrap();
        let app = read();
        assert!(app.files.is_empty());
        assert!(
            !app.unsent,
            "only what a deploy leaves out, and hidden files"
        );
        // A file a folder's deploy would send is noted, wherever it is.
        fs::create_dir_all(root.join("config")).unwrap();
        fs::write(root.join("config/app.toml"), "x").unwrap();
        assert!(read().unsent);
        fs::remove_file(root.join("config/app.toml")).unwrap();
        assert!(!read().unsent);
        fs::write(root.join("compose.yaml"), "services: {}\n").unwrap();
        assert!(read().unsent);
    }

    #[test]
    fn hashes_each_file_and_notes_whether_it_runs() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("run"), b"#!/bin/sh\n").unwrap();
        fs::set_permissions(dir.path().join("run"), fs::Permissions::from_mode(0o750)).unwrap();
        fs::write(dir.path().join("empty"), b"").unwrap();
        let files = files(dir.path(), None).unwrap();
        assert_eq!(
            files,
            [
                FileEntry {
                    path: "empty".to_owned(),
                    hash: blake3::hash(b"").to_hex().to_string(),
                    size: 0,
                    executable: false,
                },
                FileEntry {
                    path: "run".to_owned(),
                    hash: blake3::hash(b"#!/bin/sh\n").to_hex().to_string(),
                    size: 10,
                    executable: true,
                },
            ]
        );
    }

    #[test]
    fn refuses_symlinks_and_other_kinds_of_file() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        std::os::unix::fs::symlink("../elsewhere", dir.path().join("src/link")).unwrap();
        let error = files(dir.path(), None).unwrap_err();
        assert_eq!(error.code(), "app_file_symlink");
        assert!(
            error.to_string().starts_with("src/link is a symlink"),
            "{error}"
        );

        // A link the .gitignore leaves out is fine.
        fs::write(dir.path().join(".gitignore"), "src/link\n").unwrap();
        assert!(files(dir.path(), None).is_ok());
    }

    #[test]
    fn the_public_samples_pass_every_check_a_deploy_makes() {
        // Private starters are checked through the CLI in the platform integration suite.
        for group in ["../samples"] {
            let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(group);
            let mut read = 0;
            for entry in fs::read_dir(&root).unwrap() {
                let dir = entry.unwrap().path();
                if !dir.is_dir() {
                    continue;
                }
                let app = name(&dir, Path::new("/")).and_then(|name| App::read(&dir, name));
                if let Err(error) = app {
                    panic!("{}: {}: {error}", dir.display(), error.code());
                }
                read += 1;
            }
            assert!(read > 0, "{} has no apps", root.display());
        }
    }
}
