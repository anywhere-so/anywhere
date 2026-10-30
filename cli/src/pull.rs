//! `anywhere pull <app> <path> [dir]`: copies a folder from an app's disk by
//! running `tar -C <path> -cf - .` in the app through `anywhere exec`, and
//! unpacking what it sends into a local folder, named for the app without its
//! owner unless given. The stream is unpacked into a hidden staging folder
//! inside the destination, and moved into place only once all of it has come
//! and `tar` has exited 0, so a failure leaves the destination as it was.

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

use bytes::Bytes;
use serde::Serialize;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::app_ref::AppRef;
use crate::error::CliError;
use crate::exec::{self, Event};
use crate::output::{Output, ResultData};
use crate::untar::{self, Unpacked};

/// How much of what `tar` prints on standard error is kept, from its end, for
/// the error.
const MAX_SAID: usize = 4 * 1024;

/// How many chunks of the stream wait for the unpacking thread.
const CHUNKS_IN_FLIGHT: usize = 16;

/// How long the stream may bring nothing before the pull gives up. Not
/// measured: `tar` sends as it reads, so a minute of silence means it's stuck.
const IDLE: Duration = Duration::from_secs(60);

#[derive(Debug, Serialize)]
pub struct PullSummary {
    pub app: String,
    pub path: String,
    pub dir: String,
    pub files: u64,
    pub bytes: u64,
}

pub async fn pull(
    app: &str,
    path: &str,
    dir: Option<&Path>,
    output: Output,
) -> Result<ResultData, CliError> {
    let app = AppRef::parse(app)?;
    let dest = dir.map_or_else(|| PathBuf::from(app.name.as_str()), Path::to_path_buf);
    let shown = dest.display().to_string();
    let stage = Stage::new(&dest, output)?;

    let mut session = exec::start(
        &app,
        "tar".to_owned(),
        ["-C", path, "-cf", "-", "."].map(str::to_owned).to_vec(),
        output,
    )
    .await?;
    output.progress(&format!("Copying {path} from {app} into {shown}"));

    let (chunks, received) = mpsc::channel::<Bytes>(CHUNKS_IN_FLIGHT);
    let unpacking = {
        let root = stage.path.clone();
        tokio::task::spawn_blocking(move || -> Result<Unpacked, untar::UntarError> {
            let mut stream = ChannelReader::new(received);
            let unpacked = untar::unpack(&mut stream, &root)?;
            // tar pads its stream past the archive's end; reading to the
            // stream's end keeps it flowing until tar exits.
            io::copy(&mut stream, &mut io::sink()).map_err(|error| untar::UntarError::Stream {
                detail: error.to_string(),
            })?;
            Ok(unpacked)
        })
    };
    // tar reads nothing, so its standard input closes at once.
    let (_, no_input) = mpsc::channel(1);
    let mut said = Vec::new();
    let streamed = tokio::select! {
        result = async {
            loop {
                let next = tokio::time::timeout(IDLE, session.reader.next())
                    .await
                    .map_err(|_| CliError::PullStalled {
                        dir: shown.clone(),
                        waited: IDLE,
                    })?
                    .map_err(|error| match error {
                        CliError::ExecInterrupted { detail } => CliError::PullInterrupted {
                            dir: shown.clone(),
                            detail,
                        },
                        other => other,
                    })?;
                match next {
                    Event::Stdout(data) => {
                        // The unpacking stopped early, so its error says why.
                        if chunks.send(Bytes::from(data)).await.is_err() {
                            return Ok::<_, CliError>(None);
                        }
                    }
                    Event::Stderr(data) => keep_tail(&mut said, &data),
                    Event::Exited(code) => return Ok(Some(code)),
                }
            }
        } => result,
        err = exec::send_stdin(&mut session.sink, no_input, session.pongs) => Err(err),
    };
    // The unpacking ends once the stream does, and is waited for before
    // anything returns, so the staging folder is removed only once nothing
    // writes into it.
    drop(chunks);
    let unpacked = unpacking.await.map_err(|error| CliError::Internal {
        detail: format!("the unpacking stopped: {error}"),
    })?;
    let exit = streamed?;
    if exit.is_some() {
        exec::close(session.sink).await;
    }
    let said = String::from_utf8_lossy(&said).trim().to_owned();
    // A tar that failed explains a stream that's cut short.
    if let Some(code) = exit
        && code != 0
    {
        return Err(CliError::PullTarFailed {
            app: app.to_string(),
            dir: shown,
            code,
            said: if said.is_empty() {
                "nothing".to_owned()
            } else {
                said
            },
        });
    }
    let Unpacked { files, bytes } = unpacked.map_err(|source| CliError::PullRefused {
        dir: shown.clone(),
        source,
    })?;
    if exit.is_none() {
        return Err(CliError::Internal {
            detail: "the unpacking stopped reading the stream but didn't fail".to_owned(),
        });
    }
    if !said.is_empty() {
        output.progress(&format!("tar said: {said}"));
    }
    stage.commit()?;
    output.progress(&format!("Pulled {files} files from {app} into {shown}"));
    // In the folder, a plain deploy, `logs` or `secrets` names an app of your
    // own, by the folder's name.
    if app.owner.is_some() {
        let folder = shell_word(&shown);
        output.next(
            &format!("anywhere deploy {folder} --app {app}"),
            &format!(
                "in {folder}, give `anywhere logs` and `anywhere secrets` --app {app} too; without it, they act on an app of your own named after the folder"
            ),
        );
    }
    Ok(ResultData::Pull(PullSummary {
        app: app.to_string(),
        path: path.to_owned(),
        dir: shown,
        files,
        bytes,
    }))
}

