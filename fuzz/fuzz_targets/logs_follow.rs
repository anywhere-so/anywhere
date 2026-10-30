//! A follow of an app's logs as the CLI reads it: `control_api::FollowReader`
//! over the stream's bytes, cut into chunks anywhere, and
//! `control_api::LogPosition::parse`, which the control plane and the gateway
//! run on a follow's `after`. The input's first byte says where to cut. Every
//! event read writes back to bytes that read the same, however they're cut,
//! and a position that parses writes back the same text.

#![no_main]

use control_api::{FollowEvent, FollowReader, LOGS_HEARTBEAT, LogPosition, log_event};
use libfuzzer_sys::fuzz_target;

fn read(stream: &[u8], cut: usize) -> Result<(Vec<FollowEvent>, bool), control_api::FollowError> {
    let mut reader = FollowReader::default();
    let cut = cut.min(stream.len());
    let mut events = reader.push(&stream[..cut])?;
    events.extend(reader.push(&stream[cut..])?);
    Ok((events, reader.at_rest()))
}

fuzz_target!(|input: &[u8]| {
    let Some((&cut, stream)) = input.split_first() else {
        return;
    };
    if let Ok(text) = std::str::from_utf8(stream)
        && let Ok(position) = LogPosition::parse(text)
    {
        assert_eq!(position.to_string(), text);
    }
    let Ok((events, _)) = read(stream, usize::from(cut)) else {
        return;
    };
    let mut again = Vec::new();
    for event in &events {
        match event {
            FollowEvent::Data { bytes, position } => {
                again.extend_from_slice(&log_event(position, bytes));
            }
            FollowEvent::Notice(notice) => again.extend_from_slice(&notice.event()),
            FollowEvent::Heartbeat => again.extend_from_slice(LOGS_HEARTBEAT),
        }
    }
    let (reread, rest) = read(&again, usize::from(cut)).expect("what it wrote reads back");
    assert!(rest);
    assert_eq!(reread, events);
});
