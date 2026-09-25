use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;
use std::{fmt, io};

use secrecy::zeroize::Zeroize;
use secrecy::{ExposeSecret as _, SecretBox};
use serde::Deserialize;
use tokio::time::Instant;

/// When a secret leaves memory. A consumer embeds these in its own config and
/// writes durations as `10m` or `2h`; `0s` turns a bound off, and an unknown
/// field is an error.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SecretLimits {
    /// Drop a secret nobody read for this long. 10 minutes.
    #[serde(deserialize_with = "humantime_serde::deserialize")]
    pub idle: Duration,
    /// Drop a secret this long after it was stored, however often it is read.
    /// 2 hours.
    #[serde(deserialize_with = "humantime_serde::deserialize")]
    pub max_age: Duration,
    /// How often the store looks for expired secrets that nobody reads. 1
    /// minute.
    #[serde(deserialize_with = "humantime_serde::deserialize")]
    pub sweep_every: Duration,
}

impl Default for SecretLimits {
    fn default() -> Self {
        Self {
            idle: Duration::from_mins(10),
            max_age: Duration::from_hours(2),
            sweep_every: Duration::from_mins(1),
        }
    }
}

/// Secrets held in the daemon's memory only, never written anywhere. Each is
/// zeroed when it expires, is removed, or the store is dropped, and its value
/// is reachable only inside [`SecretStore::get`].
pub struct SecretStore<S: Zeroize> {
    limits: SecretLimits,
    secrets: Mutex<HashMap<String, Held<S>>>,
}

struct Held<S: Zeroize> {
    /// Shared so [`SecretStore::get`] can read it with the map unlocked; the
    /// value is zeroed when the last reader lets go.
    secret: Arc<SecretBox<S>>,
    stored: Instant,
    read: Instant,
    idle: Duration,
    max_age: Duration,
}

impl<S: Zeroize> Held<S> {
    fn expired(&self, now: Instant) -> bool {
        let past = |since: Instant, bound: Duration| !bound.is_zero() && now - since >= bound;
        past(self.read, self.idle) || past(self.stored, self.max_age)
    }
}

impl<S: Zeroize> fmt::Debug for SecretStore<S> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecretStore")
            .field("secrets", &self.lock().len())
            .finish_non_exhaustive()
    }
}

impl<S: Zeroize + Send + Sync + 'static> SecretStore<S> {
    /// Hardens the process and starts a store that sweeps expired secrets
    /// every [`SecretLimits::sweep_every`] until it is dropped.
    ///
    /// Hardening turns core dumps off, and on Linux marks the process not
    /// dumpable, which also keeps a debugger of the same user out. It applies
    /// to the whole process and lasts until it exits. It runs inside a Tokio
    /// runtime.
    ///
    /// # Errors
    ///
    /// When the process cannot be hardened.
    pub fn new(limits: SecretLimits) -> io::Result<Arc<Self>> {
        harden()?;
        let store = Arc::new(Self {
            limits,
            secrets: Mutex::default(),
        });
        if !limits.sweep_every.is_zero() {
            let every = limits.sweep_every;
            let ticks = tokio::time::interval_at(Instant::now() + every, every);
            tokio::spawn(sweep(Arc::downgrade(&store), ticks));
        }
        Ok(store)
    }
}

impl<S: Zeroize> SecretStore<S> {
    /// Stores `secret` under `key` with the store's limits, replacing and
    /// zeroing any secret already there.
    pub fn put(&self, key: impl Into<String>, secret: SecretBox<S>) {
        self.put_with(key, secret, self.limits.idle, self.limits.max_age);
    }

    /// As [`SecretStore::put`], with this secret's own idle and maximum
    /// lifetimes; `Duration::ZERO` turns a bound off.
    pub fn put_with(
        &self,
        key: impl Into<String>,
        secret: SecretBox<S>,
        idle: Duration,
        max_age: Duration,
    ) {
        let now = Instant::now();
        self.lock().insert(
            key.into(),
            Held {
                secret: Arc::new(secret),
                stored: now,
                read: now,
                idle,
                max_age,
            },
        );
    }

    /// Runs `read` on the secret under `key` and returns its result, or
    /// `None` when there is none or it expired. A read restarts the idle
    /// lifetime. The store is unlocked while `read` runs, so it may use the
    /// store.
    pub fn get<R>(&self, key: &str, read: impl FnOnce(&S) -> R) -> Option<R> {
        let now = Instant::now();
        let mut secrets = self.lock();
        if secrets.get(key)?.expired(now) {
            secrets.remove(key);
            return None;
        }
        let held = secrets.get_mut(key)?;
        held.read = now;
        let secret = Arc::clone(&held.secret);
        drop(secrets);
        Some(read(secret.expose_secret()))
    }

    /// Removes and zeroes the secret under `key`.
    pub fn remove(&self, key: &str) {
        self.lock().remove(key);
    }

    /// Removes and zeroes every secret.
    pub fn clear(&self) {
        self.lock().clear();
    }

    fn sweep(&self) {
        let now = Instant::now();
        self.lock().retain(|_, held| !held.expired(now));
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Held<S>>> {
        self.secrets.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

async fn sweep<S: Zeroize>(store: Weak<SecretStore<S>>, mut ticks: tokio::time::Interval) {
    loop {
        ticks.tick().await;
        let Some(store) = store.upgrade() else {
            return;
        };
        store.sweep();
    }
}

fn harden() -> io::Result<()> {
    use rustix::process::{Resource, Rlimit, setrlimit};
    setrlimit(
        Resource::Core,
        Rlimit {
            current: Some(0),
            maximum: Some(0),
        },
    )?;
    #[cfg(target_os = "linux")]
    rustix::process::set_dumpable_behavior(rustix::process::DumpableBehavior::NotDumpable)?;
    Ok(())
}
