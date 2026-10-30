use super::*;

fn client() -> UploadClient {
    let mut secret = HeaderValue::from_static("Bearer test-only-native-owner-credential");
    secret.set_sensitive(true);
    UploadClient::new(
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap(),
        "https://private.example.test:8443",
        secret,
    )
    .unwrap()
}

fn upload(path: Option<String>) -> Upload {
    Upload {
        url: "https://attacker.example.test/collect?credential=untrusted".to_owned(),
        path,
        missing: Vec::new(),
    }
}

#[test]
fn upload_uses_only_the_configured_origin_and_exact_operation_path() {
    let operation = Uuid::from_u128(1);
    let client = client();
    let path = format!("/v1/uploads/{operation}");
    let target = client
        .target(operation, &upload(Some(path.clone())))
        .unwrap();
    let request = client.request(&target).build().unwrap();
    assert_eq!(
        request.url().as_str(),
        format!("https://private.example.test:8443{path}")
    );
    assert_eq!(
        request.headers()[AUTHORIZATION],
        "Bearer test-only-native-owner-credential"
    );
    assert!(request.headers()[AUTHORIZATION].is_sensitive());
    for path in [
        None,
        Some(format!("{path}?x=secret")),
        Some(format!("{path}/")),
        Some(format!("https://attacker.test{path}")),
        Some(format!("//attacker.test{path}")),
        Some(format!("/v1/uploads/{}", Uuid::from_u128(2))),
        Some("/v1/uploads/%2e%2e/collect".to_owned()),
    ] {
        let error = match client.target(operation, &upload(path)) {
            Ok(_) => panic!("untrusted target accepted"),
            Err(error) => error,
        };
        assert_eq!(error.code(), "api_answer_invalid");
        assert!(!error.to_string().contains("credential=untrusted"));
    }
}

#[test]
fn api_origin_refuses_plaintext_credentials_aliases_and_url_components() {
    for origin in [
        "http://private.test",
        "ftp://private.test",
        "https://user:secret@private.test",
        "https://private.test/path",
        "https://private.test?x=1",
        "https://private.test#fragment",
        "https://PRIVATE.test",
        "https://private.test:443",
        "https://private.test:08443",
        "https://private.test//",
    ] {
        assert!(api_origin(origin).is_err(), "{origin} accepted");
    }
    assert!(api_origin("https://private.test").is_ok());
    assert!(api_origin("https://private.test:8443/").is_ok());
}

#[test]
fn plain_http_is_allowed_only_for_a_loopback_host() {
    for origin in [
        "http://127.0.0.1",
        "http://127.0.0.1:8080",
        "http://[::1]:8080/",
        "http://localhost:3000",
    ] {
        assert!(api_origin(origin).is_ok(), "{origin} refused");
    }
    for origin in [
        // Other hosts, including ones that only look or resolve local.
        "http://private.test",
        "http://10.0.0.1:8080",
        "http://0.0.0.0:8080",
        "http://127.0.0.2:8080",
        "http://localhost.private.test",
        "http://anywhere.localhost",
        // Loopback, but not canonical.
        "http://127.1:8080",
        "http://LOCALHOST:8080",
        "http://[0:0:0:0:0:0:0:1]:8080",
        "http://127.0.0.1:80",
        "http://127.0.0.1:8080/v1",
        "http://user:secret@127.0.0.1:8080",
        "http://127.0.0.1:8080?x=1",
    ] {
        assert!(api_origin(origin).is_err(), "{origin} accepted");
    }
}

#[tokio::test]
async fn clients_never_follow_same_or_other_origin_redirects() {
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    let settings = super::super::Settings {
        api: "https://private.example.test".to_owned(),
        token: "test-only-owner-token".to_owned(),
        ca: None,
    };
    let clients = settings.clients().unwrap();
    // The real builders' redirect policies are tested against a loopback HTTP
    // responder; target validation is covered above.
    let exec = settings.exec_client().unwrap();
    for client in [&clients.api, &clients.uploads.client, &exec] {
        for same_origin in [true, false] {
            let source = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let destination = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = source.local_addr().unwrap();
            let location = if same_origin {
                format!("http://{address}/redirected")
            } else {
                format!("http://{}/redirected", destination.local_addr().unwrap())
            };
            let server = tokio::spawn(async move {
                let (mut socket, _) = tokio::time::timeout(Duration::from_secs(2), source.accept())
                    .await
                    .unwrap()
                    .unwrap();
                let mut head = Vec::new();
                while !head.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    tokio::time::timeout(Duration::from_secs(2), socket.read_exact(&mut byte))
                        .await
                        .unwrap()
                        .unwrap();
                    head.extend_from_slice(&byte);
                    assert!(head.len() < 8192);
                }
                let reply = format!(
                    "HTTP/1.1 307 Temporary Redirect\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
                socket.write_all(reply.as_bytes()).await.unwrap();
            });
            let response = tokio::time::timeout(
                Duration::from_secs(2),
                client.get(format!("http://{address}/original")).send(),
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(response.status(), reqwest::StatusCode::TEMPORARY_REDIRECT);
            server.await.unwrap();
            assert!(
                tokio::time::timeout(Duration::from_millis(50), destination.accept())
                    .await
                    .is_err()
            );
        }
    }
}