/// `text` as one word for a shell: as it is when it holds only characters a
/// shell takes literally, and otherwise in single quotes.
fn shell_word(text: &str) -> String {
    let literal = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/' | '-');
    if !text.is_empty() && text.chars().all(literal) {
        return text.to_owned();
    }
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// Keeps the last [`MAX_SAID`] bytes of what's been said.
fn keep_tail(said: &mut Vec<u8>, data: &[u8]) {
    said.extend_from_slice(data);
    if said.len() > MAX_SAID {
        said.drain(..said.len() - MAX_SAID);
    }
}

/// The stream's chunks, as the unpacking thread reads them.
struct ChannelReader {
    chunks: mpsc::Receiver<Bytes>,
    chunk: Bytes,
}

impl ChannelReader {
    fn new(chunks: mpsc::Receiver<Bytes>) -> Self {
        Self {
            chunks,
            chunk: Bytes::new(),
        }
    }
}

impl Read for ChannelReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        while self.chunk.is_empty() {
            match self.chunks.blocking_recv() {
                Some(chunk) => self.chunk = chunk,
                // The stream ended.
                None => return Ok(0),
            }
        }
        let len = buf.len().min(self.chunk.len());
        buf[..len].copy_from_slice(&self.chunk.split_to(len));
        Ok(len)
    }
}

/// The destination, made if it wasn't there and otherwise empty, and a
/// hidden staging folder inside it. Dropped before [`Stage::commit`], it
/// removes the staging folder, and the destination too if it made it and
/// it's empty, never what something else put there.
struct Stage {
    dest: PathBuf,
    path: PathBuf,
    made_dest: bool,
    committed: bool,
    output: Output,
}

