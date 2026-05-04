use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::thread;

use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;

use crate::ctl_proto::{Request, Response, SharedState};

pub const CTL_SOCKET_PATH: &str = "/run/flint/ctl.sock";

/// Bind the control socket and restrict it to owner read/write (0o600).
/// Creates parent directories and removes any stale socket first.
pub(crate) fn bind_socket(sock_path: &Path) -> std::io::Result<UnixListener> {
    if let Some(parent) = sock_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _ = std::fs::remove_file(sock_path);
    let listener = UnixListener::bind(sock_path)?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(sock_path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

/// Return the effective UID of the process on the other end of the socket.
fn get_peer_uid(stream: &std::os::unix::net::UnixStream) -> Option<u32> {
    use std::os::unix::io::AsRawFd;
    let mut cred = libc::ucred { pid: 0, uid: 0, gid: 0 };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let ret = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut _ as *mut libc::c_void,
            &mut len,
        )
    };
    if ret == 0 { Some(cred.uid) } else { None }
}

/// Spawn the control socket server thread.
///
/// Binds to `CTL_SOCKET_PATH`, accepts connections in a loop, and handles
/// `status`, `stop`, `start`, `restart`, and `reload` requests.
pub fn start(shared: SharedState, cmd_tx: crate::ctl_proto::CommandSender) {
    thread::spawn(move || {
        if let Err(e) = serve(shared, cmd_tx) {
            eprintln!("[ctl] server error: {}", e);
        }
    });
}

fn serve(shared: SharedState, cmd_tx: crate::ctl_proto::CommandSender) -> std::io::Result<()> {
    let listener = bind_socket(Path::new(CTL_SOCKET_PATH))?;
    eprintln!("[ctl] listening on {}", CTL_SOCKET_PATH);

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let shared = shared.clone();
                let cmd_tx = cmd_tx.clone();
                thread::spawn(move || handle_connection(stream, shared, cmd_tx, true));
            }
            Err(e) => eprintln!("[ctl] accept error: {}", e),
        }
    }
    Ok(())
}

