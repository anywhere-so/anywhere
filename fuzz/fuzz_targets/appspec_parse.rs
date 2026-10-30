//! `app.toml`: `appspec::parse`, which the CLI and the build sandbox run on an
//! app's file. A file that parses passes `AppSpec::check`, which the control
//! plane runs again, and reads back the same after being written out as TOML or
//! sent as JSON, as the spec travels.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|text: &str| {
    let spec = match appspec::parse(text) {
        Ok(spec) => spec,
        Err(error) => {
            assert!(!error.code().is_empty());
            assert!(!error.to_string().is_empty());
            return;
        }
    };
    assert!(text.len() <= appspec::MAX_FILE_LEN);
    spec.check().unwrap();
    // A static site has no command, and every other app has one.
    spec.kind().unwrap();

    let written = toml::to_string(&spec).unwrap();
    assert_eq!(appspec::parse(&written).unwrap(), spec);

    let json = serde_json::to_vec(&spec).unwrap();
    let sent: appspec::AppSpec = serde_json::from_slice(&json).unwrap();
    assert_eq!(sent, spec);
});
