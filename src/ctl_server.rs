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
            Ok(Request::Start { service: _ })
            | Ok(Request::Restart { service: _ })
            | Ok(Request::Reload { service: _ }) => Response::Error {
                error: "not yet implemented".to_string(),
            },
            Err(e) => Response::Error {
                error: format!("invalid request: {}", e),
            },
        };

        let mut json = match serde_json::to_string(&response) {
            Ok(j) => j,
            Err(e) => format!("{{\"error\":\"serialization failed: {}\"}}", e),
        };
        json.push('\n');
        let _ = writer.write_all(json.as_bytes());
        // One request per connection — close after sending the response.
        break;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ctl_proto::{new_shared_state, ServiceInfo};
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    fn start_server_on(sock_path: &std::path::Path, shared: SharedState) {
        let sock_path = sock_path.to_path_buf();
        std::thread::spawn(move || {
            let _ = std::fs::remove_file(&sock_path);
            let listener = UnixListener::bind(&sock_path).unwrap();
            for stream in listener.incoming() {
                match stream {
                    Ok(s) => {
                        let shared = shared.clone();
                        std::thread::spawn(move || handle_connection(s, shared));
                    }
                    Err(_) => break,
                }
            }
        });
        // Give the thread a moment to bind.
        std::thread::sleep(Duration::from_millis(20));
    }

    fn send_request(sock_path: &std::path::Path, req: &str) -> String {
        let mut stream = UnixStream::connect(sock_path).unwrap();
        let mut payload = req.to_string();
        payload.push('\n');
        stream.write_all(payload.as_bytes()).unwrap();
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        line.trim_end().to_string()
    }

    #[test]
    fn status_returns_empty_when_no_services() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("ctl.sock");
        let shared = new_shared_state();
        start_server_on(&sock, shared);

        let resp = send_request(&sock, r#"{"cmd":"status"}"#);
        let v: serde_json::Value = serde_json::from_str(&resp).unwrap();
        assert_eq!(v, serde_json::json!({"services": []}));
    }

    #[test]
    fn status_returns_registered_services() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("ctl.sock");
        let shared = new_shared_state();
        shared.lock().unwrap().insert(
            "sshd".to_string(),
            ServiceInfo { name: "sshd".to_string(), state: "ready".to_string(), pid: Some(42) },
        );
        start_server_on(&sock, shared);

        let resp = send_request(&sock, r#"{"cmd":"status"}"#);
        let v: serde_json::Value = serde_json::from_str(&resp).unwrap();
        let services = v["services"].as_array().unwrap();
        assert_eq!(services.len(), 1);
        assert_eq!(services[0]["name"], "sshd");
        assert_eq!(services[0]["state"], "ready");
        assert_eq!(services[0]["pid"], 42);
    }

    #[test]
    fn stop_unknown_service_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("ctl.sock");
        let shared = new_shared_state();
        start_server_on(&sock, shared);

        let resp = send_request(&sock, r#"{"cmd":"stop","service":"ghost"}"#);
        let v: serde_json::Value = serde_json::from_str(&resp).unwrap();
        assert!(v["error"].as_str().unwrap().contains("ghost"));
    }

    #[test]
    fn stop_service_with_no_pid_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("ctl.sock");
        let shared = new_shared_state();
        shared.lock().unwrap().insert(
            "pending-svc".to_string(),
            ServiceInfo { name: "pending-svc".to_string(), state: "pending".to_string(), pid: None },
        );
        start_server_on(&sock, shared);

        let resp = send_request(&sock, r#"{"cmd":"stop","service":"pending-svc"}"#);
        let v: serde_json::Value = serde_json::from_str(&resp).unwrap();
        assert!(v["error"].is_string());
    }

    #[test]
    fn invalid_json_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("ctl.sock");
        let shared = new_shared_state();
        start_server_on(&sock, shared);

        let resp = send_request(&sock, "not json at all");
        let v: serde_json::Value = serde_json::from_str(&resp).unwrap();
        assert!(v["error"].as_str().unwrap().contains("invalid request"));
    }

    #[test]
    fn multiple_sequential_connections_are_each_handled() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("ctl.sock");
        let shared = new_shared_state();
        start_server_on(&sock, shared);

        for _ in 0..3 {
            let resp = send_request(&sock, r#"{"cmd":"status"}"#);
            let v: serde_json::Value = serde_json::from_str(&resp).unwrap();
            assert!(v["services"].is_array());
        }
    }
}
