use std::time::Duration;

use serde::Deserialize;

/// Bounds on one connection. A consumer embeds them in its own config, sets
/// only what it needs, and writes durations as `10s` or `1m`; an unknown
/// field is an error.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    /// The largest frame in either direction, newline included. 16 MiB.
    pub max_frame_bytes: usize,
    /// How long the server waits for a whole request. 10 seconds.
    #[serde(deserialize_with = "humantime_serde::deserialize")]
    pub read_timeout: Duration,
    /// How long the server waits for the client to take the response. 10
    /// seconds.
    #[serde(deserialize_with = "humantime_serde::deserialize")]
    pub write_timeout: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_frame_bytes: 16 * 1024 * 1024,
            read_timeout: Duration::from_secs(10),
            write_timeout: Duration::from_secs(10),
        }
    }
}
