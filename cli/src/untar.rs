//! Unpacking the tar stream `anywhere pull` gets from an app into a new local
//! folder. The stream is untrusted, since an app's own code or a changed `tar`
//! in it can send anything, so nothing may land outside the folder: absolute
//! paths, `..`, an entry under a link, a link whose target could leave the
//! folder, devices and FIFOs are refused, and no file is ever written twice or
//! through a link. The fuzz target `pull_untar` compiles this file in, so it
//! uses only the standard library and `tar`.

use std::collections::HashSet;
use std::ffi::OsStr;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{OpenOptionsExt, symlink};
use std::path::{Path, PathBuf};

use tar::EntryType;

/// The most entries a stream may hold: files, folders and links. Not
/// measured: room for an app with a large `node_modules`, while a stream
/// that never ends stops.
pub const MAX_ENTRIES: u64 = 1_000_000;

/// The most bytes of files a stream may hold. Not measured: ten times the
/// most a deploy may send.
pub const MAX_BYTES: u64 = 10 << 30;

/// What was unpacked: regular files, counting hard links, and their bytes.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Unpacked {
    pub files: u64,
    pub bytes: u64,
}

/// Why a stream was refused or couldn't be unpacked. Paths are shown with
/// escapes, so a strange name can't garble the message.
#[derive(Debug)]
pub enum UntarError {
    /// An absolute path, a `..`, the folder itself as a file, or a path under a
    /// link or a file.
    PathUnsafe { path: String, why: &'static str },
    /// A link whose target is absolute or could lead outside the folder, or a
    /// hard link to anything but a file already unpacked.
    LinkUnsafe { path: String, target: String },
    /// A device, a FIFO or another kind of entry that isn't a file, a folder or
    /// a link.
    Unsupported { path: String, kind: String },
    /// Two entries for the same path, or for names that differ only by case on
    /// a disk that ignores case.
    Duplicate { path: String },
    /// The stream isn't a whole tar archive.
    Stream { detail: String },
    /// The stream holds more entries than [`MAX_ENTRIES`].
    TooManyEntries,
    /// The stream holds more bytes of files than [`MAX_BYTES`].
    TooLarge,
    /// Writing the local folder failed.
    Write { path: String, source: io::Error },
}

impl UntarError {
    /// A stable code, which callers can match on.
    pub fn code(&self) -> &'static str {
        match self {
            UntarError::PathUnsafe { .. } => "pull_path_unsafe",
            UntarError::LinkUnsafe { .. } => "pull_link_unsafe",
            UntarError::Unsupported { .. } => "pull_entry_unsupported",
            UntarError::Duplicate { .. } => "pull_entry_duplicate",
            UntarError::Stream { .. } => "pull_stream_invalid",
            UntarError::TooManyEntries => "pull_too_many_entries",
            UntarError::TooLarge => "pull_too_large",
            UntarError::Write { .. } => "pull_write_failed",
        }
    }
}

const NOT_FROM_TAR: &str = "which `tar -cf` of a folder never sends, so check what `tar` the app has with `anywhere exec <app> -- which tar`";

impl fmt::Display for UntarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UntarError::PathUnsafe { path, why } => {
                write!(f, "the app sent {path}, which {why}, {NOT_FROM_TAR}")
            }
            UntarError::LinkUnsafe { path, target } => write!(
                f,
                "the app sent the link {path} to {target}, which could lead outside the folder, so pull never makes it; pull a sub-folder that doesn't hold such links, such as your code's folder (a Python virtualenv's links to its interpreter are absolute)"
            ),
            UntarError::Unsupported { path, kind } => write!(
                f,
                "the app sent {path}, a {kind}, which pull never makes; pull a folder that holds only files, folders and links"
            ),
            UntarError::Duplicate { path } => write!(
                f,
                "the app sent {path} twice, or two names that differ only by case, which this disk counts as one; pull into a folder on a case-sensitive disk, or rename one in the app"
            ),
            UntarError::Stream { detail } => write!(
                f,
                "the app's tar stream is broken ({detail}); run `anywhere pull` again"
            ),
            UntarError::TooManyEntries => write!(
                f,
                "the folder holds over {MAX_ENTRIES} files, folders and links, the most pull takes; pull a sub-folder, such as your code's folder"
            ),
            UntarError::TooLarge => write!(
                f,
                "the folder's files come to over {} GiB, the most pull takes; pull a sub-folder, such as your code's folder",
                MAX_BYTES >> 30
            ),
            UntarError::Write { path, source } => write!(
                f,
                "couldn't write {path}: {source}; fix the disk space or folder permissions, then pull again"
            ),
        }
    }
}