fn handle_connection(
    stream: std::os::unix::net::UnixStream,
    shared: SharedState,
    cmd_tx: crate::ctl_proto::CommandSender,
    require_root: bool,
) {
    if require_root {
        match get_peer_uid(&stream) {
            Some(0) => {} // root — allowed
            Some(_) | None => {
                // Non-root or credential lookup failed — send error and close.
                if let Ok(mut w) = stream.try_clone() {
                    let mut json = r#"{"error":"permission denied: root access required"}"#.to_string();
                    json.push('\n');
                    let _ = w.write_all(json.as_bytes());
                }
                return;
            }
        }
    }

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
            Ok(Request::Start { service }) => {
                let (tx, rx) = std::sync::mpsc::sync_channel(1);
                if cmd_tx.send(crate::ctl_proto::CtlCommand::Start { service, reply: tx }).is_err() {
                    Response::Error { error: "executor disconnected".to_string() }
                } else {
                    match rx.recv() {
                        Ok(Ok(())) => Response::Ok { ok: true },
                        Ok(Err(e)) => Response::Error { error: e },
                        Err(_)     => Response::Error { error: "executor disconnected".to_string() },
                    }
                }
            }
            Ok(Request::Restart { service }) => {
                let (tx, rx) = std::sync::mpsc::sync_channel(1);
                if cmd_tx.send(crate::ctl_proto::CtlCommand::Restart { service, reply: tx }).is_err() {
                    Response::Error { error: "executor disconnected".to_string() }
                } else {
                    match rx.recv() {
                        Ok(Ok(())) => Response::Ok { ok: true },
                        Ok(Err(e)) => Response::Error { error: e },
                        Err(_)     => Response::Error { error: "executor disconnected".to_string() },
                    }
                }
            }
            Ok(Request::Reload { service }) => {
                let (tx, rx) = std::sync::mpsc::sync_channel(1);
                if cmd_tx.send(crate::ctl_proto::CtlCommand::Reload { service, reply: tx }).is_err() {
                    Response::Error { error: "executor disconnected".to_string() }
                } else {
                    match rx.recv() {
                        Ok(Ok(())) => Response::Ok { ok: true },
                        Ok(Err(e)) => Response::Error { error: e },
                        Err(_)     => Response::Error { error: "executor disconnected".to_string() },
                    }
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

    fn start_server_on(
        sock_path: &std::path::Path,
        shared: SharedState,
        cmd_tx: crate::ctl_proto::CommandSender,
    ) {
        let sock_path = sock_path.to_path_buf();
        std::thread::spawn(move || {
            let _ = std::fs::remove_file(&sock_path);
            let listener = UnixListener::bind(&sock_path).unwrap();
            for stream in listener.incoming() {
                match stream {
                    Ok(s) => {
                        let shared = shared.clone();
                        let cmd_tx = cmd_tx.clone();
                        std::thread::spawn(move || handle_connection(s, shared, cmd_tx, false));
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

    fn mock_executor(cmd_rx: crate::ctl_proto::CommandReceiver) {
        std::thread::spawn(move || {
            use crate::ctl_proto::CtlCommand;
            while let Ok(cmd) = cmd_rx.recv() {
                match cmd {
                    CtlCommand::Start   { reply, .. } => { let _ = reply.send(Ok(())); }
                    CtlCommand::Restart { reply, .. } => { let _ = reply.send(Ok(())); }
                    CtlCommand::Reload  { reply, .. } => { let _ = reply.send(Ok(())); }
                }
            }
        });
    }

    #[test]
    fn status_returns_empty_when_no_services() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("ctl.sock");
        let shared = new_shared_state();
        let (cmd_tx, _cmd_rx) = crate::ctl_proto::new_command_channel();
        start_server_on(&sock, shared, cmd_tx);

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
        let (cmd_tx, _cmd_rx) = crate::ctl_proto::new_command_channel();
        start_server_on(&sock, shared, cmd_tx);

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
        let (cmd_tx, _cmd_rx) = crate::ctl_proto::new_command_channel();
        start_server_on(&sock, shared, cmd_tx);

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
        let (cmd_tx, _cmd_rx) = crate::ctl_proto::new_command_channel();
        start_server_on(&sock, shared, cmd_tx);

        let resp = send_request(&sock, r#"{"cmd":"stop","service":"pending-svc"}"#);
        let v: serde_json::Value = serde_json::from_str(&resp).unwrap();
        assert!(v["error"].is_string());
    }

    #[test]
    fn invalid_json_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("ctl.sock");
        let shared = new_shared_state();
        let (cmd_tx, _cmd_rx) = crate::ctl_proto::new_command_channel();
        start_server_on(&sock, shared, cmd_tx);

        let resp = send_request(&sock, "not json at all");
        let v: serde_json::Value = serde_json::from_str(&resp).unwrap();
        assert!(v["error"].as_str().unwrap().contains("invalid request"));
    }

    #[test]
    fn multiple_sequential_connections_are_each_handled() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("ctl.sock");
        let shared = new_shared_state();
        let (cmd_tx, _cmd_rx) = crate::ctl_proto::new_command_channel();
        start_server_on(&sock, shared, cmd_tx);

        for _ in 0..3 {
            let resp = send_request(&sock, r#"{"cmd":"status"}"#);
            let v: serde_json::Value = serde_json::from_str(&resp).unwrap();
            assert!(v["services"].is_array());
        }
    }

    #[test]
    fn start_request_returns_ok() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("ctl.sock");
        let shared = new_shared_state();
        let (cmd_tx, cmd_rx) = crate::ctl_proto::new_command_channel();
        mock_executor(cmd_rx);
        start_server_on(&sock, shared, cmd_tx);

        let resp = send_request(&sock, r#"{"cmd":"start","service":"sshd"}"#);
        let v: serde_json::Value = serde_json::from_str(&resp).unwrap();
        assert_eq!(v["ok"], true);
    }

    #[test]
    fn restart_request_returns_ok() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("ctl.sock");
        let shared = new_shared_state();
        let (cmd_tx, cmd_rx) = crate::ctl_proto::new_command_channel();
        mock_executor(cmd_rx);
        start_server_on(&sock, shared, cmd_tx);

        let resp = send_request(&sock, r#"{"cmd":"restart","service":"sshd"}"#);
        let v: serde_json::Value = serde_json::from_str(&resp).unwrap();
        assert_eq!(v["ok"], true);
    }

    #[test]
    fn reload_request_returns_ok() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("ctl.sock");
        let shared = new_shared_state();
        let (cmd_tx, cmd_rx) = crate::ctl_proto::new_command_channel();
        mock_executor(cmd_rx);
        start_server_on(&sock, shared, cmd_tx);

        let resp = send_request(&sock, r#"{"cmd":"reload","service":"sshd"}"#);
        let v: serde_json::Value = serde_json::from_str(&resp).unwrap();
        assert_eq!(v["ok"], true);
    }

    #[test]
    fn socket_has_0600_permissions_after_bind() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("ctl.sock");
        let _listener = bind_socket(&sock).unwrap();
        let meta = std::fs::metadata(&sock).unwrap();
        assert_eq!(
            meta.permissions().mode() & 0o777,
            0o600,
            "socket should be owner-read/write only"
        );
    }

    fn start_server_with_auth(
        sock_path: &std::path::Path,
        shared: SharedState,
        cmd_tx: crate::ctl_proto::CommandSender,
    ) {
        let sock_path = sock_path.to_path_buf();
        std::thread::spawn(move || {
            let _ = std::fs::remove_file(&sock_path);
            let listener = UnixListener::bind(&sock_path).unwrap();
            for stream in listener.incoming() {
                match stream {
                    Ok(s) => {
                        let shared = shared.clone();
                        let cmd_tx = cmd_tx.clone();
                        // require_root: true
                        std::thread::spawn(move || handle_connection(s, shared, cmd_tx, true));
                    }
                    Err(_) => break,
                }
            }
        });
        std::thread::sleep(Duration::from_millis(20));
    }

    #[test]
    fn non_root_connection_is_rejected_when_auth_enabled() {
        // Tests run as a non-root user; the server requires uid == 0.
        // The connection succeeds at the socket level but is refused by the auth check.
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("auth.sock");
        let shared = new_shared_state();
        let (cmd_tx, _cmd_rx) = crate::ctl_proto::new_command_channel();
        start_server_with_auth(&sock, shared, cmd_tx);

        let resp = send_request(&sock, r#"{"cmd":"status"}"#);
        let v: serde_json::Value = serde_json::from_str(&resp).unwrap();
        assert!(
            v["error"].as_str().map(|e| e.contains("permission denied")).unwrap_or(false),
            "expected permission denied error, got: {}",
            resp
        );
    }

    #[test]
    fn start_request_propagates_executor_error() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("ctl.sock");
        let shared = new_shared_state();
        let (cmd_tx, cmd_rx) = crate::ctl_proto::new_command_channel();
        // Mock executor that always errors on Start
        std::thread::spawn(move || {
            use crate::ctl_proto::CtlCommand;
            while let Ok(cmd) = cmd_rx.recv() {
                match cmd {
                    CtlCommand::Start { reply, .. } => {
                        let _ = reply.send(Err("already running".to_string()));
                    }
                    _ => {}
                }
            }
        });
        start_server_on(&sock, shared, cmd_tx);

        let resp = send_request(&sock, r#"{"cmd":"start","service":"sshd"}"#);
        let v: serde_json::Value = serde_json::from_str(&resp).unwrap();
        assert!(v["error"].as_str().unwrap().contains("already running"));
    }
}
