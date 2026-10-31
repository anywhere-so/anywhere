use std::path::PathBuf;

use futures_util::{SinkExt, StreamExt};
use prost::Message as _;
use proto::platform::exec::v1::{
    ClientMessage, Exited, ServerMessage, Started, Stderr, Stdout, client_message, server_message,
};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

use super::*;

/// One entry of a tar stream, written into its header as raw bytes, so names
/// tar's own builder refuses can be sent: its kind, name, link target and
/// contents.
struct Raw<'a>(tar::EntryType, &'a str, &'a str, &'a [u8]);

fn tar_stream(entries: &[Raw<'_>]) -> Vec<u8> {
    let mut out = Vec::new();
    for Raw(kind, name, link, data) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(*kind);
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        let old = header.as_old_mut();
        old.name[..name.len()].copy_from_slice(name.as_bytes());
        old.linkname[..link.len()].copy_from_slice(link.as_bytes());
        header.set_cksum();
        out.extend_from_slice(header.as_bytes());
        out.extend_from_slice(data);
        out.resize(out.len().next_multiple_of(512), 0);
    }
    out.resize(out.len() + 10240, 0);
    out
}

/// A stand-in exec server that answers every command with `stdout`, then
/// `stderr`, then exits with `code`, or with a `code` of [`HANG_UP`] drops the
/// connection instead, and notes each command it was given.
async fn fake_exec(
    stdout: Vec<u8>,
    stderr: &'static [u8],
    code: i32,
) -> (std::net::SocketAddr, Arc<Mutex<Vec<Vec<String>>>>) {
    fake_exec_pausing(stdout, stderr, code, Duration::ZERO).await
}

/// [`fake_exec`], waiting `pause` after the output before the command exits.
async fn fake_exec_pausing(
    stdout: Vec<u8>,
    stderr: &'static [u8],
    code: i32,
    pause: Duration,
) -> (std::net::SocketAddr, Arc<Mutex<Vec<Vec<String>>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let commands = Arc::new(Mutex::new(Vec::new()));
    let noted = Arc::clone(&commands);
    tokio::spawn(async move {
        loop {
            let (socket, _) = listener.accept().await.unwrap();
            let stdout = stdout.clone();
            let noted = Arc::clone(&noted);
            tokio::spawn(async move {
                let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
                let Some(Ok(Message::Binary(start))) = ws.next().await else {
                    panic!("no Start");
                };
                let Some(client_message::Kind::Start(start)) =
                    ClientMessage::decode(start).unwrap().kind
                else {
                    panic!("not a Start");
                };
                let mut command = vec![start.command];
                command.extend(start.args);
                noted.lock().unwrap().push(command);
                let mut messages = vec![server_message::Kind::Started(Started {})];
                for chunk in stdout.chunks(60 * 1024) {
                    messages.push(server_message::Kind::Stdout(Stdout {
                        data: chunk.to_vec(),
                    }));
                }
                if !stderr.is_empty() {
                    messages.push(server_message::Kind::Stderr(Stderr {
                        data: stderr.to_vec(),
                    }));
                }
                if code != HANG_UP {
                    messages.push(server_message::Kind::Exited(Exited { exit_code: code }));
                }
                for kind in messages {
                    if matches!(kind, server_message::Kind::Exited(_)) {
                        tokio::time::sleep(pause).await;
                    }
                    let message = ServerMessage { kind: Some(kind) };
                    // A CLI that refused the stream has hung up.
                    if let Err(error) = ws
                        .send(Message::Binary(message.encode_to_vec().into()))
                        .await
                    {
                        eprintln!("the CLI hung up: {error}");
                        return;
                    }
                }
                if code == HANG_UP {
                    return;
                }
                while let Some(Ok(_)) = ws.next().await {}
            });
        }
    });
    (address, commands)
}

/// The exit code that makes [`fake_exec`] hang up before the command exits.
const HANG_UP: i32 = -1;

/// Runs `anywhere` with `args` in `cwd` against the API at `api`.
async fn pull_in(cwd: &Path, api: &str, args: &[&str]) -> Output {
    let mut command = command();
    command
        .args(args)
        .current_dir(cwd)
        .env("ANYWHERE_API", api)
        .env("ANYWHERE_TOKEN", SECRET);
    tokio::time::timeout(Duration::from_secs(20), command.output())
        .await
        .unwrap()
        .unwrap()
}

