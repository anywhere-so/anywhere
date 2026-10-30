//! File-list validation and the client-facing upload contract.
//! A deploy lists every file it writes and uploads every hash requested by the API.
//! Each PUT is a zstd-compressed tar of regular files named by BLAKE3 hash.
//! Executable permissions come from the file list, not the tar stream.
//! Each PUT carries at most MAX_BATCH_SIZE bytes of file contents.
//! Missing reports how many files remain; UploadError gives a stable error code,
//! an actionable message, and whether repeating the upload is safe.

use std::cmp::Ordering;
use std::collections::HashMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The most bytes of file contents one upload PUT carries: 30 MiB.
pub const MAX_BATCH_SIZE: u64 = 30 * 1024 * 1024;

/// The largest file, since each file goes up in a single `PUT`.
pub const MAX_FILE_SIZE: u64 = MAX_BATCH_SIZE;

/// The most files in a deploy: far more than an app's code has, while keeping
/// each deploy's list quick to send and check.
pub const MAX_FILES: usize = 50_000;

/// The most a deploy carries in all: room for any app's code. What the app
/// writes on its disk counts toward its quota, not this.
pub const MAX_TOTAL_SIZE: u64 = 1 << 30;

/// The longest name of a file or folder in a path, in bytes: the most that Linux
/// filesystems, ZFS among them, allow.
pub const MAX_PART_LEN: usize = 255;

/// The longest path, in bytes: Linux's limit on a path.
pub const MAX_PATH_LEN: usize = 4096;

/// One file a deploy writes.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileEntry {
    /// Its path inside the app's folder, and inside `/app`, with `/` between
    /// folders, such as `src/main.py`.
    pub path: String,
    /// The BLAKE3 hash of its contents, in lowercase hex.
    pub hash: String,
    /// Its size in bytes.
    pub size: u64,
    /// Whether it can be run as a program.
    pub executable: bool,
}

/// A `PUT`'s answer, while files are missing and once none are: how many of
/// the list's hashes are still to come.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Missing {
    pub missing: u64,
}

/// An error, as a `PUT`'s answer and the result file give it. Its fields are the
/// control API's `ErrorBody`'s, which can't be used here, since the control API
/// uses this crate.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UploadError {
    /// A stable code, such as `upload_entry_hash_mismatch`, that callers can match on.
    pub code: String,
    /// What went wrong and how to fix it.
    pub message: String,
    /// Whether deploying again could work.
    pub retryable: bool,
}

/// Why a file list was refused. Each message says how to fix it, for the AI
/// agents that read them, and [`ManifestError::code`] gives each a stable code.
/// Paths are quoted with escapes, so a strange name can't garble the message.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ManifestError {
    #[error(
        "the app has {count} files; a deploy carries at most {MAX_FILES}, so leave dependencies and build output out with .gitignore"
    )]
    TooManyFiles { count: usize },
    #[error("a file has an empty path; give each file its path inside the app's folder")]
    EmptyPath,
    #[error(
        "the path {path:?} is {len} bytes long; shorten it to at most {MAX_PATH_LEN}, or leave the file out with .gitignore"
    )]
    PathTooLong { path: String, len: usize },
    #[error("the path {path:?} has a NUL byte, which no file name can hold; rename the file")]
    PathHasNul { path: String },
    #[error(
        "the path {path:?} has a backslash; use / between folders, and rename any file or folder whose name has a backslash"
    )]
    PathHasBackslash { path: String },
    #[error(
        "the path {path:?} starts with /; give each path inside the app's folder, such as src/main.py"
    )]
    PathIsAbsolute { path: String },
    #[error(
        "the path {path:?} has an empty part, from a doubled / or one at its end; remove the extra /"
    )]
    PathHasEmptyPart { path: String },
    #[error(
        "the path {path:?} has a . or .. part; give each file's path inside the app's folder without them"
    )]
    PathHasDotPart { path: String },
    #[error(
        "the path {path:?} has a name of {len} bytes; shorten each file and folder name to at most {MAX_PART_LEN}"
    )]
    PathPartTooLong { path: String, len: usize },
    #[error("the path {path:?} is listed twice; list each file once")]
    PathRepeated { path: String },
    #[error(
        "{file:?} is a file, but {inside:?} puts it as a folder; a path can't be both, so rename one of them"
    )]
    FileIsFolder { file: String, inside: String },
    #[error(
        "the hash of {path:?} is {hash:?}; give the BLAKE3 hash of its contents as 64 lowercase hex characters"
    )]
    BadHash { path: String, hash: String },
    #[error(
        "{path:?} and {other:?} have the same hash but different sizes, which can't be; hash both files again"
    )]
    HashSizeConflict { path: String, other: String },
    #[error(
        "{path:?} is {size} bytes; a file can be at most {MAX_FILE_SIZE} bytes (30 MiB), so make it smaller or leave it out with .gitignore"
    )]
    FileTooLarge { path: String, size: u64 },
    #[error(
        "the app's files add up to more than {MAX_TOTAL_SIZE} bytes (1 GiB), which a deploy can't carry; leave dependencies and build output out with .gitignore"
    )]
    TooLarge,
}

