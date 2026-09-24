//! Bounded on-disk response cache.
//!
//! Each entry has two horizons: while fresh it is served without asking
//! upstream, and after that, until its stale horizon, it may answer only when
//! upstream fails with a [`Failure`] that [serves
//! stale](Failure::serves_stale). One file lock per key, held across processes,
//! makes concurrent callers fetch a key once. Writes are atomic, and a write
//! that crosses a hard cap prunes the cache to its targets.
//!
//! The crate is synchronous and needs no async runtime, so a daemon and a
//! direct in-process call share the same cache.
//!
//! ```
//! use std::time::Duration;
//!
//! use unixd_cache::{Cache, Key, Limits, Lookup, Policy};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let root = tempfile::tempdir()?;
//! let cache = Cache::open(root.path().join("cache"), 1, Limits::default())?;
//! let key = Key::builder("search").part("query", "rust tokio")?.build();
//!
//! let lock = cache.lock(&key)?;
//! if let Lookup::Miss = lock.lookup::<String>()? {
//!     let fetched = String::from("result");
//!     lock.store(
//!         &fetched,
//!         Policy::new(Duration::from_secs(60), Duration::from_secs(3600)),
//!     )?;
//! }
//!
//! assert!(matches!(cache.lookup::<String>(&key)?, Lookup::Fresh(_)));
//! # Ok(())
//! # }
//! ```

mod cache;
mod error;
mod key;
mod policy;
mod private;

pub use cache::{
    Cache, Cached, Clock, KeyLock, Limits, Lookup, Maintenance, Prune, Stored, SystemClock, Usage,
    default_root,
};
pub use error::Error;
pub use key::{Key, KeyBuilder};
pub use policy::{Failure, Policy, Source};
