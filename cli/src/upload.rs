//! Sends a deploy's files to its upload URL, as the upload contract in
//! `crates/manifest` says: in `PUT`s of at most 30 MiB of files, each body a
//! zstd-compressed tar stream of files named by their hashes. Each body is made
//! as it's sent: a blocking thread reads and checks each file, tars it and
//! compresses the stream, a chunk at a time.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::{Bytes, BytesMut};
use http_body::Frame;
use manifest::{MAX_BATCH_SIZE, Missing, UploadError};
use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::api::types::{ErrorBody, Upload};
use crate::app::App;
use crate::error::{CliError, chain};
use crate::output::Output;

/// How much of a body is sent at a time.
const CHUNK_LEN: usize = 64 * 1024;

/// How many chunks wait to be sent, so a slow network holds the files back
/// rather than filling memory.
const CHUNKS_IN_FLIGHT: usize = 16;

/// The longest answer to a `PUT` that's read. The sandbox's are tiny.
const MAX_ANSWER_LEN: usize = 64 * 1024;

/// One file to send: where it is, and what the deploy's list says it holds.
#[derive(Clone, Debug)]
struct Outgoing {
    path: String,
    hash: String,
    size: u64,
}

/// What a `PUT`'s answer says.
#[derive(Debug, PartialEq, Eq)]
enum Answer {
    /// This many files are still missing.
    Missing(u64),
    /// None are, so the sandbox has every file and goes on to write them.
    Complete,
}

/// Sends every file `upload` asks for, which is every file the deploy lists,
/// and returns once the sandbox has them all.
pub async fn send(
    http: &crate::settings::UploadClient,
    operation: uuid::Uuid,
    upload: &Upload,
    app: &App,
    deadline: Instant,
    output: Output,
) -> Result<(), CliError> {
    let target = http.target(operation, upload)?;
    let batches = batches(outgoing(upload, app)?);
    let count = batches.len();
    for (index, batch) in batches.into_iter().enumerate() {
        let bytes: u64 = batch.iter().map(|file| file.size).sum();
        output.progress(&format!(
            "Uploading batch {}/{}: {} files, {bytes} bytes",
            index + 1,
            count,
            batch.len()
        ));
        match put(http.request(&target), batch, &app.dir, deadline).await? {
            // A retried upload: the sandbox had the rest from before.
            Answer::Complete => return Ok(()),
            Answer::Missing(missing) if index + 1 == count => {
                return Err(CliError::UploadIncomplete { missing });
            }
            Answer::Missing(_) => {}
        }
    }
    Ok(())
}

/// The files to send, one for each hash the deploy asks for.
fn outgoing(upload: &Upload, app: &App) -> Result<Vec<Outgoing>, CliError> {
    let mut by_hash = HashMap::with_capacity(app.files.len());
    for file in &app.files {
        by_hash.entry(file.hash.as_str()).or_insert(file);
    }
    let mut seen = HashSet::new();
    let mut files = Vec::new();
    for hash in &upload.missing {
        if !seen.insert(hash.as_str()) {
            continue;
        }
        let Some(file) = by_hash.get(hash.as_str()) else {
            return Err(CliError::UploadUnknownHash { hash: hash.clone() });
        };
        files.push(Outgoing {
            path: file.path.clone(),
            hash: file.hash.clone(),
            size: file.size,
        });
    }
    Ok(files)
}

/// Groups the files, in order, into `PUT`s of at most [`MAX_BATCH_SIZE`] of
/// files each. `manifest::check` caps each file at that size, so each fits.
fn batches(files: Vec<Outgoing>) -> Vec<Vec<Outgoing>> {
    let mut batches = Vec::new();
    let mut batch: Vec<Outgoing> = Vec::new();
    let mut size = 0;
    for file in files {
        if !batch.is_empty() && size + file.size > MAX_BATCH_SIZE {
            batches.push(std::mem::take(&mut batch));
            size = 0;
        }
        size += file.size;
        batch.push(file);
    }
    if !batch.is_empty() {
        batches.push(batch);
    }
    batches
}

/// Sends one `PUT`, making its body as it goes, and reads its answer.
async fn put(
    request: reqwest::RequestBuilder,
    batch: Vec<Outgoing>,
    dir: &Path,
    deadline: Instant,
) -> Result<Answer, CliError> {
    let (chunks, body) = mpsc::channel(CHUNKS_IN_FLIGHT);
    let writing = {
        let dir = dir.to_owned();
        tokio::task::spawn_blocking(move || write_body(&batch, &dir, chunks))
    };
    let sent = request
        .timeout(deadline.saturating_duration_since(Instant::now()))
        .body(reqwest::Body::wrap(ChannelBody(body)))
        .send()
        .await;
    let written = writing.await.map_err(|error| CliError::Internal {
        detail: format!("the upload's body stopped: {error}"),
    })?;
    // A file that changed or couldn't be read is the cause of whatever the
    // request then did, so it's what's reported.
    if let Err(BodyError::File(error)) = written {
        return Err(error);
    }
    let response = sent.map_err(|error| CliError::UploadFailed {
        detail: chain(&error.without_url()),
    })?;
    read_answer(response).await
}

