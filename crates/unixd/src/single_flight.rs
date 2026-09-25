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

    fn start<F>(&self, key: K, work: F) -> Flight<T, E>
    where
        F: Future<Output = Result<T, E>> + Send + 'static,
    {
        let flights = Arc::clone(&self.flights);
        let task = tokio::spawn(async move {
            let result = work.await;
            flights
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&key);
            result
        });
        async move {
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
