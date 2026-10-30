//! A follow of an app's logs, `GET /v1/apps/{app}/logs?follow=true`, as both
//! ends see it: Server-Sent Events, so the stream carries a heartbeat while the
//! app is quiet, and where each part of the log ends, which a caller whose
//! connection dropped sends back as `after` to go on without seeing anything
//! twice.
//!
//! Each part of the log is one event: an `id:` line with its [`LogPosition`],
//! a `data:` line with the part's bytes in standard base64, since they're the
//! app's and can be anything, then an empty line. A heartbeat is the comment
//! `:` and an empty line. A platform notice is `event: retention`,
//! `event: truncated` or `event: resume_lost`, followed by an empty line.
//! Finite reads request the same format with `events=true`. With
//! `after=end` and `follow=false`, the one event has empty data and names the
//! current log end, then the answer ends.

use std::fmt;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

/// How long a follow may send nothing before the control API sends a
/// heartbeat: under the 100 seconds Cloudflare, and the 120 the front doors,
/// let a response carry nothing before they end it.
pub const LOGS_HEARTBEAT_EVERY: Duration = Duration::from_secs(20);

/// How long a follow's reader waits for its next event, heartbeats included,
/// before it takes the connection as dropped: three heartbeats.
pub const LOGS_FOLLOW_QUIET: Duration = Duration::from_secs(60);

/// A follow's media type.
pub const LOGS_FOLLOW_TYPE: &str = "text/event-stream";

/// A heartbeat, as sent.
pub const LOGS_HEARTBEAT: &[u8] = b":\n\n";

/// The longest line a follow sends: a part of at most 32 KiB in base64, with
/// room to spare. Not measured; it bounds a broken stream.
const MAX_LINE: usize = 64 << 10;

/// The longest a position's text can be: four numbers of at most 20 digits
/// and three dots.
const MAX_POSITION: usize = 4 * 20 + 3;

/// Where a follow got to in an app's log, the byte after the last one sent,
/// as its host daemon counts: it means something only to that host, and to a
/// caller it's text to send back. Written as four decimal numbers joined by
/// dots, none with a leading zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LogPosition {
    /// The host daemon that read it: a random number of each start.
    pub host_daemon: u64,
    /// The log file's inode.
    pub file: u64,
    /// When the log file was made, in nanoseconds since the Unix epoch.
    pub born: u64,
    /// How many of the bytes ever written to the file come before it.
    pub offset: u64,
}

impl LogPosition {
    /// Reads a position from its text, strictly: only what `Display` writes.
    pub fn parse(text: &str) -> Result<Self, PositionError> {
        if text.len() > MAX_POSITION {
            return Err(PositionError);
        }
        let mut numbers = [0u64; 4];
        let mut parts = text.split('.');
        for number in &mut numbers {
            let part = parts.next().ok_or(PositionError)?;
            let canonical = !part.is_empty()
                && part.bytes().all(|b| b.is_ascii_digit())
                && (part == "0" || !part.starts_with('0'));
            if !canonical {
                return Err(PositionError);
            }
            *number = part.parse().map_err(|_| PositionError)?;
        }
        if parts.next().is_some() {
            return Err(PositionError);
        }
        let [host_daemon, file, born, offset] = numbers;
        // A place in a file is an offset the host reads at, which is signed.
        if i64::try_from(offset).is_err() {
            return Err(PositionError);
        }
        Ok(Self {
            host_daemon,
            file,
            born,
            offset,
        })
    }
}

impl fmt::Display for LogPosition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}.{}.{}.{}",
            self.host_daemon, self.file, self.born, self.offset
        )
    }
}

/// Text that isn't a log's position.
#[derive(Debug, PartialEq, Eq)]
pub struct PositionError;

impl fmt::Display for PositionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            "after must be the id of an event that a follow of this app's logs sent, as it sent it",
        )
    }
}

impl std::error::Error for PositionError {}

/// A part of the log, as a follow's event.
pub fn log_event(position: &LogPosition, data: &[u8]) -> Vec<u8> {
    format!("id: {position}\ndata: {}\n\n", STANDARD.encode(data)).into_bytes()
}

