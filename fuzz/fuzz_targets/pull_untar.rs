//! `anywhere pull`'s unpacking of the tar stream an app sends
//! (cli/src/untar.rs), compiled in from its source, since the CLI is a binary.
//! Whatever the stream, nothing is made outside the folder it unpacks into,
//! nothing in it is a device or a FIFO, and every link in it, followed, leads
//! inside it or nowhere.

#![no_main]

#[path = "../../cli/src/untar.rs"]
mod untar;

use std::fs;
use std::path::Path;

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let outer = tempfile::tempdir().unwrap();
    let root = outer.path().join("root");
    fs::create_dir(&root).unwrap();
    // Refused or not, the properties hold for whatever it wrote.
    let unpacked = untar::unpack(data, &root);
    if let Err(error) = &unpacked {
        assert!(error.code().starts_with("pull_"), "{error}");
    }
    let beside: Vec<_> = fs::read_dir(outer.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(beside, ["root"], "made outside the folder: {unpacked:?}");
    let canonical = fs::canonicalize(&root).unwrap();
    check(&root, &canonical);
});

/// Walks `folder` without following links: each entry is a file, a folder or
/// a link that leads inside `root`, or to nothing.
fn check(folder: &Path, root: &Path) {
    for entry in fs::read_dir(folder).unwrap() {
        let entry = entry.unwrap();
        let kind = entry.file_type().unwrap();
        if kind.is_symlink() {
            if let Ok(target) = fs::canonicalize(entry.path()) {
                assert!(
                    target.starts_with(root),
                    "{} leads to {}",
                    entry.path().display(),
                    target.display()
                );
            }
        } else if kind.is_dir() {
            check(&entry.path(), root);
        } else {
            assert!(
                kind.is_file(),
                "{} is a special file",
                entry.path().display()
            );
        }
    }
}
