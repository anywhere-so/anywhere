//! Namespace aliases preserve immutable app IDs and never acquire another site.
#![no_main]
use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use naming::public_apps::{Namespace, Target};
use uuid::Uuid;
#[derive(Arbitrary, Debug)]
struct Input {
    namespace: String,
    authority: String,
    app: [u8; 16],
    address: String,
}
fn check(namespace: &Namespace, input: &Input) {
    assert_eq!(Namespace::parse(namespace.as_str()).unwrap(), *namespace);
    if let Ok(app) = namespace.app_origin(Uuid::from_bytes(input.app), &input.address) {
        let authority = app.as_str().strip_prefix("https://").unwrap();
        assert!(namespace.contains_app_site(authority.split(':').next().unwrap()));
        let expected = if namespace.is_google() {
            Target::GoogleApp(Uuid::from_bytes(input.app))
        } else {
            Target::Address(&input.address)
        };
        assert_eq!(namespace.parse_authority(authority).unwrap(), expected);
        assert!(
            namespace
                .parse_authority(&format!("{authority}.evil.test"))
                .is_err()
        );
        assert!(
            namespace
                .parse_authority(&format!("x.{authority}"))
                .is_err()
        );
    }
    if let Ok(target) = namespace.parse_authority(&input.authority) {
        let reconstructed = match target {
            Target::GoogleApp(id) => {
                assert!(!id.is_nil());
                namespace
                    .app_origin(id, "alice-notes")
                    .unwrap()
                    .as_str()
                    .to_owned()
            }
            Target::Address(label) => namespace
                .app_origin(Uuid::from_u128(1), label)
                .unwrap()
                .as_str()
                .to_owned(),
        };
        assert_eq!(reconstructed, format!("https://{}", input.authority));
    }
}
fuzz_target!(|input: Input| {
    if let Ok(namespace) = Namespace::parse(&input.namespace) {
        check(&namespace, &input);
    }
    for name in [
        "https://apps.example.test:8443",
        "https://sample-project.uc.r.appspot.com",
    ] {
        check(&Namespace::parse(name).unwrap(), &input);
    }
});
