use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use inotify::{EventMask, Inotify, WatchMask};

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const PROBE_INTERVAL: Duration = Duration::from_millis(50);

/// Spawn a thread that watches `path` for creation using inotify.
/// When the file appears (or already exists), sends `service_name` on `tx`.
pub fn watch_pidfile(path: &Path, tx: Sender<String>, service_name: String) -> JoinHandle<()> {
    let path = path.to_path_buf();
    thread::spawn(move || {
        if watch_once(&path, &tx, &service_name, "pidfile", false).is_err() {
            eprintln!("[ready] pidfile watcher for {} failed", service_name);
        }
    })
}

/// Spawn a thread that watches `path` for creation using inotify, then probes
/// the socket with a connection attempt to confirm it is accepting connections.
pub fn watch_socket(path: &Path, tx: Sender<String>, service_name: String) -> JoinHandle<()> {
    let path = path.to_path_buf();
    thread::spawn(move || {
        if watch_once(&path, &tx, &service_name, "socket", true).is_err() {
            eprintln!("[ready] socket watcher for {} failed", service_name);
        }
    })
}

/// Attempt to connect to a Unix domain socket until success or timeout.
/// Returns true if connected, false on timeout. Fail-open: callers should
/// log a warning and signal ready anyway when this returns false.
fn probe_unix_socket(path: &Path, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if UnixStream::connect(path).is_ok() {
            return true;
        }
        thread::sleep(PROBE_INTERVAL);
    }
    false
}

fn watch_once(
    path: &PathBuf,
    tx: &Sender<String>,
    service_name: &str,
    kind: &str,
    probe: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Fast path: file already exists.
    if path.exists() {
        maybe_probe(path, service_name, probe);
        eprintln!("[ready] {} ready ({}): {:?}", service_name, kind, path);
        tx.send(service_name.to_string())?;
        return Ok(());
    }

    let parent = path.parent().ok_or("pidfile path has no parent")?;
    let filename = path.file_name().ok_or("pidfile path has no filename")?;

    let mut inotify = Inotify::init()?;
    inotify.watches().add(
        parent,
        WatchMask::CREATE | WatchMask::MOVED_TO | WatchMask::CLOSE_WRITE,
    )?;

    // Double-check after adding the watch to avoid TOCTOU race.
    if path.exists() {
        maybe_probe(path, service_name, probe);
        eprintln!("[ready] {} ready ({}): {:?}", service_name, kind, path);
        tx.send(service_name.to_string())?;
        return Ok(());
    }

    let mut buffer = [0u8; 4096];
    loop {
        let events = inotify.read_events_blocking(&mut buffer)?;
        for event in events {
            if let Some(name) = event.name {
                if name == filename
                    && (event.mask.contains(EventMask::CREATE)
                        || event.mask.contains(EventMask::MOVED_TO)
                        || event.mask.contains(EventMask::CLOSE_WRITE))
                {
                    maybe_probe(path, service_name, probe);
                    eprintln!("[ready] {} ready ({}): {:?}", service_name, kind, path);
                    tx.send(service_name.to_string())?;
                    return Ok(());
                }
            }
        }
    }
}

fn maybe_probe(path: &Path, service_name: &str, probe: bool) {
    if !probe {
        return;
    }
    if probe_unix_socket(path, PROBE_TIMEOUT) {
        eprintln!("[ready] {} socket accepting connections: {:?}", service_name, path);
    } else {
        eprintln!(
            "[ready] warning: {} socket file exists but connection timed out ({:?}), signalling ready anyway",
            service_name, path
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn detects_pidfile_creation() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("test.pid");

        let (tx, rx) = mpsc::channel();
        let path = pidfile.clone();
        let _handle = watch_pidfile(&path, tx, "test-service".to_string());

        // Give the watcher a moment to set up inotify
        std::thread::sleep(Duration::from_millis(50));

        // Create the pidfile
        std::fs::write(&pidfile, "12345").unwrap();

        // Should receive the service name within 1 second
        let received = rx.recv_timeout(Duration::from_secs(1)).expect("no signal received");
        assert_eq!(received, "test-service");
    }

    #[test]
    fn already_existing_pidfile_is_detected_immediately() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("existing.pid");

        // Create the pidfile before starting the watcher
        std::fs::write(&pidfile, "99999").unwrap();

        let (tx, rx) = mpsc::channel();
        let _handle = watch_pidfile(&pidfile, tx, "existing-service".to_string());

        let received = rx.recv_timeout(Duration::from_secs(1)).expect("no signal for existing file");
        assert_eq!(received, "existing-service");
    }

    #[test]
    fn probe_unix_socket_returns_true_when_listener_bound() {
        use std::os::unix::net::UnixListener;

        let dir = tempfile::tempdir().unwrap();
        let sock_path = dir.path().join("test.sock");

        let _listener = UnixListener::bind(&sock_path).unwrap();
        assert!(probe_unix_socket(&sock_path, Duration::from_secs(1)));
    }

    #[test]
    fn probe_unix_socket_returns_false_when_nothing_listens() {
        let dir = tempfile::tempdir().unwrap();
        let sock_path = dir.path().join("ghost.sock");

        // Create the socket path entry but bind no listener.
        std::fs::write(&sock_path, "").unwrap();

        // Short timeout so the test doesn't take 5s.
        assert!(!probe_unix_socket(&sock_path, Duration::from_millis(150)));
    }

    #[test]
    fn watch_socket_fires_after_listener_is_bound() {
        use std::os::unix::net::UnixListener;

        let dir = tempfile::tempdir().unwrap();
        let sock_path = dir.path().join("svc.sock");

        let (tx, rx) = mpsc::channel();
        let _handle = watch_socket(&sock_path, tx, "svc".to_string());

        std::thread::sleep(Duration::from_millis(50));

        // Binding the listener creates the socket file and accepts the probe.
        let _listener = UnixListener::bind(&sock_path).unwrap();

        let received = rx.recv_timeout(Duration::from_secs(2)).expect("no signal received");
        assert_eq!(received, "svc");
    }
}
