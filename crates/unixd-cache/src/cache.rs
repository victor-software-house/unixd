use std::fs::{self, File, Metadata, TryLockError};
use std::io::{BufRead as _, BufReader, ErrorKind, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::{env, fmt, path};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::key::FORMAT;
use crate::{Error, Key, Policy, Source, private};

const ENTRIES: &str = "entries";
const LOCKS: &str = "locks";
const MAINTENANCE: &str = "maintenance.lock";
const PRUNE_LOCK: &str = "prune.lock";
const TEMPORARY_SUFFIX: &str = ".tmp";
const CACHEDIR_TAG: &str = "CACHEDIR.TAG";
const CACHEDIR_TAG_TEXT: &str = "Signature: 8a477f597d28d172789f06886806bc55
# This file is a cache directory tag created by unixd-cache.
# For information about cache directory tags, see https://bford.info/cachedir/
";

/// The per-user cache directory for `name`: `~/Library/Caches/<name>` on
/// macOS, and `$XDG_CACHE_HOME/<name>` or `~/.cache/<name>` elsewhere. `None`
/// when `HOME` is unset or relative. The OS cleans neither, so [`Limits`]
/// bounds the cache.
#[must_use]
pub fn default_root(name: &str) -> Option<PathBuf> {
    let home = env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute());
    let base = if cfg!(target_os = "macos") {
        home?.join("Library").join("Caches")
    } else {
        env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| home.map(|home| home.join(".cache")))?
    };
    Some(base.join(name))
}

/// The current time in milliseconds since the Unix epoch.
///
/// Tests pass their own clock to [`Cache::with_clock`] to move time without
/// sleeping.
pub trait Clock: Send + Sync {
    /// Milliseconds since the Unix epoch.
    fn now_ms(&self) -> u64;
}

/// The system clock.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, millis)
    }
}

/// Size and age bounds. A write that takes the cache over a hard cap prunes
/// it down to the targets, least recently used first. A target above its hard
/// cap acts as the hard cap.
///
/// It deserializes from a consumer's config: every field is optional and
/// defaults as below, durations read like `30d` or `1h`, and an unknown field
/// is an error.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    /// An entry larger than this is returned to the caller but not stored.
    pub max_entry_bytes: u64,
    /// Entry count that triggers a prune.
    pub hard_entries: u64,
    /// Total bytes that trigger a prune.
    pub hard_bytes: u64,
    /// Entry count a prune reduces to.
    pub target_entries: u64,
    /// Total bytes a prune reduces to.
    pub target_bytes: u64,
    /// A prune removes an entry not read or written for this long, even
    /// unexpired.
    #[serde(deserialize_with = "humantime_serde::deserialize")]
    pub unused_after: Duration,
    /// A write prunes when the last prune is older than this, even under the
    /// caps.
    #[serde(deserialize_with = "humantime_serde::deserialize")]
    pub sweep_every: Duration,
    /// A hit records its use at most this often, so most reads write nothing.
    #[serde(deserialize_with = "humantime_serde::deserialize")]
    pub touch_after: Duration,
}

impl Default for Limits {
    /// 8 MiB per entry, and 10,000 entries or 256 MiB before pruning to 8,000
    /// entries and 200 MiB. Entries unused for 30 days go at the next daily
    /// sweep; a hit records its use at most hourly.
    fn default() -> Self {
        Self {
            max_entry_bytes: 8 * 1024 * 1024,
            hard_entries: 10_000,
            hard_bytes: 256 * 1024 * 1024,
            target_entries: 8_000,
            target_bytes: 200 * 1024 * 1024,
            unused_after: Duration::from_hours(30 * 24),
            sweep_every: Duration::from_hours(24),
            touch_after: Duration::from_hours(1),
        }
    }
}