/// Whether `name` is an AppleDouble file, which macOS's tar adds for each
/// entry with extended attributes, as macOS gives every new file. GNU tar, in
/// the app's base image, adds none.
fn apple_double(name: &std::ffi::OsStr) -> bool {
    cfg!(target_os = "macos") && name.as_encoded_bytes().starts_with(b"._")
}

/// What's in `folder`, by path: each file's contents, `-> target` for each
/// link, and `/` for each folder.
fn tree(folder: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    let mut found = std::collections::BTreeMap::new();
    let mut folders = vec![PathBuf::new()];
    while let Some(sub) = folders.pop() {
        for entry in std::fs::read_dir(folder.join(&sub)).unwrap() {
            let entry = entry.unwrap();
            if apple_double(&entry.file_name()) {
                continue;
            }
            let path = sub.join(entry.file_name());
            let shown = path.to_str().unwrap().to_owned();
            let kind = entry.file_type().unwrap();
            if kind.is_symlink() {
                let target = std::fs::read_link(entry.path()).unwrap();
                found.insert(shown, format!("-> {}", target.display()).into_bytes());
            } else if kind.is_dir() {
                found.insert(shown, b"/".to_vec());
                folders.push(path);
            } else {
                found.insert(shown, std::fs::read(entry.path()).unwrap());
            }
        }
    }
    found
}

#[tokio::test]
async fn pulls_an_app_you_edit_into_a_folder_named_for_it_and_says_how_to_deploy_it_back() {
    let stream = tar_stream(&[Raw(
        tar::EntryType::Regular,
        "./app.toml",
        "",
        TOML.as_bytes(),
    )]);
    let (exec, _) = fake_exec(stream, b"", 0).await;
    let (api, api_task) =
        exec_api_for("/v1/apps/wiki/tickets?owner=alice", "alice-wiki", exec).await;
    let here = tempfile::tempdir().unwrap();

    // Into ./wiki, without the owner, and a plain deploy of it would make an
    // app of your own, so the next step names the app.
    let output = pull_in(here.path(), &api, &["pull", "alice/wiki", "/app"]).await;
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), "wiki\n");
    let stderr = text(&output.stderr);
    assert!(
        stderr.contains("progress: Copying /app from alice/wiki into wiki\n"),
        "{stderr}"
    );
    assert!(
        stderr.ends_with(
            "\nnext: anywhere deploy wiki --app alice/wiki\nnote: in wiki, give `anywhere logs` and `anywhere secrets` --app alice/wiki too; this copy has no saved project binding until you deploy or explicitly bind it\n"
        ),
        "{stderr}"
    );
    assert_eq!(
        std::fs::read_to_string(here.path().join("wiki/app.toml")).unwrap(),
        TOML
    );

    let output = pull_in(
        here.path(),
        &api,
        &["--json", "pull", "alice/wiki", "/app", "copy"],
    )
    .await;
    let result = success(&output);
    assert_eq!(result["app"], "alice/wiki");
    assert_eq!(result["dir"], "copy");
    let last: Value = serde_json::from_str(text(&output.stderr).lines().last().unwrap()).unwrap();
    assert_eq!(
        last,
        json!({"event": "next", "command": "anywhere deploy copy --app alice/wiki",
            "message": "anywhere deploy copy --app alice/wiki; in copy, give `anywhere logs` and `anywhere secrets` --app alice/wiki too; this copy has no saved project binding until you deploy or explicitly bind it",
            "operation": null})
    );

    // A folder the shell would split is quoted, so the line pastes as it is.
    let output = pull_in(
        here.path(),
        &api,
        &["pull", "alice/wiki", "/app", "my wiki"],
    )
    .await;
    assert!(output.status.success(), "{}", text(&output.stderr));
    let stderr = text(&output.stderr);
    assert!(
        stderr.contains("\nnext: anywhere deploy 'my wiki' --app alice/wiki\nnote: in 'my wiki', "),
        "{stderr}"
    );
    api_task.abort();
}

