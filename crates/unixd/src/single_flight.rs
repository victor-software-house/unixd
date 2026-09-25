use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::hash::Hash;
use std::sync::{Arc, Mutex, PoisonError};

use futures_util::FutureExt as _;
use futures_util::future::{BoxFuture, Shared};

type Flight<T, E> = Shared<BoxFuture<'static, Result<T, FlightError<E>>>>;

/// Runs work once for concurrent calls with the same key. The work runs as
/// its own task, so a caller that goes away does not cancel it for the
/// others. A finished flight is forgotten at once; the disk cache, not this
/// map, keeps results.
pub struct SingleFlight<K, T, E> {
    flights: Arc<Mutex<HashMap<K, Flight<T, E>>>>,
}

/// Why a shared run failed. Every caller of the flight gets the same error.
#[derive(Debug)]
pub enum FlightError<E> {
    /// The work returned this error.
    Failed(Arc<E>),
    /// The work panicked.
    Panicked,
}

impl<E> Clone for FlightError<E> {
    fn clone(&self) -> Self {
        match self {
            Self::Failed(error) => Self::Failed(Arc::clone(error)),
            Self::Panicked => Self::Panicked,
        }
    }
}

impl<E: fmt::Display> fmt::Display for FlightError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Failed(error) => error.fmt(formatter),
            Self::Panicked => formatter.write_str("the shared work panicked"),
        }
    }
}

/// Forgets the flight when its task ends, whether the work returned,
/// panicked, or was cancelled, so no later call joins a dead flight.
struct Landed<K: Eq + Hash, T, E> {
    flights: Arc<Mutex<HashMap<K, Flight<T, E>>>>,
    key: Option<K>,
}

impl<K: Eq + Hash, T, E> Drop for Landed<K, T, E> {
    fn drop(&mut self) {
        if let Some(key) = self.key.take() {
            self.flights
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&key);
        }
    }
}

impl<K, T, E> Default for SingleFlight<K, T, E> {
    fn default() -> Self {
        Self {
            flights: Arc::default(),
        }
    }
}

impl<K, T, E> fmt::Debug for SingleFlight<K, T, E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SingleFlight")
    }
}

impl<K, T, E> SingleFlight<K, T, E>
where
    K: Eq + Hash + Clone + Send + 'static,
    T: Clone + Send + Sync + 'static,
    E: Send + Sync + 'static,
{
    /// A map with no flight in it.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Joins the flight for `key`, or starts one with `work` when none runs.
    /// `work` is called with the map locked, so it only builds the future and
    /// must not call this `SingleFlight`.
    ///
    /// # Errors
    ///
    /// The flight's error, shared with every caller that joined it.
    pub async fn run<W, F>(&self, key: K, work: W) -> Result<T, FlightError<E>>
    where
        W: FnOnce() -> F,
        F: Future<Output = Result<T, E>> + Send + 'static,
    {
        let flight = {
            let mut flights = self.flights.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(flight) = flights.get(&key) {
                flight.clone()
            } else {
                let flight = self.start(key.clone(), work());
                flights.insert(key, flight.clone());
                flight
            }
        };
        flight.await
    }

    /// The task starts on the flight's first poll, after [`SingleFlight::run`]
    /// has released the map, so no drop of `Landed` can find the map locked
    /// by its own thread.
    fn start<F>(&self, key: K, work: F) -> Flight<T, E>
    where
        F: Future<Output = Result<T, E>> + Send + 'static,
    {
        let landed = Landed {
            flights: Arc::clone(&self.flights),
            key: Some(key),
        };
        async move {
            let task = tokio::spawn(async move {
                let _landed = landed;
                work.await
            });
            match task.await {
                Ok(Ok(value)) => Ok(value),
                Ok(Err(error)) => Err(FlightError::Failed(Arc::new(error))),
                Err(_) => Err(FlightError::Panicked),
            }
        }
        .boxed()
        .shared()
    }
}