/// The result of reading a key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lookup<T> {
    /// Nothing usable is stored.
    Miss,
    /// Stored and inside its fresh horizon: serve it without asking upstream.
    Fresh(Cached<T>),
    /// Past its fresh horizon but inside its stale horizon: ask upstream, and
    /// serve this only if the upstream [`Failure`](crate::Failure)
    /// [serves stale](crate::Failure::serves_stale).
    Stale(Cached<T>),
}

/// A stored value and when it was stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cached<T> {
    /// The stored value.
    pub value: T,
    /// When it was stored, in milliseconds since the Unix epoch.
    pub stored_at_ms: u64,
    /// Milliseconds since it was stored.
    pub age_ms: u64,
    /// Where its horizons came from.
    pub source: Source,
}

/// The result of a store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stored {
    /// The entry was written.
    Written {
        /// Whether the write crossed a hard cap, and how the prune went.
        maintenance: Maintenance,
    },
    /// The entry exceeded [`Limits::max_entry_bytes`] and was not written.
    TooLarge {
        /// The serialized size.
        bytes: u64,
    },
}

/// Pruning that followed a write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Maintenance {
    /// The cache was within its hard caps and no sweep was due.
    NotNeeded,
    /// The write crossed a hard cap or a sweep was due, and the cache was
    /// pruned.
    Pruned(Prune),
    /// The cache needs pruning but it failed. The write itself succeeded, and
    /// the next write over a cap retries.
    ///
    /// [`Error::Lock`] means another prune, or a [`Cache::clear`], was running.
    Deferred(Error),
}

/// What a prune removed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Prune {
    /// Entries before the prune.
    pub before_entries: u64,
    /// Bytes before the prune.
    pub before_bytes: u64,
    /// Entries after the prune.
    pub after_entries: u64,
    /// Bytes after the prune.
    pub after_bytes: u64,
    /// Entries removed because they were expired or invalid.
    pub expired_removed: u64,
    /// Unexpired entries removed because they were unused for
    /// [`Limits::unused_after`].
    pub unused_removed: u64,
    /// Entries removed, least recently used first, to reach the targets.
    pub capacity_removed: u64,
    /// Temporary files left by writers that died mid-write.
    pub temporary_removed: u64,
    /// Lock files that no process held, such as those of a process that died.
    pub locks_removed: u64,
}

/// Entry count and total bytes on disk.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    /// Entry files.
    pub entries: u64,
    /// Their total size.
    pub bytes: u64,
}

impl Usage {
    fn remove(&mut self, bytes: u64) {
        self.entries = self.entries.saturating_sub(1);
        self.bytes = self.bytes.saturating_sub(bytes);
    }
}

struct Inner {
    root: PathBuf,
    schema: u32,
    limits: Limits,
    clock: Arc<dyn Clock>,
}

/// A bounded on-disk cache under one private root.
///
/// Every process that opens the same root shares its entries and its locks.
#[derive(Clone)]
pub struct Cache {
    inner: Arc<Inner>,
}

impl fmt::Debug for Cache {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Cache")
            .field("schema", &self.inner.schema)
            .field("limits", &self.inner.limits)
            .finish_non_exhaustive()
    }
}

impl Cache {
    /// Opens the cache at `root`, creating it `0700` when missing.
    ///
    /// `schema` is the version of the caller's stored value type. Raise it
    /// when that type changes: entries from a lower schema are then served
    /// only as [`Lookup::Stale`], and entries from a higher one are ignored.
    ///
    /// `root` is made absolute once, here, so a later change of working
    /// directory cannot move the cache. A `CACHEDIR.TAG` there tells backup
    /// tools to skip it. [`default_root`] gives the usual place.
    ///
    /// # Errors
    ///
    /// [`Error::UnsafeRoot`] when the root or a directory under it is a
    /// symlink or owned by another user, and [`Error::Io`] when it cannot be
    /// created.
    pub fn open(root: impl AsRef<Path>, schema: u32, limits: Limits) -> Result<Self, Error> {
        Self::with_clock(root, schema, limits, Arc::new(SystemClock))
    }

