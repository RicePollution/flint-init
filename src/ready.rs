use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::thread::{self, JoinHandle};

use inotify::{EventMask, Inotify, WatchMask};

/// Spawn a thread that watches `path` for creation using inotify.
/// When the file appears (or already exists), sends `service_name` on `tx`.
pub fn watch_pidfile(path: &Path, tx: Sender<String>, service_name: String) -> JoinHandle<()> {
    let path = path.to_path_buf();
    thread::spawn(move || {
        if watch_once(&path, &tx, &service_name).is_err() {
            eprintln!("[ready] watcher for {} failed", service_name);
        }
    })
}

fn watch_once(
    path: &PathBuf,
    tx: &Sender<String>,
    service_name: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Fast path: file already exists.
    if path.exists() {
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
                    tx.send(service_name.to_string())?;
                    return Ok(());
                }
            }
        }
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
}
