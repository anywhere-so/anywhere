//! Hostnames: `naming::AppAddress::parse`, which reads one label of every
//! request's hostname in the gateway, and the names it's made of. A label that
//! parses is exactly the address it names, written back, and fits in one DNS
//! label; nothing else does.

#![no_main]

use libfuzzer_sys::fuzz_target;
use naming::{AppAddress, AppName, MAX_ADDRESS_LEN, SecretName, Username};

fuzz_target!(|label: &str| {
    match AppAddress::parse(label) {
        Ok(address) => {
            assert_eq!(address.to_string(), label);
            assert!(label.len() <= MAX_ADDRESS_LEN);
            assert!(
                label
                    .bytes()
                    .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'-'))
            );
            let (user, app) = label.split_once('-').unwrap();
            assert_eq!(address.user().as_str(), user);
            assert_eq!(address.app().as_str(), app);
            let made = AppAddress::new(address.user().clone(), address.app().clone()).unwrap();
            assert_eq!(made, address);
        }
        Err(error) => {
            assert!(!error.code().is_empty());
            // Every message is for an agent to read, so it must render.
            assert!(!error.to_string().is_empty());
        }
    }
    if let Ok(user) = Username::parse(label) {
        assert_eq!(user.as_str(), label);
        assert!(!label.contains('-'));
        if let Err(error) = user.ensure_not_blocked() {
            assert_eq!(error.code(), "username_reserved");
        }
    }
    if let Ok(app) = AppName::parse(label) {
        assert_eq!(app.as_str(), label);
        assert!(!label.starts_with('-') && !label.ends_with('-'));
    }
    if let Ok(secret) = SecretName::parse(label) {
        assert_eq!(secret.as_str(), label);
        assert!(label.len() <= naming::MAX_SECRET_NAME_LEN);
        assert!(!label.starts_with(naming::RESERVED_SECRET_PREFIX));
    }
});