    /// [`Cache::open`] with a caller-supplied clock.
    ///
    /// # Errors
    ///
    /// As [`Cache::open`].
    pub fn with_clock(
        root: impl AsRef<Path>,
        schema: u32,
        limits: Limits,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, Error> {
        let root = path::absolute(root).map_err(|error| Error::io(&error))?;
        private::ensure_root(&root)?;
        private::ensure_dir(&root.join(ENTRIES))?;
        private::ensure_dir(&root.join(LOCKS))?;
        drop(private::open_lock(&root.join(MAINTENANCE))?);
        private::create_private(&root.join(CACHEDIR_TAG), CACHEDIR_TAG_TEXT.as_bytes())?;
        let pruning = root.join(PRUNE_LOCK);
        if fs::symlink_metadata(&pruning).is_err() {
            drop(private::open_lock(&pruning)?);
            private::touch(&pruning, at(clock.now_ms()))?;
        }
        Ok(Self {
            inner: Arc::new(Inner {
                root,
                schema,
                limits,
                clock,
            }),
        })
    }

    /// The absolute root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.inner.root
    }

    /// Where the entry for `key` lives, for diagnostics.
    #[must_use]
    pub fn entry_path(&self, key: &Key) -> PathBuf {
        entry_path(&self.inner, key.digest())
    }

    /// Reads `key` without taking its lock.
    ///
    /// A corrupt, mismatched, or expired entry reads as [`Lookup::Miss`] and
    /// stays on disk, since a lock holder may be replacing it; only
    /// [`KeyLock::lookup`] and [`Cache::prune`] remove it. A value that no
    /// longer deserializes into `T` counts as corrupt.
    ///
    /// # Errors
    ///
    /// [`Error::UnsafeRoot`] or [`Error::Io`] when the entry cannot be read
    /// safely.
    pub fn lookup<T: DeserializeOwned>(&self, key: &Key) -> Result<Lookup<T>, Error> {
        read(&self.inner, key, false)
    }

    /// Takes the lock for `key`, blocking until no other process or thread
    /// holds it.
    ///
    /// Hold it across "look up, fetch upstream, store" so concurrent callers
    /// for the same key fetch once.
    ///
    /// # Errors
    ///
    /// [`Error::Lock`] when a lock cannot be taken, [`Error::UnsafeRoot`] when
    /// a lock file is not private, and [`Error::Io`] when one cannot be opened.
    pub fn lock(&self, key: &Key) -> Result<KeyLock, Error> {
        private::validate_root(&self.inner.root)?;
        private::ensure_dir(&self.inner.root.join(LOCKS))?;
        let maintenance = private::open_lock(&self.inner.root.join(MAINTENANCE))?;
        maintenance.lock_shared().map_err(|_| Error::Lock)?;
        let held = private::lock_exclusive(&lock_path(&self.inner, key.digest()))?;
        Ok(KeyLock {
            inner: Arc::clone(&self.inner),
            key: key.clone(),
            _key: held,
            _maintenance: maintenance,
        })
    }

    /// Counts entry files and their bytes.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] when the entries directory cannot be listed.
    pub fn usage(&self) -> Result<Usage, Error> {
        usage(&self.inner)
    }

    /// Removes expired, invalid, and unused entries, and the temporary and
    /// lock files of processes that died. When the cache was over a hard cap,
    /// it then removes the least recently used entries until both targets
    /// hold.
    ///
    /// It removes a file only under its key's lock, taken without waiting, and
    /// skips a key someone holds; so it never blocks [`Cache::lock`] and may
    /// run while this thread holds a [`KeyLock`]. One prune runs at a time.
    ///
    /// # Errors
    ///
    /// [`Error::Lock`] when the maintenance lock cannot be taken, and
    /// [`Error::Io`] when a file cannot be removed.
    pub fn prune(&self) -> Result<Prune, Error> {
        prune(&self.inner, Wait::Block)
    }

