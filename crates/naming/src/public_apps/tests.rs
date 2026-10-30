use super::*;

fn app() -> Uuid {
    Uuid::from_u128(0x123456789abcdef0123456789abcdef0)
}
const BASE: &str = "sample-project.uc.r.appspot.com";

#[test]
fn apps_are_sites_of_their_own_only_under_a_public_suffix_google_or_localhost() {
    for host in ["localhost", "up.railway.app", BASE] {
        assert!(
            Namespace::parse(&format!("https://{host}"))
                .unwrap()
                .isolated_sites(),
            "{host}"
        );
    }
    for host in [
        "anywhereapps.localhost",
        "apps.example.com",
        "apps.anywhere.so",
        "anywhere.so",
    ] {
        assert!(
            !Namespace::parse(&format!("https://{host}"))
                .unwrap()
                .isolated_sites(),
            "{host}"
        );
    }
}

#[test]
fn platform_hosts_must_not_live_inside_an_apps_site() {
    let namespace = Namespace::parse("https://localhost:8443").unwrap();
    for host in ["alice-notes.localhost", "api.alice-notes.localhost"] {
        assert!(namespace.contains_app_site(host));
    }
    for host in [
        "anywhere.localhost",
        "api.anywhere.localhost",
        "anywhere.so",
    ] {
        assert!(!namespace.contains_app_site(host));
    }
    let namespace = Namespace::parse(&format!("https://{BASE}")).unwrap();
    let host = namespace.app_origin(app(), "alice-notes").unwrap();
    assert!(namespace.contains_app_site(&host.as_str()[8..]));
    assert!(namespace.contains_app_site(&format!("api.{}", &host.as_str()[8..])));
    assert!(!namespace.contains_app_site(BASE));
}

#[test]
fn off_the_list_apps_share_one_site_and_the_platform_must_be_outside_it() {
    // .test names, which the list can never hold, with and without a port.
    for origin in ["https://anywhereapp.test", "https://anywhereapp.test:8443"] {
        let namespace = Namespace::parse(origin).unwrap();
        assert!(!namespace.isolated_sites(), "{origin}");
        for host in [
            "anywhereapp.test",
            "alice-notes.anywhereapp.test",
            "api.anywhereapp.test",
        ] {
            assert!(namespace.contains_app_site(host), "{origin} {host}");
        }
        for host in [
            "anywhere.test",
            "api.anywhere.test",
            "myanywhereapp.test",
            "anywhereapp.test.evil.test",
        ] {
            assert!(!namespace.contains_app_site(host), "{origin} {host}");
        }
    }
    // An apps namespace under the platform's own domain shares its site.
    let namespace = Namespace::parse("https://apps.anywhere.test").unwrap();
    for host in ["anywhere.test", "www.anywhere.test", "api.anywhere.test"] {
        assert!(namespace.contains_app_site(host), "{host}");
    }
    // Under a listed suffix, the namespace's registrable domain is the site.
    let namespace = Namespace::parse("https://apps.github.io").unwrap();
    assert!(namespace.contains_app_site("alice-notes.apps.github.io"));
    for host in ["github.io", "anywhere.github.io"] {
        assert!(!namespace.contains_app_site(host), "{host}");
    }
}

#[test]
fn google_aliases_round_trip_without_display_name_length_changes() {
    let namespace = Namespace::parse(&format!("https://{BASE}")).unwrap();
    assert!(namespace.is_google());
    assert_eq!(namespace.host(), BASE);
    assert_eq!(namespace.authority(), BASE);
    let address = format!("a-{}", "b".repeat(61));
    assert_eq!(namespace.address_origin(&address), Ok(None));
    let origin = namespace.app_origin(app(), &address).unwrap();
    assert_eq!(
        origin.as_str(),
        format!("https://app-{}-dot-{BASE}", app().simple())
    );
    assert_eq!(
        namespace.parse_authority(&origin.as_str()[8..]),
        Ok(Target::GoogleApp(app()))
    );
    assert!(
        namespace
            .parse_authority(&format!("files-dot-{BASE}"))
            .is_err()
    );
}

