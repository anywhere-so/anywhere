//! The exec WebSocket protocol, generated from the checked-in Protobuf contracts.
include!(concat!(env!("OUT_DIR"), "/protos.rs"));

/// Default deploy follow window, coordinated with the service deadlines.
pub const DEPLOY_FOLLOW: std::time::Duration = std::time::Duration::from_secs(25 * 60);
