use ksni::blocking::TrayMethods;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use crate::ipc;
use crate::model::{self, state_path};
use crate::session;
use crate::util::{due_label, now_ms, one_line};

static QUIT: AtomicBool = AtomicBool::new(false);
static SHOW_REQ: AtomicBool = AtomicBool::new(false);

extern "C" fn on_quit(_: libc::c_int) {
    QUIT.store(true, Ordering::SeqCst);
}
extern "C" fn on_show(_: libc::c_int) {
    SHOW_REQ.store(true, Ordering::SeqCst);
}

#[derive(Debug, Clone)]
enum Cmd {
    Toggle,
    Show,
    Hide,
    AddPrompt,
    Quit,
}

struct TasksTray {
    tx: Sender<Cmd>,
    menu_open: Mutex<bool>,
}

impl TasksTray {
    fn send(&self, cmd: Cmd) {
        let _ = self.tx.send(cmd);
    }
}

impl ksni::Tray for TasksTray {
    fn id(&self) -> String {
        "tasks-rs".into()
    }
    fn title(&self) -> String {
        "Tasks".into()
    }
    fn icon_name(&self) -> String {
        "tasks-rs".into()
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        vec![tray_icon(32)]
    }
    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "Tasks".into(),
            description: "Task manager — click to toggle".into(),
            ..Default::default()
        }
    }
    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(Cmd::Toggle);
    }
    fn secondary_activate(&mut self, _x: i32, _y: i32) {
        self.send(Cmd::Show);
    }
    fn menu_about_to_show(&mut self) {
        if let Ok(mut open) = self.menu_open.lock() {
            *open = true;
        }
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;
        let show = StandardItem {
            label: "Show".into(),
            activate: Box::new(|t: &mut Self| t.send(Cmd::Show)),
            ..Default::default()
        };
        let hide = StandardItem {
            label: "Hide".into(),
            activate: Box::new(|t: &mut Self| t.send(Cmd::Hide)),
            ..Default::default()
        };
        let add = StandardItem {
            label: "Add task…".into(),
            activate: Box::new(|t: &mut Self| t.send(Cmd::AddPrompt)),
            ..Default::default()
        };
        let quit = StandardItem {
            label: "Quit".into(),
            activate: Box::new(|t: &mut Self| t.send(Cmd::Quit)),
            ..Default::default()
        };
        vec![
            show.into(),
            hide.into(),
            add.into(),
            MenuItem::Separator,
            quit.into(),
        ]
    }
}

fn put(data: &mut [u8], w: usize, x: i32, y: i32, c: [u8; 4]) {
    if x < 0 || y < 0 || x as usize >= w {
        return;
    }
    let i = ((y as usize) * w + x as usize) * 4;
    if i + 3 < data.len() {
        data[i..i + 4].copy_from_slice(&c);
    }
}

fn tray_icon(n: i32) -> ksni::Icon {
    let w = n as usize;
    let mut data = vec![0u8; w * w * 4];
    let bg = [255u8, 0x12, 0x0d, 0x21];
    let accent = [255u8, 0xf9, 0x91, 0xc5];
    let accent2 = [255u8, 0xc1, 0x66, 0xd5];
    let r = 7.0f32;
    for y in 0..n {
        for x in 0..n {
            let fx = x as f32 + 0.5;
            let fy = y as f32 + 0.5;
            let cx = fx.clamp(r, n as f32 - r);
            let cy = fy.clamp(r, n as f32 - r);
            let d = ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt();
            if d <= r + 0.2 {
                let edge = d > r - 1.2;
                put(&mut data, w, x, y, if edge { accent } else { bg });
            }
        }
    }
    // A tiny branching graph: three nodes joined by two strokes.
    let line = |data: &mut [u8], x0: i32, y0: i32, x1: i32, y1: i32, c: [u8; 4]| {
        let steps = 30;
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let x = x0 as f32 + (x1 - x0) as f32 * t;
            let y = y0 as f32 + (y1 - y0) as f32 * t;
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    if dx.abs() + dy.abs() <= 1 {
                        put(data, w, x.round() as i32 + dx, y.round() as i32 + dy, c);
                    }
                }
            }
        }
    };
    line(&mut data, 10, 10, 10, 21, accent);
    line(&mut data, 10, 21, 22, 21, accent2);
    for (cx, cy, c) in [(10, 9, accent), (10, 22, accent), (22, 22, accent2)] {
        for dy in -3..=3 {
            for dx in -3..=3 {
                if dx * dx + dy * dy <= 9 {
                    put(&mut data, w, cx + dx, cy + dy, c);
                }
            }
        }
    }
    ksni::Icon {
        width: n,
        height: n,
        data,
    }
}

