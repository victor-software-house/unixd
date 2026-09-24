//! Behaviour of the cache on a real filesystem.

#![expect(
    clippy::unwrap_used,
    reason = "setup helpers outside #[test] functions panic on failure like the tests do"
)]

use std::os::unix::fs::{PermissionsExt as _, symlink};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;
use std::{fs, thread};

use unixd_cache::{
    Cache, Clock, Error, Failure, Key, Limits, Lookup, Maintenance, Policy, Source, Stored,
};

struct ManualClock(AtomicU64);

impl ManualClock {
    fn advance(&self, ms: u64) {
        self.0.fetch_add(ms, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

const START_MS: u64 = 1_000_000;

fn open(root: &Path, schema: u32, limits: Limits) -> (Cache, Arc<ManualClock>) {
    let clock = Arc::new(ManualClock(AtomicU64::new(START_MS)));
    let cache = Cache::with_clock(root, schema, limits, clock.clone()).unwrap();
    (cache, clock)
}

fn key(query: &str) -> Key {
    Key::builder("search").part("query", query).unwrap().build()
}

fn minute_then_hour() -> Policy {
    Policy::new(Duration::from_secs(60), Duration::from_secs(3600))
}

fn store(cache: &Cache, key: &Key, value: &str) -> Stored {
    cache
        .lock(key)
        .unwrap()
        .store(value, minute_then_hour())
        .unwrap()
}

fn mode(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
fn only_transient_failures_serve_stale() {
    for failure in [
        Failure::Timeout,
        Failure::Network,
        Failure::Status(429),
        Failure::Status(500),
        Failure::Status(503),
        Failure::Status(599),
    ] {
        assert!(failure.serves_stale(), "{failure:?}");
    }
    for failure in [
        Failure::Status(401),
        Failure::Status(403),
        Failure::Status(404),
        Failure::Status(400),
        Failure::Status(200),
        Failure::InvalidJson,
        Failure::SchemaMismatch,
    ] {
        assert!(!failure.serves_stale(), "{failure:?}");
    }
}

#[test]
fn duplicate_part_is_refused() {
    let duplicate = Key::builder("search")
        .part("query", "a")
        .unwrap()
        .part("query", "b");
    assert!(matches!(duplicate, Err(Error::DuplicateKeyPart(_))));
}

#[test]
fn part_order_does_not_change_the_key() {
    let first = Key::builder("search")
        .part("query", "rust")
        .unwrap()
        .part("limit", "10")
        .unwrap()
        .build();
    let second = Key::builder("search")
        .part("limit", "10")
        .unwrap()
        .part("query", "rust")
        .unwrap()
        .build();
    assert_eq!(first, second);
    assert_eq!(first.digest().len(), 64);
}

#[test]
fn part_boundaries_and_namespaces_change_the_key() {
    let joined = Key::builder("n").part("a", "bc").unwrap().build();
    let split = Key::builder("n").part("ab", "c").unwrap().build();
    assert_ne!(joined, split);
    assert_ne!(
        key("rust"),
        Key::builder("fetch").part("query", "rust").unwrap().build()
    );
}

#[test]
fn fresh_then_stale_then_miss() {
    let root = tempfile::tempdir().unwrap();
    let (cache, clock) = open(root.path(), 1, Limits::default());
    let key = key("rust");
    assert_eq!(cache.lookup::<String>(&key).unwrap(), Lookup::Miss);

    store(&cache, &key, "result");
    let Lookup::Fresh(cached) = cache.lookup::<String>(&key).unwrap() else {
        panic!("expected fresh");
    };
    assert_eq!(cached.value, "result");
    assert_eq!(cached.stored_at_ms, START_MS);
    assert_eq!(cached.source, Source::CallerDefault);

    clock.advance(60_000);
    assert!(matches!(
        cache.lookup::<String>(&key).unwrap(),
        Lookup::Fresh(_)
    ));
    clock.advance(1);
    let Lookup::Stale(cached) = cache.lookup::<String>(&key).unwrap() else {
        panic!("expected stale");
    };
    assert_eq!(cached.age_ms, 60_001);

    clock.advance(3_600_000);
    assert_eq!(cache.lookup::<String>(&key).unwrap(), Lookup::Miss);
    assert!(cache.entry_path(&key).exists());
    assert_eq!(cache.prune().unwrap().expired_removed, 1);
    assert!(!cache.entry_path(&key).exists());
}

#[test]
fn cache_control_source_is_kept() {
    let root = tempfile::tempdir().unwrap();
    let (cache, _) = open(root.path(), 1, Limits::default());
    let key = key("rust");
    cache
        .lock(&key)
        .unwrap()
        .store(
            "result",
            Policy::from_cache_control(Duration::from_secs(5), Duration::ZERO),
        )
        .unwrap();
    let Lookup::Fresh(cached) = cache.lookup::<String>(&key).unwrap() else {
        panic!("expected fresh");
    };
    assert_eq!(cached.source, Source::CacheControl);
}

#[test]
fn older_schema_is_stale_and_newer_schema_is_ignored() {
    let root = tempfile::tempdir().unwrap();
    let key = key("rust");
    let (old, _) = open(root.path(), 1, Limits::default());
    store(&old, &key, "v1");

    let (new, _) = open(root.path(), 2, Limits::default());
    assert!(matches!(
        new.lookup::<String>(&key).unwrap(),
        Lookup::Stale(_)
    ));
    store(&new, &key, "v2");
    assert!(matches!(
        new.lookup::<String>(&key).unwrap(),
        Lookup::Fresh(_)
    ));

    assert_eq!(old.lookup::<String>(&key).unwrap(), Lookup::Miss);
    assert!(old.entry_path(&key).exists());
    assert_eq!(
        old.lock(&key).unwrap().lookup::<String>().unwrap(),
        Lookup::Miss
    );
    assert!(old.entry_path(&key).exists());
    old.prune().unwrap();
    assert!(old.entry_path(&key).exists());
}

#[test]
fn unlocked_lookup_keeps_and_locked_lookup_removes_invalid_entries() {
    let root = tempfile::tempdir().unwrap();
    let (cache, _) = open(root.path(), 1, Limits::default());
    let key = key("rust");
    fs::write(cache.entry_path(&key), b"{not json").unwrap();
    assert_eq!(cache.lookup::<String>(&key).unwrap(), Lookup::Miss);
    assert!(cache.entry_path(&key).exists());
    assert_eq!(
        cache.lock(&key).unwrap().lookup::<String>().unwrap(),
        Lookup::Miss
    );
    assert!(!cache.entry_path(&key).exists());

    store(&cache, &key, "text");
    assert_eq!(
        cache.lock(&key).unwrap().lookup::<u64>().unwrap(),
        Lookup::Miss
    );
    assert!(!cache.entry_path(&key).exists());

    let other = self::key("other");
    store(&cache, &other, "other");
    fs::rename(cache.entry_path(&other), cache.entry_path(&key)).unwrap();
    assert_eq!(cache.lookup::<String>(&key).unwrap(), Lookup::Miss);

    let path = cache.entry_path(&key);
    fs::remove_file(&path).unwrap();
    symlink(cache.entry_path(&other), &path).unwrap();
    assert_eq!(cache.lookup::<String>(&key).unwrap(), Lookup::Miss);
    assert!(fs::symlink_metadata(&path).is_ok());
    assert_eq!(
        cache.lock(&key).unwrap().lookup::<String>().unwrap(),
        Lookup::Miss
    );
    assert!(fs::symlink_metadata(&path).is_err());
}

#[test]
fn concurrent_opens_of_a_new_root_all_succeed() {
    for _ in 0..50 {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("cache");
        let opens: Vec<_> = (0..8)
            .map(|_| {
                let path = path.clone();
                thread::spawn(move || Cache::open(&path, 1, Limits::default()).map(drop))
            })
            .collect();
        for open in opens {
            assert_eq!(open.join().unwrap(), Ok(()));
        }
    }
}

#[test]
fn a_target_above_its_hard_cap_prunes_to_the_cap() {
    let root = tempfile::tempdir().unwrap();
    let limits = Limits {
        hard_entries: 1,
        ..Limits::default()
    };
    let (cache, _) = open(root.path(), 1, limits);
    store(&cache, &key("first"), "one");
    store(&cache, &key("second"), "two");
    assert_eq!(cache.usage().unwrap().entries, 1);
}

#[test]
fn prune_removes_leftover_temporary_files() {
    let root = tempfile::tempdir().unwrap();
    let (cache, _) = open(root.path(), 1, Limits::default());
    let key = key("rust");
    store(&cache, &key, "result");
    let leftover = root
        .path()
        .join("entries")
        .join(format!(".{}.abc123.tmp", key.digest()));
    fs::write(&leftover, b"{\"partial").unwrap();

    assert!(matches!(
        cache.lookup::<String>(&key).unwrap(),
        Lookup::Fresh(_)
    ));
    let prune = cache.prune().unwrap();
    assert_eq!(prune.temporary_removed, 1);
    assert!(!leftover.exists());
    assert!(matches!(
        cache.lookup::<String>(&key).unwrap(),
        Lookup::Fresh(_)
    ));
}

#[test]
fn prune_removes_expired_before_oldest() {
    let root = tempfile::tempdir().unwrap();
    let limits = Limits {
        hard_entries: 10,
        target_entries: 8,
        ..Limits::default()
    };
    let (cache, clock) = open(root.path(), 1, limits);
    let expiring = key("expiring");
    cache
        .lock(&expiring)
        .unwrap()
        .store("x", Policy::new(Duration::from_millis(1), Duration::ZERO))
        .unwrap();
    clock.advance(10);

    let keys: Vec<Key> = (0..10).map(|index| key(&format!("q{index}"))).collect();
    let mut last = None;
    for key in &keys {
        clock.advance(1);
        last = Some(store(&cache, key, "value"));
    }

    let Some(Stored::Written {
        maintenance: Maintenance::Pruned(prune),
    }) = last
    else {
        panic!("expected a prune, got {last:?}");
    };
    assert_eq!(prune.before_entries, 11);
    assert_eq!(prune.expired_removed, 1);
    assert_eq!(prune.capacity_removed, 2);
    assert_eq!(prune.after_entries, 8);
    assert_eq!(cache.usage().unwrap().entries, 8);
    for removed in &keys[..2] {
        assert_eq!(cache.lookup::<String>(removed).unwrap(), Lookup::Miss);
    }
    for kept in &keys[2..] {
        assert!(matches!(
            cache.lookup::<String>(kept).unwrap(),
            Lookup::Fresh(_)
        ));
    }
}

#[test]
fn write_under_the_cap_needs_no_prune() {
    let root = tempfile::tempdir().unwrap();
    let (cache, _) = open(root.path(), 1, Limits::default());
    assert_eq!(
        store(&cache, &key("rust"), "value"),
        Stored::Written {
            maintenance: Maintenance::NotNeeded
        }
    );
}

#[test]
fn oversized_entry_is_not_stored() {
    let root = tempfile::tempdir().unwrap();
    let limits = Limits {
        max_entry_bytes: 256,
        ..Limits::default()
    };
    let (cache, _) = open(root.path(), 1, limits);
    let key = key("rust");
    let stored = store(&cache, &key, &"x".repeat(1024));
    assert!(matches!(stored, Stored::TooLarge { bytes } if bytes > 1024));
    assert_eq!(cache.lookup::<String>(&key).unwrap(), Lookup::Miss);
}

#[test]
fn clear_removes_every_entry() {
    let root = tempfile::tempdir().unwrap();
    let (cache, _) = open(root.path(), 1, Limits::default());
    store(&cache, &key("a"), "a");
    store(&cache, &key("b"), "b");
    assert_eq!(cache.usage().unwrap().entries, 2);
    cache.clear().unwrap();
    assert_eq!(cache.usage().unwrap().entries, 0);
}

#[test]
fn files_and_directories_are_private() {
    let root = tempfile::tempdir().unwrap();
    let cache_root = root.path().join("nested").join("cache");
    let (cache, _) = open(&cache_root, 1, Limits::default());
    let key = key("rust");
    store(&cache, &key, "result");

    assert_eq!(mode(&cache_root), 0o700);
    assert_eq!(mode(&cache_root.join("entries")), 0o700);
    assert_eq!(mode(&cache_root.join("locks")), 0o700);
    assert_eq!(mode(&cache.entry_path(&key)), 0o600);
    assert_eq!(mode(&cache_root.join("maintenance.lock")), 0o600);
}

#[test]
fn symlinked_root_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let real = root.path().join("real");
    fs::create_dir(&real).unwrap();
    let link = root.path().join("link");
    symlink(&real, &link).unwrap();
    assert_eq!(
        Cache::open(&link, 1, Limits::default()).unwrap_err(),
        Error::UnsafeRoot
    );
}

#[test]
fn symlinked_entries_directory_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let cache_root = root.path().join("cache");
    let elsewhere = root.path().join("elsewhere");
    fs::create_dir_all(&cache_root).unwrap();
    fs::create_dir(&elsewhere).unwrap();
    symlink(&elsewhere, cache_root.join("entries")).unwrap();
    assert_eq!(
        Cache::open(&cache_root, 1, Limits::default()).unwrap_err(),
        Error::UnsafeRoot
    );
}

#[test]
fn a_key_lock_excludes_a_second_holder() {
    let root = tempfile::tempdir().unwrap();
    let (cache, _) = open(root.path(), 1, Limits::default());
    let key = key("rust");
    let held = cache.lock(&key).unwrap();

    let (sender, receiver) = mpsc::channel();
    let waiter = {
        let cache = cache.clone();
        let key = key.clone();
        thread::spawn(move || {
            let lock = cache.lock(&key).unwrap();
            let seen = lock.lookup::<String>().unwrap();
            sender.send(()).unwrap();
            seen
        })
    };

    assert!(receiver.recv_timeout(Duration::from_millis(200)).is_err());
    held.store("from the first holder", minute_then_hour())
        .unwrap();
    receiver.recv_timeout(Duration::from_secs(5)).unwrap();
    let Lookup::Fresh(cached) = waiter.join().unwrap() else {
        panic!("expected the first holder's entry");
    };
    assert_eq!(cached.value, "from the first holder");
}

#[test]
fn different_keys_do_not_block_each_other() {
    let root = tempfile::tempdir().unwrap();
    let (cache, _) = open(root.path(), 1, Limits::default());
    let _first = cache.lock(&key("a")).unwrap();
    let second = cache.lock(&key("b")).unwrap();
    second.store("b", minute_then_hour()).unwrap();
}

#[test]
fn relative_root_is_made_absolute() {
    let root = tempfile::tempdir().unwrap();
    let (cache, _) = open(root.path(), 1, Limits::default());
    assert!(cache.root().is_absolute());
}

#[test]
fn store_over_the_cap_defers_while_another_key_is_locked() {
    let root = tempfile::tempdir().unwrap();
    let limits = Limits {
        hard_entries: 1,
        target_entries: 1,
        ..Limits::default()
    };
    let (cache, _) = open(root.path(), 1, limits);
    store(&cache, &key("a"), "a");
    let held = cache.lock(&key("b")).unwrap();
    let stored = store(&cache, &key("c"), "c");
    assert!(matches!(
        stored,
        Stored::Written {
            maintenance: Maintenance::Deferred(Error::Lock)
        }
    ));
    assert!(matches!(
        cache.lookup::<String>(&key("c")).unwrap(),
        Lookup::Fresh(_)
    ));
    assert!(root.path().join("prune.pending").exists());
    drop(held);
    assert!(matches!(
        store(&cache, &key("d"), "d"),
        Stored::Written {
            maintenance: Maintenance::Pruned(_)
        }
    ));
}

#[test]
fn prune_and_clear_remove_lock_files() {
    let root = tempfile::tempdir().unwrap();
    let (cache, clock) = open(root.path(), 1, Limits::default());
    let locks = root.path().join("locks");
    cache
        .lock(&key("expiring"))
        .unwrap()
        .store("x", Policy::new(Duration::from_millis(1), Duration::ZERO))
        .unwrap();
    store(&cache, &key("kept"), "kept");
    drop(cache.lock(&key("never stored")).unwrap());
    assert_eq!(fs::read_dir(&locks).unwrap().count(), 3);

    clock.advance(10);
    assert_eq!(cache.prune().unwrap().locks_removed, 2);
    assert_eq!(fs::read_dir(&locks).unwrap().count(), 1);

    fs::write(root.path().join("prune.pending"), b"").unwrap();
    cache.clear().unwrap();
    assert_eq!(fs::read_dir(&locks).unwrap().count(), 0);
    assert!(!root.path().join("prune.pending").exists());
}

#[test]
fn next_lock_runs_a_deferred_prune() {
    let root = tempfile::tempdir().unwrap();
    let limits = Limits {
        hard_entries: 1,
        target_entries: 1,
        ..Limits::default()
    };
    let (cache, _) = open(root.path(), 1, limits);
    store(&cache, &key("a"), "a");
    let held = cache.lock(&key("b")).unwrap();
    store(&cache, &key("c"), "c");
    drop(held);
    assert_eq!(cache.usage().unwrap().entries, 2);

    drop(cache.lock(&key("d")).unwrap());
    assert_eq!(cache.usage().unwrap().entries, 1);
    assert!(!root.path().join("prune.pending").exists());
}

#[test]
fn a_marker_under_the_caps_is_cleared_by_the_next_lock() {
    let root = tempfile::tempdir().unwrap();
    let (cache, _) = open(root.path(), 1, Limits::default());
    store(&cache, &key("a"), "a");
    let marker = root.path().join("prune.pending");
    fs::write(&marker, b"").unwrap();
    drop(cache.lock(&key("b")).unwrap());
    assert!(!marker.exists());
    assert_eq!(cache.usage().unwrap().entries, 1);
}
