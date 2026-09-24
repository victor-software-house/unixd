use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Where an entry's horizons came from.
///
/// It is stored with the entry, so an entry written under one regime is not
/// reinterpreted under another.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// A default the caller chose for this kind of request.
    CallerDefault,
    /// The upstream response's `Cache-Control` header.
    CacheControl,
}

/// How long a stored value is fresh, and how much longer it may be served
/// after an upstream failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Policy {
    /// Time after storing during which the value is served without asking
    /// upstream.
    pub fresh: Duration,
    /// Time after `fresh` ends during which the value may still be served when
    /// the upstream request fails with a [`Failure`] that
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
    /// Only transient failures serve stale: a timeout, a network failure,
    /// HTTP 429, and HTTP 5xx. An authorization failure, a missing resource,
    /// or a malformed response means the stored value may be wrong, so those
    /// never serve stale.
    #[must_use]
    pub const fn serves_stale(self) -> bool {
        match self {
            Self::Timeout | Self::Network => true,
            Self::Status(code) => matches!(code, 429 | 500..=599),
            Self::InvalidJson | Self::SchemaMismatch => false,
        }
    }
}
