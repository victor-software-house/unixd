//! Single flight: shared runs, distinct keys, shared failures, and a leader
//! that goes away.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use unixd::{FlightError, SingleFlight};

type Flights = Arc<SingleFlight<&'static str, u32, &'static str>>;

/// Counts its runs, waits, and then gives `result`.
fn work(
    runs: &Arc<AtomicUsize>,
    millis: u64,
    result: Result<u32, &'static str>,
) -> impl FnOnce() -> std::pin::Pin<Box<dyn Future<Output = Result<u32, &'static str>> + Send>> + use<>
{
    let runs = Arc::clone(runs);
    move || {
        Box::pin(async move {
            runs.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(millis)).await;
            result
        })
    }
}

#[tokio::test]
async fn ten_identical_calls_share_one_run() {
    let flights = Flights::default();
    let runs = Arc::new(AtomicUsize::new(0));
    let calls: Vec<_> = (0..10)
        .map(|_| {
            let flights = Arc::clone(&flights);
            let work = work(&runs, 200, Ok(7));
            tokio::spawn(async move { flights.run("key", work).await })
        })
        .collect();
    for call in calls {
        assert_eq!(call.await.unwrap().unwrap(), 7);
    }
    assert_eq!(runs.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn distinct_keys_run_at_the_same_time() {
    let flights = Flights::default();
    let runs = Arc::new(AtomicUsize::new(0));
    let started = Instant::now();
    let (a, b) = tokio::join!(
        flights.run("a", work(&runs, 200, Ok(1))),
        flights.run("b", work(&runs, 200, Ok(2)))
    );
    assert_eq!((a.unwrap(), b.unwrap()), (1, 2));
    assert_eq!(runs.load(Ordering::SeqCst), 2);
    assert!(started.elapsed() < Duration::from_millis(350));
}

#[tokio::test]
async fn every_caller_gets_the_shared_failure_and_nothing_is_kept() {
    let flights = Flights::default();
    let runs = Arc::new(AtomicUsize::new(0));
    let (a, b, c) = tokio::join!(
        flights.run("key", work(&runs, 100, Err("timeout"))),
        flights.run("key", work(&runs, 100, Err("timeout"))),
        flights.run("key", work(&runs, 100, Err("timeout")))
    );
    for result in [a, b, c] {
        let Err(FlightError::Failed(error)) = result else {
            panic!("expected the shared failure");
        };
        assert_eq!(*error, "timeout");
    }
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    assert_eq!(flights.run("key", work(&runs, 0, Ok(3))).await.unwrap(), 3);
    assert_eq!(runs.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_dropped_leader_does_not_cancel_the_work() {
    let flights = Flights::default();
    let runs = Arc::new(AtomicUsize::new(0));
    let leader = {
        let flights = Arc::clone(&flights);
        let work = work(&runs, 200, Ok(9));
        tokio::spawn(async move { flights.run("key", work).await })
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    leader.abort();
    assert_eq!(
        flights.run("key", work(&runs, 200, Ok(0))).await.unwrap(),
        9
    );
    assert_eq!(runs.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_panicked_flight_is_forgotten() {
    let flights = Flights::default();
    let result = flights.run("key", || async { panic!("work failed") }).await;
    assert!(matches!(result, Err(FlightError::Panicked)));
    let runs = Arc::new(AtomicUsize::new(0));
    assert_eq!(flights.run("key", work(&runs, 0, Ok(4))).await.unwrap(), 4);
}
