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

const SERVICES_DIR: &str = "/etc/flint/services";

fn prompt(msg: &str) -> bool {
    use std::io::Write;
    print!("{}", msg);
    std::io::stdout().flush().ok();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).ok();
    matches!(line.trim().to_lowercase().as_str(), "y" | "yes")
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

fn fetch_and_install(name: &str, distro: &str) {
    let services_dir = std::path::Path::new(SERVICES_DIR);
    let dest = services_dir.join(format!("{}.toml", name));

    if dest.exists() && !prompt(&format!("{}.toml already exists — overwrite? [y/N] ", name)) {
        return;
    }

    let toml_str = catalog::fetch_service_toml(distro, name).unwrap_or_else(|e| {
        eprintln!("flint-ctl: {}", e);
        process::exit(1);
    });

    let missing = catalog::missing_deps(&toml_str, services_dir).unwrap_or_else(|e| {
        eprintln!("flint-ctl: failed to parse deps for {}: {}", name, e);
        process::exit(1);
    });

    std::fs::create_dir_all(services_dir).unwrap_or_else(|e| {
        eprintln!("flint-ctl: cannot create {}: {}", SERVICES_DIR, e);
        process::exit(1);
    });
    std::fs::write(&dest, &toml_str).unwrap_or_else(|e| {
        eprintln!("flint-ctl: cannot write {}: {}", dest.display(), e);
        process::exit(1);
    });
    println!("installed: {}", dest.display());

    let mut skipped: Vec<String> = Vec::new();
    for dep in missing {
        if prompt(&format!("{} requires {} — fetch it too? [y/N] ", name, dep)) {
            fetch_and_install(&dep, distro);
        } else {
            skipped.push(dep);
        }
    }
    for dep in &skipped {
        eprintln!("warning: {} installed but {} is missing.", name, dep);
    }
}

fn cmd_get(name: &str) {
    let distro = catalog::detect_distro();
    let cat = catalog::fetch_catalog().unwrap_or_else(|e| {
        eprintln!("flint-ctl: {}", e);
        process::exit(1);
    });

    let entry = cat.get(name).unwrap_or_else(|| {
        eprintln!(
            "flint-ctl: \"{}\" not found in catalog. Run flint-ctl get --list to see available services.",
            name
        );
        process::exit(1);
    });

    if let Some(distros) = &entry.distros {
        if !distros.iter().any(|d| d == &distro) {
            eprintln!("flint-ctl: \"{}\" is not available for {}.", name, distro);
            process::exit(1);
        }
    }

    fetch_and_install(name, &distro);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if let [cmd, flag] = args.as_slice() {
        if cmd == "get" && flag == "--list" {
            cmd_get_list();
            return;
        }
    }

    if let [cmd, name] = args.as_slice() {
        if cmd == "get" {
            cmd_get(name);
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