#[tokio::test]
async fn runs_tar_on_the_path_and_refuses_what_could_leave_the_folder() {
    let file = |name| Raw(tar::EntryType::Regular, name, "", b"x");
    let link = |name, target| Raw(tar::EntryType::Symlink, name, target, b"");
    let cases: Vec<(&str, Vec<Raw<'_>>)> = vec![
        ("pull_path_unsafe", vec![file("./ok"), file("/etc/passwd")]),
        ("pull_path_unsafe", vec![file("./ok"), file("../outside")]),
        ("pull_path_unsafe", vec![file("./a/../../outside")]),
        (
            "pull_link_unsafe",
            vec![link("./.venv/bin/python3", "/usr/bin/python3")],
        ),
        ("pull_link_unsafe", vec![link("./up", "../..")]),
        (
            "pull_path_unsafe",
            vec![link("./l", "."), file("./l/through")],
        ),
        (
            "pull_entry_unsupported",
            vec![Raw(tar::EntryType::Fifo, "./fifo", "", b"")],
        ),
        (
            "pull_entry_unsupported",
            vec![Raw(tar::EntryType::Char, "./null", "", b"")],
        ),
    ];
    for (code, entries) in cases {
        let names: Vec<&str> = entries.iter().map(|raw| raw.1).collect();
        let (exec, commands) = fake_exec(tar_stream(&entries), b"", 0).await;
        let (api, api_task) = exec_api("spend", exec).await;
        let here = tempfile::tempdir().unwrap();
        // An empty folder that's there already stays as it was, and one that
        // wasn't isn't left behind.
        std::fs::create_dir(here.path().join("there")).unwrap();
        for dir in ["there", "new"] {
            let output =
                pull_in(here.path(), &api, &["--json", "pull", "spend", "/app", dir]).await;
            let error = failure(&output, code);
            assert_eq!(error["retryable"], false, "{names:?}");
            assert!(
                error["message"]
                    .as_str()
                    .unwrap()
                    .ends_with(&format!("; nothing was written to {dir}")),
                "{error}"
            );
        }
        assert_eq!(
            tree(here.path()),
            [("there".to_owned(), b"/".to_vec())].into(),
            "{names:?}"
        );
        assert_eq!(
            commands.lock().unwrap()[0],
            ["tar", "-C", "/app", "-cf", "-", "."]
        );
        api_task.abort();
    }
}

#[tokio::test]
async fn a_stream_cut_short_is_refused_and_tars_words_are_kept() {
    let whole = tar_stream(&[Raw(tar::EntryType::Regular, "./big", "", &[7; 3000])]);
    let (exec, _) = fake_exec(whole[..2000].to_vec(), b"", 0).await;
    let (api, api_task) = exec_api("spend", exec).await;
    let here = tempfile::tempdir().unwrap();
    let output = pull_in(here.path(), &api, &["--json", "pull", "spend", "/app"]).await;
    // A pull only reads, so repeating it is safe.
    assert_eq!(failure(&output, "pull_stream_invalid")["retryable"], true);
    assert!(tree(here.path()).is_empty());
    api_task.abort();

    // A connection that drops partway leaves nothing behind either, once the
    // unpacking has stopped writing.
    let (exec, _) = fake_exec(whole[..2000].to_vec(), b"", HANG_UP).await;
    let (api, api_task) = exec_api("spend", exec).await;
    let output = pull_in(here.path(), &api, &["--json", "pull", "spend", "/app"]).await;
    assert_eq!(failure(&output, "pull_interrupted")["retryable"], true);
    assert!(tree(here.path()).is_empty());
    api_task.abort();

    // tar exits 1 when a running app's file changes as it reads it.
    let (exec, _) = fake_exec(
        whole.clone(),
        b"tar: ./big: file changed as we read it\n",
        1,
    )
    .await;
    let (api, api_task) = exec_api("spend", exec).await;
    let output = pull_in(here.path(), &api, &["--json", "pull", "spend", "/app"]).await;
    let error = failure(&output, "pull_tar_failed");
    assert_eq!(error["retryable"], true);
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("as a running app's files do, so pull again"),
        "{error}"
    );
    assert!(tree(here.path()).is_empty());
    api_task.abort();

    // A tar that fails explains the stream it cut short, in its own words,
    // escaped.
    let (exec, _) = fake_exec(
        whole[..2000].to_vec(),
        b"tar: ./big: file changed as we read it\x1b[2J\n",
        2,
    )
    .await;
    let (api, api_task) = exec_api("spend", exec).await;
    let output = pull_in(here.path(), &api, &["pull", "spend", "/app"]).await;
    assert_eq!(output.status.code(), Some(1));
    let stderr = text(&output.stderr);
    assert!(
        stderr.contains(
            "\nerror: pull_tar_failed: `tar` in the app exited with 2, so nothing was written to spend; it said: tar: ./big: file changed as we read it\\u{1b}[2J; check the path exists"
        ),
        "{stderr}"
    );
    assert!(tree(here.path()).is_empty());
    api_task.abort();
}

