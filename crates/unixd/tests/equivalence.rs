//! Verification step 9: one request through the daemon and through the
//! direct path gives the same output and the same cache entry.

#![expect(
    clippy::unwrap_used,
    reason = "setup helpers outside #[test] functions panic on failure like the tests do"
)]

use std::convert::Infallible;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use std::{fmt, fs};

use serde_json::Value;
use tokio::net::UnixListener;
use unixd::{Client, Failure, Handler, Limits, SingleFlight, Version, serve_connection};
use unixd_cache::{Cache, Clock, Key, Lookup, Policy};

const V1: Version = Version { major: 1, minor: 0 };

struct Fixed;

impl Clock for Fixed {
    fn now_ms(&self) -> u64 {
        1_800_000_000_000
    }
}

struct Never(Infallible);

impl fmt::Display for Never {
    fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {}
    }
}

impl Failure for Never {
    fn code(&self) -> &'static str {
        match self.0 {}
    }
    fn retryable(&self) -> bool {
        match self.0 {}
    }
}

fn open(root: &Path) -> Cache {
    Cache::with_clock(root, 1, unixd_cache::Limits::default(), Arc::new(Fixed)).unwrap()
}

/// The consumer's code shared by both paths: the cache first, then a fixed
/// upstream that answers with the query reversed.
fn answer(cache: &Cache, query: &str) -> String {
    let key = Key::builder("reverse")
        .part("query", query)
        .unwrap()
        .build();
    let lock = cache.lock(&key).unwrap();
    if let Lookup::Fresh(cached) = lock.lookup::<String>().unwrap() {
        return cached.value;
    }
    let value: String = query.chars().rev().collect();
    lock.store(
        &value,
        Policy::new(Duration::from_secs(60), Duration::from_secs(3600)),
    )
    .unwrap();
    value
}

struct Daemon {
    cache: Arc<Cache>,
    flights: SingleFlight<String, String, Infallible>,
}

impl Handler for Daemon {
    type Request = String;
    type Response = String;
    type Error = Never;

    async fn handle(&self, query: String) -> Result<String, Never> {
        let cache = Arc::clone(&self.cache);
        let key = query.clone();
        let reply = self
            .flights
            .run(key, move || async move {
                Ok(tokio::task::spawn_blocking(move || answer(&cache, &query))
                    .await
                    .unwrap())
            })
            .await;
        Ok(reply.unwrap_or_default())
    }
}

/// The entry's value and horizons, without the fields that name the file.
fn entry(root: &Path) -> Value {
    let dir = root.join("entries");
    let file = fs::read_dir(&dir).unwrap().next().unwrap().unwrap().path();
    let mut entry: Value = serde_json::from_slice(&fs::read(file).unwrap()).unwrap();
    let fields = entry.as_object_mut().unwrap();
    fields.retain(|name, _| {
        matches!(
            name.as_str(),
            "value" | "stored_at_ms" | "fresh_until_ms" | "stale_until_ms" | "schema" | "source"
        )
    });
    entry
}

#[tokio::test(flavor = "multi_thread")]
async fn the_daemon_and_the_direct_path_agree() {
    let roots = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let socket = roots.0.path().join("d.sock");
    let daemon_cache = roots.0.path().join("cache");
    let direct_cache = roots.1.path().join("cache");

    let listener = UnixListener::bind(&socket).unwrap();
    let daemon = Arc::new(Daemon {
        cache: Arc::new(open(&daemon_cache)),
        flights: SingleFlight::new(),
    });
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            serve_connection(stream, &*daemon, V1, Limits::default()).await;
        }
    });
    let through_daemon: String =
        tokio::task::spawn_blocking(move || Client::new(&socket, V1).call(&"unixd").unwrap())
            .await
            .unwrap();
    let direct = answer(&open(&direct_cache), "unixd");

    assert_eq!(through_daemon, direct);
    assert_eq!(entry(&daemon_cache), entry(&direct_cache));
}