impl Stage {
    fn new(dest: &Path, output: Output) -> Result<Self, CliError> {
        let shown = dest.display().to_string();
        let write = |source| CliError::PullWrite {
            path: shown.clone(),
            source,
        };
        let made_dest = match fs::symlink_metadata(dest) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                if let Some(parent) = dest.parent().filter(|p| !p.as_os_str().is_empty()) {
                    fs::create_dir_all(parent).map_err(|source| CliError::PullWrite {
                        path: parent.display().to_string(),
                        source,
                    })?;
                }
                fs::create_dir(dest).map_err(write)?;
                true
            }
            Err(error) => return Err(write(error)),
            Ok(metadata) if !metadata.is_dir() => {
                return Err(CliError::PullDestinationExists {
                    dir: shown,
                    detail: "exists and isn't a folder".to_owned(),
                });
            }
            Ok(_) => {
                let mut names = Vec::new();
                for entry in fs::read_dir(dest).map_err(write)? {
                    names.push(entry.map_err(write)?.file_name());
                }
                if !names.is_empty() {
                    let leftover = names
                        .iter()
                        .all(|name| name.as_encoded_bytes().starts_with(b".anywhere-pull-"));
                    return Err(CliError::PullDestinationExists {
                        detail: if leftover {
                            format!(
                                "holds only what an interrupted pull left, a folder named .anywhere-pull-…; remove it with rm -rf {shown}/.anywhere-pull-*, then pull again"
                            )
                        } else {
                            "already exists and isn't empty".to_owned()
                        },
                        dir: shown,
                    });
                }
                false
            }
        };
        let mut stage = Self {
            dest: dest.to_owned(),
            path: dest.join(format!(".anywhere-pull-{}", Uuid::now_v7())),
            made_dest,
            committed: false,
            output,
        };
        if let Err(source) = fs::create_dir(&stage.path) {
            // Drop then removes only the destination, if it was made.
            stage.path = PathBuf::new();
            return Err(write(source));
        }
        Ok(stage)
    }

    /// Moves everything staged up into the destination, which must hold
    /// nothing else. If one move fails, the ones before it are moved back.
    fn commit(mut self) -> Result<(), CliError> {
        let shown = self.dest.display().to_string();
        let write = |source| CliError::PullWrite {
            path: shown.clone(),
            source,
        };
        let stage_name = self.path.file_name().map(ToOwned::to_owned);
        for existing in fs::read_dir(&self.dest).map_err(write)? {
            if Some(existing.map_err(write)?.file_name()) != stage_name {
                return Err(CliError::PullDestinationExists {
                    dir: shown,
                    detail: "had something else written into it during the pull".to_owned(),
                });
            }
        }
        let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
        for entry in fs::read_dir(&self.path).map_err(write)? {
            let from = entry.map_err(write)?.path();
            let Some(name) = from.file_name() else {
                continue;
            };
            let to = self.dest.join(name);
            if let Err(source) = fs::rename(&from, &to) {
                for (back, moved_to) in moved.into_iter().rev() {
                    if let Err(error) = fs::rename(&moved_to, &back) {
                        // What's left can't be told from the pull, so it stays
                        // for the caller to see, and isn't removed.
                        self.committed = true;
                        return Err(CliError::PullWrite {
                            path: to.display().to_string(),
                            source: io::Error::other(format!(
                                "{source}, and moving {} back failed too ({error}), so {shown} holds part of the pull; remove it and pull again",
                                moved_to.display()
                            )),
                        });
                    }
                }
                return Err(CliError::PullWrite {
                    path: to.display().to_string(),
                    source,
                });
            }
            moved.push((from, to));
        }
        fs::remove_dir(&self.path).map_err(write)?;
        self.committed = true;
        Ok(())
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        // Only what the pull wrote: its staging folder, then the destination
        // if the pull made it and nothing else has been put there.
        let mut removed = if self.path.as_os_str().is_empty() {
            Ok(())
        } else {
            fs::remove_dir_all(&self.path)
        };
        if removed.is_ok() && self.made_dest {
            removed = match fs::remove_dir(&self.dest) {
                Err(error) if error.kind() == io::ErrorKind::DirectoryNotEmpty => Ok(()),
                other => other,
            };
        }
        if let Err(error) = removed {
            // The error that failed the pull is the one reported; this says
            // what's left of it.
            self.output.progress(&format!(
                "couldn't remove what the failed pull wrote in {} ({error}); remove it yourself",
                self.dest.display()
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_in_a_hint_pastes_into_a_shell_as_one_word() {
        for (folder, word) in [
            ("wiki", "wiki"),
            ("../apps/my-wiki_2.0", "../apps/my-wiki_2.0"),
            ("my dir", "'my dir'"),
            ("it's", r"'it'\''s'"),
            ("$HOME;rm", "'$HOME;rm'"),
            ("", "''"),
        ] {
            assert_eq!(shell_word(folder), word, "{folder:?}");
        }
    }
}