    /// Removes every entry and lock file. It waits for every key lock, so a
    /// thread holding a [`KeyLock`] must drop it first.
    ///
    /// # Errors
    ///
    /// [`Error::Lock`] when the maintenance lock cannot be taken, and
    /// [`Error::Io`] when a file cannot be removed.
    pub fn clear(&self) -> Result<(), Error> {
        private::validate_root(&self.inner.root)?;
        let maintenance = private::open_lock(&self.inner.root.join(MAINTENANCE))?;
        maintenance.lock().map_err(|_| Error::Lock)?;
        let entries = self.inner.root.join(ENTRIES);
        private::ensure_dir(&entries)?;
        for item in fs::read_dir(&entries).map_err(|error| Error::io(&error))? {
            private::remove(&item.map_err(|error| Error::io(&error))?.path())?;
        }
        private::sync_dir(&entries)?;
        let locks = self.inner.root.join(LOCKS);
        private::ensure_dir(&locks)?;
        for item in fs::read_dir(&locks).map_err(|error| Error::io(&error))? {
            private::remove(&item.map_err(|error| Error::io(&error))?.path())?;
        }
        private::sync_dir(&locks)
    }
}

/// The lock on one key, and the right to store it.
///
/// Dropping it releases the lock.
pub struct KeyLock {
    inner: Arc<Inner>,
    key: Key,
    _key: private::Held,
    _maintenance: File,
}

impl fmt::Debug for KeyLock {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("KeyLock")
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

impl KeyLock {
    /// Reads the locked key. Another caller may have stored it while this one
    /// waited for the lock.
    ///
    /// A corrupt, mismatched, or expired entry is removed and reads as
    /// [`Lookup::Miss`].
    ///
    /// # Errors
    ///
    /// As [`Cache::lookup`].
    pub fn lookup<T: DeserializeOwned>(&self) -> Result<Lookup<T>, Error> {
        read(&self.inner, &self.key, true)
    }

    /// Stores `value` atomically under `policy`, then releases the lock.
    ///
    /// Store only a successful, normalized response: there is no negative
    /// caching. A reader sees the previous entry or this one, never a partial
    /// file. A write over a hard cap prunes before returning (see
    /// [`Cache::prune`]).
    ///
    /// # Errors
    ///
    /// [`Error::Encode`] when `value` cannot be serialized, and [`Error::Io`]
    /// when the entry cannot be written.
    pub fn store<T: Serialize + ?Sized>(self, value: &T, policy: Policy) -> Result<Stored, Error> {
        let now = self.inner.clock.now_ms();
        let fresh_until_ms = now.saturating_add(millis(policy.fresh));
        let entry = Written {
            format: FORMAT,
            schema: self.inner.schema,
            namespace: self.key.namespace(),
            digest: self.key.digest(),
            stored_at_ms: now,
            fresh_until_ms,
            stale_until_ms: fresh_until_ms.saturating_add(millis(policy.stale_if_error)),
            source: policy.source,
            value,
        };
        let bytes = serde_json::to_vec(&entry).map_err(|_| Error::Encode)?;
        let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if size > self.inner.limits.max_entry_bytes {
            return Ok(Stored::TooLarge { bytes: size });
        }
        write_atomic(&self.inner, self.key.digest(), &bytes)?;
        let inner = Arc::clone(&self.inner);
        drop(self);
        let maintenance = match usage(&inner) {
            Ok(usage)
                if usage.entries > inner.limits.hard_entries
                    || usage.bytes > inner.limits.hard_bytes
                    || sweep_due(&inner) =>
            {
                match prune(&inner, Wait::Skip) {
                    Ok(prune) => Maintenance::Pruned(prune),
                    Err(error) => Maintenance::Deferred(error),
                }
            }
            Ok(_) => Maintenance::NotNeeded,
            Err(error) => Maintenance::Deferred(error),
        };
        Ok(Stored::Written { maintenance })
    }
}

/// The entry as written. The value is borrowed, so storing never clones it.
#[derive(Serialize)]
struct Written<'a, T: ?Sized> {
    format: u8,
    schema: u32,
    namespace: &'a str,
    digest: &'a str,
    stored_at_ms: u64,
    fresh_until_ms: u64,
    stale_until_ms: u64,
    source: Source,
    value: &'a T,
}