#[test]
fn ordinary_namespace_retains_exact_label_and_port_contract() {
    for port in ["", ":8443"] {
        let namespace = Namespace::parse(&format!("https://apps.test{port}")).unwrap();
        assert!(!namespace.is_google());
        assert_eq!(
            namespace
                .address_origin("alice-a--b")
                .unwrap()
                .unwrap()
                .as_str(),
            format!("https://alice-a--b.apps.test{port}")
        );
        assert_eq!(
            namespace.app_origin(app(), "alice-a--b").unwrap().as_str(),
            format!("https://alice-a--b.apps.test{port}")
        );
        assert_eq!(
            namespace.parse_authority(&format!("alice-a--b.apps.test{port}")),
            Ok(Target::Address("alice-a--b"))
        );
        for host in [
            "apps.test",
            "files.apps.test",
            "foo.apps.test",
            "xn--abc.apps.test",
            "alice--a.apps.test",
            "a.b.apps.test",
            "files-dot-apps.test",
        ] {
            assert!(namespace.parse_authority(host).is_err(), "{host}");
        }
    }
}

#[test]
fn malformed_google_bases_cannot_select_wildcard_semantics() {
    for base in [
        "appspot.com",
        "sample-project.appspot.com",
        "uc.r.appspot.com",
        "a.sample-project.uc.r.appspot.com",
        "sample-project.uc.x.appspot.com",
        "abc.uc.r.appspot.com",
        "123456.uc.r.appspot.com",
        "sample-dot-project.uc.r.appspot.com",
        "sample-project.u-dot-c.r.appspot.com",
    ] {
        assert!(
            Namespace::parse(&format!("https://{base}")).is_err(),
            "{base}"
        );
    }
    for suffix in [":443", ":8443", "/", "/x", "?x", "#x", "."] {
        assert!(Namespace::parse(&format!("https://{BASE}{suffix}")).is_err());
    }
    for text in [
        format!("http://{BASE}"),
        format!("https://{}", BASE.to_uppercase()),
        format!("https://user@{BASE}"),
    ] {
        assert!(Namespace::parse(&text).is_err());
    }
    assert!(Namespace::parse(&format!("https://{}.uc.r.appspot.com", "a".repeat(22))).is_ok());
    assert!(Namespace::parse(&format!("https://{}.uc.r.appspot.com", "a".repeat(23))).is_err());
}

#[test]
fn generated_authorities_have_one_canonical_unambiguous_spelling() {
    let namespace = Namespace::parse(&format!("https://{BASE}")).unwrap();
    let good = format!("app-{}-dot-{BASE}", app().simple());
    for host in [
        BASE.to_owned(),
        format!("files.{BASE}"),
        format!("alice-notes.{BASE}"),
        format!("rel-1-dot-{BASE}"),
        format!("default-dot-{BASE}"),
        format!("app-{}-dot-{BASE}", app()),
        format!(
            "app-{}-dot-{BASE}",
            app().simple().to_string().to_uppercase()
        ),
        format!("app-{}-dot-{BASE}", Uuid::nil().simple()),
        format!("{good}:443"),
        format!("{good}:8443"),
        format!("{good}."),
        format!("{good}.attacker.test"),
        format!("x.{good}"),
        format!("x-dot-{good}"),
        format!("files-dot-default-dot-{BASE}"),
        format!("{good}@attacker.test"),
        format!("{good}/path"),
        format!("{good}?x"),
    ] {
        assert!(namespace.parse_authority(&host).is_err(), "{host}");
    }
}

#[test]
fn issuance_still_requires_valid_original_address_and_nonempty_app_identity() {
    for base in [
        "https://apps.test",
        "https://sample-project.uc.r.appspot.com",
    ] {
        let namespace = Namespace::parse(base).unwrap();
        assert!(namespace.app_origin(Uuid::nil(), "alice-notes").is_err());
        for address in [
            "",
            "files",
            "Alice-notes",
            "alice--notes",
            "alice-notes-",
            "a.b-notes",
            "alice-notes:443",
            "alice-notes/",
            &"a".repeat(64),
        ] {
            assert!(namespace.app_origin(app(), address).is_err(), "{address}");
            assert!(namespace.address_origin(address).is_err(), "{address}");
        }
    }
}
