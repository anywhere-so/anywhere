//! `manifest::check`, which the control plane runs on every file list a deploy
//! sends, since the CLI that makes it is outside our trust. The list is built
//! from a few short parts and hashes, so paths repeat, nest and clash, and the
//! check is compared with a plain one that tries every pair of files: both pass
//! the same lists. The control plane reads lists as JSON, so the target also
//! reads its input as JSON when it is.

#![no_main]

use std::collections::HashMap;

use arbitrary::{Arbitrary, Unstructured};
use libfuzzer_sys::fuzz_target;
use manifest::{FileEntry, MAX_FILE_SIZE, MAX_PART_LEN, MAX_PATH_LEN, MAX_TOTAL_SIZE};

/// What a path is made of: short parts that clash, and the bytes that a path
/// mustn't hold.
const PARTS: &[&str] = &[
    "a", "b", "a.b", "a!", ".", "..", "", "\0", "\\", "é", "a b", "\u{202e}",
];

/// A few valid hashes, so files share them, and a few that aren't.
const HASHES: &[&str] = &[
    "0000000000000000000000000000000000000000000000000000000000000000",
    "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262",
    "AF1349B9F5F9A1A6A0404DEA36DCC9499BCB25C9ADC112B7CC9A93CAE41F3262",
    "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f326",
    "",
];

#[derive(Debug)]
struct Files(Vec<FileEntry>);

impl<'a> Arbitrary<'a> for Files {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let count = u.int_in_range(0..=12)?;
        let mut files = Vec::with_capacity(count);
        for _ in 0..count {
            let path = if u.ratio(1, 16)? {
                // Sometimes any text at all, or a part as long as one may be.
                match u.int_in_range(0..=2)? {
                    0 => String::arbitrary(u)?,
                    1 => "n".repeat(MAX_PART_LEN + u.int_in_range(0..=1)?),
                    _ => "n/".repeat(MAX_PATH_LEN / 2) + &"n".repeat(u.int_in_range(0..=2)?),
                }
            } else {
                let parts = u.int_in_range(1..=4)?;
                let mut path = if u.ratio(1, 20)? {
                    "/".to_owned()
                } else {
                    String::new()
                };
                for i in 0..parts {
                    if i > 0 {
                        path.push('/');
                    }
                    path.push_str(u.choose(PARTS)?);
                }
                path
            };
            let hash = if u.ratio(1, 16)? {
                String::arbitrary(u)?
            } else {
                (*u.choose(HASHES)?).to_owned()
            };
            let size = match u.int_in_range(0..=3)? {
                0 => u.int_in_range(0..=2)?,
                1 => MAX_FILE_SIZE + u.int_in_range(0..=1)?,
                2 => MAX_TOTAL_SIZE / 2 + u.int_in_range(0..=1)?,
                _ => u64::arbitrary(u)?,
            };
            files.push(FileEntry {
                path,
                hash,
                size,
                executable: bool::arbitrary(u)?,
            });
        }
        Ok(Files(files))
    }
}

/// The rules, written the plain way: each file on its own, then every pair.
fn plainly_valid(files: &[FileEntry]) -> bool {
    if files.len() > manifest::MAX_FILES {
        return false;
    }
    let path_ok = |path: &str| {
        !path.is_empty()
            && path.len() <= MAX_PATH_LEN
            && !path.contains('\0')
            && !path.contains('\\')
            && path.split('/').all(|part| {
                !part.is_empty() && part != "." && part != ".." && part.len() <= MAX_PART_LEN
            })
    };
    let hash_ok = |hash: &str| {
        hash.len() == 64 && hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    };
    let mut total: u64 = 0;
    let mut sizes = HashMap::new();
    for file in files {
        if !path_ok(&file.path) || !hash_ok(&file.hash) || file.size > MAX_FILE_SIZE {
            return false;
        }
        total += file.size;
        if total > MAX_TOTAL_SIZE {
            return false;
        }
        if *sizes.entry(file.hash.as_str()).or_insert(file.size) != file.size {
            return false;
        }
    }
    for (i, a) in files.iter().enumerate() {
        for b in &files[i + 1..] {
            let inside = |file: &str, other: &str| {
                other
                    .strip_prefix(file)
                    .is_some_and(|rest| rest.starts_with('/'))
            };
            if a.path == b.path || inside(&a.path, &b.path) || inside(&b.path, &a.path) {
                return false;
            }
        }
    }
    true
}

fuzz_target!(|data: &[u8]| {
    // The control plane's own reading: JSON, then the check.
    if let Ok(files) = serde_json::from_slice::<Vec<FileEntry>>(data) {
        let checked = manifest::check(&files);
        assert_eq!(
            checked.is_ok(),
            plainly_valid(&files),
            "{files:?}: {checked:?}"
        );
    }
    let Ok(Files(files)) = Files::arbitrary_take_rest(Unstructured::new(data)) else {
        return;
    };
    let checked = manifest::check(&files);
    if let Err(error) = &checked {
        assert!(!error.code().is_empty());
        assert!(!error.to_string().is_empty());
    }
    assert_eq!(
        checked.is_ok(),
        plainly_valid(&files),
        "{files:?}: {checked:?}"
    );
});