impl std::error::Error for UntarError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            UntarError::Write { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Unpacks the tar stream in `reader` into `root`, an empty folder. On an
/// error, `root` may hold part of the stream, so the caller removes it.
pub fn unpack(reader: impl Read, root: &Path) -> Result<Unpacked, UntarError> {
    let stream = |error: io::Error| UntarError::Stream {
        detail: error.to_string(),
    };
    let mut unpacked = Unpacked::default();
    let mut entries: u64 = 0;
    // What's been made, by path inside `root`: files a hard link may copy, and
    // links nothing may go under.
    let mut files: HashSet<PathBuf> = HashSet::new();
    let mut links: HashSet<PathBuf> = HashSet::new();
    let mut archive = tar::Archive::new(reader);
    for entry in archive.entries().map_err(stream)? {
        let mut entry = entry.map_err(stream)?;
        entries += 1;
        if entries > MAX_ENTRIES {
            return Err(UntarError::TooManyEntries);
        }
        let kind = entry.header().entry_type();
        // pax's global header holds settings for the entries after it, not a file.
        if kind == EntryType::XGlobalHeader {
            continue;
        }
        let raw = entry.path_bytes().into_owned();
        let path = shown(&raw);
        let Some(rel) = relative(&raw, &path)? else {
            // `.`, the folder itself, comes first from `tar -C <path> -cf - .`.
            if kind == EntryType::Directory {
                continue;
            }
            return Err(UntarError::PathUnsafe {
                path,
                why: "names the folder itself as something other than a folder",
            });
        };
        parents(root, &rel, &links, &path)?;
        let full = root.join(&rel);
        let write = |source| UntarError::Write {
            path: path.clone(),
            source,
        };
        match kind {
            EntryType::Regular | EntryType::Continuous => {
                // The size as read, which a pax header may set, and the
                // copy keeps to what's left besides, so no header gets past.
                let budget = MAX_BYTES - unpacked.bytes;
                if entry.size() > budget {
                    return Err(UntarError::TooLarge);
                }
                let mode = entry.header().mode().map_err(stream)?;
                let mut file = create(&full, mode, &path)?;
                unpacked.bytes += copy(&mut entry, &mut file, budget, &path)?;
                unpacked.files += 1;
                files.insert(rel);
            }
            EntryType::Directory => match fs::create_dir(&full) {
                Ok(()) => {}
                // Made already, as a parent of an earlier entry.
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    let metadata = fs::symlink_metadata(&full).map_err(write)?;
                    if !metadata.is_dir() {
                        return Err(UntarError::Duplicate { path });
                    }
                }
                Err(error) => return Err(write(error)),
            },
            EntryType::Symlink => {
                let target = target(&entry);
                if !link_stays_inside(&rel, &target) {
                    return Err(UntarError::LinkUnsafe {
                        path,
                        target: shown(&target),
                    });
                }
                match symlink(OsStr::from_bytes(&target), &full) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                        return Err(UntarError::Duplicate { path });
                    }
                    Err(error) => return Err(write(error)),
                }
                links.insert(rel);
            }
            EntryType::Link => {
                let target = target(&entry);
                let unsafe_link = || UntarError::LinkUnsafe {
                    path: path.clone(),
                    target: shown(&target),
                };
                // Only to a file this stream wrote, which is a real file in a
                // real folder, so the link reaches nothing outside `root`,
                // and costs no bytes.
                let source = match relative(&target, &shown(&target)) {
                    Ok(Some(source)) if files.contains(&source) => source,
                    _ => return Err(unsafe_link()),
                };
                match fs::hard_link(root.join(&source), &full) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                        return Err(UntarError::Duplicate { path });
                    }
                    Err(error) => return Err(write(error)),
                }
                unpacked.files += 1;
                files.insert(rel);
            }
            other => {
                return Err(UntarError::Unsupported {
                    path,
                    kind: describe(other),
                });
            }
        }
    }
    Ok(unpacked)
}

/// A link's target, empty if it has none, which no rule allows.
fn target(entry: &tar::Entry<'_, impl Read>) -> Vec<u8> {
    entry
        .link_name_bytes()
        .map(std::borrow::Cow::into_owned)
        .unwrap_or_default()
}