fn exe_path() -> PathBuf {
    let raw = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("tasks-rs"));
    let text = raw.to_string_lossy().to_string();
    let clean = text.strip_suffix(" (deleted)").unwrap_or(&text);
    let clean = PathBuf::from(clean);
    if clean.exists() {
        return clean;
    }
    let fallback = crate::model::home().join("Projects/tasks-rs/target/release/tasks-rs");
    if fallback.exists() {
        return fallback;
    }
    raw
}

fn spawn_gui(focus_add: bool) -> bool {
    let exe = exe_path();
    for _attempt in 0..2 {
        let mut cmd = session::command(&exe.to_string_lossy());
        cmd.arg("--gui")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        if cmd.spawn().is_err() {
            return false;
        }
        for _ in 0..40 {
            std::thread::sleep(Duration::from_millis(50));
            if ipc::gui_alive() {
                if focus_add {
                    for _ in 0..20 {
                        if ipc::send_gui("add-prompt").is_ok() {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(50));
                    }
                }
                return true;
            }
        }
        // A previous GUI can leave a socket behind while it is exiting. Clear
        // the dead endpoint and try once more so a click never silently fails.
        if !ipc::gui_alive() {
            ipc::remove_gui_socket();
        }
    }
    false
}
fn show_gui(focus_add: bool) {
    if ipc::send_gui(if focus_add { "add-prompt" } else { "show" }).is_ok() {
        return;
    }
    let _ = spawn_gui(focus_add);
}

fn hide_gui() {
    let _ = ipc::send_gui("hide");
}

fn toggle_gui() {
    if ipc::send_gui("toggle").is_ok() {
        return;
    }
    let _ = spawn_gui(false);
}

fn add_from_cli(text: &str) -> Result<(), String> {
    let text = one_line(text);
    if text.is_empty() {
        return Err("empty task title".into());
    }
    if ipc::send_gui(&format!("add {text}")).is_ok() {
        let _ = ipc::send_gui("show");
        return Ok(());
    }
    let path = state_path();
    let mut doc = model::load_doc(&path)?.unwrap_or_default();
    doc.items.insert(0, model::new_task(&text, ""));
    model::save_doc(&path, &doc)?;
    let _ = spawn_gui(false);
    Ok(())
}

pub fn tick_reminders() {
    let path = state_path();
    let Ok(Some(mut doc)) = model::load_doc(&path) else {
        return;
    };
    let now = now_ms();
    let mut fired: Vec<TaskReminder> = Vec::new();
    for t in &doc.items {
        if t.deleted_at == 0 && !t.done && t.remind_at > 0 && !t.notified && t.remind_at <= now {
            fired.push(TaskReminder {
                id: t.id.clone(),
                title: t.title.clone(),
                due: t.due,
            });
        }
    }
    if fired.is_empty() {
        return;
    }
    for r in &fired {
        let body = if r.due > 0 {
            format!("{} — {}", r.title, due_label(r.due))
        } else {
            r.title.clone()
        };
        crate::util::notify("Task due", &body);
    }
    for r in &fired {
        if let Some(t) = doc.items.iter_mut().find(|t| t.id == r.id) {
            t.notified = true;
        }
    }
    if model::save_doc(&path, &doc).is_ok() {
        let _ = ipc::send_gui("reload");
    }
}

struct TaskReminder {
    id: String,
    title: String,
    due: i64,
}

fn status_text(tray: bool) -> String {
    format!(
        "daemon=ok pid={}\ngui={}\ntray={}",
        std::process::id(),
        if ipc::gui_alive() {
            "running"
        } else {
            "stopped"
        },
        if tray { "active" } else { "unavailable" }
    )
}

fn handle_client(mut stream: UnixStream, tray_ok: bool) -> bool {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut line = String::new();
    let mut reader = match stream.try_clone() {
        Ok(s) => BufReader::new(s),
        Err(_) => return false,
    };
    if reader.read_line(&mut line).is_err() {
        return false;
    }
    let line = line.trim_end_matches(['\r', '\n']);
    let (cmd, rest) = match line.split_once(' ') {
        Some((c, r)) => (c, r),
        None => (line, ""),
    };
    let mut quit = false;
    let response = match cmd {
        "ping" => "ok".to_string(),
        "toggle" => {
            toggle_gui();
            "ok".to_string()
        }
        "show" => {
            show_gui(false);
            "ok".to_string()
        }
        "hide" => {
            hide_gui();
            "ok".to_string()
        }
        "add-prompt" => {
            show_gui(true);
            "ok".to_string()
        }
        "add" => match add_from_cli(rest) {
            Ok(()) => "ok".to_string(),
            Err(e) => format!("err {e}"),
        },
        "status" => status_text(tray_ok),
        "quit" => {
            let _ = ipc::send_gui("quit");
            quit = true;
            "ok".to_string()
        }
        _ => "err unknown command".to_string(),
    };
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.write_all(b"\n");
    quit
}

