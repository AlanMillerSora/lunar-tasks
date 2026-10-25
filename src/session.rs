use std::fs;
use std::path::PathBuf;
use std::process::Command;

use crate::ipc;

fn newest_dir(best: &mut Option<(std::time::SystemTime, String)>, path: &PathBuf) {
    if let Ok(meta) = path.metadata() {
        let modified = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if best.as_ref().map(|(t, _)| modified > *t).unwrap_or(true) {
            *best = Some((modified, name));
        }
    }
}

pub fn hypr_signature() -> Option<String> {
    let dir = ipc::runtime_dir().join("hypr");
    let mut best: Option<(std::time::SystemTime, String)> = None;
    for entry in fs::read_dir(&dir).ok()?.flatten() {
        if entry.path().is_dir() {
            newest_dir(&mut best, &entry.path());
        }
    }
    best.map(|(_, n)| n)
}

pub fn wayland_display() -> Option<String> {
    let dir = ipc::runtime_dir();
    let mut best: Option<(std::time::SystemTime, String)> = None;
    for entry in fs::read_dir(&dir).ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with("wayland-") && !name.ends_with(".lock") {
            newest_dir(&mut best, &entry.path());
        }
    }
    best.map(|(_, n)| n)
}

pub fn x_display() -> Option<String> {
    for n in 0..4 {
        if std::path::Path::new(&format!("/tmp/.X11-unix/X{n}")).exists() {
            return Some(format!(":{n}"));
        }
    }
    None
}

pub fn apply(cmd: &mut Command) {
    let runtime = ipc::runtime_dir();
    if std::env::var("XDG_RUNTIME_DIR").is_err() {
        cmd.env("XDG_RUNTIME_DIR", &runtime);
    }
    if std::env::var("WAYLAND_DISPLAY")
        .unwrap_or_default()
        .is_empty()
    {
        if let Some(d) = wayland_display() {
            cmd.env("WAYLAND_DISPLAY", d);
        }
    }
    if std::env::var("HYPRLAND_INSTANCE_SIGNATURE")
        .unwrap_or_default()
        .is_empty()
    {
        if let Some(sig) = hypr_signature() {
            cmd.env("HYPRLAND_INSTANCE_SIGNATURE", sig);
        }
    }
    if std::env::var("DISPLAY").unwrap_or_default().is_empty() {
        if let Some(d) = x_display() {
            cmd.env("DISPLAY", d);
        }
    }
    if std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .is_empty()
    {
        cmd.env("XDG_CURRENT_DESKTOP", "Hyprland");
    }
}

pub fn command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    apply(&mut cmd);
    cmd
}

/// Called once at daemon startup, before any threads are created.
pub fn discover_process_env() {
    unsafe {
        if std::env::var("XDG_RUNTIME_DIR").is_err() {
            std::env::set_var("XDG_RUNTIME_DIR", ipc::runtime_dir());
        }
        if std::env::var("WAYLAND_DISPLAY")
            .unwrap_or_default()
            .is_empty()
        {
            if let Some(d) = wayland_display() {
                std::env::set_var("WAYLAND_DISPLAY", d);
            }
        }
        if std::env::var("HYPRLAND_INSTANCE_SIGNATURE")
            .unwrap_or_default()
            .is_empty()
        {
            if let Some(sig) = hypr_signature() {
                std::env::set_var("HYPRLAND_INSTANCE_SIGNATURE", sig);
            }
        }
        if std::env::var("DISPLAY").unwrap_or_default().is_empty() {
            if let Some(d) = x_display() {
                std::env::set_var("DISPLAY", d);
            }
        }
        if std::env::var("XDG_CURRENT_DESKTOP")
            .unwrap_or_default()
            .is_empty()
        {
            std::env::set_var("XDG_CURRENT_DESKTOP", "Hyprland");
        }
    }
}