impl ManifestError {
    /// A stable code for the error, which callers can match on.
    pub fn code(&self) -> &'static str {
        match self {
            ManifestError::TooManyFiles { .. } => "manifest_too_many_files",
            ManifestError::EmptyPath
            | ManifestError::PathTooLong { .. }
            | ManifestError::PathHasNul { .. }
            | ManifestError::PathHasBackslash { .. }
            | ManifestError::PathIsAbsolute { .. }
            | ManifestError::PathHasEmptyPart { .. }
            | ManifestError::PathHasDotPart { .. }
            | ManifestError::PathPartTooLong { .. } => "manifest_path_invalid",
            ManifestError::PathRepeated { .. } | ManifestError::FileIsFolder { .. } => {
                "manifest_path_conflict"
            }
            ManifestError::BadHash { .. } => "manifest_hash_invalid",
            ManifestError::HashSizeConflict { .. } => "manifest_hash_conflict",
            ManifestError::FileTooLarge { .. } => "manifest_file_too_large",
            ManifestError::TooLarge => "manifest_too_large",
        }
    }
}

/// Checks a deploy's file list against every rule: each path, hash and size,
/// that no path is listed twice or is a folder of another, that files with the
/// same hash have the same size, and the limits on the whole deploy.
pub fn check(files: &[FileEntry]) -> Result<(), ManifestError> {
    // First, so the rest never works through an oversized list.
    if files.len() > MAX_FILES {
        return Err(ManifestError::TooManyFiles { count: files.len() });
    }

    let mut total: u64 = 0;
    let mut sizes: HashMap<&str, (u64, &str)> = HashMap::with_capacity(files.len());
    for file in files {
        check_path(&file.path)?;
        if !is_hash(&file.hash) {
            return Err(ManifestError::BadHash {
                path: file.path.clone(),
                hash: file.hash.clone(),
            });
        }
        if file.size > MAX_FILE_SIZE {
            return Err(ManifestError::FileTooLarge {
                path: file.path.clone(),
                size: file.size,
            });
        }
        // Each size is at most 30 MiB and there are at most 50,000, so this can't overflow.
        total += file.size;
        if total > MAX_TOTAL_SIZE {
            return Err(ManifestError::TooLarge);
        }
        let (size, path) = *sizes
            .entry(file.hash.as_str())
            .or_insert((file.size, file.path.as_str()));
        if size != file.size {
            return Err(ManifestError::HashSizeConflict {
                path: path.to_owned(),
                other: file.path.clone(),
            });
        }
    }

    // Sorted with `/` before every other byte, a file comes straight before
    // anything listed inside it, so comparing neighbours finds every clash.
    let mut paths: Vec<&str> = files.iter().map(|file| file.path.as_str()).collect();
    paths.sort_unstable_by(|a, b| folders_first(a, b));
    for pair in paths.windows(2) {
        let [first, second] = pair else { continue };
        if first == second {
            return Err(ManifestError::PathRepeated {
                path: (*first).to_owned(),
            });
        }
        if second
            .strip_prefix(*first)
            .is_some_and(|rest| rest.starts_with('/'))
        {
            return Err(ManifestError::FileIsFolder {
                file: (*first).to_owned(),
                inside: (*second).to_owned(),
            });
        }
    }
    Ok(())
}

