use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process;

use serde_json::json;
use flint_init::catalog;

const CTL_SOCKET_PATH: &str = "/run/flint/ctl.sock";

fn usage() -> ! {
    eprintln!("Usage: flint-ctl <command> [args]");
    eprintln!("Commands:");
    eprintln!("  status               List all service states");
    eprintln!("  stop <service>       Send SIGTERM to a running service");
    eprintln!("  get --list           List services available in the catalog");
    eprintln!("  get <service>        Fetch and install a service from the catalog");
    process::exit(1);
}

fn cmd_get_list() {
    let distro = catalog::detect_distro();
    let cat = catalog::fetch_catalog().unwrap_or_else(|e| {
        eprintln!("flint-ctl: {}", e);
        process::exit(1);
    });
    let mut names: Vec<&String> = cat
        .keys()
        .filter(|name| match &cat[*name].distros {
            None => true,
            Some(distros) => distros.iter().any(|d| d == &distro),
        })
        .collect();
    names.sort();
    for name in names {
        println!("{:<24} {}", name, cat[name].description);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if let [cmd, flag] = args.as_slice() {
        if cmd == "get" && flag == "--list" {
            cmd_get_list();
            return;
        }
    }

    let request = match args.as_slice() {
        [cmd] if cmd == "status" => json!({"cmd": "status"}),
        [cmd, svc] if cmd == "stop" => json!({"cmd": "stop", "service": svc}),
        _ => usage(),
    };

    let mut stream = UnixStream::connect(CTL_SOCKET_PATH).unwrap_or_else(|e| {
        eprintln!("flint-ctl: cannot connect to {}: {}", CTL_SOCKET_PATH, e);
        process::exit(1);
    });

    let mut payload = request.to_string();
    payload.push('\n');
    stream.write_all(payload.as_bytes()).unwrap_or_else(|e| {
        eprintln!("flint-ctl: write error: {}", e);
        process::exit(1);
    });

    let reader = BufReader::new(stream);
    for line in reader.lines() {
        match line {
            Ok(l) => {
                // Pretty-print JSON if possible, otherwise raw.
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&l) {
                    println!("{}", serde_json::to_string_pretty(&v).unwrap_or(l));
                } else {
                    println!("{}", l);
                }
            }
            Err(e) => {
                eprintln!("flint-ctl: read error: {}", e);
                process::exit(1);
            }
        }
    }
}