/// Everything but the value, read first so an entry from a newer schema is
/// recognized before its value fails to parse.
#[derive(Deserialize)]
struct Header {
    format: u8,
    schema: u32,
    namespace: String,
    digest: String,
    stored_at_ms: u64,
    fresh_until_ms: u64,
    stale_until_ms: u64,
    source: Source,
}

impl Header {
    fn consistent(&self) -> bool {
        self.format == FORMAT
            && self.stored_at_ms <= self.fresh_until_ms
            && self.fresh_until_ms <= self.stale_until_ms
    }
}

#[derive(Deserialize)]
struct Value<T> {
    value: T,
}

fn read<T: DeserializeOwned>(inner: &Inner, key: &Key, locked: bool) -> Result<Lookup<T>, Error> {
    private::ensure_dir(&inner.root.join(ENTRIES))?;
    let path = entry_path(inner, key.digest());
    let Some((bytes, metadata)) = private::read(&path)? else {
        return Ok(discard(&path, locked));
    };
    let Ok(header) = serde_json::from_slice::<Header>(&bytes) else {
        return Ok(discard(&path, locked));
    };
    if header.schema > inner.schema {
        return Ok(Lookup::Miss);
    }
    if !header.consistent() || header.digest != key.digest() || header.namespace != key.namespace()
    {
        return Ok(discard(&path, locked));
    }
    let now = inner.clock.now_ms();
    if now > header.stale_until_ms {
        return Ok(discard(&path, locked));
    }
    let Ok(Value { value }) = serde_json::from_slice::<Value<T>>(&bytes) else {
        return Ok(discard(&path, locked));
    };
    if now.saturating_sub(modified_ms(&metadata)) > millis(inner.limits.touch_after) {
        let _ = private::touch(&path, at(now));
    }
    let cached = Cached {
        value,
        stored_at_ms: header.stored_at_ms,
        age_ms: now.saturating_sub(header.stored_at_ms),
        source: header.source,
    };
    Ok(
        if header.schema == inner.schema && now <= header.fresh_until_ms {
            Lookup::Fresh(cached)
        } else {
            Lookup::Stale(cached)
        },
    )
}

/// Removes the entry only under the key lock: unlocked, a writer may be
/// replacing it.
fn discard<T>(path: &Path, locked: bool) -> Lookup<T> {
    if locked {
        let _ = private::remove(path);
    }
    Lookup::Miss
}

fn write_atomic(inner: &Inner, digest: &str, bytes: &[u8]) -> Result<(), Error> {
    let entries = inner.root.join(ENTRIES);
    private::ensure_dir(&entries)?;
    let mut file = tempfile::Builder::new()
        .prefix(&format!(".{digest}."))
        .suffix(TEMPORARY_SUFFIX)
        .tempfile_in(&entries)
        .map_err(|error| Error::io(&error))?;
    file.write_all(bytes).map_err(|error| Error::io(&error))?;
    file.as_file()
        .set_modified(at(inner.clock.now_ms()))
        .map_err(|error| Error::io(&error))?;
    file.as_file()
        .sync_all()
        .map_err(|error| Error::io(&error))?;
    file.persist(entry_path(inner, digest))
        .map_err(|error| Error::io(&error.error))?;
    private::sync_dir(&entries)
}

#[derive(Clone, Copy)]
enum Wait {
    Block,
    Skip,
}