/// An entry's path as a relative path of plain names, or none for the folder
/// itself. Empty names and `.` are skipped, as tar does.
fn relative(raw: &[u8], path: &str) -> Result<Option<PathBuf>, UntarError> {
    if raw.first() == Some(&b'/') {
        return Err(UntarError::PathUnsafe {
            path: path.to_owned(),
            why: "is absolute",
        });
    }
    let mut rel = PathBuf::new();
    for name in raw.split(|&byte| byte == b'/') {
        match name {
            b"" | b"." => {}
            b".." => {
                return Err(UntarError::PathUnsafe {
                    path: path.to_owned(),
                    why: "holds `..`",
                });
            }
            name => rel.push(OsStr::from_bytes(name)),
        }
    }
    Ok((!rel.as_os_str().is_empty()).then_some(rel))
}

/// Makes the folders `rel` is in, inside `root`, each a real folder: none may
/// be a link, or anything else, so nothing is written through one.
fn parents(
    root: &Path,
    rel: &Path,
    links: &HashSet<PathBuf>,
    path: &str,
) -> Result<(), UntarError> {
    let Some(parent) = rel.parent() else {
        return Ok(());
    };
    let mut folder = PathBuf::new();
    for name in parent.iter() {
        folder.push(name);
        if links.contains(&folder) {
            return Err(UntarError::PathUnsafe {
                path: path.to_owned(),
                why: "is under a link",
            });
        }
        let full = root.join(&folder);
        match fs::symlink_metadata(&full) {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => {
                return Err(UntarError::PathUnsafe {
                    path: path.to_owned(),
                    why: "is under a file or a link",
                });
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                fs::create_dir(&full).map_err(|source| UntarError::Write {
                    path: shown(folder.as_os_str().as_bytes()),
                    source,
                })?;
            }
            Err(source) => {
                return Err(UntarError::Write {
                    path: path.to_owned(),
                    source,
                });
            }
        }
    }
    Ok(())
}

/// Whether a link at `rel` to `target` can only lead inside the folder: the
/// target is relative, its `..`s come first and climb no higher than the
/// folder, and no `..` follows a name, which could itself be a link. Since
/// every link here keeps these rules and no entry is under a link, following
/// any chain of them stays inside.
fn link_stays_inside(rel: &Path, target: &[u8]) -> bool {
    if target.is_empty() || target.first() == Some(&b'/') {
        return false;
    }
    let mut depth = rel.iter().count().saturating_sub(1);
    let mut named = false;
    for name in target.split(|&byte| byte == b'/') {
        match name {
            b"" | b"." => {}
            b".." => {
                if named || depth == 0 {
                    return false;
                }
                depth -= 1;
            }
            _ => named = true,
        }
    }
    true
}

/// A new file at `full`, which mustn't exist yet: `create_new` refuses a
/// path that's a link, even one to nothing. It keeps the entry's permission
/// bits but group and others' write, and never setuid, setgid or sticky.
fn create(full: &Path, mode: u32, path: &str) -> Result<File, UntarError> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode & 0o755)
        .open(full)
        .map_err(|source| {
            if source.kind() == io::ErrorKind::AlreadyExists {
                UntarError::Duplicate {
                    path: path.to_owned(),
                }
            } else {
                UntarError::Write {
                    path: path.to_owned(),
                    source,
                }
            }
        })
}

/// Copies an entry's bytes into its file, at most `budget` of them, telling
/// a broken stream from a failed write.
fn copy(
    entry: &mut impl Read,
    file: &mut File,
    budget: u64,
    path: &str,
) -> Result<u64, UntarError> {
    let mut buffer = vec![0; 64 * 1024];
    let mut copied = 0;
    loop {
        let read = match entry.read(&mut buffer) {
            Ok(0) => return Ok(copied),
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                return Err(UntarError::Stream {
                    detail: format!("{path}: {error}"),
                });
            }
        };
        file.write_all(&buffer[..read])
            .map_err(|source| UntarError::Write {
                path: path.to_owned(),
                source,
            })?;
        copied += read as u64;
        if copied > budget {
            return Err(UntarError::TooLarge);
        }
    }
}

/// A path or link target from the stream, quoted, with escapes for anything
/// that isn't printable UTF-8.
fn shown(raw: &[u8]) -> String {
    format!("{:?}", String::from_utf8_lossy(raw))
}

