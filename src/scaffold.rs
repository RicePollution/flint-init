use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const FOREGROUND_FLAGS: &[&str] = &[
    "--foreground",
    "--no-daemon",
    "--nodaemon",
    "--no-fork",
    "--nofork",
    "-f",
];

/// Run `binary --help` capturing stdout+stderr, with a 2-second timeout.
/// Returns None if the binary is not found, hangs, or errors.
fn run_help_with_timeout(binary: &str) -> Option<String> {
    let mut child = Command::new(binary)
        .arg("--help")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;

    let start = Instant::now();
    let timeout = Duration::from_secs(2);

    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }

    let mut stdout = String::new();
    let mut stderr_str = String::new();
    if let Some(mut s) = child.stdout.take() {
        let _ = s.read_to_string(&mut stdout);
    }
    if let Some(mut s) = child.stderr.take() {
        let _ = s.read_to_string(&mut stderr_str);
    }
    stdout.push_str(&stderr_str);
    Some(stdout)
}

/// Scan --help output for a foreground flag. Returns the first matching flag.
pub fn detect_foreground_flag(help_output: &str) -> Option<&'static str> {
    FOREGROUND_FLAGS.iter().copied().find(|&f| help_output.contains(f))
}

/// Scan --help output for readiness hints.
/// Returns a formatted [ready] block string, or None if nothing detected.
pub fn detect_readiness_block(name: &str, help_output: &str) -> Option<String> {
    let lower = help_output.to_lowercase();
    if lower.contains("socket") {
        Some(format!(
            "[ready]\nstrategy = \"socket\"\npath = \"/run/{}.sock\"   # verify path",
            name
        ))
    } else if lower.contains("pidfile") || lower.contains("pid-file") || lower.contains("pid file") {
        Some(format!(
            "[ready]\nstrategy = \"pidfile\"\npath = \"/run/{}/{}.pid\"   # verify path",
            name, name
        ))
    } else {
        None
    }
}

/// Generate and print a scaffold TOML for `name` at path `binary`.
pub fn print_scaffold(name: &str, binary: &str) {
    let help = run_help_with_timeout(binary);

    let exec_line = match &help {
        None => format!(
            "# exec = \"{}\"   # binary not found or foreground flag unknown — verify manually",
            binary
        ),
        Some(output) => match detect_foreground_flag(output) {
            Some(flag) => format!("exec = \"{} {}\"   # auto-detected", binary, flag),
            None => format!(
                "exec = \"{}\"   # foreground flag unknown — verify manually",
                binary
            ),
        },
    };

    let ready_block = match &help {
        None => "# [ready]  # could not probe binary — add manually".to_string(),
        Some(output) => match detect_readiness_block(name, output) {
            Some(block) => block,
            None => "# [ready]  # readiness strategy unknown — add manually".to_string(),
        },
    };

    print!(
        r#"[service]
name = "{name}"
{exec_line}
restart = "on-failure"

# [deps]
# needs = ["dbus"]

{ready_block}
"#
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_no_daemon_flag() {
        let help = "Usage: mysqld [OPTIONS]\n  --no-daemon   Run in foreground\n";
        assert_eq!(detect_foreground_flag(help), Some("--no-daemon"));
    }

    #[test]
    fn detects_nofork_flag() {
        let help = "  --nofork   Do not fork into background\n";
        assert_eq!(detect_foreground_flag(help), Some("--nofork"));
    }

    #[test]
    fn returns_none_when_no_flag_found() {
        let help = "Usage: myapp [OPTIONS]\n  --verbose   Be verbose\n";
        assert_eq!(detect_foreground_flag(help), None);
    }

    #[test]
    fn detects_socket_readiness() {
        let help = "  --socket /run/foo.sock   Unix socket path\n";
        let block = detect_readiness_block("foo", help).unwrap();
        assert!(block.contains("socket"));
        assert!(block.contains("/run/foo.sock"));
    }

    #[test]
    fn detects_pidfile_readiness() {
        let help = "  --pidfile /run/foo.pid   Write PID to file\n";
        let block = detect_readiness_block("foo", help).unwrap();
        assert!(block.contains("pidfile"));
        assert!(block.contains("/run/foo/foo.pid"));
    }

    #[test]
    fn socket_takes_priority_over_pidfile() {
        let help = "  --socket /run/foo.sock\n  --pidfile /run/foo.pid\n";
        let block = detect_readiness_block("foo", help).unwrap();
        assert!(block.contains("strategy = \"socket\""));
    }

    #[test]
    fn returns_none_when_no_readiness_hint() {
        let help = "Usage: myapp\n  --verbose\n";
        assert!(detect_readiness_block("myapp", help).is_none());
    }

    #[test]
    fn foreground_flags_checked_in_priority_order() {
        // Both --foreground and --no-daemon present: --foreground wins
        let help = "  --no-daemon\n  --foreground\n";
        assert_eq!(detect_foreground_flag(help), Some("--foreground"));
    }
}
