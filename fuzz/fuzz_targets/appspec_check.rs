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
    cpu_millicores: Option<u32>,
    memory_mib: Option<u64>,
    disk_mib: Option<u64>,
    title: String,
    icon: Option<Folder>,
    web: Option<String>,
    worker: Option<String>,
    port: Option<u16>,
    build: Option<String>,
    static_folder: Option<Folder>,
    image: Option<Image>,
    start_timeout: Option<u32>,
}

/// Images an app may name, valid and not.
const IMAGES: &[&str] = &[
    "nginx",
    "nginx:alpine",
    "ghcr.io/every-app/open-seo:latest",
    "localhost:5000/a",
    "Nginx",
    "",
];

#[derive(Arbitrary, Debug)]
enum Image {
    Known(u8),
    Any(String),
}

impl Image {
    fn reference(self) -> String {
        match self {
            Image::Known(i) => IMAGES[usize::from(i) % IMAGES.len()].to_owned(),
            Image::Any(reference) => reference,
        }
    }
}

fuzz_target!(|spec: Spec| {
    let spec = AppSpec {
        title: spec.title,
        icon: spec.icon.map(Folder::path),
        limits: appspec::LimitRequests {
            cpu_millicores: spec.cpu_millicores,
            memory_bytes: spec.memory_mib.map(|n| n.saturating_mul(1 << 20)),
            disk_bytes: spec.disk_mib.map(|n| n.saturating_mul(1 << 20)),
        },
        processes: Processes {
            web: spec.web,
            worker: spec.worker,
        },
        port: spec.port,
        build: spec.build,
        static_folder: spec.static_folder.map(Folder::path),
        image: spec.image.map(Image::reference),
        start_timeout: spec.start_timeout,
    };
    let checked = spec.check();
    if let Err(error) = &checked {
        assert!(!error.code().is_empty());
        assert!(!error.to_string().is_empty());
    }
    // TOML cannot serialize an integer beyond i64, and neither limit may
    // exceed it on the canonical wire. Keep that overflow coverage too.
    if [spec.limits.memory_bytes, spec.limits.disk_bytes]
        .into_iter()
        .flatten()
        .any(|n| n > i64::MAX as u64)
    {
        assert!(checked.is_err());
        return;
    }
    let written = anywhere_fuzz::app_toml(&spec);
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