/// Checks one path by the rules every path in a file list keeps: relative, with
/// `/` between folders, no empty, `.` or `..` part, no NUL or backslash, and within
/// Linux's limits on names and paths. `app.toml`'s `static` keeps them too.
pub fn check_path(path: &str) -> Result<(), ManifestError> {
    let owned = || path.to_owned();
    if path.is_empty() {
        return Err(ManifestError::EmptyPath);
    }
    if path.len() > MAX_PATH_LEN {
        return Err(ManifestError::PathTooLong {
            path: owned(),
            len: path.len(),
        });
    }
    if path.contains('\0') {
        return Err(ManifestError::PathHasNul { path: owned() });
    }
    if path.contains('\\') {
        return Err(ManifestError::PathHasBackslash { path: owned() });
    }
    if path.starts_with('/') {
        return Err(ManifestError::PathIsAbsolute { path: owned() });
    }
    for part in path.split('/') {
        if part.is_empty() {
            return Err(ManifestError::PathHasEmptyPart { path: owned() });
        }
        if part == "." || part == ".." {
            return Err(ManifestError::PathHasDotPart { path: owned() });
        }
        if part.len() > MAX_PART_LEN {
            return Err(ManifestError::PathPartTooLong {
                path: owned(),
                len: part.len(),
            });
        }
    }
    Ok(())
}