fn describe(kind: EntryType) -> String {
    match kind {
        EntryType::Char => "character device".to_owned(),
        EntryType::Block => "block device".to_owned(),
        EntryType::Fifo => "FIFO".to_owned(),
        EntryType::GNUSparse => "sparse file".to_owned(),
        other => format!("tar entry of type {:?}", char::from(other.as_byte())),
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    use super::*;

    /// One entry of a stream: its kind, name, link target and contents,
    /// written into the header as raw bytes, so names tar's own builder
    /// refuses can be sent.
    struct Raw<'a>(EntryType, &'a str, &'a str, &'a [u8]);

    fn stream(entries: &[Raw<'_>]) -> Vec<u8> {
        let mut out = Vec::new();
        for Raw(kind, name, link, data) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(*kind);
            header.set_size(data.len() as u64);
            header.set_mode(if name.ends_with(".sh") { 0o755 } else { 0o644 });
            let old = header.as_old_mut();
            old.name[..name.len()].copy_from_slice(name.as_bytes());
            old.linkname[..link.len()].copy_from_slice(link.as_bytes());
            header.set_cksum();
            out.extend_from_slice(header.as_bytes());
            out.extend_from_slice(data);
            out.resize(out.len().next_multiple_of(512), 0);
        }
        // The end of the archive, then padding, as GNU tar writes.
        out.resize(out.len() + 1024 + 4096, 0);
        out
    }

    /// Unpacks into `root`, a folder alone inside another, and checks that
    /// nothing was made beside it, whatever happened.
    fn unpack_alone(bytes: &[u8]) -> (tempfile::TempDir, Result<Unpacked, UntarError>) {
        let outer = tempfile::tempdir().unwrap();
        let root = outer.path().join("root");
        fs::create_dir(&root).unwrap();
        let result = unpack(bytes, &root);
        let beside: Vec<_> = fs::read_dir(outer.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(beside, ["root"], "something was made outside the folder");
        (outer, result)
    }

    #[test]
    fn unpacks_files_folders_and_links_that_stay_inside() {
        let (outer, result) = unpack_alone(&stream(&[
            Raw(EntryType::Directory, "./", "", b""),
            Raw(EntryType::Regular, "./app.toml", "", b"title = \"Hi\"\n"),
            Raw(EntryType::Directory, "./bin/", "", b""),
            Raw(EntryType::Regular, "./bin/run.sh", "", b"#!/bin/sh\n"),
            Raw(EntryType::Symlink, "./bin/again", "run.sh", b""),
            Raw(EntryType::Regular, "./node_modules/pkg/cli.js", "", b"x"),
            Raw(
                EntryType::Symlink,
                "./node_modules/.bin/cli",
                "../pkg/cli.js",
                b"",
            ),
            Raw(EntryType::Symlink, "./here", ".", b""),
            Raw(EntryType::Link, "./copy.toml", "./app.toml", b""),
            Raw(EntryType::XGlobalHeader, "pax_global_header", "", b""),
        ]));
        assert_eq!(
            result.unwrap(),
            Unpacked {
                files: 4,
                // The hard link costs no bytes.
                bytes: 13 + 10 + 1
            }
        );
        let root = outer.path().join("root");
        assert_eq!(
            fs::read(root.join("copy.toml")).unwrap(),
            b"title = \"Hi\"\n"
        );
        assert_eq!(fs::metadata(root.join("copy.toml")).unwrap().nlink(), 2);
        let mode = |path: &str| fs::metadata(root.join(path)).unwrap().permissions().mode() & 0o111;
        assert_ne!(mode("bin/run.sh"), 0);
        assert_eq!(mode("app.toml"), 0);
        assert_eq!(
            fs::read_link(root.join("node_modules/.bin/cli")).unwrap(),
            Path::new("../pkg/cli.js")
        );
        assert_eq!(fs::read(root.join("node_modules/.bin/cli")).unwrap(), b"x");
        assert_eq!(fs::read(root.join("bin/again")).unwrap(), b"#!/bin/sh\n");
    }

    #[test]
    fn refuses_anything_that_could_leave_the_folder() {
        let file = |name| Raw(EntryType::Regular, name, "", b"x");
        let link = |name, target| Raw(EntryType::Symlink, name, target, b"");
        let cases: Vec<(&str, Vec<Raw<'_>>)> = vec![
            ("pull_path_unsafe", vec![file("/etc/passwd")]),
            ("pull_path_unsafe", vec![file("../outside")]),
            ("pull_path_unsafe", vec![file("./a/../../outside")]),
            ("pull_path_unsafe", vec![file("./")]),
            ("pull_link_unsafe", vec![link("./etc", "/etc")]),
            ("pull_link_unsafe", vec![link("./up", "..")]),
            ("pull_link_unsafe", vec![link("./a/b", "../../x")]),
            // `here` could be a link, so a `..` after it could climb anywhere.
            (
                "pull_link_unsafe",
                vec![link("./here", "."), link("./up", "here/..")],
            ),
            ("pull_link_unsafe", vec![link("./empty", "")]),
            // Nothing is written through a link, even one that stays inside.
            (
                "pull_path_unsafe",
                vec![
                    Raw(EntryType::Directory, "./d/", "", b""),
                    link("./l", "d"),
                    file("./l/x"),
                ],
            ),
            ("pull_path_unsafe", vec![file("./f"), file("./f/x")]),
            ("pull_entry_duplicate", vec![file("./f"), file("./f")]),
            ("pull_entry_duplicate", vec![link("./l", "f"), file("./l")]),
            (
                "pull_entry_duplicate",
                vec![file("./f"), Raw(EntryType::Directory, "./f/", "", b"")],
            ),
            (
                "pull_link_unsafe",
                vec![Raw(EntryType::Link, "./h", "/etc/passwd", b"")],
            ),
            (
                "pull_link_unsafe",
                vec![Raw(EntryType::Link, "./h", "./nothing", b"")],
            ),
            (
                "pull_link_unsafe",
                vec![
                    file("./f"),
                    link("./l", "f"),
                    Raw(EntryType::Link, "./h", "./l", b""),
                ],
            ),
            (
                "pull_entry_unsupported",
                vec![Raw(EntryType::Fifo, "./fifo", "", b"")],
            ),
            (
                "pull_entry_unsupported",
                vec![Raw(EntryType::Char, "./null", "", b"")],
            ),
            (
                "pull_entry_unsupported",
                vec![Raw(EntryType::Block, "./disk", "", b"")],
            ),
        ];
        for (code, entries) in cases {
            let names: Vec<&str> = entries.iter().map(|raw| raw.1).collect();
            let (_outer, result) = unpack_alone(&stream(&entries));
            let error = result.expect_err(&format!("{names:?} was unpacked"));
            assert_eq!(error.code(), code, "{names:?}: {error}");
            if code == "pull_link_unsafe" {
                assert!(
                    error
                        .to_string()
                        .contains("pull a sub-folder that doesn't hold such links"),
                    "{error}"
                );
            }
        }
    }

    #[test]
    fn a_stream_cut_short_or_garbled_is_refused() {
        let whole = stream(&[Raw(EntryType::Regular, "./big", "", &[7; 2000])]);
        let (_outer, result) = unpack_alone(&whole[..1000]);
        assert_eq!(result.unwrap_err().code(), "pull_stream_invalid");

        let mut garbled = whole.clone();
        garbled[100] ^= 0xff;
        let (_outer, result) = unpack_alone(&garbled);
        assert_eq!(result.unwrap_err().code(), "pull_stream_invalid");

        // A file bigger than pull takes is refused before its bytes are read.
        let mut huge = tar::Header::new_gnu();
        huge.set_entry_type(EntryType::Regular);
        huge.set_path("./huge").unwrap();
        huge.set_size(MAX_BYTES + 1);
        huge.set_cksum();
        let (_outer, result) = unpack_alone(huge.as_bytes());
        assert_eq!(result.unwrap_err().code(), "pull_too_large");

        // So is one whose pax header says so, over a header saying 0.
        let record = format!(" size={}\n", MAX_BYTES + 1);
        let record = format!("{}{record}", record.len() + 2);
        assert_eq!(
            record.len(),
            record.split(' ').next().unwrap().parse::<usize>().unwrap()
        );
        let (_outer, result) = unpack_alone(&stream(&[
            Raw(
                EntryType::XHeader,
                "./PaxHeaders/huge",
                "",
                record.as_bytes(),
            ),
            Raw(EntryType::Regular, "./huge", "", b""),
        ]));
        assert_eq!(result.unwrap_err().code(), "pull_too_large");

        // An empty stream, as a tar that failed at once sends, is no files.
        let (_outer, result) = unpack_alone(b"");
        assert_eq!(result.unwrap(), Unpacked::default());
    }

    #[test]
    fn a_link_stays_inside_only_by_the_rules() {
        let inside = |rel: &str, target: &str| link_stays_inside(Path::new(rel), target.as_bytes());
        assert!(inside("a", "b"));
        assert!(inside("a", "./b/c"));
        assert!(inside("d/a", "../b"));
        assert!(inside("d/e/a", "../../b/c"));
        assert!(!inside("a", "../b"));
        assert!(!inside("d/a", "../../b"));
        assert!(!inside("a", "b/../c"));
        assert!(!inside("a", "/b"));
        assert!(!inside("a", ""));
    }
}
