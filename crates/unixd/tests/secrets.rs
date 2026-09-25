//! The secret store on Tokio's paused clock: lifetimes, zeroing, redaction,
//! and hardening.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::time::advance;
use unixd::{SecretBox, SecretLimits, SecretStore, Zeroize};

fn secret(text: &str) -> SecretBox<String> {
    SecretBox::new(Box::new(text.to_owned()))
}

fn read(store: &SecretStore<String>, key: &str) -> Option<String> {
    store.get(key, Clone::clone)
}

/// Records that it was zeroed.
struct Probe(Arc<AtomicBool>);

impl Zeroize for Probe {
    fn zeroize(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[tokio::test(start_paused = true)]
async fn an_unread_secret_expires_after_the_idle_lifetime() {
    let store = SecretStore::new(SecretLimits::default()).unwrap();
    store.put("token", secret("s3cret"));
    advance(Duration::from_mins(9)).await;
    assert_eq!(read(&store, "token").as_deref(), Some("s3cret"));
    advance(Duration::from_mins(11)).await;
    assert_eq!(read(&store, "token"), None);
}

#[tokio::test(start_paused = true)]
async fn a_secret_read_often_still_expires_at_its_maximum_age() {
    let store = SecretStore::new(SecretLimits::default()).unwrap();
    store.put("token", secret("s3cret"));
    for _ in 0..23 {
        advance(Duration::from_mins(5)).await;
        assert!(read(&store, "token").is_some());
    }
    advance(Duration::from_mins(6)).await;
    assert_eq!(read(&store, "token"), None);
}

#[tokio::test(start_paused = true)]
async fn a_secret_without_bounds_stays() {
    let store = SecretStore::new(SecretLimits::default()).unwrap();
    store.put_with("token", secret("s3cret"), Duration::ZERO, Duration::ZERO);
    advance(Duration::from_hours(24 * 30)).await;
    assert_eq!(read(&store, "token").as_deref(), Some("s3cret"));
}

#[tokio::test(start_paused = true)]
async fn removed_and_swept_secrets_are_zeroed() {
    let store = SecretStore::new(SecretLimits::default()).unwrap();
    let removed = Arc::new(AtomicBool::new(false));
    let swept = Arc::new(AtomicBool::new(false));
    store.put(
        "removed",
        SecretBox::new(Box::new(Probe(Arc::clone(&removed)))),
    );
    store.put("swept", SecretBox::new(Box::new(Probe(Arc::clone(&swept)))));
    store.remove("removed");
    assert!(removed.load(Ordering::SeqCst));
    tokio::time::sleep(Duration::from_mins(12)).await;
    assert!(swept.load(Ordering::SeqCst));
}

#[tokio::test]
async fn a_secret_never_prints() {
    let store = SecretStore::new(SecretLimits::default()).unwrap();
    store.put("token", secret("s3cret"));
    assert!(!format!("{store:?} {:?}", secret("s3cret")).contains("s3cret"));
}

#[tokio::test]
async fn a_store_turns_core_dumps_off() {
    let _store = SecretStore::<String>::new(SecretLimits::default()).unwrap();
    let limit = rustix::process::getrlimit(rustix::process::Resource::Core);
    assert_eq!((limit.current, limit.maximum), (Some(0), Some(0)));
    #[cfg(target_os = "linux")]
    assert_eq!(
        rustix::process::dumpable_behavior().unwrap(),
        rustix::process::DumpableBehavior::NotDumpable
    );
}

#[test]
fn limits_read_from_a_config_with_defaults_for_the_rest() {
    let limits: SecretLimits = serde_json::from_str(r#"{"idle": "5m"}"#).unwrap();
    assert_eq!(limits.idle, Duration::from_mins(5));
    assert_eq!(limits.max_age, Duration::from_hours(2));
    assert!(serde_json::from_str::<SecretLimits>(r#"{"idel": "5m"}"#).is_err());
}
