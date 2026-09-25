use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Where an entry's horizons came from. Stored with the entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// A default the caller chose for this kind of request.
    CallerDefault,
    /// The upstream response's `Cache-Control` header.
    CacheControl,
}

/// How long a value is fresh, and how long it may then serve stale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Policy {
    /// Served without asking upstream.
    pub fresh: Duration,
    /// After `fresh`, served only when the upstream failure
    /// [serves stale](Failure::serves_stale).
    pub stale_if_error: Duration,
    /// Where these two durations came from.
    pub source: Source,
}

impl Policy {
    /// A policy from the caller's own defaults.
    #[must_use]
    pub const fn new(fresh: Duration, stale_if_error: Duration) -> Self {
        Self {
            fresh,
            stale_if_error,
            source: Source::CallerDefault,
        }
    }

    /// A policy read from the upstream `Cache-Control` header.
    #[must_use]
    pub const fn from_cache_control(fresh: Duration, stale_if_error: Duration) -> Self {
        Self {
            fresh,
            stale_if_error,
            source: Source::CacheControl,
        }
    }
}

/// Why an upstream request failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The request ran out of time.
    Timeout,
    /// The connection could not be made or broke.
    Network,
    /// The upstream answered with this HTTP status.
    Status(u16),
    /// The response body was not valid JSON.
    InvalidJson,
    /// The response was JSON but did not match the expected schema.
    SchemaMismatch,
}

impl Failure {
    /// Whether a stale entry may answer the request instead of this failure.
    ///
    /// Only transient failures do: timeout, network, 429, and 5xx. Any other
    /// failure may mean the stored value is wrong.
    #[must_use]
    pub const fn serves_stale(self) -> bool {
        match self {
            Self::Timeout | Self::Network => true,
            Self::Status(code) => matches!(code, 429 | 500..=599),
            Self::InvalidJson | Self::SchemaMismatch => false,
        }
    }
}
