mod app;
mod daemon;
mod hypr;
mod ipc;
mod model;
mod session;
mod theme;
mod util;

use std::process::exit;

fn usage() {
    println!(
        "Tasks — native task manager with a resident tray daemon

USAGE:
    tasks-rs [COMMAND]

COMMANDS:
    toggle          Show the window, hide it if already visible (default)
    show            Show the window (spawn the GUI if needed)
    hide            Hide the window
    add <text>      Add a task and show the window
    status          Daemon/GUI status
    quit            Quit the GUI and the daemon

OPTIONS:
    --daemon        Run the resident tray daemon (normally a systemd service)
    --gui           Run only the GUI window (one instance)
    --remind        Fire due reminders once (fallback for the systemd timer)
    --version       Print version
    -h, --help      This help

STATE:
    ~/.local/share/tasks-rs/state.json
    ~/.local/share/tasks-rs/settings.json
SOCKETS:
    $XDG_RUNTIME_DIR/tasks-rs.sock
    $XDG_RUNTIME_DIR/tasks-rs-gui.sock"
    );
}

fn cli(args: &[String]) -> i32 {
    let cmd = args.first().map(|s| s.as_str()).unwrap_or("toggle");
    match cmd {
        "toggle" | "show" | "hide" | "add" | "quit" => {
            if !daemon::ensure_daemon() {
                eprintln!("tasks-rs: could not start the daemon");
                return 1;
            }
            let line = match cmd {
                "add" => {
                    let text = args[1..].join(" ");
                    if text.trim().is_empty() {
                        eprintln!("usage: tasks-rs add <text>");
                        return 2;
                    }
                    format!("add {text}")
                }
                other => other.to_string(),
            };
            match ipc::send_daemon(&line) {
                Ok(resp) if resp.starts_with("err") => {
                    eprintln!("tasks-rs: {}", resp.trim_start_matches("err "));
                    1
                }
                Ok(_) => 0,
                Err(e) => {
                    eprintln!("tasks-rs: {e}");
                    1
                }
            }
        }
        "status" => {
            if ipc::daemon_alive() {
                match ipc::send_daemon("status") {
                    Ok(s) => {
                        println!("{s}");
                        0
                    }
                    Err(e) => {
                        eprintln!("tasks-rs: {e}");
                        1
                    }
                }
            } else {
                println!(
                    "daemon=stopped\ngui={}",
                    if ipc::gui_alive() {
                        "running"
                    } else {
                        "stopped"
                    }
                );
                0
            }
        }
        "--version" | "-V" => {
            println!("tasks-rs {}", env!("CARGO_PKG_VERSION"));
            0
        }
        "-h" | "--help" | "help" => {
            usage();
            0
        }
        other => {
            eprintln!("tasks-rs: unknown command '{other}'");
            usage();
            2
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--daemon") {
        exit(daemon::run_daemon());
    }
    if args.iter().any(|a| a == "--remind") {
        exit(daemon::remind_fallback());
    }
    if args.iter().any(|a| a == "--gui") {
        match app::run_gui(false) {
            Ok(()) => exit(0),
            Err(e) => {
                eprintln!("tasks-rs: GUI failed: {e}");
                exit(1);
            }
        }
    }
    if args.iter().any(|a| a == "--hidden") {
        match app::run_gui(true) {
            Ok(()) => exit(0),
            Err(e) => {
                eprintln!("tasks-rs: GUI failed: {e}");
                exit(1);
            }
        }
    }
    exit(cli(&args));
}