/// Whether `text` is a BLAKE3 hash in lowercase hex.
pub fn is_hash(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Orders paths as if `/` came before every other byte. Paths hold no NUL, so
/// mapping `/` to it does that.
fn folders_first(a: &str, b: &str) -> Ordering {
    let key = |byte: u8| if byte == b'/' { 0 } else { byte };
    a.bytes().map(key).cmp(b.bytes().map(key))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262";

    fn file(path: &str) -> FileEntry {
        FileEntry {
            path: path.to_owned(),
            hash: HASH.to_owned(),
            size: 0,
            executable: false,
        }
    }

    fn sized(path: &str, hash: &str, size: u64) -> FileEntry {
        FileEntry {
            path: path.to_owned(),
            hash: hash.to_owned(),
            size,
            executable: false,
        }
    }

    /// A different valid hash for each number.
    fn hash(n: usize) -> String {
        format!("{n:064x}")
    }

    fn check_one(path: &str) -> Result<(), ManifestError> {
        check(&[file(path)])
    }

    #[test]
    fn accepts_a_typical_deploy() {
        let files = [
            file("app.toml"),
            file("index.html"),
            file("src/main.py"),
            file("src/lib/util.py"),
            file(".well-known/security.txt"),
            file("a"),
            file("ab/c"),
            file("a!b"),
            file("a.b/c"),
            file("names with spaces/and…unicode ✓"),
        ];
        assert_eq!(check(&files), Ok(()));
        assert_eq!(check(&[]), Ok(()));
    }

    #[test]
    fn paths_are_relative_with_slashes_and_no_empty_or_dot_parts() {
        let cases = [
            ("", ManifestError::EmptyPath),
            (
                "/etc/passwd",
                ManifestError::PathIsAbsolute {
                    path: "/etc/passwd".to_owned(),
                },
            ),
            (
                "a//b",
                ManifestError::PathHasEmptyPart {
                    path: "a//b".to_owned(),
                },
            ),
            (
                "a/",
                ManifestError::PathHasEmptyPart {
                    path: "a/".to_owned(),
                },
            ),
            (
                ".",
                ManifestError::PathHasDotPart {
                    path: ".".to_owned(),
                },
            ),
            (
                "./a",
                ManifestError::PathHasDotPart {
                    path: "./a".to_owned(),
                },
            ),
            (
                "a/./b",
                ManifestError::PathHasDotPart {
                    path: "a/./b".to_owned(),
                },
            ),
            (
                "..",
                ManifestError::PathHasDotPart {
                    path: "..".to_owned(),
                },
            ),
            (
                "a/../../b",
                ManifestError::PathHasDotPart {
                    path: "a/../../b".to_owned(),
                },
            ),
            (
                "a\0b",
                ManifestError::PathHasNul {
                    path: "a\0b".to_owned(),
                },
            ),
            (
                "a\\b",
                ManifestError::PathHasBackslash {
                    path: "a\\b".to_owned(),
                },
            ),
        ];
        for (path, want) in cases {
            assert_eq!(check_one(path), Err(want), "{path:?}");
        }
        // Dots are fine inside a name.
        for path in [".env.example", "..a", "a..", "a/.b/c..d"] {
            assert_eq!(check_one(path), Ok(()), "{path:?}");
        }
    }

    #[test]
    fn each_name_is_at_most_255_bytes() {
        let longest = "n".repeat(MAX_PART_LEN);
        assert_eq!(check_one(&format!("{longest}/{longest}")), Ok(()));
        // Counted in bytes: each of these characters is two.
        let longest_accented = "é".repeat(MAX_PART_LEN / 2);
        assert_eq!(check_one(&longest_accented), Ok(()));

        let too_long = format!("src/{}", "n".repeat(MAX_PART_LEN + 1));
        assert_eq!(
            check_one(&too_long),
            Err(ManifestError::PathPartTooLong {
                path: too_long.clone(),
                len: MAX_PART_LEN + 1,
            })
        );
        let too_long_accented = "é".repeat(MAX_PART_LEN / 2 + 1);
        assert_eq!(
            check_one(&too_long_accented).map_err(|e| e.code()),
            Err("manifest_path_invalid")
        );
    }

    #[test]
    fn the_whole_path_is_at_most_4096_bytes() {
        let longest = "a/".repeat(MAX_PATH_LEN / 2 - 1) + "ab";
        assert_eq!(longest.len(), MAX_PATH_LEN);
        assert_eq!(check_one(&longest), Ok(()));

        let too_long = format!("{longest}c");
        assert_eq!(
            check_one(&too_long),
            Err(ManifestError::PathTooLong {
                path: too_long.clone(),
                len: MAX_PATH_LEN + 1,
            })
        );
    }

    #[test]
    fn paths_are_unique() {
        assert_eq!(
            check(&[file("a/b"), file("c"), file("a/b")]),
            Err(ManifestError::PathRepeated {
                path: "a/b".to_owned()
            })
        );
    }

    #[test]
    fn no_file_is_a_folder_of_another() {
        for files in [
            [file("a"), file("a/b")],
            [file("a/b/c"), file("a")],
            [file("a/b"), file("a/b/c/d")],
        ] {
            let error = check(&files).unwrap_err();
            assert_eq!(error.code(), "manifest_path_conflict", "{files:?}");
            assert!(matches!(error, ManifestError::FileIsFolder { .. }));
        }
        // Names that sort between a file and what's inside it don't hide the clash.
        assert_eq!(
            check(&[file("a/b"), file("a!"), file("a.b"), file("a"), file("a-")]),
            Err(ManifestError::FileIsFolder {
                file: "a".to_owned(),
                inside: "a/b".to_owned(),
            })
        );
        // A name that only starts like another is fine.
        assert_eq!(check(&[file("a"), file("ab/c"), file("a.d/e")]), Ok(()));
    }

    #[test]
    fn hashes_are_64_lowercase_hex_characters() {
        let digits = "0123456789abcdef".repeat(4);
        assert_eq!(check(&[sized("a", &digits, 0)]), Ok(()));
        for bad in [
            String::new(),
            digits[..63].to_owned(),
            format!("{digits}0"),
            digits.to_uppercase(),
            format!("{}g", &digits[..63]),
            format!("{} ", &digits[..63]),
        ] {
            assert_eq!(
                check(&[sized("a", &bad, 0)]),
                Err(ManifestError::BadHash {
                    path: "a".to_owned(),
                    hash: bad.clone()
                }),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn a_file_is_at_most_90_mb() {
        assert_eq!(check(&[sized("big", HASH, MAX_FILE_SIZE)]), Ok(()));
        assert_eq!(
            check(&[sized("big", HASH, MAX_FILE_SIZE + 1)]),
            Err(ManifestError::FileTooLarge {
                path: "big".to_owned(),
                size: MAX_FILE_SIZE + 1
            })
        );
    }

    #[test]
    fn a_deploy_has_at_most_50_000_files() {
        let mut files: Vec<FileEntry> = (0..MAX_FILES)
            .map(|n| sized(&format!("f{n}"), &hash(n), 1))
            .collect();
        assert_eq!(check(&files), Ok(()));
        files.push(sized("one-more", &hash(MAX_FILES), 1));
        assert_eq!(
            check(&files),
            Err(ManifestError::TooManyFiles {
                count: MAX_FILES + 1
            })
        );
    }

    #[test]
    fn a_deploy_carries_at_most_1_gib() {
        // As many files of the largest size as fit, and one that brings the
        // total to exactly 1 GiB.
        let full = MAX_TOTAL_SIZE / MAX_FILE_SIZE;
        let count = usize::try_from(full).unwrap();
        let mut files: Vec<FileEntry> = (0..count)
            .map(|n| sized(&format!("f{n}"), &hash(n), MAX_FILE_SIZE))
            .collect();
        files.push(sized(
            "last",
            &hash(count),
            MAX_TOTAL_SIZE - full * MAX_FILE_SIZE,
        ));
        assert_eq!(check(&files), Ok(()));

        files.push(sized("one-byte-more", &hash(count + 1), 1));
        assert_eq!(check(&files), Err(ManifestError::TooLarge));
    }

    #[test]
    fn files_with_the_same_hash_have_the_same_size() {
        // The same contents in two places is fine.
        assert_eq!(check(&[sized("a", HASH, 5), sized("b/a", HASH, 5)]), Ok(()));
        assert_eq!(
            check(&[sized("a", HASH, 5), sized("b", HASH, 6)]),
            Err(ManifestError::HashSizeConflict {
                path: "a".to_owned(),
                other: "b".to_owned()
            })
        );
    }

    #[test]
    fn errors_have_stable_codes_and_say_how_to_fix_them() {
        let cases = [
            (check_one("/a").unwrap_err(), "manifest_path_invalid"),
            (
                check(&[file("a"), file("a")]).unwrap_err(),
                "manifest_path_conflict",
            ),
            (
                check(&[sized("a", "x", 0)]).unwrap_err(),
                "manifest_hash_invalid",
            ),
            (
                check(&[sized("a", HASH, MAX_FILE_SIZE + 1)]).unwrap_err(),
                "manifest_file_too_large",
            ),
            (
                check(&[sized("a", HASH, 1), sized("b", HASH, 2)]).unwrap_err(),
                "manifest_hash_conflict",
            ),
            (ManifestError::TooLarge, "manifest_too_large"),
            (
                ManifestError::TooManyFiles { count: 1 },
                "manifest_too_many_files",
            ),
        ];
        for (error, code) in cases {
            assert_eq!(error.code(), code, "{error}");
        }
        assert_eq!(
            check_one("src\\main.py").unwrap_err().to_string(),
            "the path \"src\\\\main.py\" has a backslash; use / between folders, and rename any file or folder whose name has a backslash"
        );
        // Control characters in a path are escaped, so they can't garble the message.
        let error = check_one("a\u{1b}[2J/").unwrap_err().to_string();
        assert!(error.contains(r#""a\u{1b}[2J/""#), "{error}");
    }

    #[test]
    fn an_entry_reads_and_writes_as_json_and_refuses_unknown_fields() {
        let entry = FileEntry {
            path: "bin/run".to_owned(),
            hash: HASH.to_owned(),
            size: 12,
            executable: true,
        };
        let json = serde_json::to_value(&entry).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"path": "bin/run", "hash": HASH, "size": 12, "executable": true})
        );
        assert_eq!(serde_json::from_value::<FileEntry>(json).unwrap(), entry);

        let extra = serde_json::json!({"path": "a", "hash": HASH, "size": 1, "executable": false, "mode": 511});
        assert!(serde_json::from_value::<FileEntry>(extra).is_err());
    }

    #[test]
    fn an_answer_says_how_many_files_are_missing() {
        assert_eq!(
            serde_json::to_value(Missing { missing: 2 }).unwrap(),
            serde_json::json!({"missing": 2})
        );
        assert!(
            serde_json::from_value::<Missing>(serde_json::json!({"missing": 2, "more": 1}))
                .is_err()
        );
    }
}