fn handle_tray_cmd(cmd: Cmd) -> bool {
    match cmd {
        Cmd::Toggle => toggle_gui(),
        Cmd::Show => show_gui(false),
        Cmd::Hide => hide_gui(),
        Cmd::AddPrompt => show_gui(true),
        Cmd::Quit => {
            let _ = ipc::send_gui("quit");
            return true;
        }
    }
    false
}

fn daemon_loop(listener: UnixListener, rx: Receiver<Cmd>, tray_ok: bool) {
    let mut last_reminder = Instant::now() - Duration::from_secs(20);
    let mut last_gui_check = Instant::now();
    let mut gui_was_alive = false;
    while !QUIT.load(Ordering::SeqCst) {
        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    if handle_client(stream, tray_ok) {
                        QUIT.store(true, Ordering::SeqCst);
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }
        if SHOW_REQ.swap(false, Ordering::SeqCst) {
            toggle_gui();
        }
        while let Ok(cmd) = rx.try_recv() {
            if handle_tray_cmd(cmd) {
                QUIT.store(true, Ordering::SeqCst);
            }
        }
        if last_reminder.elapsed() >= Duration::from_secs(10) {
            last_reminder = Instant::now();
            tick_reminders();
        }
        if last_gui_check.elapsed() >= Duration::from_secs(2) {
            last_gui_check = Instant::now();
            let alive = ipc::gui_alive();
            if gui_was_alive && !alive {
                // The GUI saves itself before exiting; nothing else to do here.
            }
            gui_was_alive = alive;
        }
        // Reap spawned GUI / notify-send children without touching SIGCHLD
        // (Rust's std::process relies on wait()).
        unsafe {
            let mut status = 0;
            while libc::waitpid(-1, &mut status, libc::WNOHANG) > 0 {}
        }
        std::thread::sleep(Duration::from_millis(70));
    }
}

pub fn run_daemon() -> i32 {
    if ipc::daemon_alive() {
        eprintln!("tasks-rs: daemon already running");
        return 0;
    }
    session::discover_process_env();
    unsafe {
        libc::signal(libc::SIGTERM, on_quit as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, on_quit as *const () as libc::sighandler_t);
        libc::signal(libc::SIGUSR1, on_show as *const () as libc::sighandler_t);
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }

    let path = ipc::daemon_socket();
    let _ = fs::remove_file(&path);
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("tasks-rs: cannot bind {}: {e}", path.display());
            return 1;
        }
    };
    let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    let _ = listener.set_nonblocking(true);

    let (tx, rx) = channel::<Cmd>();
    let tray = TasksTray {
        tx: tx.clone(),
        menu_open: Mutex::new(false),
    };
    let tray_handle = match tray.assume_sni_available(true).spawn() {
        Ok(h) => Some(h),
        Err(e) => {
            eprintln!("tasks-rs: tray unavailable: {e}");
            None
        }
    };
    let tray_ok = tray_handle.is_some();

    let started = std::time::Instant::now();
    daemon_loop(listener, rx, tray_ok);
    let _ = ipc::send_gui("quit");

    if let Some(h) = tray_handle {
        h.shutdown().wait();
    }
    let _ = fs::remove_file(&path);
    eprintln!("tasks-rs: daemon stopped after {:?}", started.elapsed());
    0
}

/// Used by the launcher/CLI: start a detached daemon if none is running.
pub fn ensure_daemon() -> bool {
    if ipc::daemon_alive() {
        return true;
    }
    // Prefer systemd so the daemon stays supervised. `restart` also copes
    // with a unit that is still winding down right after `tasks-rs quit`.
    for _ in 0..2 {
        let _ = session::command("systemctl")
            .args(["--user", "restart", "tasks-rs.service"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        for _ in 0..40 {
            std::thread::sleep(Duration::from_millis(50));
            if ipc::daemon_alive() {
                return true;
            }
        }
    }
    // Fallback for machines without the user unit installed.
    let exe = exe_path();
    let mut cmd = session::command(&exe.to_string_lossy());
    cmd.arg("--daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    if cmd.spawn().is_err() {
        return false;
    }
    for _ in 0..40 {
        std::thread::sleep(Duration::from_millis(50));
        if ipc::daemon_alive() {
            return true;
        }
    }
    false
}

/// Reminder fallback used by the systemd timer. It deliberately does nothing
/// while the daemon is running so notifications are never duplicated.
pub fn remind_fallback() -> i32 {
    if ipc::daemon_alive() {
        return 0;
    }
    tick_reminders();
    0
}
