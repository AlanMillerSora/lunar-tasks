use std::collections::VecDeque;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

pub fn runtime_dir() -> PathBuf {
    std::env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
}

pub fn daemon_socket() -> PathBuf {
    runtime_dir().join("tasks-rs.sock")
}

pub fn gui_socket() -> PathBuf {
    runtime_dir().join("tasks-rs-gui.sock")
}

pub fn send_to(path: &PathBuf, line: &str) -> std::io::Result<String> {
    let mut stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(Duration::from_millis(1500)))?;
    stream.set_write_timeout(Some(Duration::from_millis(1000)))?;
    stream.write_all(line.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    let mut reader = BufReader::new(stream);
    let mut out = String::new();
    let _ = reader.read_to_string(&mut out);
    Ok(out.trim_end().to_string())
}

pub fn daemon_alive() -> bool {
    send_to(&daemon_socket(), "ping")
        .map(|r| r == "ok")
        .unwrap_or(false)
}

pub fn gui_alive() -> bool {
    send_to(&gui_socket(), "ping")
        .map(|r| r == "ok")
        .unwrap_or(false)
}

pub fn send_daemon(line: &str) -> std::io::Result<String> {
    send_to(&daemon_socket(), line)
}

pub fn send_gui(line: &str) -> std::io::Result<String> {
    send_to(&gui_socket(), line)
}

pub fn remove_gui_socket() {
    let _ = fs::remove_file(gui_socket());
}

/// Bind the GUI's private socket. Returns None when another GUI already owns it
/// (in that case the existing window is told to show itself).
pub fn claim_gui_socket() -> Option<UnixListener> {
    let path = gui_socket();
    if path.exists() {
        if gui_alive() {
            let _ = send_gui("show");
            return None;
        }
        let _ = fs::remove_file(&path);
    }
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let listener = UnixListener::bind(&path).ok()?;
    let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    Some(listener)
}

static GUI_CMDS: OnceLock<Mutex<VecDeque<String>>> = OnceLock::new();

fn queue() -> &'static Mutex<VecDeque<String>> {
    GUI_CMDS.get_or_init(|| Mutex::new(VecDeque::new()))
}

pub fn take_gui_cmds() -> Vec<String> {
    let mut q = queue().lock().unwrap_or_else(|e| e.into_inner());
    q.drain(..).collect()
}

pub fn push_gui_cmd(cmd: &str) {
    queue()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push_back(cmd.to_string());
}

/// Start accepting GUI commands. `ctx.request_repaint` wakes a hidden window too.
pub fn start_gui_listener(listener: UnixListener, ctx: eframe::egui::Context) {
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let Ok(reader_stream) = stream.try_clone() else {
                continue;
            };
            let mut reader = BufReader::new(reader_stream);
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            let cmd = line.trim_end_matches(['\r', '\n']);
            if cmd == "ping" {
                let _ = stream.write_all(b"ok\n");
                continue;
            }
            push_gui_cmd(cmd);
            ctx.request_repaint();
            let _ = stream.write_all(b"ok\n");
        }
    });
}