async fn read_answer(mut response: reqwest::Response) -> Result<Answer, CliError> {
    let status = response.status();
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| CliError::UploadFailed {
            detail: chain(&error.without_url()),
        })?
    {
        if body.len() + chunk.len() > MAX_ANSWER_LEN {
            return Err(CliError::UploadAnswer {
                status,
                detail: format!("it's over {MAX_ANSWER_LEN} bytes"),
            });
        }
        body.extend_from_slice(&chunk);
    }
    let unclear = |detail: String| CliError::UploadAnswer { status, detail };
    match status {
        reqwest::StatusCode::OK | reqwest::StatusCode::CREATED => {
            let answer: Missing = serde_json::from_slice(&body)
                .map_err(|error| unclear(format!("{error}, in {}", quote(&body))))?;
            match (status, answer.missing) {
                (reqwest::StatusCode::CREATED, 0) => Ok(Answer::Complete),
                (reqwest::StatusCode::CREATED, _) => Err(unclear(quote(&body))),
                (_, missing) => Ok(Answer::Missing(missing)),
            }
        }
        _ => match serde_json::from_slice::<ErrorBody>(&body) {
            Ok(error) if status.is_client_error() || status.is_server_error() => {
                Err(CliError::UploadRefused(UploadError {
                    code: error.code,
                    message: error.message,
                    retryable: error.retryable,
                }))
            }
            _ => Err(unclear(quote(&body))),
        },
    }
}

/// The start of an answer, quoted, for an error message.
fn quote(body: &[u8]) -> String {
    let text: String = String::from_utf8_lossy(body).chars().take(200).collect();
    format!("{text:?}")
}

/// Why a body stopped before its end.
enum BodyError {
    /// A file couldn't be read, or changed after it was hashed.
    File(CliError),
    /// The request stopped taking the body, for a reason the request reports.
    Stopped,
}

/// Writes the body for `batch` into `chunks`. If it stops early, the body ends
/// with an error, so the request is cut off rather than looking complete.
fn write_body(
    batch: &[Outgoing],
    dir: &Path,
    chunks: mpsc::Sender<io::Result<Bytes>>,
) -> Result<(), BodyError> {
    let written = write_archive(batch, dir, ChannelWriter::new(chunks.clone()));
    if written.is_err()
        && let Err(unsent) =
            chunks.blocking_send(Err(io::Error::other("the upload's body stopped")))
    {
        // The request had already stopped taking the body, so there's nothing left to cut off.
        drop(unsent);
    }
    written
}

fn write_archive(batch: &[Outgoing], dir: &Path, writer: ChannelWriter) -> Result<(), BodyError> {
    let stopped = |_: io::Error| BodyError::Stopped;
    let encoder = zstd::stream::write::Encoder::new(writer, zstd::DEFAULT_COMPRESSION_LEVEL)
        .map_err(stopped)?;
    let mut archive = tar::Builder::new(encoder);
    for file in batch {
        let mut header = tar::Header::new_ustar();
        header.set_entry_type(tar::EntryType::Regular);
        header.set_path(&file.hash).map_err(|error| {
            BodyError::File(CliError::Internal {
                detail: format!("a tar entry can't be named {}: {error}", file.hash),
            })
        })?;
        header.set_size(file.size);
        header.set_mode(0o644);
        header.set_cksum();
        let source = File::open(dir.join(&file.path)).map_err(|source| {
            BodyError::File(CliError::FileUnreadable {
                path: file.path.clone(),
                source,
            })
        })?;
        let mut checked = Checked::new(source, file);
        if archive.append(&header, &mut checked).is_err() {
            // The file's own problem, if it had one; otherwise the request stopped.
            return Err(checked
                .problem
                .take()
                .map_or(BodyError::Stopped, BodyError::File));
        }
    }
    let encoder = archive.into_inner().map_err(stopped)?;
    encoder.finish().map_err(stopped)?.finish().map_err(stopped)
}

/// Reads a file as it's sent, and checks it's still what was hashed: exactly
/// the listed size, with the listed hash. Otherwise it fails, noting why.
struct Checked<'a> {
    file: File,
    listed: &'a Outgoing,
    left: u64,
    hasher: blake3::Hasher,
    problem: Option<CliError>,
}

impl<'a> Checked<'a> {
    fn new(file: File, listed: &'a Outgoing) -> Self {
        Self {
            file,
            listed,
            left: listed.size,
            hasher: blake3::Hasher::new(),
            problem: None,
        }
    }

    fn fail(&mut self, problem: CliError) -> io::Error {
        self.problem = Some(problem);
        io::Error::other("the file couldn't be sent as it was hashed")
    }

    fn changed(&mut self) -> io::Error {
        let path = self.listed.path.clone();
        self.fail(CliError::FileChanged { path })
    }