/// Holds the maintenance lock shared, so only [`Cache::clear`] excludes it,
/// and `prune.lock`, so one prune runs at a time. Each file goes only under
/// its key's lock, taken without waiting; a held key is skipped. A writer
/// holds its key's lock while its temporary file exists.
fn prune(inner: &Inner, wait: Wait) -> Result<Prune, Error> {
    private::validate_root(&inner.root)?;
    let maintenance = private::open_lock(&inner.root.join(MAINTENANCE))?;
    let pruning = private::open_lock(&inner.root.join(PRUNE_LOCK))?;
    match wait {
        Wait::Block => {
            maintenance.lock_shared().map_err(|_| Error::Lock)?;
            pruning.lock().map_err(|_| Error::Lock)?;
        }
        Wait::Skip => {
            maintenance.try_lock_shared().map_err(lock_error)?;
            pruning.try_lock().map_err(lock_error)?;
        }
    }
    let entries = inner.root.join(ENTRIES);
    private::ensure_dir(&entries)?;
    let mut outcome = Prune {
        locks_removed: remove_unheld_locks(inner)?,
        ..Prune::default()
    };
    let before = usage(inner)?;
    outcome.before_entries = before.entries;
    outcome.before_bytes = before.bytes;
    let mut left = before;
    let now = inner.clock.now_ms();
    let mut kept = Vec::new();
    for item in fs::read_dir(&entries).map_err(|error| Error::io(&error))? {
        let path = item.map_err(|error| Error::io(&error))?.path();
        if is_temporary(&path) {
            let held = temporary_digest(&path)
                .map(|digest| try_key(inner, digest))
                .transpose()?;
            if !matches!(held, Some(None)) {
                private::remove(&path)?;
                outcome.temporary_removed += 1;
            }
            continue;
        }
        let Some(digest) = entry_digest(&path) else {
            continue;
        };
        let Some(_held) = try_key(inner, digest)? else {
            continue;
        };
        let Some(metadata) = private::regular(&path)? else {
            continue;
        };
        let used_ms = modified_ms(&metadata);
        let removed = if !live(&path, now, inner.limits.max_entry_bytes) {
            &mut outcome.expired_removed
        } else if now.saturating_sub(used_ms) > millis(inner.limits.unused_after) {
            &mut outcome.unused_removed
        } else {
            kept.push((used_ms, digest.to_owned(), metadata.len()));
            continue;
        };
        private::remove(&path)?;
        *removed += 1;
        left.remove(metadata.len());
    }
    if before.entries > inner.limits.hard_entries || before.bytes > inner.limits.hard_bytes {
        kept.sort();
        let target_entries = inner.limits.target_entries.min(inner.limits.hard_entries);
        let target_bytes = inner.limits.target_bytes.min(inner.limits.hard_bytes);
        for (used_ms, digest, size) in kept {
            if left.entries <= target_entries && left.bytes <= target_bytes {
                break;
            }
            let Some(_held) = try_key(inner, &digest)? else {
                continue;
            };
            let path = entry_path(inner, &digest);
            if private::regular(&path)?.is_none_or(|metadata| modified_ms(&metadata) != used_ms) {
                continue;
            }
            private::remove(&path)?;
            outcome.capacity_removed += 1;
            left.remove(size);
        }
    }
    private::sync_dir(&entries)?;
    pruning
        .set_modified(at(now))
        .map_err(|error| Error::io(&error))?;
    outcome.after_entries = left.entries;
    outcome.after_bytes = left.bytes;
    Ok(outcome)
}

fn lock_error(error: TryLockError) -> Error {
    match error {
        TryLockError::WouldBlock => Error::Lock,
        TryLockError::Error(error) => Error::io(&error),
    }
}

fn try_key(inner: &Inner, digest: &str) -> Result<Option<private::Held>, Error> {
    private::try_lock_exclusive(&lock_path(inner, digest))
}

