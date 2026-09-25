//! Units under the real service manager. The tests that install are ignored
//! by default, since they change the user's launchd or systemd state; run
//! them on each platform with the echo example built and its path in
//! `UNIXD_ECHO_BIN`:
//!
//! ```text
//! cargo build --example echo
//! UNIXD_ECHO_BIN=target/debug/examples/echo cargo nextest run --run-ignored only
//! ```

#![expect(
    clippy::unwrap_used,
    reason = "setup helpers outside #[test] functions panic on failure like the tests do"
)]

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;
use std::{env, fs, process, thread};

use serde_json::Value;
use unixd::{Client, InstallError, Service, Version, install, uninstall};

const V1: Version = Version { major: 1, minor: 0 };

fn echo() -> PathBuf {
    let path = env::var_os("UNIXD_ECHO_BIN").map(PathBuf::from).unwrap();
    fs::canonicalize(path).unwrap()
}

/// Returns the pid of the daemon that answered.
fn pid(service: &Service) -> u64 {
    let reply: Value = Client::new(service.socket_path().unwrap(), V1)
        .with_deadline(Duration::from_secs(10))
        .call(&"hello")
        .unwrap();
    reply["pid"].as_u64().unwrap()
}

/// Uninstalls when the test ends, even by a panic, so a failed run leaves no
/// unit behind.
struct Installed(Service);

impl Drop for Installed {
    fn drop(&mut self) {
        let _ = uninstall(&self.0);
    }
}

fn service(test: &str) -> Installed {
    Installed(Service::new(format!("unixd-{test}-{}", process::id()), V1))
}

#[test]
#[ignore = "installs a real unit"]
fn install_twice_serve_uninstall_and_install_again() {
    let installed = service("round");
    let service = &installed.0;
    install(service, &echo(), &[]).unwrap();
    install(service, &echo(), &[]).unwrap();
    pid(service);
    uninstall(service).unwrap();
    assert!(!service.socket_path().unwrap().parent().unwrap().exists());
    install(service, &echo(), &[]).unwrap();
    pid(service);
    uninstall(service).unwrap();
    uninstall(service).unwrap();
}

#[test]
#[ignore = "installs a real unit"]
fn the_socket_outlives_an_idle_exit_and_a_removed_client_directory() {
    let installed = service("idle");
    let service = &installed.0;
    install(service, &echo(), &[OsString::from("1")]).unwrap();
    let scratch = tempfile::tempdir().unwrap();
    env::set_current_dir(scratch.path()).unwrap();
    let first = pid(service);
    drop(scratch);
    assert_eq!(pid(service), first);
    thread::sleep(Duration::from_millis(2500));
    assert!(service.socket_path().unwrap().exists());
    assert_ne!(pid(service), first);
    uninstall(service).unwrap();
}

/// Runs itself in a child process with fixed base directories, since a test
/// may not set the environment of its own process.
#[test]
fn a_socket_path_over_the_limit_is_refused_before_any_write() {
    const NAME: &str = "a_socket_path_over_the_limit_is_refused_before_any_write";
    if env::var_os("UNIXD_INSTALL_CHILD").is_some() {
        let service = Service::new("x".repeat(120), V1);
        let result = install(&service, &PathBuf::from("/bin/false"), &[]);
        assert!(
            matches!(result, Err(InstallError::SocketPathTooLong { .. })),
            "{result:?}"
        );
        return;
    }
    let base = tempfile::tempdir().unwrap();
    let output = Command::new(env::current_exe().unwrap())
        .args(["--exact", NAME])
        .env("UNIXD_INSTALL_CHILD", "1")
        .env("HOME", base.path())
        .env("XDG_RUNTIME_DIR", base.path())
        .env("XDG_CONFIG_HOME", base.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}");
    assert!(stdout.contains("1 passed"), "{stdout}");
    assert_eq!(fs::read_dir(base.path()).unwrap().count(), 0);
}