/// A host-observed limit on the evidence available, separate from app output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogNotice {
    Retention,
    Truncated,
    ResumeLost,
}
impl LogNotice {
    pub fn parse(code: &str) -> Option<Self> {
        match code {
            "retention" => Some(Self::Retention),
            "truncated" => Some(Self::Truncated),
            "resume_lost" => Some(Self::ResumeLost),
            _ => None,
        }
    }
    pub fn code(self) -> &'static str {
        match self {
            Self::Retention => "retention",
            Self::Truncated => "truncated",
            Self::ResumeLost => "resume_lost",
        }
    }
    pub fn message(self) -> &'static str {
        match self {
            Self::Retention => {
                "Only retained output from the current and previous run is available. Each log keeps its newest 1 MiB; truncation before the host restarted cannot be measured."
            }
            Self::Truncated => {
                "Log bytes were removed before this reader could read them; this output is incomplete."
            }
            Self::ResumeLost => {
                "The previous log position is no longer available; retained output starts again and may omit or repeat bytes."
            }
        }
    }
    pub fn event(self) -> Vec<u8> {
        format!("event: {}\n\n", self.code()).into_bytes()
    }
}

/// What a follow sends.
#[derive(Debug, PartialEq, Eq)]
pub enum FollowEvent {
    Notice(LogNotice),
    /// A part of the log, and where the log is after it.
    Data {
        bytes: Vec<u8>,
        position: LogPosition,
    },
    /// Nothing new; the connection's alive.
    Heartbeat,
}

/// A stream that isn't a follow's events.
#[derive(Debug, PartialEq, Eq)]
pub struct FollowError(pub String);

impl fmt::Display for FollowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for FollowError {}

/// Reads a follow's events from its bytes as they come, in chunks split
/// anywhere, strictly: anything but what [`log_event`] and [`LOGS_HEARTBEAT`]
/// write is an error.
#[derive(Debug, Default)]
pub struct FollowReader {
    /// The line not yet ended.
    line: Vec<u8>,
    /// The event so far.
    id: Option<LogPosition>,
    data: Option<Vec<u8>>,
    heartbeat: bool,
    notice: Option<LogNotice>,
}

impl FollowReader {
    /// Takes the next chunk, and gives the events it ends, in order.
    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<FollowEvent>, FollowError> {
        let mut events = Vec::new();
        let mut rest = chunk;
        while let Some(end) = rest.iter().position(|&b| b == b'\n') {
            self.extend(&rest[..end])?;
            rest = &rest[end + 1..];
            let line = std::mem::take(&mut self.line);
            if let Some(event) = self.line_ended(&line)? {
                events.push(event);
            }
        }
        self.extend(rest)?;
        Ok(events)
    }

    /// Whether the stream may end here: between events.
    pub fn at_rest(&self) -> bool {
        self.line.is_empty()
            && self.id.is_none()
            && self.data.is_none()
            && !self.heartbeat
            && self.notice.is_none()
    }

    fn extend(&mut self, bytes: &[u8]) -> Result<(), FollowError> {
        if self.line.len() + bytes.len() > MAX_LINE {
            return Err(FollowError(format!(
                "the follow sent a line over {MAX_LINE} bytes"
            )));
        }
        self.line.extend_from_slice(bytes);
        Ok(())
    }

