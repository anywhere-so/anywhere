//! `AppSpec::check`, which the control plane runs on every spec it's given,
//! since the CLI that parses `app.toml` is outside our
//! trust. Built as a structure, so every field reaches the check. Whatever the
//! spec, the check and the parser agree: it passes exactly when the same spec,
//! written as `app.toml`, parses.

#![no_main]

use appspec::{AppSpec, Processes};
use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

/// Paths a static site may name, valid and not.
const PATHS: &[&str] = &[
    "package.json",
    "package-lock.json",
    "src",
    "src/a",
    "data",
    "data/x",
    "/abs",
    "a//b",
    "..",
    "",
    "public",
    ".git",
    "site/.env",
    ".env.local",
];

#[derive(Arbitrary, Debug)]
enum Folder {
    Known(u8),
    Any(String),
}

impl Folder {
    fn path(self) -> String {
        match self {
            Folder::Known(i) => PATHS[usize::from(i) % PATHS.len()].to_owned(),
            Folder::Any(path) => path,
        }
    }
}

#[derive(Arbitrary, Debug)]
struct Spec {
    size: u8,
    disk_gib: u64,
    title: String,
    web: Option<String>,
    worker: Option<String>,
    port: Option<u16>,
    build: Option<String>,
    static_folder: Option<Folder>,
}

fuzz_target!(|spec: Spec| {
    let spec = AppSpec {
        title: spec.title,
        resources: appspec::Resources {
            size: [
                appspec::Size::Small,
                appspec::Size::Medium,
                appspec::Size::Large,
                appspec::Size::Xl,
            ][usize::from(spec.size % 4)],
            disk_gib: spec.disk_gib,
        },
        processes: Processes {
            web: spec.web,
            worker: spec.worker,
        },
        port: spec.port,
        build: spec.build,
        static_folder: spec.static_folder.map(Folder::path),
    };
    let checked = spec.check();
    if let Err(error) = &checked {
        assert!(!error.code().is_empty());
        assert!(!error.to_string().is_empty());
    }
    // TOML's integers are signed 64-bit, so no `app.toml` holds this disk, and
    // the check refuses it.
    if i64::try_from(spec.resources.disk_gib).is_err() {
        assert!(
            matches!(checked, Err(appspec::SpecError::InvalidDisk)),
            "{spec:?}: {checked:?}"
        );
        return;
    }
    let written = toml::to_string(&spec).unwrap();
    // Within the file's limit, which the check doesn't know of, the two agree.
    if written.len() <= appspec::MAX_FILE_LEN {
        let parsed = appspec::parse(&written);
        assert_eq!(
            parsed.is_ok(),
            checked.is_ok(),
            "{spec:?}: {parsed:?} {checked:?}"
        );
        if let Ok(parsed) = parsed {
            assert_eq!(parsed, spec);
        }
    }
});