    fn unreadable(&mut self, source: io::Error) -> io::Error {
        let path = self.listed.path.clone();
        self.fail(CliError::FileUnreadable { path, source })
    }

    /// Once the listed size is read, the file must end there, with the listed hash.
    fn check_end(&mut self) -> io::Result<()> {
        let mut more = [0; 1];
        match self.file.read(&mut more) {
            Ok(0) if self.hasher.finalize().to_hex().as_str() == self.listed.hash => Ok(()),
            Ok(_) => Err(self.changed()),
            Err(source) => Err(self.unreadable(source)),
        }
    }
}

impl Read for Checked<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.left == 0 {
            return self.check_end().map(|()| 0);
        }
        let len = buf
            .len()
            .min(usize::try_from(self.left).unwrap_or(usize::MAX));
        let read = match self.file.read(&mut buf[..len]) {
            Ok(0) => return Err(self.changed()),
            Ok(read) => read,
            Err(source) => return Err(self.unreadable(source)),
        };
        self.hasher.update(&buf[..read]);
        self.left -= read as u64;
        Ok(read)
    }
}

/// The body's end of the channel, as the blocking thread writes it.
struct ChannelWriter {
    chunks: mpsc::Sender<io::Result<Bytes>>,
    buffer: BytesMut,
}

impl ChannelWriter {
    fn new(chunks: mpsc::Sender<io::Result<Bytes>>) -> Self {
        Self {
            chunks,
            buffer: BytesMut::with_capacity(CHUNK_LEN),
        }
    }

    fn send(&mut self) -> io::Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        let chunk = self.buffer.split().freeze();
        self.chunks.blocking_send(Ok(chunk)).map_err(|_| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                "the request stopped taking the body",
            )
        })
    }

    /// Sends what's left. Without this, the body ends short.
    fn finish(mut self) -> io::Result<()> {
        self.send()
    }
}

impl Write for ChannelWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.buffer.extend_from_slice(data);
        if self.buffer.len() >= CHUNK_LEN {
            self.send()?;
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.send()
    }
}

/// The request's end of the channel: the chunks, as an HTTP body.
struct ChannelBody(mpsc::Receiver<io::Result<Bytes>>);

impl http_body::Body for ChannelBody {
    type Data = Bytes;
    type Error = io::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, io::Error>>> {
        self.0
            .poll_recv(cx)
            .map(|chunk| chunk.map(|chunk| chunk.map(Frame::data)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outgoing(sizes: &[u64]) -> Vec<Outgoing> {
        sizes
            .iter()
            .enumerate()
            .map(|(n, &size)| Outgoing {
                path: format!("f{n}"),
                hash: format!("{n:064x}"),
                size,
            })
            .collect()
    }

    fn sizes(batches: &[Vec<Outgoing>]) -> Vec<Vec<u64>> {
        batches
            .iter()
            .map(|batch| batch.iter().map(|file| file.size).collect())
            .collect()
    }

    #[test]
    fn each_put_carries_at_most_30_mib_of_files() {
        let mb = 1_000_000;
        assert_eq!(sizes(&batches(outgoing(&[]))), Vec::<Vec<u64>>::new());
        assert_eq!(sizes(&batches(outgoing(&[1, 2, 3]))), [vec![1, 2, 3]]);
        assert_eq!(
            sizes(&batches(outgoing(&[20 * mb, 11 * mb, mb]))),
            [vec![20 * mb, 11 * mb], vec![mb]]
        );
        assert_eq!(
            sizes(&batches(outgoing(&[MAX_BATCH_SIZE, MAX_BATCH_SIZE, 0]))),
            [vec![MAX_BATCH_SIZE], vec![MAX_BATCH_SIZE, 0]]
        );
        assert_eq!(
            sizes(&batches(outgoing(&[20 * mb, 20 * mb, 20 * mb]))),
            [vec![20 * mb], vec![20 * mb], vec![20 * mb]]
        );
    }

    #[test]
    fn a_file_must_still_be_what_was_hashed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file");
        std::fs::write(&path, b"hello").unwrap();
        let listed = |size: u64, contents: &[u8]| Outgoing {
            path: "file".to_owned(),
            hash: blake3::hash(contents).to_hex().to_string(),
            size,
        };
        let read = |listed: &Outgoing| {
            let mut checked = Checked::new(File::open(&path).unwrap(), listed);
            let mut sent = Vec::new();
            let read = checked.read_to_end(&mut sent);
            (
                read.map(|_| sent).map_err(|error| error.to_string()),
                checked.problem.map(|error| error.code().to_owned()),
            )
        };

        let same = listed(5, b"hello");
        assert_eq!(read(&same), (Ok(b"hello".to_vec()), None));
        for changed in [
            listed(4, b"hell"),
            listed(6, b"hello!"),
            listed(5, b"HELLO"),
        ] {
            let (sent, problem) = read(&changed);
            assert!(sent.is_err());
            assert_eq!(problem.as_deref(), Some("app_file_changed"));
        }
    }
}