/// A lock file nobody holds belongs to a process that died, or to a waiter
/// about to retry on the path's new file.
fn remove_unheld_locks(inner: &Inner) -> Result<u64, Error> {
    let locks = inner.root.join(LOCKS);
    private::ensure_dir(&locks)?;
    let mut removed = 0;
    for item in fs::read_dir(&locks).map_err(|error| Error::io(&error))? {
        let path = item.map_err(|error| Error::io(&error))?.path();
        if private::regular(&path)?.is_some() && private::try_lock_exclusive(&path)?.is_some() {
            removed += 1;
        }
    }
    private::sync_dir(&locks)?;
    Ok(removed)
}

fn live(path: &Path, now: u64, limit: u64) -> bool {
    read_header(path, limit).is_some_and(|header| {
        header.consistent()
            && entry_digest(path) == Some(header.digest.as_str())
            && now <= header.stale_until_ms
    })
}

/// Reads up to the `value` key and no further. `Written` serializes `value`
/// last, and a quote inside a string is escaped, so the first `,"value":` is
/// the key. A corrupt value is left for [`KeyLock::lookup`] to find. It reads
/// at most `limit` bytes, the most a stored entry can hold.
fn read_header(path: &Path, limit: u64) -> Option<Header> {
    const VALUE_KEY: &[u8] = b",\"value\":";
    let mut reader = BufReader::new(File::open(path).ok()?.take(limit));
    let mut prefix = Vec::new();
    loop {
        let chunk = reader.fill_buf().ok()?;
        if chunk.is_empty() {
            return None;
        }
        let read = chunk.len();
        let start = prefix.len().saturating_sub(VALUE_KEY.len() - 1);
        prefix.extend_from_slice(chunk);
        reader.consume(read);
        if let Some(end) = prefix[start..]
            .windows(VALUE_KEY.len())
            .position(|window| window == VALUE_KEY)
        {
            prefix.truncate(start + end);
            prefix.push(b'}');
            return serde_json::from_slice(&prefix).ok();
        }
    }
}

/// `prune.lock`'s modification time records the last prune, on the cache's
/// clock.
fn sweep_due(inner: &Inner) -> bool {
    fs::symlink_metadata(inner.root.join(PRUNE_LOCK)).is_ok_and(|metadata| {
        inner.clock.now_ms().saturating_sub(modified_ms(&metadata))
            > millis(inner.limits.sweep_every)
    })
}

/// Entry and `prune.lock` modification times are set from the cache's clock,
/// so they compare with [`Clock::now_ms`].
fn modified_ms(metadata: &Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, millis)
}

fn at(ms: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(ms)
}

fn usage(inner: &Inner) -> Result<Usage, Error> {
    let entries = inner.root.join(ENTRIES);
    private::ensure_dir(&entries)?;
    let mut usage = Usage::default();
    for item in fs::read_dir(&entries).map_err(|error| Error::io(&error))? {
        let item = item.map_err(|error| Error::io(&error))?;
        let path = item.path();
        if entry_digest(&path).is_none() {
            continue;
        }
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => return Err(Error::io(&error)),
        };
        if metadata.file_type().is_file() {
            usage.entries += 1;
            usage.bytes = usage.bytes.saturating_add(metadata.len());
        }
    }
    Ok(usage)
}

fn entry_path(inner: &Inner, digest: &str) -> PathBuf {
    inner.root.join(ENTRIES).join(format!("{digest}.json"))
}

fn lock_path(inner: &Inner, digest: &str) -> PathBuf {
    inner.root.join(LOCKS).join(format!("{digest}.lock"))
}

fn entry_digest(path: &Path) -> Option<&str> {
    path.file_name()?
        .to_str()?
        .strip_suffix(".json")
        .filter(|digest| digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

/// A temporary file is named `.<digest>.<random>.tmp` by its writer.
fn temporary_digest(path: &Path) -> Option<&str> {
    let name = path.file_name()?.to_str()?;
    let digest = name.strip_prefix('.')?.split('.').next()?;
    (digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())).then_some(digest)
}

fn is_temporary(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with('.') && name.ends_with(TEMPORARY_SUFFIX))
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}
