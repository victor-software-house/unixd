use std::fs::{self, File, TryLockError};
use std::io::{BufReader, ErrorKind, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::{fmt, path};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::key::FORMAT;
use crate::{Error, Key, Policy, Source, private};

const ENTRIES: &str = "entries";
const LOCKS: &str = "locks";
const MAINTENANCE: &str = "maintenance.lock";
const PRUNE_PENDING: &str = "prune.pending";
const TEMPORARY_SUFFIX: &str = ".tmp";

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

/// Size bounds. A write that takes the cache over a hard cap prunes it down to
/// the targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
}

impl Default for Limits {
    /// 8 MiB per entry, and 10,000 entries or 256 MiB before pruning to 8,000
    /// entries and 200 MiB.
    fn default() -> Self {
        Self {
            max_entry_bytes: 8 * 1024 * 1024,
            hard_entries: 10_000,
            hard_bytes: 256 * 1024 * 1024,
            target_entries: 8_000,
            target_bytes: 200 * 1024 * 1024,
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
    /// The cache was within its hard caps.
    NotNeeded,
    /// The write crossed a hard cap and the cache was pruned.
    Pruned(Prune),
    /// The cache needs pruning but it failed. The write itself succeeded.
    ///
    /// [`Error::Lock`] means a key lock was held, so the prune was skipped
    /// rather than waited for. A marker file records the skipped prune, and
    /// every later [`Cache::lock`] tries it again before taking its own locks.
    /// While key locks overlap without a gap, the cache can pass its hard
    /// caps; a service that expects that load should call [`Cache::prune`]
    /// on a schedule from a thread that holds no key lock.
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
    /// Unexpired entries removed, oldest first, to reach the targets.
    pub capacity_removed: u64,
    /// Temporary files left by writers that died mid-write.
    pub temporary_removed: u64,
    /// Lock files whose entry no longer exists.
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
    /// While two schema versions share a root, each binary overwrites the
    /// other's entry on its next store of that key, at the cost of one
    /// upstream fetch per switch. The older binary never reads a newer
    /// entry. The newer binary reads an older entry as stale while it is
    /// inside its stale horizon, so it serves it only after an upstream
    /// failure that serves stale.
    ///
    /// `root` is made absolute once, here, so a later change of working
    /// directory cannot move the cache.
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
    /// A corrupt, mismatched, or expired entry reads as [`Lookup::Miss`] and is
    /// left in place: a writer holding the key lock may replace it at any
    /// moment, so only [`KeyLock::lookup`] and [`Cache::prune`] remove it.
    /// Such an entry still counts toward [`Cache::usage`], so the hard caps
    /// bound it. A value that no longer deserializes into `T` counts as
    /// corrupt.
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
    /// [`Error::Lock`] when a lock cannot be taken, and [`Error::UnsafeRoot`]
    /// when a lock file is not private.
    pub fn lock(&self, key: &Key) -> Result<KeyLock, Error> {
        private::validate_root(&self.inner.root)?;
        retry_pending_prune(&self.inner);
        private::ensure_dir(&self.inner.root.join(LOCKS))?;
        let maintenance = private::open_lock(&self.inner.root.join(MAINTENANCE))?;
        maintenance.lock_shared().map_err(|_| Error::Lock)?;
        let key_file = private::open_lock(
            &self
                .inner
                .root
                .join(LOCKS)
                .join(format!("{}.lock", key.digest())),
        )?;
        key_file.lock().map_err(|_| Error::Lock)?;
        Ok(KeyLock {
            inner: Arc::clone(&self.inner),
            key: key.clone(),
            _maintenance: maintenance,
            _key: key_file,
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

    /// Removes expired and invalid entries and leftover temporary files.
    /// It opens and reads every entry while it holds the maintenance lock, so
    /// every [`Cache::lock`] waits for it; [`Cache::lookup`] does not. The
    /// pause grows with the number and size of entries: normally near
    /// `hard_entries` and `hard_bytes`, but past them while prunes are
    /// deferred (see [`Maintenance::Deferred`]). When the
    /// cache was over a hard cap, it then removes the oldest entries until
    /// both targets hold.
    ///
    /// It also removes lock files whose entry is gone. It waits for every
    /// holder of a key lock to finish, so a thread that holds a [`KeyLock`]
    /// must drop it before calling this.
    ///
    /// # Errors
    ///
    /// [`Error::Lock`] when the maintenance lock cannot be taken, and
    /// [`Error::Io`] when a file cannot be removed.
    pub fn prune(&self) -> Result<Prune, Error> {
        prune(&self.inner, Wait::Block)
    }

    /// Removes every entry and lock file. It waits for every holder of a key
    /// lock to finish, so a thread that holds a [`KeyLock`] must drop it
    /// before calling this.
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
    _maintenance: File,
    _key: File,
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
    /// file. When the write takes the cache over a hard cap, the cache is
    /// pruned after the lock is released. That prune does not wait: while any
    /// key lock is held, in this process or another, it reports
    /// [`Maintenance::Deferred`] and the next write over the cap tries again.
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
                    || usage.bytes > inner.limits.hard_bytes =>
            {
                match prune(&inner, Wait::Skip) {
                    Ok(prune) => Maintenance::Pruned(prune),
                    Err(error) => {
                        let _ = private::open_lock(&inner.root.join(PRUNE_PENDING));
                        Maintenance::Deferred(error)
                    }
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
    let Some(bytes) = private::read(&path)? else {
        return Ok(Lookup::Miss);
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

/// Reports a miss for an unusable entry, and removes it when the key lock is
/// held. Removal is best effort: a failure leaves a file the next prune
/// removes.
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
        .sync_all()
        .map_err(|error| Error::io(&error))?;
    file.persist(entry_path(inner, digest))
        .map_err(|error| Error::io(&error.error))?;
    private::sync_dir(&entries)
}

/// Whether a prune waits for the exclusive maintenance lock.
#[derive(Clone, Copy)]
enum Wait {
    /// Block until every key lock is released.
    Block,
    /// Give up with [`Error::Lock`] when any key lock is held.
    Skip,
}

fn prune(inner: &Inner, wait: Wait) -> Result<Prune, Error> {
    private::validate_root(&inner.root)?;
    let maintenance = private::open_lock(&inner.root.join(MAINTENANCE))?;
    match wait {
        Wait::Block => maintenance.lock().map_err(|_| Error::Lock)?,
        Wait::Skip => maintenance.try_lock().map_err(|error| match error {
            TryLockError::WouldBlock => Error::Lock,
            TryLockError::Error(error) => Error::io(&error),
        })?,
    }
    // Cleared before the scan, so a writer that skips its prune during the
    // scan leaves a fresh marker. A prune that fails is not retried by every
    // lock(); the next write over a hard cap prunes again.
    let _ = private::remove(&inner.root.join(PRUNE_PENDING));
    let entries = inner.root.join(ENTRIES);
    private::ensure_dir(&entries)?;
    let before = usage(inner)?;
    let now = inner.clock.now_ms();
    let mut outcome = Prune {
        before_entries: before.entries,
        before_bytes: before.bytes,
        ..Prune::default()
    };
    let mut kept = Vec::new();
    for item in fs::read_dir(&entries).map_err(|error| Error::io(&error))? {
        let path = item.map_err(|error| Error::io(&error))?.path();
        if is_temporary(&path) {
            private::remove(&path)?;
            outcome.temporary_removed += 1;
            continue;
        }
        if entry_digest(&path).is_none() {
            continue;
        }
        let Some(metadata) = private::regular(&path)? else {
            continue;
        };
        if let Some(stored_at_ms) = live_stored_at(&path, now) {
            kept.push((stored_at_ms, path, metadata.len()));
        } else {
            private::remove(&path)?;
            outcome.expired_removed += 1;
        }
    }
    let mut entries_left = u64::try_from(kept.len()).unwrap_or(u64::MAX);
    let mut bytes_left = kept
        .iter()
        .fold(0_u64, |total, (_, _, size)| total.saturating_add(*size));
    if before.entries > inner.limits.hard_entries || before.bytes > inner.limits.hard_bytes {
        kept.sort();
        for (_, path, size) in kept {
            if entries_left <= inner.limits.target_entries
                && bytes_left <= inner.limits.target_bytes
            {
                break;
            }
            private::remove(&path)?;
            entries_left -= 1;
            bytes_left = bytes_left.saturating_sub(size);
            outcome.capacity_removed += 1;
        }
    }
    private::sync_dir(&entries)?;
    outcome.locks_removed = remove_orphan_locks(inner)?;
    outcome.after_entries = entries_left;
    outcome.after_bytes = bytes_left;
    Ok(outcome)
}

/// Runs a skipped prune if a marker records one, without waiting for locks.
///
/// Failure is not reported. While the maintenance lock is busy the marker
/// stays and the next call tries again. Once a prune starts, it clears the
/// marker, and a later write over a hard cap records a new one if needed.
fn retry_pending_prune(inner: &Inner) {
    if fs::symlink_metadata(inner.root.join(PRUNE_PENDING)).is_ok() {
        let _ = prune(inner, Wait::Skip);
    }
}

/// Removes lock files whose entry is gone.
///
/// Called only under the exclusive maintenance lock. Every key lock is taken
/// after a shared maintenance lock, so no process holds or is opening a key
/// lock file here.
fn remove_orphan_locks(inner: &Inner) -> Result<u64, Error> {
    let locks = inner.root.join(LOCKS);
    private::ensure_dir(&locks)?;
    let mut removed = 0;
    for item in fs::read_dir(&locks).map_err(|error| Error::io(&error))? {
        let path = item.map_err(|error| Error::io(&error))?.path();
        let entry = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_suffix(".lock"))
            .map(|digest| entry_path(inner, digest));
        if entry.is_none_or(|entry| !entry.exists()) {
            private::remove(&path)?;
            removed += 1;
        }
    }
    private::sync_dir(&locks)?;
    Ok(removed)
}

/// When a still-usable entry was stored, or `None` when it is expired,
/// corrupt, or not the entry its file name claims.
///
/// The header is read through a buffered reader and the value is skipped
/// without being kept, so memory stays small whatever the entry size.
fn live_stored_at(path: &Path, now: u64) -> Option<u64> {
    let file = File::open(path).ok()?;
    let header = serde_json::from_reader::<_, Header>(BufReader::new(file)).ok()?;
    (header.consistent()
        && entry_digest(path) == Some(header.digest.as_str())
        && now <= header.stale_until_ms)
        .then_some(header.stored_at_ms)
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

/// The digest an entry file is named after, or `None` for any other file.
fn entry_digest(path: &Path) -> Option<&str> {
    path.file_name()?
        .to_str()?
        .strip_suffix(".json")
        .filter(|digest| digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn is_temporary(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with('.') && name.ends_with(TEMPORARY_SUFFIX))
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}