#[tokio::test]
async fn refuses_a_destination_that_is_not_empty_before_asking_the_app() {
    let server = Server::start(vec![reply(json!({}))]).await;
    let here = tempfile::tempdir().unwrap();
    std::fs::create_dir(here.path().join("spend")).unwrap();
    std::fs::write(here.path().join("spend/keep.txt"), b"mine").unwrap();
    std::fs::write(here.path().join("file"), b"mine").unwrap();
    for dir in ["spend", "file"] {
        let mut args = vec!["--json", "pull", "spend", "/app"];
        if dir != "spend" {
            args.push(dir);
        }
        let output = pull_in(here.path(), &server.url, &args).await;
        failure(&output, "destination_exists");
    }
    assert_eq!(
        std::fs::read(here.path().join("spend/keep.txt")).unwrap(),
        b"mine"
    );

    // What an interrupted pull left says how to clear it.
    std::fs::create_dir_all(here.path().join("left/.anywhere-pull-0190")).unwrap();
    let output = pull_in(
        here.path(),
        &server.url,
        &["--json", "pull", "spend", "/app", "left"],
    )
    .await;
    let error = failure(&output, "destination_exists");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("an interrupted pull left, a folder named .anywhere-pull-…; remove it with rm -rf left/.anywhere-pull-*"),
        "{error}"
    );
    assert!(server.paths().is_empty());
}

#[tokio::test]
async fn needs_a_path_and_has_no_versions() {
    let server = Server::start(vec![reply(json!({}))]).await;
    for args in [
        &["--json", "pull", "spend"][..],
        &["--json", "pull", "spend", "--version", "2", "/app"],
    ] {
        let output = server.run(args).await;
        assert_eq!(output.status.code(), Some(2));
        failure(&output, "argument_invalid");
    }
    for removed in [&["--json", "rollback"][..], &["--json", "versions"]] {
        let output = server.run(removed).await;
        assert_eq!(output.status.code(), Some(2), "{removed:?}");
        failure(&output, "argument_invalid");
    }
    assert!(server.paths().is_empty());
}

#[tokio::test]
async fn a_failed_pull_never_removes_what_something_else_put_in_the_destination() {
    let stream = tar_stream(&[Raw(tar::EntryType::Regular, "./app.toml", "", b"x")]);
    let (exec, _) = fake_exec_pausing(stream, b"", 0, Duration::from_secs(2)).await;
    let (api, api_task) = exec_api("spend", exec).await;
    let here = tempfile::tempdir().unwrap();
    let dest = here.path().join("new");
    let pulling = {
        let cwd = here.path().to_owned();
        let api = api.clone();
        tokio::spawn(async move {
            pull_in(&cwd, &api, &["--json", "pull", "spend", "/app", "new"]).await
        })
    };
    // Once the pull has made its staging folder, something else writes beside it.
    let staged = || {
        std::fs::read_dir(&dest).is_ok_and(|mut entries| {
            entries.any(|entry| {
                entry
                    .unwrap()
                    .file_name()
                    .as_encoded_bytes()
                    .starts_with(b".anywhere-pull-")
            })
        })
    };
    while !staged() {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    std::fs::write(dest.join("theirs.txt"), b"not the pull's").unwrap();
    let output = pulling.await.unwrap();
    failure(&output, "destination_exists");
    assert_eq!(
        tree(here.path()),
        [
            ("new".to_owned(), b"/".to_vec()),
            ("new/theirs.txt".to_owned(), b"not the pull's".to_vec()),
        ]
        .into()
    );
    api_task.abort();
}