    fn line_ended(&mut self, line: &[u8]) -> Result<Option<FollowEvent>, FollowError> {
        let bad = |what: &str| FollowError(format!("the follow sent {what}"));
        if line.is_empty() {
            if let Some(notice) = self.notice.take() {
                if self.id.is_some() || self.data.is_some() || self.heartbeat {
                    return Err(bad("mixed notice and output"));
                }
                return Ok(Some(FollowEvent::Notice(notice)));
            }
            let heartbeat = std::mem::take(&mut self.heartbeat);
            return match (self.id.take(), self.data.take(), heartbeat) {
                (None, None, true) => Ok(Some(FollowEvent::Heartbeat)),
                (Some(position), Some(bytes), false) => {
                    Ok(Some(FollowEvent::Data { bytes, position }))
                }
                _ => Err(bad("an event without both its id and its data")),
            };
        }
        if self.notice.is_some() {
            return Err(bad("more after a notice"));
        }
        if let Some(code) = line.strip_prefix(b"event: ") {
            if self.id.is_some() || self.data.is_some() || self.heartbeat {
                return Err(bad("notice inside output"));
            }
            let code = std::str::from_utf8(code).map_err(|_| bad("invalid notice"))?;
            self.notice = Some(LogNotice::parse(code).ok_or_else(|| bad("unknown notice"))?);
            return Ok(None);
        }
        if self.heartbeat {
            return Err(bad("more after a heartbeat"));
        }
        if line == b":" {
            if self.id.is_some() || self.data.is_some() {
                return Err(bad("a heartbeat inside an event"));
            }
            self.heartbeat = true;
            return Ok(None);
        }
        if let Some(id) = line.strip_prefix(b"id: ") {
            if self.id.is_some() || self.data.is_some() {
                return Err(bad("an id out of place"));
            }
            let position = std::str::from_utf8(id)
                .ok()
                .and_then(|id| LogPosition::parse(id).ok())
                .ok_or_else(|| bad("an id that isn't a position"))?;
            self.id = Some(position);
            return Ok(None);
        }
        if let Some(data) = line.strip_prefix(b"data: ") {
            if self.id.is_none() || self.data.is_some() {
                return Err(bad("data out of place"));
            }
            let bytes = STANDARD
                .decode(data)
                .map_err(|error| bad(&format!("data that isn't base64 ({error})")))?;
            self.data = Some(bytes);
            return Ok(None);
        }
        Err(bad("a line that isn't part of an event"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AT: LogPosition = LogPosition {
        host_daemon: u64::MAX,
        file: 1234,
        born: 1_761_600_000_123_456_789,
        offset: 0,
    };

    #[test]
    fn completeness_notices_stay_separate_from_app_bytes_at_every_split() {
        let mut sent = LogNotice::Truncated.event();
        sent.extend(log_event(&AT, b"event: retention\n\n"));
        for cut in 0..=sent.len() {
            let mut reader = FollowReader::default();
            let mut events = reader.push(&sent[..cut]).unwrap();
            events.extend(reader.push(&sent[cut..]).unwrap());
            assert_eq!(
                events,
                [
                    FollowEvent::Notice(LogNotice::Truncated),
                    FollowEvent::Data {
                        bytes: b"event: retention\n\n".to_vec(),
                        position: AT
                    }
                ]
            );
            assert!(reader.at_rest());
        }
        for bad in [
            b"event: unknown\n\n".as_slice(),
            b"event: truncated\ndata: abc\n\n",
        ] {
            assert!(FollowReader::default().push(bad).is_err());
        }
    }

    #[test]
    fn a_position_reads_back_what_it_writes_and_nothing_else() {
        let text = AT.to_string();
        assert_eq!(text, "18446744073709551615.1234.1761600000123456789.0");
        assert_eq!(LogPosition::parse(&text), Ok(AT));
        for bad in [
            "",
            "1.2.3",
            "1.2.3.4.5",
            "1.2.3.04",
            "1.2.3.-4",
            "1.2.3.+4",
            "1.2..4",
            "1.2.3.4 ",
            "1.2.3.18446744073709551616",
            "1.2.3.9223372036854775808",
            "1.2.3.0x4",
            "\u{661}.2.3.4",
        ] {
            assert_eq!(LogPosition::parse(bad), Err(PositionError), "{bad}");
        }
    }

    #[test]
    fn events_read_back_wherever_the_stream_is_cut() {
        let later = LogPosition { offset: 9, ..AT };
        let mut sent = log_event(&AT, b"\x1b[2Jhello\n\xff");
        sent.extend_from_slice(LOGS_HEARTBEAT);
        sent.extend_from_slice(&log_event(&later, b""));
        for cut in 0..=sent.len() {
            let mut reader = FollowReader::default();
            let mut events = reader.push(&sent[..cut]).unwrap();
            events.extend(reader.push(&sent[cut..]).unwrap());
            assert!(reader.at_rest());
            assert_eq!(
                events,
                [
                    FollowEvent::Data {
                        bytes: b"\x1b[2Jhello\n\xff".to_vec(),
                        position: AT
                    },
                    FollowEvent::Heartbeat,
                    FollowEvent::Data {
                        bytes: Vec::new(),
                        position: later
                    },
                ]
            );
        }
    }

    #[test]
    fn anything_else_is_refused() {
        for bad in [
            &b"data: aGk=\n\n"[..],
            b"id: 1.2.3.4\n\n",
            b"id: 1.2.3.4\nid: 1.2.3.4\ndata: aGk=\n\n",
            b"id: 1.2.3.4\ndata: aGk=\ndata: aGk=\n\n",
            b"id: 1.2.3.4\ndata: !!\n\n",
            b"id: x\ndata: aGk=\n\n",
            b": hi\n\n",
            b":\n:\n\n",
            b":\nid: 1.2.3.4\n\n",
            b"event: logs\n\n",
            b"\n",
        ] {
            assert!(
                FollowReader::default().push(bad).is_err(),
                "{}",
                String::from_utf8_lossy(bad)
            );
        }
        assert!(
            FollowReader::default()
                .push(&vec![b'a'; MAX_LINE + 1])
                .is_err()
        );
        let mut reader = FollowReader::default();
        assert_eq!(reader.push(b"id: 1.2.3.4\n"), Ok(Vec::new()));
        assert!(!reader.at_rest());
    }
}
