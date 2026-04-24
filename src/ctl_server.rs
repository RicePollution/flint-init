use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::thread;

use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;

use crate::ctl_proto::{Request, Response, SharedState};

pub const CTL_SOCKET_PATH: &str = "/run/flint/ctl.sock";

/// Spawn the control socket server thread.
///
/// Binds to `CTL_SOCKET_PATH`, accepts connections in a loop, and handles
/// `status` and `stop` requests by reading/writing `shared`.
pub fn start(shared: SharedState) {
    thread::spawn(move || {
        if let Err(e) = serve(shared) {
            eprintln!("[ctl] server error: {}", e);
        }
    });
}

fn serve(shared: SharedState) -> std::io::Result<()> {
    let sock_path = Path::new(CTL_SOCKET_PATH);
    if let Some(parent) = sock_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Remove stale socket from a previous run.
    let _ = std::fs::remove_file(sock_path);

    let listener = UnixListener::bind(sock_path)?;
    eprintln!("[ctl] listening on {}", CTL_SOCKET_PATH);

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let shared = shared.clone();
                thread::spawn(move || handle_connection(stream, shared));
            }
            Err(e) => eprintln!("[ctl] accept error: {}", e),
        }
    }
    Ok(())
}

fn handle_connection(stream: std::os::unix::net::UnixStream, shared: SharedState) {
    let mut writer = match stream.try_clone() {
        Ok(w) => w,
        Err(e) => {
            eprintln!("[ctl] stream clone failed: {}", e);
            return;
        }
    };
    let reader = BufReader::new(stream);

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };

        let response = match serde_json::from_str::<Request>(&line) {
            Ok(Request::Status) => {
                let services = shared
                    .lock()
                    .map(|map| map.values().cloned().collect::<Vec<_>>())
                    .unwrap_or_default();
                Response::Status { services }
            }
            Ok(Request::Stop { service }) => {
                let pid = shared.lock().ok().and_then(|map| {
                    map.get(&service).and_then(|info| info.pid)
                });
                match pid {
                    Some(p) => {
                        let _ = kill(Pid::from_raw(p as i32), Signal::SIGTERM);
                        Response::Ok { ok: true }
                    }
                    None => Response::Error {
                        error: format!("service '{}' not found or not running", service),
                    },
                }
            }
            Err(e) => Response::Error {
                error: format!("invalid request: {}", e),
            },
        };

        let mut json = match serde_json::to_string(&response) {
            Ok(j) => j,
            Err(e) => format!("{{\"error\":\"serialization failed: {}\"}}", e),
        };
        json.push('\n');
        if writer.write_all(json.as_bytes()).is_err() {
            break;
        }
    }
}
